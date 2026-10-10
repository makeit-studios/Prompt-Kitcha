use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;

use super::{ChatRequest, ChunkStream, Provider, ProviderError};

pub struct OpenAi {
    provider: &'static str,
    api_key: Option<String>,
    base_url: String,
    client: reqwest::Client,
}

impl OpenAi {
    pub fn new(api_key: Option<String>, base_url: Option<&str>) -> Self {
        Self::compatible("openai", api_key, base_url, "https://api.openai.com/v1")
    }

    pub fn compatible(provider: &'static str, api_key: Option<String>, base_url: Option<&str>, default_base_url: &str) -> Self {
        Self {
            provider,
            api_key,
            base_url: base_url
                .map(str::trim)
                .filter(|url| !url.is_empty())
                .unwrap_or(default_base_url)
                .trim_end_matches('/')
                .to_string(),
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Deserialize)]
struct Delta { content: Option<String> }
#[derive(Deserialize)]
struct Choice {
    delta: Delta,
    #[serde(default)]
    finish_reason: Option<String>,
}
#[derive(Deserialize)]
struct Chunk {
    #[serde(default)]
    choices: Vec<Choice>,
    error: Option<serde_json::Value>,
}

fn decode_chunk(data: &str) -> Result<Option<String>, ProviderError> {
    if data.trim().is_empty() { return Ok(None); }
    let chunk: Chunk = serde_json::from_str(data).map_err(|e| ProviderError::Decode(e.to_string()))?;
    if chunk.error.is_some() {
        return Err(ProviderError::Upstream("The provider returned a streaming error. Check the selected model, quota, and API key.".into()));
    }
    match chunk.choices.into_iter().next() {
        Some(choice) => {
            if matches!(choice.finish_reason.as_deref(), Some("length" | "content_filter")) {
                return Err(ProviderError::Upstream("The provider truncated or blocked the response. Try a different model or a shorter prompt; the partial result cannot be accepted.".into()));
            }
            Ok(choice.delta.content)
        }
        None => Ok(None),
    }
}

impl OpenAi {
    fn request_body(&self, req: ChatRequest) -> serde_json::Value {
        let mut body = json!({
            "model": req.model,
            "stream": true,
            "temperature": req.temperature,
            "messages": [
                { "role": "system", "content": req.system },
                { "role": "user", "content": req.user },
            ],
        });
        let token_field = if self.provider == "groq" { "max_completion_tokens" } else { "max_tokens" };
        body[token_field] = json!(req.max_tokens);
        if let Some(schema) = req.response_schema {
            if self.provider == "gemini" {
                body["response_format"] = json!({ "type": "json_schema", "json_schema": { "name": "promptkitcha", "strict": true, "schema": schema } });
            } else if self.provider == "groq" {
                body["response_format"] = json!({ "type": "json_object" });
            }
        }
        if self.provider == "gemini" && req.model.starts_with("gemini-3") {
            body["reasoning_effort"] = json!("minimal");
        } else if self.provider == "groq" && req.model.starts_with("openai/gpt-oss-") {
            body["reasoning_effort"] = json!("low");
        }
        body
    }
}

#[async_trait]
impl Provider for OpenAi {
    fn name(&self) -> &'static str { self.provider }

    async fn ping(&self) -> Result<(), ProviderError> {
        if self.api_key.as_ref().is_none_or(|key| key.trim().is_empty()) {
            return Err(ProviderError::MissingKey(self.provider.into()));
        }
        Ok(())
    }

    async fn stream_chat(&self, req: ChatRequest) -> Result<ChunkStream, ProviderError> {
        let key = self
            .api_key
            .as_ref()
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(|| ProviderError::MissingKey(self.provider.into()))?;

        let body = self.request_body(req);

        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(key)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let s = resp.status();
            let t = resp.text().await.unwrap_or_default().replace(key, "[redacted]");
            return Err(ProviderError::Upstream(format!("{s}: {t}")));
        }

        let stream = resp.bytes_stream().eventsource()
            .take_while(|r| futures_util::future::ready(!matches!(r, Ok(ev) if ev.data.trim() == "[DONE]")))
            .filter_map(|r| async move {
            match r {
                Ok(ev) => {
                    match decode_chunk(&ev.data) {
                        Ok(text) => text.map(Ok),
                        Err(e) => Some(Err(e)),
                    }
                }
                Err(e) => Some(Err(ProviderError::Http(e.to_string()))),
            }
        });

        Ok(Box::pin(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn request(model: &str) -> ChatRequest {
        ChatRequest { model: model.into(), system: "Return JSON only".into(), user: "  First line\r\n日本語 🙂\nFINAL_SENTINEL".into(), temperature: 0.15, max_tokens: 4096, response_schema: Some(crate::prompt_builder::enhancement_response_schema()) }
    }

    fn provider(name: &'static str, key: Option<&str>, base: Option<&str>) -> OpenAi {
        let default = match name {
            "groq" => "https://api.groq.com/openai/v1",
            "gemini" => "https://generativelanguage.googleapis.com/v1beta/openai",
            _ => "https://api.openai.com/v1",
        };
        OpenAi::compatible(name, key.map(str::to_owned), base, default)
    }

    fn server(status: &str, body: &str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut received = Vec::new();
            let mut chunk = [0; 4096];
            loop {
                let n = socket.read(&mut chunk).unwrap();
                assert_ne!(n, 0);
                received.extend_from_slice(&chunk[..n]);
                if let Some(end) = received.windows(4).position(|s| s == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&received[..end]).to_lowercase();
                    let length: usize = headers.lines().find_map(|line| line.strip_prefix("content-length:")).unwrap().trim().parse().unwrap();
                    if received.len() >= end + 4 + length { break; }
                }
            }
            for chunk in response.as_bytes().chunks(7) { socket.write_all(chunk).unwrap(); }
            String::from_utf8(received).unwrap()
        });
        (base, handle)
    }

    #[test]
    fn compatible_endpoints_names_and_overrides_are_distinct() {
        for name in ["groq", "gemini"] {
            let p = provider(name, None, None);
            assert_eq!(p.name(), name);
            assert!(p.base_url.contains(if name == "groq" { "api.groq.com" } else { "generativelanguage.googleapis.com" }));
            assert_eq!(provider(name, None, Some("  ")).base_url, p.base_url);
            assert_eq!(provider(name, None, Some(" http://localhost:1234/v1/ ")).base_url, "http://localhost:1234/v1");
        }
        assert_eq!(OpenAi::new(None, None).name(), "openai");
    }

    #[test]
    fn request_bodies_preserve_source_and_use_provider_specific_budgets() {
        for (name, model, field, effort) in [
            ("groq", "openai/gpt-oss-20b", "max_completion_tokens", "low"),
            ("gemini", "gemini-3.5-flash-lite", "max_tokens", "minimal"),
        ] {
            let req = request(model);
            let original = req.user.clone();
            let body = provider(name, Some("mock-key"), None).request_body(req);
            assert_eq!(body["model"], model);
            assert_eq!(body[field], 4096);
            assert_eq!(body["reasoning_effort"], effort);
            assert_eq!(body["response_format"]["type"], if name == "gemini" { "json_schema" } else { "json_object" });
            if name == "gemini" {
                assert_eq!(body["response_format"]["json_schema"]["schema"], crate::prompt_builder::enhancement_response_schema());
            }
            assert_eq!(body["messages"][1]["content"], original);
            assert_eq!(body["messages"][0]["role"], "system");
            assert_eq!(body["stream"], true);
            assert!(body.get(if field == "max_tokens" { "max_completion_tokens" } else { "max_tokens" }).is_none());
        }
        assert!(provider("gemini", None, None).request_body(request("gemini-2.5-flash")).get("reasoning_effort").is_none());
        assert!(provider("groq", None, None).request_body(request("custom-model")).get("reasoning_effort").is_none());
        assert!(OpenAi::new(None, None).request_body(request("custom-model")).get("reasoning_effort").is_none());
        assert!(OpenAi::new(None, None).request_body(request("custom-model")).get("response_format").is_none());
        for name in ["groq", "gemini"] {
            let mut ping = request("test-model");
            ping.response_schema = None;
            assert!(provider(name, None, None).request_body(ping).get("response_format").is_none());
        }
    }

    #[tokio::test]
    async fn missing_and_blank_keys_report_the_correct_provider() {
        for name in ["groq", "gemini"] {
            for key in [None, Some(" \n")] {
                let p = provider(name, key, None);
                assert!(matches!(p.ping().await, Err(ProviderError::MissingKey(id)) if id == name));
                assert!(matches!(p.stream_chat(request("test-model")).await, Err(ProviderError::MissingKey(id)) if id == name));
            }
        }
    }

    #[tokio::test]
    async fn both_compatible_providers_stream_full_unicode_and_authenticate_the_request() {
        let output = "日本語 🙂\nFINAL_SENTINEL";
        let data = json!({"choices": [{"delta": {"content": output}, "finish_reason": null}]}).to_string();
        let body = format!("data: {{\"choices\":[{{\"delta\":{{\"role\":\"assistant\"}}}}]}}\n\ndata: {data}\n\ndata: {{\"choices\":[]}}\n\ndata: [DONE]\n\ndata: invalid-after-done\n\n");
        for (name, model) in [("groq", "openai/gpt-oss-20b"), ("gemini", "gemini-3.5-flash-lite")] {
            let (base, handle) = server("200 OK", &body);
            let mut stream = provider(name, Some("mock-key"), Some(&base)).stream_chat(request(model)).await.unwrap();
            let mut actual = String::new();
            while let Some(chunk) = stream.next().await { actual.push_str(&chunk.unwrap()); }
            assert_eq!(actual, output);
            let received = handle.join().unwrap();
            assert!(received.starts_with("POST /v1/chat/completions HTTP/1.1"));
            assert!(received.to_lowercase().contains("authorization: bearer mock-key\r\n"));
            let body: serde_json::Value = serde_json::from_str(received.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(body["model"], model);
            assert_eq!(body["messages"][1]["content"], request(model).user);
        }
    }

    #[tokio::test]
    async fn unauthorized_and_quota_errors_are_visible_without_echoing_the_key() {
        for name in ["groq", "gemini"] {
            for status in ["401 Unauthorized", "429 Too Many Requests"] {
                let (base, handle) = server(status, "Quota or authentication error: mock-key");
                let error = match provider(name, Some("mock-key"), Some(&base)).stream_chat(request("test-model")).await {
                    Ok(_) => panic!("Expected an upstream error"),
                    Err(error) => error.to_string(),
                };
                assert!(error.contains(&status[..3]));
                assert!(!error.contains("mock-key"));
                assert!(error.contains("[redacted]"));
                handle.join().unwrap();
            }
        }
    }

    #[tokio::test]
    async fn sse_decoder_handles_single_byte_packets_and_unicode() {
        let text = "日本語 🙂";
        let event = format!("data: {}\n\n", json!({"choices": [{"delta": {"content": text}}]}));
        let packets: Vec<_> = event.as_bytes().iter().map(|byte| Ok::<Vec<u8>, std::io::Error>(vec![*byte])).collect();
        let mut events = futures_util::stream::iter(packets).eventsource();
        assert_eq!(decode_chunk(&events.next().await.unwrap().unwrap().data).unwrap().as_deref(), Some(text));
        assert!(events.next().await.is_none());
    }

    #[test]
    fn malformed_failed_and_truncated_events_are_not_accepted() {
        assert!(matches!(decode_chunk("not JSON"), Err(ProviderError::Decode(_))));
        assert!(matches!(decode_chunk(r#"{"error":{"message":"mock-key"}}"#), Err(ProviderError::Upstream(_))));
        for reason in ["length", "content_filter"] {
            let data = json!({"choices": [{"delta": {"content": "Partial"}, "finish_reason": reason}]}).to_string();
            assert!(matches!(decode_chunk(&data), Err(ProviderError::Upstream(_))));
        }
        assert_eq!(decode_chunk(r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#).unwrap(), None);
        assert_eq!(decode_chunk(r#"{"choices":[],"usage":{"total_tokens":10}}"#).unwrap(), None);
    }

    #[tokio::test]
    #[ignore]
    async fn live_gemini_multiline_enhancement_pipeline() {
        let key_path = std::env::var("PROMPTKITCHA_GEMINI_KEY_FILE").expect("Set the key file path explicitly before running this live test");
        let key_file = std::fs::read_to_string(key_path).expect("Could not read the Gemini key file");
        let keys: std::collections::HashSet<_> = key_file.lines().map(str::trim).filter(|line| {
            line.len() >= 30 && (line.starts_with("AIza") || line.starts_with("AQ."))
                && line.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        }).collect();
        assert_eq!(keys.len(), 1, "The supplied file must contain exactly one distinct Gemini key");
        let key = *keys.iter().next().unwrap();
        let model = std::env::var("PROMPTKITCHA_GEMINI_TEST_MODEL").unwrap_or_else(|_| "gemini-3.5-flash-lite".into());
        let code: String = (1..=96).map(|i| format!("print(\"ITEM_{i:03}\")\n")).collect();
        let source = format!("Review this Python script; do not modify files. Preserve every code line unchanged and in the original order in the enhanced prompt.\nPath C:\\data\\example.py; URL https://example.org/a?x=2; Unicode 日本語 🙂.\n```python\n{code}```");
        assert_eq!(source.lines().count(), 100);
        let chat = ChatRequest {
            model: model.clone(),
            system: crate::prompt_builder::AI_SYSTEM.into(),
            user: serde_json::json!({"request": source, "feedback": null, "previous_draft": null}).to_string(),
            temperature: 0.1,
            max_tokens: (source.len() / 2 + 600).clamp(1024, 8192) as u32,
            response_schema: Some(crate::prompt_builder::enhancement_response_schema()),
        };
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(std::time::Duration::from_secs(120), async {
            let mut stream = provider("gemini", Some(&key), None).stream_chat(chat).await.map_err(|e| e.to_string())?;
            let mut parser = crate::optimizer::StreamingJsonExtractor::new();
            let mut raw = String::new();
            let mut streamed = String::new();
            let mut chunks = 0;
            while let Some(delta) = stream.next().await {
                let delta = delta.map_err(|e| e.to_string())?;
                chunks += 1;
                raw.push_str(&delta);
                if raw.len() > 256_000 { return Err("The AI returned too much output.".into()); }
                for event in parser.push(&delta) {
                    if let crate::optimizer::ExtractorEvent::PromptDelta(delta) = event { streamed.push_str(&delta); }
                }
            }
            let (intent, prompt) = parser.finalize(&raw).map_err(|error| format!("{error} Output characters={}; starts_with_fence={}; json_error={:?}", raw.chars().count(), raw.trim().starts_with("```"), serde_json::from_str::<serde_json::Value>(&raw).err()))?;
            crate::prompt_builder::validate_ai_output(&prompt)?;
            if streamed != prompt { return Err("Streaming and final enhanced text differ.".into()); }
            let prompt = crate::prompt_builder::preserve_source_context(&prompt, &source, None)?;
            if !prompt.contains(&source) { return Err("The final prompt did not preserve the entire original source verbatim.".into()); }
            let mut code_offset = 0;
            for line in code.lines() {
                let Some(position) = prompt[code_offset..].find(line) else {
                    return Err("A code line was changed, omitted, or reordered in the enhanced prompt.".into());
                };
                code_offset += position + line.len();
            }
            for literal in ["C:\\data\\example.py", "https://example.org/a?x=2", "日本語", "🙂"] {
                if !prompt.contains(literal) { return Err("A required literal was missing from the enhanced prompt.".into()); }
            }
            if !prompt.to_lowercase().contains("do not modify") { return Err("The review-only constraint was lost.".into()); }
            println!("Live Gemini PASS: model={model}; input_lines=100; code_lines_preserved_in_order=96; input_chars={}; output_chars={}; streamed_chunks={chunks}; intent={intent}; elapsed_ms={}", source.chars().count(), prompt.chars().count(), started.elapsed().as_millis());
            Ok::<_, String>(())
        }).await.expect("Live Gemini enhancement timed out after 120 seconds");
        if let Err(error) = result { panic!("Live Gemini enhancement failed: {error}"); }
    }
}
