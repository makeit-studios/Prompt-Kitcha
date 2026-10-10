pub mod anthropic;
pub mod ollama;
pub mod openai;
pub mod openrouter;

use std::pin::Pin;

use futures_util::Stream;
use serde::{Deserialize, Serialize};

use crate::credentials::get_key_internal;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("missing api key for provider: {0}")]
    MissingKey(String),
    #[error("unknown provider: {0}")]
    Unknown(String),
    #[error("http error: {0}")]
    Http(String),
    #[error("decode error: {0}")]
    Decode(String),
    #[error("provider error: {0}")]
    Upstream(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub system: String,
    pub user: String,
    pub temperature: f32,
    pub max_tokens: u32,
    pub response_schema: Option<serde_json::Value>,
}

pub type ChunkStream =
    Pin<Box<dyn Stream<Item = Result<String, ProviderError>> + Send + 'static>>;

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &'static str;
    async fn stream_chat(&self, req: ChatRequest) -> Result<ChunkStream, ProviderError>;
    async fn ping(&self) -> Result<(), ProviderError>;
}

pub fn build(name: &str, base_url: Option<&str>) -> Result<Box<dyn Provider>, ProviderError> {
    let normalized_base_url = base_url
        .map(str::trim)
        .filter(|url| !url.is_empty());

    match name {
        "openai" => Ok(Box::new(openai::OpenAi::new(
            get_key_internal("openai"),
            normalized_base_url,
        ))),
        "anthropic" => Ok(Box::new(anthropic::Anthropic::new(
            get_key_internal("anthropic"),
            normalized_base_url,
        ))),
        "openrouter" => Ok(Box::new(openrouter::OpenRouter::new(
            get_key_internal("openrouter"),
            normalized_base_url,
        ))),
        "groq" => Ok(Box::new(openai::OpenAi::compatible(
            "groq", get_key_internal("groq"), normalized_base_url,
            "https://api.groq.com/openai/v1",
        ))),
        "gemini" => Ok(Box::new(openai::OpenAi::compatible(
            "gemini", get_key_internal("gemini"), normalized_base_url,
            "https://generativelanguage.googleapis.com/v1beta/openai",
        ))),
        "ollama" => Ok(Box::new(ollama::Ollama::new(normalized_base_url))),
        other => Err(ProviderError::Unknown(other.into())),
    }
}

#[derive(Serialize)]
pub struct ProviderInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub needs_key: bool,
    pub default_model: &'static str,
    pub key_help_url: &'static str,
}

#[tauri::command]
pub fn list_providers() -> Vec<ProviderInfo> {
    vec![
        ProviderInfo {
            id: "openai",
            label: "OpenAI",
            needs_key: true,
            default_model: "gpt-4o-mini",
            key_help_url: "https://platform.openai.com/api-keys",
        },
        ProviderInfo {
            id: "anthropic",
            label: "Anthropic",
            needs_key: true,
            default_model: "claude-haiku-4-5",
            key_help_url: "https://console.anthropic.com/settings/keys",
        },
        ProviderInfo {
            id: "openrouter",
            label: "OpenRouter",
            needs_key: true,
            default_model: "openai/gpt-4o-mini",
            key_help_url: "https://openrouter.ai/keys",
        },
        ProviderInfo {
            id: "groq",
            label: "Groq",
            needs_key: true,
            default_model: "openai/gpt-oss-20b",
            key_help_url: "https://console.groq.com/keys",
        },
        ProviderInfo {
            id: "gemini",
            label: "Gemini (Google AI Studio)",
            needs_key: true,
            default_model: "gemini-3.5-flash-lite",
            key_help_url: "https://aistudio.google.com/apikey",
        },
        ProviderInfo {
            id: "ollama",
            label: "Ollama (local)",
            needs_key: false,
            default_model: "llama3.1",
            key_help_url: "https://ollama.com/download",
        },
    ]
}

#[tauri::command]
pub async fn test_connection(
    provider: String,
    model: String,
    base_url: Option<String>,
) -> Result<String, String> {
    let p = build(&provider, base_url.as_deref()).map_err(|e| e.to_string())?;
    p.ping().await.map_err(|e| e.to_string())?;

    let req = ChatRequest {
        model,
        system: "Say only the single word: ok".into(),
        user: "ping".into(),
        temperature: 0.0,
        max_tokens: 8,
        response_schema: None,
    };
    let req = ChatRequest { max_tokens: if matches!(provider.as_str(), "groq" | "gemini") { 1024 } else { req.max_tokens }, ..req };
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut s = p.stream_chat(req).await.map_err(|e| e.to_string())?;
        let mut out = String::new();
        use futures_util::StreamExt;
        while let Some(c) = s.next().await {
            out.push_str(&c.map_err(|e| e.to_string())?);
            if out.len() > 32 { break; }
        }
        if out.trim().is_empty() {
            return Err("The provider returned no text. Check model availability and its output budget.".into());
        }
        Ok(out)
    }).await.map_err(|_| "The connection test timed out after 30 seconds.".to_string())?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_keeps_existing_providers_and_adds_distinct_byok_entries() {
        let providers = list_providers();
        let mut ids: Vec<_> = providers.iter().map(|p| p.id).collect();
        ids.sort_unstable();
        assert_eq!(ids, ["anthropic", "gemini", "groq", "ollama", "openai", "openrouter"]);
        for (id, model) in [("groq", "openai/gpt-oss-20b"), ("gemini", "gemini-3.5-flash-lite")] {
            let entry = providers.iter().find(|p| p.id == id).unwrap();
            assert!(entry.needs_key);
            assert_eq!(entry.default_model, model);
            assert!(entry.key_help_url.starts_with("https://"));
        }
    }
}
