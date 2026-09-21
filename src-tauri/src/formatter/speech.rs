//! Speech cleanup before optional model polishing. Architecture reference:
//! OpenWhispr a77fdce, en/prompts.json + services/ReasoningService.ts:
//! transcript -> cleanup-only model -> completed text; never an agent request.
//! Rules below are independently implemented and preserve raw recovery text.
use super::disfluency::{tokens, Token};

fn bare(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’')
        .replace('’', "'")
        .to_lowercase()
}

fn noise(token: &Token<'_>) -> bool {
    let word = token.text.trim_matches(|c: char| !c.is_alphabetic());
    // UM/UH/ER may be abbreviations; dictionary terms, quotes and code are data.
    if token.protected || word.chars().filter(|c| c.is_uppercase()).count() > 1 {
        return false;
    }
    let lower = word.to_lowercase();
    matches!(lower.as_str(), "um" | "uh" | "erm")
        || lower.strip_prefix('u').is_some_and(|tail| {
            (2..=6).contains(&tail.len())
                && (tail.chars().all(|c| c == 'm') || tail.chars().all(|c| c == 'h'))
        })
}

fn initial_case(original: &str, output: &mut String) {
    if original.starts_with(char::is_uppercase) && output.starts_with(char::is_lowercase) {
        let first = output.chars().next().unwrap();
        output.replace_range(
            ..first.len_utf8(),
            &first.to_uppercase().collect::<String>(),
        );
    }
}

/// Remove only unmistakable vocal fillers, including punctuated/elongated forms.
/// Meaningful "like", "you know", "hmm", "er/err" are for contextual polishing.
pub fn remove_fillers(text: &str, vocabulary: &[String]) -> String {
    let words = tokens(text, vocabulary);
    let removed: Vec<_> = words.iter().map(noise).collect();
    if !removed.iter().any(|&v| v) {
        return text.to_string();
    }
    let mut output = String::with_capacity(text.len());
    let mut copied = 0;
    for (i, word) in words.iter().enumerate() {
        if !removed[i] {
            continue;
        }
        output.push_str(&text[copied..word.start]);
        copied = words.get(i + 1).map(|w| w.start).unwrap_or(text.len());
        if i + 1 == words.len() {
            // Preserve an utterance's final punctuation without leaving "hello um.".
            let end = word.text.chars().last();
            if !output.trim().is_empty() && matches!(end, Some('.' | '!' | '?')) {
                output.truncate(
                    output
                        .trim_end_matches(|c: char| c.is_whitespace() || c == ',')
                        .len(),
                );
                if !output.ends_with(['.', '!', '?']) {
                    output.push(end.unwrap());
                }
            }
        }
    }
    output.push_str(&text[copied..]);
    let mut output = output.trim().to_string();
    initial_case(text.trim_start(), &mut output);
    output
}

fn category(word: &str) -> Option<&'static str> {
    let word = bare(word);
    if matches!(
        word.as_str(),
        "monday" | "tuesday" | "wednesday" | "thursday" | "friday" | "saturday" | "sunday"
    ) {
        return Some("weekday");
    }
    if matches!(
        word.as_str(),
        "january"
            | "february"
            | "march"
            | "april"
            | "may"
            | "june"
            | "july"
            | "august"
            | "september"
            | "october"
            | "november"
            | "december"
    ) {
        return Some("month");
    }
    if word.chars().any(|c| c.is_ascii_digit())
        && word
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, ':' | '.' | '-' | '+' | '$' | '%'))
    {
        return Some("number");
    }
    if matches!(
        word.as_str(),
        "zero"
            | "one"
            | "two"
            | "three"
            | "four"
            | "five"
            | "six"
            | "seven"
            | "eight"
            | "nine"
            | "ten"
            | "eleven"
            | "twelve"
            | "thirteen"
            | "fourteen"
            | "fifteen"
            | "sixteen"
            | "seventeen"
            | "eighteen"
            | "nineteen"
            | "twenty"
    ) {
        return Some("number");
    }
    None
}

fn content_word(word: &str) -> bool {
    !word.is_empty()
        && word.chars().all(char::is_alphabetic)
        && !matches!(
            word,
            "i" | "you"
                | "we"
                | "he"
                | "she"
                | "they"
                | "it"
                | "a"
                | "an"
                | "the"
                | "to"
                | "of"
                | "and"
                | "or"
                | "but"
                | "is"
                | "are"
                | "was"
                | "were"
                | "be"
                | "been"
                | "have"
                | "has"
                | "had"
                | "do"
                | "does"
                | "did"
                | "not"
                | "no"
                | "never"
                | "without"
                | "what"
                | "which"
                | "how"
                | "when"
                | "why"
                | "that"
                | "this"
                | "sorry"
                | "mean"
                | "meant"
                | "actually"
        )
}

/// Resolve bounded, explicit adjacent replacement: "Tuesday, I mean Wednesday".
/// "Actually"/bare "no" require matching value categories; ordinary emphasis,
/// incomplete corrections, numbers embedded in larger phrases, and quotes stay.
pub fn resolve_corrections(text: &str, vocabulary: &[String]) -> String {
    let mut output = text.to_string();
    for _ in 0..8 {
        let words = tokens(&output, vocabulary);
        let mut edit = None;
        for i in 1..words.len().saturating_sub(1) {
            let rest: Vec<_> = words[i..].iter().take(4).map(|w| bare(w.text)).collect();
            let (count, strong) = match rest.as_slice() {
                [a, b, c, ..]
                    if a == "sorry" && b == "i" && matches!(c.as_str(), "mean" | "meant") =>
                {
                    (3, true)
                }
                [a, b, ..]
                    if (a == "i" && matches!(b.as_str(), "mean" | "meant"))
                        || (a == "no" && b == "wait")
                        || (a == "wait" && b == "no")
                        || (a == "scratch" && b == "that") =>
                {
                    (2, true)
                }
                [a, ..] if matches!(a.as_str(), "actually" | "sorry" | "no") => (1, false),
                _ => continue,
            };
            let Some(next) = words.get(i + count) else {
                continue;
            };
            let previous = &words[i - 1];
            let before = bare(previous.text);
            let after = bare(next.text);
            let same_category =
                category(previous.text).is_some() && category(previous.text) == category(next.text);
            if previous.protected || words[i..=i+count].iter().any(|w| w.protected)
                || output[previous.start..next.start].contains(['.', '!', '?', '\n', ';'])
                || !(same_category || (strong && content_word(&before) && content_word(&after)))
                // Avoid partially changing multiword numbers/times/dates.
                || (i>1 && category(words[i-2].text)==Some("number"))
                || words.get(i+count+1).is_some_and(|w| category(w.text)==Some("number"))
            {
                continue;
            }
            edit = Some((previous.start, next.start));
            break;
        }
        let Some((start, end)) = edit else { break };
        output.replace_range(start..end, "");
    }
    initial_case(text, &mut output);
    output
}

/// The fast path is independent of Ollama and available in both cleanup modes.
pub fn prepare(
    text: &str,
    vocabulary: &[String],
    fillers: bool,
    stutters: bool,
    corrections: bool,
) -> String {
    let mut output = if fillers {
        remove_fillers(text, vocabulary)
    } else {
        text.to_string()
    };
    if corrections {
        output = resolve_corrections(&output, vocabulary);
    }
    if stutters {
        output = super::disfluency::clean(&output, vocabulary).into_owned();
    }
    output
}

#[derive(Debug)]
struct Word {
    value: String,
    start: usize,
    protected: bool,
}
fn words(text: &str, vocabulary: &[String]) -> Vec<Word> {
    let protected = tokens(text, vocabulary);
    let mut protected_index = 0;
    let mut output = Vec::new();
    let mut start = None;
    for (at, ch) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        if ch.is_alphanumeric() || matches!(ch, '\'' | '’') {
            start.get_or_insert(at);
        } else if let Some(begin) = start.take() {
            while protected
                .get(protected_index)
                .is_some_and(|w| w.end <= begin)
            {
                protected_index += 1;
            }
            let source = &text[begin..at];
            let acronym = source.chars().filter(|c| c.is_uppercase()).count() > 1
                && !source.chars().any(char::is_lowercase);
            output.push(Word {
                value: bare(source),
                start: begin,
                protected: acronym
                    || protected
                        .get(protected_index)
                        .is_some_and(|w| w.protected && begin < w.end && at > w.start),
            });
        }
    }
    output
}

fn allowed_gap(raw: &str, source: &[Word], start: usize, end: usize) -> bool {
    let gap = &source[start..end];
    if gap.is_empty() {
        return true;
    }
    if gap.len() > 16 || gap.iter().any(|w| w.protected) {
        return false;
    }
    if gap
        .iter()
        .all(|w| matches!(w.value.as_str(), "um" | "uh" | "erm"))
    {
        return true;
    }
    let next = source.get(end);
    let parenthetical = start == 0 || raw[..gap[0].start].trim_end().ends_with(',');
    if parenthetical
        && (gap.iter().map(|w| w.value.as_str()).eq(["you", "know"])
            || (gap.len() == 1 && matches!(gap[0].value.as_str(), "like" | "basically" | "hmm")))
    {
        return true;
    }
    let Some(next) = next else { return false }; // Never permit unexplained tail truncation.
    if raw[gap[0].start..next.start].contains(['.', '!', '?', ';', '\n']) {
        return false;
    }
    if gap[0].value != next.value {
        return false;
    }
    let pronoun = matches!(
        next.value.as_str(),
        "i" | "i'm"
            | "i'd"
            | "i'll"
            | "i've"
            | "we"
            | "we're"
            | "you"
            | "you're"
            | "he"
            | "she"
            | "they"
            | "they're"
    );
    if !pronoun {
        return false;
    }
    let shared = gap
        .iter()
        .zip(&source[end..])
        .take_while(|(a, b)| a.value == b.value)
        .count();
    let fragments = gap.iter().any(|w| {
        w.value.len() == 1
            && !matches!(w.value.as_str(), "i" | "a")
            && w.value.chars().all(char::is_alphabetic)
    });
    // Intentional "I think I think..." / "I know I know..." are not restarts.
    let ambiguous = gap
        .get(1)
        .is_some_and(|w| matches!(w.value.as_str(), "think" | "know" | "said"));
    // A full duplicate prefix is a restart. For a divergent final word require
    // a shared spoken stem; two different completed objects are not a stutter.
    let similar_ending = shared >= 2
        && gap.len() == shared + 1
        && source.get(end + shared).is_some_and(|word| {
            let abandoned = &gap[shared].value;
            abandoned.chars().count() >= 4
                && word.value.chars().count() >= 4
                && abandoned.chars().take(3).eq(word.value.chars().take(3))
        });
    !ambiguous && (fragments || shared == gap.len() || similar_ending)
}

fn grammatical_variant(a: &str, b: &str) -> bool {
    // Small grammatical repairs, never content-word substitutions or a new fact.
    matches!(
        (a, b),
        ("a", "an")
            | ("an", "a")
            | ("is", "are")
            | ("are", "is")
            | ("was", "were")
            | ("were", "was")
            | ("has", "have")
            | ("have", "has")
    )
}

/// Give the cleanup model a readable draft instead of a chain of ASR fragments.
/// Only Polished mode uses this: the same bounded alignment rules used to
/// validate its output identify repeated attempts, never unrelated sentences.
pub fn polish_input(raw: &str, vocabulary: &[String]) -> String {
    let mut output = prepare(raw, vocabulary, true, true, true);
    for _ in 0..32 {
        let source = words(&output, vocabulary);
        let mut edit = None;
        'search: for start in 0..source.len() {
            for end in start + 1..=(start + 16).min(source.len().saturating_sub(1)) {
                let gap = &source[start..end];
                // Contextual fillers such as "like" are for the model. This
                // draft pass is restricted to restarts of the same pronoun.
                if source[start].value != source[end].value {
                    continue;
                }
                // Never resolve a possible restart by dropping a value or a
                // negation. Such an edit needs an explicit spoken replacement.
                if gap.iter().any(|w| {
                    category(&w.value).is_some()
                        || matches!(w.value.as_str(), "not" | "no" | "never" | "without")
                        || w.value.ends_with("n't")
                }) {
                    continue;
                }
                if allowed_gap(&output, &source, start, end) {
                    edit = Some((source[start].start, source[end].start));
                    break 'search;
                }
            }
        }
        let Some((start, end)) = edit else { break };
        output.replace_range(start..end, "");
    }
    initial_case(raw, &mut output);
    output
}

/// Validate a model's bounded speech edits while preserving word order. This is
/// deliberately narrower than unrestricted rewriting. Numeric/negative/name/
/// identifier anchors and quoted text are checked separately by the caller.
pub fn wording_preserved(raw: &str, candidate: &str, vocabulary: &[String]) -> bool {
    let source = words(raw, vocabulary);
    let target = words(candidate, vocabulary);
    if source.len() > 4096 || target.len() > 4096 {
        return false;
    }
    let mut reachable = vec![false; source.len() + 1];
    reachable[0] = true;
    for word in &target {
        let mut next = vec![false; source.len() + 1];
        for (at, &possible) in reachable.iter().enumerate() {
            if !possible {
                continue;
            }
            for skip in 0..=16.min(source.len().saturating_sub(at)) {
                let end = at + skip;
                let Some(current) = source.get(end) else {
                    break;
                };
                if current.value != word.value
                    && (current.protected || !grammatical_variant(&current.value, &word.value))
                {
                    continue;
                }
                if allowed_gap(raw, &source, at, end) {
                    next[end + 1] = true;
                }
            }
        }
        reachable = next;
    }
    reachable
        .iter()
        .enumerate()
        .any(|(at, &possible)| possible && allowed_gap(raw, &source, at, source.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn punctuated_fillers_and_filler_only_audio() {
        for (raw, expected) in [
            ("This is an um test.", "This is an test."),
            ("Um, hello uh world.", "Hello world."),
            ("hello um.", "hello."),
            ("um, uh. erm", ""),
            ("Ummm I need this", "I need this"),
        ] {
            assert_eq!(remove_fillers(raw, &[]), expected);
        }
        for raw in [
            "album spectrum",
            "Say 'um' exactly",
            "Use `uh` here",
            "UM and ER",
            "I like this",
            "hmm",
            "Err on the safe side",
        ] {
            assert_eq!(remove_fillers(raw, &[]), raw);
        }
        assert_eq!(
            remove_fillers("ask um today", &["um".into()]),
            "ask um today"
        );
    }
    #[test]
    fn explicit_corrections_require_replacement_not_emphasis() {
        for (raw, expected) in [
            ("Send it Thursday no wait Friday", "Send it Friday"),
            ("at 2, I mean 3", "at 3"),
            ("Tuesday, actually Wednesday", "Wednesday"),
            ("I want coffee I meant tea", "I want tea"),
            (
                "send John, sorry I meant Jane the file",
                "send Jane the file",
            ),
        ] {
            assert_eq!(resolve_corrections(raw, &[]), expected);
        }
        for raw in [
            "I actually like Tuesday",
            "I know what I mean",
            "I meant to call",
            "No, do not delete",
            "three hundred I mean four hundred",
            "Say \"Tuesday I mean Wednesday\"",
            "Tuesday. I mean Wednesday",
            "Tuesday actually",
        ] {
            assert_eq!(resolve_corrections(raw, &[]), raw);
        }
    }
    #[test]
    fn model_can_remove_abandoned_speech_without_accepting_truncation() {
        let raw="This is an test. Yeah, I'm not sure. I'm looking at Parla right now. I'm the b l I'd be I'm purposely stubborn I'm purposely stuttering my words right now.";
        let expected="This is a test. Yeah, I'm not sure. I'm looking at Parla right now. I'm purposely stuttering my words right now.";
        assert!(wording_preserved(raw, expected, &["Parla".into()]));
        for (raw, out) in [
            ("The dog bit the man", "The man bit the dog"),
            (
                "Please send the report tomorrow morning",
                "Please send the report",
            ),
            ("send the message", "delete the message"),
            ("I love cats. I love dogs.", "I love dogs."),
            ("I love cats I love dogs", "I love dogs"),
            (
                "I'm making dinner I'm making breakfast",
                "I'm making breakfast",
            ),
            ("I think I think too much", "I think too much"),
            ("I like apples", "I apples"),
            ("Do you know Alex?", "Do Alex?"),
        ] {
            assert!(!wording_preserved(raw, out, &[]), "{raw} -> {out}");
        }
    }
    #[test]
    fn disabled_controls_and_contracted_stutters() {
        let raw = "Um I'm I'm at Tuesday I mean Wednesday";
        assert_eq!(prepare(raw, &[], false, false, false), raw);
        assert_eq!(prepare(raw, &[], true, true, true), "I'm at Wednesday");
    }
    #[test]
    fn polished_draft_resolves_fragments_but_keeps_complete_thoughts() {
        assert_eq!(polish_input("I'm the b l I'd be I'm purposely stubborn I'm purposely stuttering my words right now.",&[]),"I'm purposely stuttering my words right now.");
        assert_eq!(
            polish_input("I need the s d I'd I need the schedule by noon.", &[]),
            "I need the schedule by noon."
        );
        for raw in [
            "I love cats I love dogs",
            "I'm making dinner I'm making breakfast",
            "I think I think too much",
            "I do not I do want that",
            "I need 5 I need 6",
            "Say 'I I' exactly",
            "Use `I I`",
            "I'm looking at Paris I'm looking at London",
            "Like father like son",
            "You know the answer",
            "UM is an abbreviation",
        ] {
            assert_eq!(polish_input(raw, &[]), raw, "{raw}");
        }
        assert_eq!(
            polish_input("I need Claude I need this", &["Claude".into()]),
            "I need Claude I need this"
        );
    }
}
