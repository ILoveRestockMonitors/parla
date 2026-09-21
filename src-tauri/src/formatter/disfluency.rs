//! Conservative, local stutter cleanup. This is text processing, not a speech
//! diagnosis: ambiguous repetition is preserved, and raw ASR remains available.
use std::borrow::Cow;

#[derive(Debug)]
pub(super) struct Token<'a> {
    pub start: usize,
    pub end: usize,
    pub text: &'a str,
    pub protected: bool,
}

pub(super) fn tokens<'a>(text: &'a str, vocabulary: &[String]) -> Vec<Token<'a>> {
    let mut out = Vec::new();
    let mut quoted = None;
    let mut code = false;
    let mut start = None;
    let mut protected = false;
    for (at, ch) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        if ch.is_whitespace() {
            if let Some(begin) = start.take() {
                let word = &text[begin..at];
                let bare = word.trim_matches(|c: char| !c.is_alphabetic());
                out.push(Token {
                    start: begin,
                    end: at,
                    text: word,
                    protected: protected
                        || vocabulary.iter().any(|term| {
                            term.split_whitespace()
                                .any(|part| part.eq_ignore_ascii_case(bare))
                        }),
                });
            }
            continue;
        }
        if start.is_none() {
            start = Some(at);
            protected = quoted.is_some() || code;
        }
        if ch == '`' {
            code = !code;
            protected = true;
        }
        if matches!(ch, '"' | '“' | '”') {
            quoted = if quoted.is_some() { None } else { Some('"') };
            protected = true;
        }
        if matches!(ch, '\'' | '‘' | '’') {
            // Apostrophes inside contractions are not quotation delimiters.
            let opening = start == Some(at);
            let closing = quoted == Some('\'')
                && text[at + ch.len_utf8()..]
                    .chars()
                    .next()
                    .is_none_or(|next| !next.is_alphanumeric());
            if closing {
                quoted = None;
                protected = true;
            } else if opening {
                quoted = Some('\'');
                protected = true;
            }
        }
    }
    out
}

fn key(text: &str) -> &str {
    text.trim_end_matches(',')
}

fn repeatable(token: &Token<'_>) -> bool {
    let word = key(token.text);
    !token.protected
        && !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_alphabetic() || matches!(c, '\'' | '’'))
        && !matches!(
            word.to_ascii_lowercase().as_str(),
            "no" | "not"
                | "never"
                | "without"
                | "zero"
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
                | "thirty"
                | "forty"
                | "fifty"
                | "sixty"
                | "seventy"
                | "eighty"
                | "ninety"
                | "hundred"
                | "thousand"
                | "million"
                | "billion"
        )
}

fn starter(word: &str) -> bool {
    matches!(
        word.to_ascii_lowercase().as_str(),
        "i" | "we"
            | "you"
            | "he"
            | "she"
            | "they"
            | "it"
            | "i'm"
            | "i’m"
            | "i'd"
            | "i’d"
            | "i'll"
            | "i’ll"
            | "i've"
            | "i’ve"
            | "we're"
            | "we’re"
            | "you're"
            | "you’re"
            | "they're"
            | "they’re"
            | "the"
            | "a"
            | "an"
            | "to"
    )
}

fn restart_phrase(words: &[Token<'_>]) -> bool {
    words.len() == 1
        || matches!(
            key(words[1].text).to_ascii_lowercase().as_str(),
            "want"
                | "need"
                | "can"
                | "could"
                | "will"
                | "would"
                | "should"
                | "must"
                | "am"
                | "are"
                | "is"
                | "was"
                | "were"
                | "have"
                | "has"
        )
}

/// Recognize explicit partial-word repetitions, not ordinary hyphenated words.
/// Two matching fragments are required: `b-b-book`, but not `co-operate` or
/// `re-re-enter`. The final word and any following punctuation are preserved.
fn fragment_offset(token: &Token<'_>) -> Option<usize> {
    if token.protected {
        return None;
    }
    let word = token.text.trim_end_matches([',', '.', '!', '?', ';', ':']);
    let parts: Vec<_> = word.split('-').collect();
    if parts.len() < 3 || parts.len() > 12 {
        return None;
    }
    let last = *parts.last()?;
    let first = parts[0];
    if !(1..=3).contains(&first.chars().count())
        || last.chars().count() <= first.chars().count()
        || !last.chars().all(char::is_alphabetic)
        || !first.chars().all(char::is_alphabetic)
        || matches!(
            first.to_ascii_lowercase().as_str(),
            "re" | "co" | "ex" | "non" | "pre"
        )
        || !last.to_lowercase().starts_with(&first.to_lowercase())
        || !parts[..parts.len() - 1]
            .iter()
            .all(|p| p.eq_ignore_ascii_case(first))
    {
        return None;
    }
    Some(parts[..parts.len() - 1].iter().map(|p| p.len() + 1).sum())
}

/// Removes explicit repeated fragments and adjacent pronoun/phrase restarts.
/// It deliberately keeps `had had`, `that that`, `very very`, numbers, names
/// in the dictionary, code, quotations, negation, and sentence repetitions.
/// No sentence is rewritten and no external model or service is called.
pub fn clean<'a>(text: &'a str, vocabulary: &[String]) -> Cow<'a, str> {
    let words = tokens(text, vocabulary);
    if words.is_empty() {
        return Cow::Borrowed(text);
    }
    let mut edits: Vec<(usize, usize)> = Vec::new();
    let mut index = 0;
    while index < words.len() {
        let mut repeated = None;
        // Only pronoun-led phrase restarts are unambiguous enough for this
        // conservative default. Limit the search to six words, linear in input.
        if repeatable(&words[index]) && starter(key(words[index].text)) {
            for length in (1..=6.min((words.len() - index) / 2)).rev() {
                let left = &words[index..index + length];
                let right = &words[index + length..index + length * 2];
                if restart_phrase(left)
                    && left.iter().zip(right).all(|(a, b)| {
                        repeatable(a)
                            && repeatable(b)
                            && key(a.text).eq_ignore_ascii_case(key(b.text))
                    })
                    && text[words[index].start..words[index + length * 2 - 1].end]
                        .chars()
                        .all(|c| c.is_alphabetic() || matches!(c, ' ' | '\t' | ',' | '\'' | '’'))
                {
                    repeated = Some(length);
                    break;
                }
            }
        }
        if let Some(length) = repeated {
            edits.push((words[index].start, words[index + length].start));
            index += length;
            continue;
        }
        if let Some(offset) = fragment_offset(&words[index]) {
            edits.push((words[index].start, words[index].start + offset));
        }
        index += 1;
    }
    if edits.is_empty() {
        return Cow::Borrowed(text);
    }
    let mut output = String::with_capacity(text.len());
    let mut copied = 0;
    for (start, end) in edits {
        output.push_str(&text[copied..start]);
        copied = end;
    }
    output.push_str(&text[copied..]);
    // A restart at the beginning should not discard initial capitalization.
    if text.starts_with(char::is_uppercase) && output.starts_with(char::is_lowercase) {
        let first = output.chars().next().unwrap();
        output.replace_range(
            ..first.len_utf8(),
            &first.to_uppercase().collect::<String>(),
        );
    }
    Cow::Owned(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removes_explicit_stutters_without_rewriting() {
        for (raw, expected) in [
            ("I I I need the report", "I need the report"),
            ("I I", "I"),
            ("I, I want to send it", "I want to send it"),
            (
                "I want I want to send the report",
                "I want to send the report",
            ),
            ("we need to we need to we need to leave", "we need to leave"),
            ("b-b-book the t-t-taxi.", "book the taxi."),
            ("Th-th-thanks for your help!", "Thanks for your help!"),
            ("I I need b-b-booking help", "I need booking help"),
        ] {
            assert_eq!(clean(raw, &[]), expected, "{raw}");
        }
    }
    #[test]
    fn retains_ambiguous_emphasis_grammar_numbers_and_negation() {
        for raw in [
            "very very good",
            "She had had enough",
            "I know that that works",
            "no no no",
            "I do not I do not agree",
            "one one five",
            "1 1 5",
            "I need one I need one item",
            "go go go",
            "co-operate",
            "I think I think too much",
            "I know I know him",
            "I said I said no",
            "re-re-enter",
            "p-p-value",
            "I need it. I need it.",
            "I want\nI want",
            "I! I need help",
            "foo_bar foo_bar",
            "a-b-test",
        ] {
            assert_eq!(clean(raw, &[]), raw, "{raw}");
        }
    }
    #[test]
    fn quotations_and_dictionary_terms_are_protected() {
        for raw in [
            "Say \"I I need b-b-book\" exactly",
            "Use `I I` and `b-b-book`.",
            "“I I”",
            "Say 'I I need b-b-book' exactly",
            "Say ‘I I need b-b-book’ exactly",
        ] {
            assert_eq!(clean(raw, &[]), raw);
        }
        assert_eq!(clean("I I need it", &["I".into()]), "I I need it");
        assert_eq!(clean("b-b-book", &["b-b-book".into()]), "b-b-book");
        assert_eq!(
            clean("I want I want it", &["want".into()]),
            "I want I want it"
        );
    }
    #[test]
    fn stable_unicode_whitespace_and_idempotence() {
        for raw in [
            "",
            "   ",
            "I I need café",
            "é-é-école",
            "I want I want b-b-book",
            "I I need it  ",
        ] {
            let once = clean(raw, &[]);
            assert_eq!(clean(&once, &[]), once);
        }
        assert!(matches!(clean("Already fluent.", &[]), Cow::Borrowed(_)));
    }
}
