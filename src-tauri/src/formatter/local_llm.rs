// Local LLM formatter client (Ollama /api/chat).
// Streams are bounded and collected to completion; the pipeline only commits
// a validated complete candidate. Faithful mode bypasses this client.
use super::prompt::{user_message, SYSTEM_PROMPT};
use super::{ContextEnvelope, DoneFrame, Formatter};
use std::io::Read;

pub struct OllamaFormatter {
    pub base_url: String,
    pub model: String,
    pub num_ctx: u32,
}

impl OllamaFormatter {
    pub fn new(port: u16, model: &str) -> Self {
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            model: model.to_string(),
            num_ctx: 2048,
        }
    }
}

impl Formatter for OllamaFormatter {
    fn format(
        &self,
        envelope: &ContextEnvelope,
        on_token: &mut dyn FnMut(String),
    ) -> Result<DoneFrame, String> {
        #[derive(serde::Serialize)]
        struct ChatOptions {
            temperature: f32,
            num_predict: u32,
            num_ctx: u32,
        }
        #[derive(serde::Serialize)]
        struct Message<'a> {
            role: &'a str,
            content: String,
        }
        #[derive(serde::Serialize)]
        struct ChatBody<'a> {
            model: &'a str,
            messages: [Message<'a>; 2],
            stream: bool,
            think: bool,
            // TOP-LEVEL, not inside options: this Ollama build rejects a
            // nested keep_alive ("invalid option provided") and silently
            // downgrades to the default 5-min expiry. Proven 2026-08-22.
            keep_alive: i32,
            options: ChatOptions,
        }

        // Reserve output for the complete candidate. The old fixed 256 token
        // budget silently accepted clipped paragraphs.
        let input_words = envelope.raw_transcript.split_whitespace().count() as u32;
        let prompt_estimate =
            ((SYSTEM_PROMPT.len() + user_message(envelope).len()) as u32 / 4).saturating_add(64);
        let available = self.num_ctx.saturating_sub(prompt_estimate);
        let requested = envelope
            .options
            .max_tokens
            .max(input_words.saturating_mul(3).saturating_add(32));
        if available < requested {
            return Err(format!("formatter input exceeds context budget (need {requested}, available {available}); use lossless segmentation"));
        }
        let output_budget = requested.min(4096);
        let body = ChatBody {
            model: &self.model,
            // Pin at TOP LEVEL: this Ollama build rejects keep_alive nested
            // in options ("invalid option provided") and silently falls back
            // to the 5-min expiry. Proven live on this box 2026-08-22.
            messages: [
                Message {
                    role: "system",
                    content: SYSTEM_PROMPT.to_string(),
                },
                Message {
                    role: "user",
                    content: user_message(envelope),
                },
            ],
            stream: true,
            think: false,
            keep_alive: -1,
            options: ChatOptions {
                temperature: 0.0,
                num_predict: output_budget,
                num_ctx: self.num_ctx,
            },
        };

        let url = format!("{}/api/chat", self.base_url);
        let resp = ureq::post(&url)
            .timeout(std::time::Duration::from_secs(45))
            .send_json(body)
            .map_err(|e| format!("formatter http: {e}"))?;

        // NDJSON: one JSON object per line; deltas in message.content until done:true
        use std::io::BufRead;
        let mut reader = std::io::BufReader::new(resp.into_reader());
        let mut full = String::new();
        let mut completed = false;
        let mut done_reason: Option<String> = None;
        let mut eval_count = None;
        let mut eval_duration_ns = None;
        let mut load_duration_ns = None;
        let mut line = Vec::with_capacity(4096);
        loop {
            line.clear();
            let read = (&mut reader)
                .take(64 * 1024 + 1)
                .read_until(b'\n', &mut line)
                .map_err(|e| format!("formatter stream read: {e}"))?;
            if read == 0 {
                break;
            }
            if line.len() > 64 * 1024 || full.len().saturating_add(line.len()) > 64 * 1024 {
                return Err("formatter response exceeds bounded candidate size".into());
            }
            let line = std::str::from_utf8(&line)
                .map_err(|e| format!("formatter ndjson is not UTF-8: {e}"))?;
            if line.trim().is_empty() {
                continue;
            }
            let obj: serde_json::Value =
                serde_json::from_str(&line).map_err(|e| format!("formatter ndjson: {e}"))?;
            if obj.get("error").is_some() {
                return Err(format!(
                    "formatter stream error: {}",
                    obj["error"].as_str().unwrap_or("unknown")
                ));
            }
            if let Some(delta) = obj["message"]["content"].as_str() {
                if !delta.is_empty() {
                    full.push_str(delta);
                }
            }
            if obj.get("done").and_then(serde_json::Value::as_bool) == Some(true) {
                completed = true;
                done_reason = obj
                    .get("done_reason")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                eval_count = obj.get("eval_count").and_then(|v| v.as_u64());
                eval_duration_ns = obj.get("eval_duration").and_then(|v| v.as_u64());
                load_duration_ns = obj.get("load_duration").and_then(|v| v.as_u64());
                break;
            }
        }

        if !completed {
            return Err("formatter stream ended without terminal done frame".into());
        }
        if matches!(
            done_reason.as_deref(),
            Some("length" | "max_tokens" | "limit")
        ) {
            return Err(format!(
                "formatter completion truncated ({})",
                done_reason.unwrap_or_default()
            ));
        }
        if done_reason.as_deref() != Some("stop") {
            return Err(format!(
                "formatter unknown completion reason {:?}",
                done_reason
            ));
        }

        let content = full.trim().to_string();
        if content.is_empty() {
            return Err("formatter returned empty output".into());
        }

        on_token(content.clone());
        Ok(DoneFrame {
            confidence: None,
            edited: !content.eq_ignore_ascii_case(&envelope.raw_transcript),
            review_suggested: false,
            completion_reason: done_reason,
            eval_count,
            eval_duration_ns,
            load_duration_ns,
        })
    }

    fn name(&self) -> &'static str {
        "ollama"
    }
}
