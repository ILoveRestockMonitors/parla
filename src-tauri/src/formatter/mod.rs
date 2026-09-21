// Formatter trait + §7.2 ContextEnvelope + output contracts.
pub mod disfluency;
pub mod layout;
pub mod lists;
pub mod local_llm;
pub mod numbers;
pub mod prompt;
pub mod shortcircuit;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ContextEnvelope {
    pub mode: String, // "dictation" | "command"
    pub raw_transcript: String,
    pub context: FieldContext,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_style: Option<serde_json::Value>,
    pub language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vocabulary: Option<Vec<String>>,
    pub options: Options,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct FieldContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    pub app_category: String, // email|work_chat|personal_chat|docs|code|other
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_after: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Options {
    pub max_tokens: u32,
    pub stream: bool,
}

#[derive(Debug, serde::Deserialize)]
pub struct DoneFrame {
    /// None means the backend did not provide a validated confidence score.
    pub confidence: Option<f32>,
    pub edited: bool,
    #[serde(default)]
    pub review_suggested: bool,
    #[serde(default)]
    pub completion_reason: Option<String>,
    #[serde(default)]
    pub eval_count: Option<u64>,
    #[serde(default)]
    pub eval_duration_ns: Option<u64>,
    #[serde(default)]
    pub load_duration_ns: Option<u64>,
}

/// Candidate callbacks are collected and validated before any insertion.
pub trait Formatter: Send + Sync {
    /// Emit tokens via on_token; return the trailing metadata frame.
    fn format(
        &self,
        envelope: &ContextEnvelope,
        on_token: &mut dyn FnMut(String),
    ) -> Result<DoneFrame, String>;
    fn name(&self) -> &'static str;
}

#[allow(dead_code)]
pub enum FormatterOutput {
    Token(String),
}

#[derive(Debug)]
pub struct FormatResult {
    pub text: String,
    pub metadata: DoneFrame,
}

/// Collect a formatter candidate without coupling the final commit to a
/// streaming callback. This is the safe phase-two entry point.
pub fn format_complete(
    formatter: &dyn Formatter,
    envelope: &ContextEnvelope,
) -> Result<FormatResult, String> {
    let mut text = String::new();
    let mut overflow = false;
    let metadata = formatter.format(envelope, &mut |token| {
        if text.len().saturating_add(token.len()) <= 64 * 1024 {
            text.push_str(&token);
        } else {
            overflow = true;
        }
    })?;
    if overflow || text.len() > 64 * 1024 {
        return Err("formatter candidate exceeds limit".into());
    }
    if text.trim().is_empty() {
        return Err("formatter produced empty candidate".into());
    }
    let candidate = text.trim().to_string();
    validate_candidate(
        &envelope.raw_transcript,
        &candidate,
        envelope.vocabulary.as_deref(),
    )?;
    if metadata.completion_reason.as_deref() != Some("stop") {
        return Err("formatter completion is not a normal stop".into());
    }
    Ok(FormatResult {
        text: candidate,
        metadata,
    })
}

pub fn validate_candidate(
    raw: &str,
    candidate: &str,
    vocabulary: Option<&[String]>,
) -> Result<(), String> {
    if candidate.len() > 64 * 1024 {
        return Err("formatter candidate exceeds limit".into());
    }
    let all_raw: Vec<String> = raw.split_whitespace().map(anchor_token).collect();
    let raw_tokens: Vec<String> = all_raw
        .iter()
        .cloned()
        .filter(|s| s.chars().any(|c| c.is_ascii_digit()))
        .collect();
    let out_tokens: Vec<String> = candidate.split_whitespace().map(anchor_token).collect();
    for tok in raw_tokens.iter() {
        if out_tokens.iter().filter(|x| *x == tok).count()
            != raw_tokens.iter().filter(|x| *x == tok).count()
        {
            return Err(format!("formatter dropped numeric anchor {tok}"));
        }
    }
    let out_numbers: Vec<&String> = out_tokens
        .iter()
        .filter(|t| t.chars().any(|c| c.is_ascii_digit()))
        .collect();
    if out_numbers
        .iter()
        .any(|n| !raw_tokens.iter().any(|r| *r == **n))
    {
        return Err("formatter added numeric anchor".into());
    }
    for neg in ["not", "never", "no", "without", "n't"] {
        let a = all_raw
            .iter()
            .filter(|t| *t == neg || (neg == "n't" && (t.contains("n't") || t.contains("n’t"))))
            .count();
        let b = out_tokens
            .iter()
            .filter(|t| *t == neg || (neg == "n't" && (t.contains("n't") || t.contains("n’t"))))
            .count();
        if a != b {
            return Err(format!("formatter dropped negation anchor {neg}"));
        }
    }
    if let Some(vocab) = vocabulary {
        for term in vocab {
            if boundary_count(candidate, term, true) != boundary_count(raw, term, false) {
                return Err(format!("formatter changed protected term {term}"));
            }
        }
    }
    // Preserve code-like strings exactly, including case and multiplicity.
    let identifiers = |s: &str| -> std::collections::BTreeMap<String, usize> {
        let mut counts = std::collections::BTreeMap::new();
        for word in s.split_whitespace() {
            let word = word
                .trim_matches(|c: char| ",!?;:()[]{}\"'".contains(c))
                .trim_end_matches('.');
            if word.contains(['_', '/', '\\', '@', '.']) || word.contains('-') {
                *counts.entry(word.to_string()).or_insert(0) += 1;
            }
        }
        counts
    };
    if identifiers(raw) != identifiers(candidate) {
        return Err("formatter changed an identifier".into());
    }
    let words = |s: &str| -> Vec<String> {
        s.replace(['’', '‘'], "'")
            .split(|c: char| !c.is_alphanumeric() && c != '\'')
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .filter(|w| !matches!(w.as_str(), "um" | "uh" | "erm" | "hmm"))
            .collect()
    };
    let source = words(raw);
    let output = words(candidate);
    // A word-count threshold can admit truncated paragraphs or reversed
    // subjects. Require the full substantive word sequence at every length.
    if source != output {
        return Err("formatter changed the wording or word order".into());
    }
    if all_raw.len() >= 8 && candidate.split_whitespace().count() * 2 < all_raw.len() {
        return Err("formatter candidate deletes too much source text".into());
    }
    Ok(())
}

fn anchor_token(s: &str) -> String {
    s.trim_matches(|c: char| ",.!?;:)]}".contains(c))
        .replace(['’', '‘'], "'")
        .to_lowercase()
}
fn boundary_count(text: &str, needle: &str, exact: bool) -> usize {
    let separator = |c: char| !c.is_alphanumeric() && c != '_' && c != '\'';
    let hay: Vec<_> = text.split(separator).filter(|w| !w.is_empty()).collect();
    let term: Vec<_> = needle.split(separator).filter(|w| !w.is_empty()).collect();
    if term.is_empty() {
        return 0;
    }
    hay.windows(term.len())
        .filter(|window| {
            window.iter().zip(&term).all(|(a, b)| {
                if exact {
                    a == b
                } else {
                    a.eq_ignore_ascii_case(b)
                }
            })
        })
        .count()
}

#[cfg(test)]
mod integrity_tests {
    use super::*;
    struct Fake {
        chunks: Vec<String>,
        reason: Option<String>,
    }
    impl Formatter for Fake {
        fn format(
            &self,
            _: &ContextEnvelope,
            cb: &mut dyn FnMut(String),
        ) -> Result<DoneFrame, String> {
            for c in &self.chunks {
                cb(c.clone());
            }
            Ok(DoneFrame {
                confidence: None,
                edited: false,
                review_suggested: false,
                completion_reason: self.reason.clone(),
                eval_count: None,
                eval_duration_ns: None,
                load_duration_ns: None,
            })
        }
        fn name(&self) -> &'static str {
            "fake"
        }
    }
    fn env(raw: &str) -> ContextEnvelope {
        ContextEnvelope {
            mode: "dictation".into(),
            raw_transcript: raw.into(),
            context: FieldContext {
                app: None,
                app_category: "other".into(),
                text_before: None,
                selected_text: None,
                text_after: None,
            },
            user_style: None,
            language: "en".into(),
            vocabulary: None,
            options: Options {
                max_tokens: 64,
                stream: false,
            },
        }
    }
    #[test]
    fn complete_normal_stop() {
        assert_eq!(
            format_complete(
                &Fake {
                    chunks: vec!["hello".into()],
                    reason: Some("stop".into())
                },
                &env("hello")
            )
            .unwrap()
            .text,
            "hello"
        );
    }
    #[test]
    fn incomplete_or_nonstop_rejected() {
        for reason in [None, Some("length"), Some("other")] {
            assert!(format_complete(
                &Fake {
                    chunks: vec!["hello".into()],
                    reason: reason.map(str::to_string)
                },
                &env("hello")
            )
            .is_err());
        }
    }
    #[test]
    fn oversized_single_callback_rejected() {
        assert!(format_complete(
            &Fake {
                chunks: vec!["x".repeat(64 * 1024 + 1)],
                reason: Some("stop".into())
            },
            &env("hello")
        )
        .is_err());
    }
    #[test]
    fn semantic_anchors_reject_edits() {
        for (raw, out) in [
            ("do not delete", "delete"),
            ("do not delete", "do delete"),
            ("-3 items", "3 items"),
            ("3 items", "30 items"),
            ("add 7", "add 8"),
        ] {
            assert!(validate_candidate(raw, out, None).is_err());
        }
        assert!(validate_candidate("don't go", "don't go", None).is_ok());
    }
    #[test]
    fn identifiers_names_and_context_echoes_are_guarded() {
        assert!(validate_candidate(
            "The dog bit the man near the house",
            "The man bit the dog near the house",
            None
        )
        .is_err());
        assert!(validate_candidate(
            "Please send the complete report about the project tomorrow morning",
            "Please send the complete report about the project",
            None
        )
        .is_err());
        let vocab = vec!["Claude Code".to_string()];
        assert!(
            validate_candidate("Ask Claude Code today", "Ask Claude today", Some(&vocab)).is_err()
        );
        assert!(validate_candidate("Open src/main.rs", "Open src/test.rs", None).is_err());
        assert!(validate_candidate("send the message", "delete the message", None).is_err());
        assert!(validate_candidate("hello", "Hello. Thanks for your email!", None).is_err());
        assert!(
            validate_candidate("um hello Claude Code", "Hello, Claude Code.", Some(&vocab)).is_ok()
        );
    }
}
