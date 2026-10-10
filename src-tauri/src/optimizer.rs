use std::sync::atomic::{AtomicU64, Ordering};

use futures_util::StreamExt;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use parking_lot::Mutex;
use tokio::sync::watch;

use crate::providers::{build, ChatRequest};
use crate::SharedState;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OptimizeRequest {
    pub text: String,
    pub job_id: u64,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    #[serde(default)]
    pub feedback: Option<String>,
    #[serde(default)]
    pub previous_output: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct OptimizeEvent {
    pub job_id: u64,
    pub kind: &'static str,
    pub intent: Option<String>,
    pub prompt_delta: Option<String>,
    pub full_prompt: Option<String>,
    pub error: Option<String>,
    pub elapsed_ms: u64,
    pub mode: String,
}

static NEXT_JOB: AtomicU64 = AtomicU64::new(1);
static JOBS: Lazy<Mutex<HashMap<u64, watch::Sender<bool>>>> = Lazy::new(|| Mutex::new(HashMap::new()));

struct JobGuard(u64);

impl Drop for JobGuard {
    fn drop(&mut self) {
        JOBS.lock().remove(&self.0);
    }
}

pub fn next_job_id() -> u64 { NEXT_JOB.fetch_add(1, Ordering::Relaxed) }

#[tauri::command]
pub fn cancel_optimize(job_id: u64) {
    if let Some(sender) = JOBS.lock().get(&job_id) {
        let _ = sender.send(true);
    }
}

#[tauri::command]
pub async fn optimize_text(
    app: AppHandle,
    state: State<'_, SharedState>,
    req: OptimizeRequest,
) -> Result<u64, String> {
    let started = Instant::now();
    let job_id = if req.job_id == 0 { next_job_id() } else { req.job_id };
    let cfg = state.config.read().clone();
    if req.text.trim().is_empty() {
        return Err("Select or enter a prompt first.".into());
    }
    if req.text.chars().count() > cfg.max_input_chars {
        return Err(format!("This request exceeds the {} character input limit. Shorten it or increase the limit in Settings; no requirements were truncated.", cfg.max_input_chars));
    }
    if req.feedback.as_deref().unwrap_or_default().chars().count() > cfg.max_input_chars {
        return Err("Refinement instructions exceed the configured input limit.".into());
    }
    if req.previous_output.as_deref().unwrap_or_default().len() > 256_000 {
        return Err("The previous draft is too large to refine.".into());
    }
    let mode = cfg.enhancement_mode.clone();
    let event = |kind, intent, full_prompt, error| OptimizeEvent {
        job_id, kind, intent, prompt_delta: None, full_prompt, error,
        elapsed_ms: started.elapsed().as_millis() as u64,
        mode: mode.clone(),
    };

    if mode == "grounded" {
        let prompt = crate::prompt_builder::render_grounded(&req.text, req.feedback.as_deref());
        app.emit("pw:optimize", event("done", Some(crate::prompt_builder::intent(&req.text).into()), Some(prompt), None))
            .map_err(|e| e.to_string())?;
        return Ok(job_id);
    }
    if mode != "ai" {
        return Err("Unknown enhancement mode.".into());
    }

    let provider_id = req.provider.unwrap_or(cfg.provider);
    let model = req.model.unwrap_or(cfg.model);
    let base_url = req.base_url.or(cfg.base_url);
    let provider = build(&provider_id, base_url.as_deref()).map_err(|e| e.to_string())?;
    let max_tokens = ((req.text.len() + req.feedback.as_deref().unwrap_or_default().len()) / 2 + 600)
        .clamp(1024, 8192) as u32;
    let user = serde_json::json!({
        "request": req.text,
        "feedback": req.feedback,
        "previous_draft": req.previous_output,
    }).to_string();
    let chat = ChatRequest {
        model,
        system: crate::prompt_builder::AI_SYSTEM.to_string(),
        user,
        temperature: 0.1,
        max_tokens,
        response_schema: Some(crate::prompt_builder::enhancement_response_schema()),
    };
    let source = req.text;
    let feedback = req.feedback;
    let (cancel, mut cancellation) = watch::channel(false);
    {
        let mut jobs = JOBS.lock();
        if jobs.contains_key(&job_id) {
            return Err("An enhancement with this job ID is already running.".into());
        }
        jobs.insert(job_id, cancel);
    }

    tauri::async_runtime::spawn(async move {
        let _guard = JobGuard(job_id);
        let emit = |kind, intent, delta, full_prompt, error| {
            let _ = app.emit("pw:optimize", OptimizeEvent {
                job_id, kind, intent, prompt_delta: delta, full_prompt, error,
                elapsed_ms: started.elapsed().as_millis() as u64,
                mode: mode.clone(),
            });
        };
        let generation = async {
            let mut stream = provider.stream_chat(chat).await.map_err(|e| e.to_string())?;
            let mut raw = String::new();
            let mut parser = StreamingJsonExtractor::new();
            while let Some(chunk) = stream.next().await {
                let delta = chunk.map_err(|e| e.to_string())?;
                raw.push_str(&delta);
                if raw.len() > 256_000 {
                    return Err("The AI returned too much output.".to_string());
                }
                for event in parser.push(&delta) {
                    match event {
                        ExtractorEvent::Intent(intent) => emit("intent", Some(intent), None, None, None),
                        ExtractorEvent::PromptDelta(delta) if cfg.stream => emit("delta", None, Some(delta), None, None),
                        _ => {}
                    }
                }
            }
            let (intent, prompt) = parser.finalize(&raw)?;
            let prompt = crate::prompt_builder::preserve_source_context(&prompt, &source, feedback.as_deref())?;
            Ok::<_, String>((intent, prompt))
        };
        tokio::select! {
            _ = cancellation.changed() => emit("cancelled", None, None, None, None),
            result = tokio::time::timeout(Duration::from_secs(120), generation) => {
                match result {
                    Ok(Ok((intent, prompt))) => emit("done", Some(intent), None, Some(prompt), None),
                    Ok(Err(error)) => emit("error", None, None, None, Some(error)),
                    Err(_) => emit("error", None, None, None, Some("The AI provider timed out. Try again or use Instant grounded mode.".into())),
                }
            }
        }
    });
    Ok(job_id)
}

enum ExtractorState {
    SeekingObject,
    SeekingKey,
    String { key: Option<String>, decoder: crate::json_string::JsonString },
    Colon(String),
    Value(String),
    AfterValue,
    Done,
}

pub enum ExtractorEvent {
    Intent(String),
    PromptDelta(String),
}

pub struct StreamingJsonExtractor {
    state: ExtractorState,
    seen_prompt: bool,
}

impl StreamingJsonExtractor {
    pub fn new() -> Self {
        Self { state: ExtractorState::SeekingObject, seen_prompt: false }
    }

    pub fn push(&mut self, delta: &str) -> Vec<ExtractorEvent> {
        let mut events = Vec::new();
        let mut prompt_delta = String::new();
        for character in delta.chars() {
            let state = std::mem::replace(&mut self.state, ExtractorState::Done);
            self.state = match state {
                ExtractorState::SeekingObject if character == '{' => ExtractorState::SeekingKey,
                ExtractorState::SeekingKey if character == '"' => ExtractorState::String { key: None, decoder: Default::default() },
                ExtractorState::String { key, mut decoder } => {
                    let prior_bytes = decoder.value.len();
                    match decoder.push(character) {
                        Ok(done) => {
                            if key.as_deref() == Some("prompt") {
                                prompt_delta.push_str(&decoder.value[prior_bytes..]);
                            }
                            if done {
                                match key {
                                None if decoder.value == "prompt" && self.seen_prompt => ExtractorState::Done,
                                None => {
                                    if decoder.value == "prompt" { self.seen_prompt = true; }
                                    ExtractorState::Colon(decoder.value)
                                },
                                Some(key) => {
                                    if key == "intent" { events.push(ExtractorEvent::Intent(decoder.value)); }
                                    ExtractorState::AfterValue
                                }
                                }
                            } else {
                                ExtractorState::String { key, decoder }
                            }
                        }
                        Err(_) => ExtractorState::Done,
                    }
                }
                ExtractorState::Colon(key) if character == ':' => ExtractorState::Value(key),
                ExtractorState::Value(key) if character == '"' => ExtractorState::String { key: Some(key), decoder: Default::default() },
                ExtractorState::AfterValue if character == ',' => ExtractorState::SeekingKey,
                ExtractorState::AfterValue if character == '}' => ExtractorState::Done,
                other => other,
            };
        }
        if !prompt_delta.is_empty() {
            events.push(ExtractorEvent::PromptDelta(prompt_delta));
        }
        events
    }

    pub fn finalize(&self, raw: &str) -> Result<(String, String), String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Response { intent: String, prompt: String }
        let response: Response = serde_json::from_str(raw.trim())
            .map_err(|_| "The AI returned incomplete or invalid prompt JSON. Try again.".to_string())?;
        if !["code_agent", "email", "message", "note", "search", "code", "content", "question", "analysis", "summarize", "rewrite", "brainstorm", "data", "other"].contains(&response.intent.as_str())
            || response.prompt.trim().is_empty() {
            return Err("The AI returned an invalid intent or empty prompt.".into());
        }
        Ok((response.intent, response.prompt))
    }
}

#[cfg(test)]
mod stream_tests {
    use super::*;

    #[tokio::test]
    async fn cancellation_is_observable_before_generation_and_job_is_cleaned_up() {
        let job_id = next_job_id();
        let (sender, mut receiver) = watch::channel(false);
        JOBS.lock().insert(job_id, sender);
        let guard = JobGuard(job_id);
        cancel_optimize(job_id);
        receiver.changed().await.unwrap();
        assert!(*receiver.borrow());
        drop(guard);
        assert!(!JOBS.lock().contains_key(&job_id));
    }

    #[test]
    fn cancelling_an_unknown_job_is_a_noop() {
        let job_id = next_job_id();
        cancel_optimize(job_id);
        assert!(!JOBS.lock().contains_key(&job_id));
    }

    #[test]
    fn streams_arbitrary_chunk_boundaries_and_unicode_escapes() {
        let raw = r#"{"intent" : "code_agent", "prompt" : "Line\nquote: \"ok\"; \u65e5\u672c; \ud83d\ude42"}"#;
        for chunk_size in 1..=raw.len() {
            let mut parser = StreamingJsonExtractor::new();
            let mut streamed = String::new();
            for chunk in raw.as_bytes().chunks(chunk_size) {
                for event in parser.push(std::str::from_utf8(chunk).unwrap()) {
                    if let ExtractorEvent::PromptDelta(delta) = event { streamed.push_str(&delta); }
                }
            }
            let (_, final_text) = parser.finalize(raw).unwrap();
            assert_eq!(streamed, final_text, "chunk size {chunk_size}");
            assert!(final_text.contains("日本"));
            assert!(final_text.contains('🙂'));
        }
    }

    #[test]
    fn invalid_or_truncated_json_is_rejected_instead_of_pasted() {
        let parser = StreamingJsonExtractor::new();
        for raw in ["hello", r#"{"intent":"other","prompt":"partial"#, r#"{"intent":"other","prompt":""}"#] {
            assert!(parser.finalize(raw).is_err());
        }
    }

    #[test]
    fn duplicate_prompt_fields_cannot_reset_stream_offsets_or_be_accepted() {
        let raw = r#"{"intent":"other","prompt":"abcd","prompt":"\u65e5\u672c"}"#;
        let mut parser = StreamingJsonExtractor::new();
        for character in raw.chars() { parser.push(&character.to_string()); }
        assert!(parser.finalize(raw).is_err());
    }
}

const IMAGE_JSON_SYSTEM: &str = r#"You are PromptKitcha — an expert at writing text-to-image prompts (DALL-E, Midjourney, SD/FLUX, Imagen, etc).
Output ONLY a JSON object. No markdown fences, no commentary before or after.
Schema: {"prompt":"<one detailed image prompt, English unless the user idea is in another language — then match that language>","tips":["2-4 short tips as strings"]}
Rules: The "prompt" must be a single ready-to-paste block (you may use commas or short lines). Apply the given TECHNIQUE to structure style, subject, lighting, camera, and composition. Preserve concrete subjects from the user; add clarifying detail without inventing unrelated facts."#;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ImagePromptGenRequest {
    pub technique_id: String,
    pub technique_name: String,
    pub technique_blurb: String,
    pub user_input: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ImagePromptGenResponse {
    pub prompt: String,
    pub tips: Vec<String>,
}

fn parse_image_json_output(raw: &str) -> Result<ImagePromptGenResponse, String> {
    if let Some(start) = raw.find('{') {
        if let Some(end) = raw.rfind('}') {
            if end > start {
                let candidate = &raw[start..=end];
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(candidate) {
                    let Some(p) = val.get("prompt").and_then(|v| v.as_str()) else {
                        return Err("missing prompt field in model JSON".to_string());
                    };
                    let prompt = p.to_string();
                    let tips: Vec<String> = val
                        .get("tips")
                        .and_then(|t| t.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    return Ok(ImagePromptGenResponse { prompt, tips });
                }
            }
        }
    }
    Err("no valid JSON in model output".to_string())
}

#[tauri::command]
pub async fn generate_image_prompt(
    state: State<'_, SharedState>,
    req: ImagePromptGenRequest,
) -> Result<ImagePromptGenResponse, String> {
    let cfg = state.config.read().clone();
    let provider_id = cfg.provider;
    let model = cfg.model;
    let base_url = cfg.base_url;
    let max_chars = cfg.max_input_chars;
    let body: String = if req.user_input.chars().count() > max_chars {
        let t: String = req.user_input.chars().take(max_chars).collect();
        format!("{t}\n\n[truncated at {max_chars} chars]")
    } else {
        req.user_input.clone()
    };
    let user = format!(
        "TECHNIQUE_ID: {}\nTECHNIQUE: {}\nWHAT THIS MEANS: {}\n\nUSER IDEA:\n{}",
        req.technique_id, req.technique_name, req.technique_blurb, body
    );
    let provider = build(&provider_id, base_url.as_deref()).map_err(|e| e.to_string())?;
    let chat = ChatRequest {
        model,
        system: IMAGE_JSON_SYSTEM.to_string(),
        user,
        temperature: 0.45,
        max_tokens: 2000,
        response_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": { "type": "string" },
                "tips": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["prompt", "tips"],
            "additionalProperties": false
        })),
    };
    let mut stream = provider
        .stream_chat(chat)
        .await
        .map_err(|e| e.to_string())?;
    let mut raw = String::new();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(t) => raw.push_str(&t),
            Err(e) => return Err(e.to_string()),
        }
    }
    parse_image_json_output(&raw)
}

