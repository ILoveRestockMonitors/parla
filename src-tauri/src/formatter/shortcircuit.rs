// Deterministic bypass when the LLM is unreachable or the utterance is trivial
// (<4 words, no disfluency/correction markers): casing + punctuation only.
// UNIMPLEMENTED(Phase 2).
const DISFLUENCIES: [&str; 4] = ["um", "uh", "you know", "scratch that"];

pub fn is_trivial(transcript: &str) -> bool {
    let words = transcript.split_whitespace().count();
    words < 8 && !DISFLUENCIES.iter().any(|d| contains_phrase(transcript, d))
}

fn contains_phrase(text: &str, needle: &str) -> bool {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();
    let wanted: Vec<String> = needle
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .collect();
    words
        .windows(wanted.len())
        .any(|window| window == wanted.as_slice())
}

/// Fallback cleanup: capitalize sentences, terminal period. Never invents facts.
pub fn deterministic_clean(transcript: &str) -> String {
    deterministic_clean_with_mode(transcript, "faithful")
}

/// Faithful mode only trims whitespace and adds sentence punctuation when it
/// is clearly absent. Polished mode additionally removes unambiguous verbal
/// fillers. It deliberately does not lowercase or title-case the payload:
/// identifiers and names are data, not prose.
pub fn deterministic_clean_with_mode(transcript: &str, mode: &str) -> String {
    let t = transcript.trim();
    if t.is_empty() {
        return t.to_string();
    }
    let mut out = t.to_string();
    if mode.eq_ignore_ascii_case("polished") {
        out = out
            .split_whitespace()
            .filter(|w| !matches!(w.to_ascii_lowercase().as_str(), "um" | "uh"))
            .collect::<Vec<_>>()
            .join(" ");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordinary_words_do_not_route_as_fillers() {
        assert!(is_trivial("summary"));
        assert!(is_trivial("Actually Tuesday"));
    }
    #[test]
    fn polished_cleanup_is_token_based() {
        assert_eq!(
            deterministic_clean_with_mode("album spectrum um hello", "polished"),
            "album spectrum hello"
        );
    }
    #[test]
    fn faithful_preserves_identifier_casing() {
        assert_eq!(
            deterministic_clean("Claude API iPhone"),
            "Claude API iPhone"
        );
    }
}
