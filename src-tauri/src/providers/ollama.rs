use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;

use super::{ChatRequest, ChunkStream, Provider, ProviderError};

pub struct Ollama {
    base_url: String,
    client: reqwest::Client,
}

impl Ollama {
    pub fn new(base_url: Option<&str>) -> Self {
        Self {
            base_url: base_url
                .unwrap_or("http://localhost:11434")
                .trim_end_matches('/')
                .to_string(),
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Deserialize)]
struct OllamaChunk {
    message: Option<Message>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize)]
struct Message { content: String }

#[derive(Default)]
struct NdjsonDecoder {
    buffer: Vec<u8>,
}

impl NdjsonDecoder {
    fn push(&mut self, bytes: &[u8]) -> Vec<Result<String, ProviderError>> {
        self.buffer.extend_from_slice(bytes);
        let mut output = Vec::new();
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            if line.iter().all(|byte| byte.is_ascii_whitespace()) { continue; }
            match serde_json::from_slice::<OllamaChunk>(&line) {
                Ok(chunk) => {
                    if let Some(error) = chunk.error {
                        output.push(Err(ProviderError::Upstream(error)));
                    } else if let Some(message) = chunk.message {
                        if !message.content.is_empty() { output.push(Ok(message.content)); }
                    }
                }
                Err(error) => output.push(Err(ProviderError::Decode(error.to_string()))),
            }
        }
        if self.buffer.len() > 256_000 {
            self.buffer.clear();
            output.push(Err(ProviderError::Decode("Ollama returned an oversized stream record.".into())));
        }
        output
    }
}

#[async_trait]
impl Provider for Ollama {
    fn name(&self) -> &'static str { "ollama" }

    async fn ping(&self) -> Result<(), ProviderError> {
        let url = format!("{}/api/tags", self.base_url);
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| ProviderError::Http(format!("ollama not running: {e}")))?;
        if !resp.status().is_success() {
            return Err(ProviderError::Upstream(format!(
                "ollama returned {}",
                resp.status()
            )));
        }
        Ok(())
    }

    async fn stream_chat(&self, req: ChatRequest) -> Result<ChunkStream, ProviderError> {
        let mut body = json!({
            "model": req.model,
            "stream": true,
            "think": false,
            "keep_alive": "15m",
            "options": { "temperature": req.temperature, "num_predict": req.max_tokens },
            "messages": [
                { "role": "system", "content": req.system },
                { "role": "user", "content": req.user },
            ],
        });
        if req.system.contains("JSON") {
            body["format"] = json!("json");
        }

        let resp = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let s = resp.status();
            let t = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Upstream(format!("{s}: {t}")));
        }

        let mut decoder = NdjsonDecoder::default();
        let stream = resp.bytes_stream()
            .chain(futures_util::stream::once(async { Ok("\n".into()) }))
            .flat_map(move |chunk| {
            let items: Vec<Result<String, ProviderError>> = match chunk {
                Ok(bytes) => {
                    decoder.push(&bytes)
                }
                Err(e) => vec![Err(ProviderError::Http(e.to_string()))],
            };
            futures_util::stream::iter(items)
        });

        Ok(Box::pin(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_chunks_cannot_corrupt_multibyte_text_or_drop_final_content() {
        let response = "{\"message\":{\"content\":\"\u{65e5}\u{672c}\u{1f642}\"},\"done\":false}\n{\"message\":{\"content\":\"end\"},\"done\":true}";
        for size in 1..response.len() {
            let mut decoder = NdjsonDecoder::default();
            let mut text = String::new();
            for chunk in response.as_bytes().chunks(size) {
                for result in decoder.push(chunk) { text.push_str(&result.unwrap()); }
            }
            for result in decoder.push(b"\n") { text.push_str(&result.unwrap()); }
            assert_eq!(text, "\u{65e5}\u{672c}\u{1f642}end", "chunk size {size}");
        }
    }

    #[test]
    fn upstream_error_is_not_silently_treated_as_empty_output() {
        let mut decoder = NdjsonDecoder::default();
        let output = decoder.push(b"{\"error\":\"model unavailable\"}\n");
        assert!(matches!(&output[0], Err(ProviderError::Upstream(message)) if message == "model unavailable"));
    }
}
