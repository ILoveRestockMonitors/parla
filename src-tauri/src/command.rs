//! Command Mode v1 (spec §7.3): short utterances that ACT instead of dictating.
//!   "scratch that" -> delete the verified last Parla insert
//!   "bullets"      -> reformat the last insert as a bullet list
//! Design: classify() and the session bookkeeping are pure/unit-tested;
//! execute_* are the only places that touch SendInput, and both refuse to act
//! unless the foreground window still owns the remembered insert.
//! Commands are not utterances: their outcomes never enter history (privacy).

use crate::inject::transaction::{self, CommitReceipt};

/// A recognized spoken command.
#[derive(Debug, PartialEq)]
pub enum Command {
    ScratchThat,
    Bullets,
}

/// Recognize a command in a raw transcript. Tolerant of case/punctuation/
/// extra whitespace because ASR may return "Scratch THAT." etc.
pub fn classify(raw: &str) -> Option<Command> {
    match normalize(raw).as_str() {
        "scratch that" => Some(Command::ScratchThat),
        "bullets" => Some(Command::Bullets),
        _ => None,
    }
}

fn normalize(s: &str) -> String {
    s.trim()
        .trim_end_matches(['.', '!', '?', ','])
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// What we remember about the last successful insert.
pub struct LastInsert {
    receipt: Option<CommitReceipt>,
    /// Final formatted text WITHOUT the trailing seam space.
    pub text: String,
    /// Bookkeeping only. Replacement requires an exact verified UIA range.
    utf16_len: usize,
    /// Foreground window at insert time; scratch/reformat refuses elsewhere.
    hwnd: isize,
}

/// Per-run-loop memory of the last insert. Nothing here persists to disk.
pub struct Session {
    last: Option<LastInsert>,
}

impl Session {
    pub fn new() -> Self {
        Self { last: None }
    }

    /// Record a successful insertion (called by the pipeline on its success
    /// path, with the verified foreground window).
    pub fn remember(&mut self, text_without_seam: &str, hwnd: isize) {
        let utf16_len = text_without_seam.encode_utf16().count() + 1; // +1 seam space
        self.last = Some(LastInsert {
            receipt: None,
            text: text_without_seam.to_string(),
            utf16_len,
            hwnd,
        });
    }

    pub fn last(&self) -> Option<&LastInsert> {
        self.last.as_ref()
    }

    pub fn restore_target(&self) -> Option<&crate::context::target::TargetSnapshot> {
        let receipt = self.last.as_ref()?.receipt.as_ref()?;
        receipt.verified.then_some(&receipt.after)
    }

    pub fn remember_receipt(&mut self, text: &str, receipt: CommitReceipt) {
        self.last = Some(LastInsert {
            text: text.into(),
            utf16_len: receipt.payload.encode_utf16().count(),
            hwnd: receipt.after.hwnd,
            receipt: Some(receipt),
        });
    }

    pub fn restore_original(&mut self, original: &str) -> Result<(), String> {
        let last = self.last.take().ok_or("nothing to restore")?;
        let receipt = last
            .receipt
            .ok_or("previous insertion could not be verified")?;
        if let Some(updated) = transaction::replace(&receipt, original)? {
            self.remember_receipt(original, updated);
        }
        Ok(())
    }

    pub fn forget(&mut self) {
        self.last = None;
    }
}

impl LastInsert {
    pub fn utf16_len(&self) -> usize {
        self.utf16_len
    }
    pub fn hwnd(&self) -> isize {
        self.hwnd
    }
}

/// "scratch that": erase the remembered insert in the SAME window it landed
/// in. Returns a human-readable status line for the console.
pub fn execute_scratch(session: &mut Session) -> Result<String, String> {
    let last = session.last.take().ok_or("nothing to scratch yet")?;
    let receipt = last
        .receipt
        .ok_or("previous insertion could not be verified; use recovered text")?;
    transaction::replace(&receipt, "")?;
    Ok("removed the verified previous insertion".into())
}

/// "bullets": erase the remembered insert and re-inject it as a list.
pub fn execute_bullets(session: &mut Session) -> Result<String, String> {
    let last = session.last.take().ok_or("nothing to reformat yet")?;
    let Some(bulleted) = to_bullets(&last.text) else {
        return Err("last insert has no sentences to bullet".into());
    };
    let receipt = last
        .receipt
        .ok_or("previous insertion could not be verified; use recovered text")?;
    let payload = crate::pipeline::seam_spaced(&bulleted);
    if let Some(updated) = transaction::replace(&receipt, &payload)? {
        session.remember_receipt(&bulleted, updated);
    }
    Ok("reformatted as bullets".into())
}

/// Deterministic bullets: split on sentence enders, one bullet per sentence.
/// Pure function so output is unit-testable (no LLM in the loop).
pub fn to_bullets(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut out = String::new();
    let mut parts = 0usize;
    let mut rest = trimmed;
    while !rest.is_empty() {
        let cut = rest
            .char_indices()
            .filter(|(_, c)| matches!(c, '.' | '!' | '?'))
            .map(|(i, _)| i)
            .min();
        let Some(i) = cut else { break };
        let sentence = rest[..=i].trim();
        rest = &rest[i + 1..];
        if !sentence.is_empty() {
            out.push_str("- ");
            out.push_str(sentence);
            out.push('\n');
            parts += 1;
        }
    }
    let tail = rest.trim();
    if !tail.is_empty() {
        out.push_str("- ");
        out.push_str(tail);
        out.push('\n');
        parts += 1;
    }
    if parts == 0 {
        None
    } else {
        Some(out.trim_end().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scratch_that_variants_match() {
        assert_eq!(classify("scratch that"), Some(Command::ScratchThat));
        assert_eq!(classify("  Scratch   THAT. "), Some(Command::ScratchThat));
        assert_eq!(classify("SCRATCH THAT!"), Some(Command::ScratchThat));
    }

    #[test]
    fn bullets_variants_match() {
        assert_eq!(classify("bullets"), Some(Command::Bullets));
        assert_eq!(classify("Bullets."), Some(Command::Bullets));
    }

    #[test]
    fn plain_speech_is_not_a_command() {
        assert_eq!(classify("scratch that off my list please"), None);
        assert_eq!(classify("hello world"), None);
        assert_eq!(classify(""), None);
        assert_eq!(classify("bulletin board"), None);
    }

    #[test]
    fn session_counts_utf16_units_plus_seam() {
        let mut s = Session::new();
        assert!(s.last().is_none());
        s.remember("a \u{1F600}b", 1234); // 'a',' ',surrogate-pair,'b' = 5 units
        let last = s.last().unwrap();
        assert_eq!(last.utf16_len(), 6); // 5 + seam space
        assert_eq!(last.hwnd(), 1234);
        s.forget();
        assert!(s.last().is_none());
    }

    #[test]
    fn session_remember_overwrites_previous() {
        let mut s = Session::new();
        s.remember("first", 1);
        s.remember("second", 2);
        assert_eq!(s.last().unwrap().text, "second");
        assert_eq!(s.last().unwrap().hwnd(), 2);
    }

    #[test]
    fn bullets_split_sentences() {
        assert_eq!(
            to_bullets("Buy milk. Walk dog. Call mom.").as_deref(),
            Some("- Buy milk.\n- Walk dog.\n- Call mom.")
        );
    }

    #[test]
    fn bullets_trailing_fragment_gets_own_bullet() {
        assert_eq!(
            to_bullets("First thing. second").as_deref(),
            Some("- First thing.\n- second")
        );
    }

    #[test]
    fn bullets_single_sentence_and_empty() {
        assert_eq!(to_bullets("Just one.").as_deref(), Some("- Just one."));
        assert_eq!(to_bullets(""), None);
        assert_eq!(to_bullets("no ender").as_deref(), Some("- no ender"));
    }
}
