//! Automatic plain-text lists for clear list cues and concise enumerations.
//! Formatting is local and deterministic: preserve item text/order, never infer
//! word boundaries from an unpunctuated phrase ("cheese pizza" is one item).

const CUES: &[(&str, bool)] = &[
    ("i need these in order", true),
    ("we need these in order", true),
    ("i need the following items", true),
    ("we need the following items", true),
    ("i need the following", true),
    ("we need the following", true),
    ("here are the items", true),
    ("here is my list", true),
    ("here's my list", true),
    ("here’s my list", true),
    ("here is the list", true),
    ("here's the list", true),
    ("the following items", true),
    ("the following steps", true),
    ("my grocery list is", true),
    ("my shopping list is", true),
    ("grocery list is", true),
    ("shopping list is", true),
    ("packing list is", true),
    ("to-do list is", true),
    ("grocery list", true),
    ("shopping list", true),
    ("packing list", true),
    ("to-do list", true),
    ("todo list", true),
    ("my list is", true),
    ("i need to buy", false),
    ("we need to buy", false),
    ("i want to buy", false),
    ("i have to buy", false),
    ("please pick up", false),
    ("please buy", false),
    ("i need", false),
    ("we need", false),
];
const ORDINALS: &[&str] = &[
    "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth",
];

fn word_boundary(text: &str, start: usize, end: usize) -> bool {
    !text[..start]
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric)
        && !text[end..]
            .chars()
            .next()
            .is_some_and(char::is_alphanumeric)
}

fn intro(text: &str) -> Option<(usize, bool)> {
    let lower = text.to_ascii_lowercase();
    CUES.iter()
        .flat_map(|&(cue, explicit)| {
            lower.match_indices(cue).filter_map(move |(start, _)| {
                let end = start + cue.len();
                word_boundary(text, start, end).then_some((start, end, explicit))
            })
        })
        .min_by_key(|&(start, end, _)| (start, std::cmp::Reverse(end)))
        .map(|(_, end, explicit)| (end, explicit))
}

fn trim_item(text: &str) -> &str {
    text.trim().trim_matches([',', ';', '.', ':']).trim()
}
fn clause(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    // These indicate prose rather than short noun/action items. Explicit
    // first/second markers handle full-sentence tasks separately.
    [
        "i ",
        "i'm ",
        "i’m ",
        "i'll ",
        "i’ll ",
        "i've ",
        "we ",
        "we're ",
        "we’ll ",
        "you ",
        "he ",
        "she ",
        "they ",
        "it ",
        "it's ",
        "it’s ",
        "this ",
        "that ",
        "there ",
        "but ",
        "because ",
        "although ",
        "if ",
        "when ",
        "which ",
        "so ",
        "then ",
        "however",
        "not ",
        "no ",
        "without ",
        "actually",
        "honestly",
        "in fact",
        "to be ",
        "to go ",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
        || lower.split_whitespace().any(|word| {
            matches!(
                word,
                "is" | "are" | "was" | "were" | "because" | "although" | "whereas"
            )
        })
}

/// Ignore separators inside quotes/parentheses, decimal prices and digit groups.
fn split_items(body: &str, periods: bool) -> Option<Vec<&str>> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quote = false;
    let mut depth = 0usize;
    for (index, c) in body.char_indices() {
        match c {
            '"' | '“' | '”' => {
                quote = !quote;
                continue;
            }
            '(' | '[' | '{' if !quote => {
                depth += 1;
                continue;
            }
            ')' | ']' | '}' if !quote => {
                depth = depth.checked_sub(1)?;
                continue;
            }
            _ => {}
        }
        if quote || depth > 0 {
            continue;
        }
        let before = body[..index].chars().next_back();
        let after = body[index + c.len_utf8()..].chars().next();
        let digit_separator =
            before.is_some_and(|c| c.is_ascii_digit()) && after.is_some_and(|c| c.is_ascii_digit());
        let terminal_period = c == '.'
            && periods
            && after.is_some_and(char::is_whitespace)
            && !is_abbreviation(&body[..index]);
        if !digit_separator && (c == ',' || c == ';' || terminal_period) {
            let part = trim_item(&body[start..index]);
            if part.is_empty() {
                return None;
            }
            parts.push(part);
            start = index + c.len_utf8();
        }
    }
    if quote || depth != 0 {
        return None;
    }
    let tail = trim_item(&body[start..]);
    if tail.is_empty() {
        return None;
    }
    parts.push(tail);
    Some(parts)
}

fn is_abbreviation(before: &str) -> bool {
    let word = before.split_whitespace().next_back().unwrap_or("");
    word.chars().count() == 1
        || ["dr", "mr", "mrs", "ms", "st", "vs", "etc", "e.g", "i.e"]
            .contains(&word.to_ascii_lowercase().as_str())
}

fn ordered_items(body: &str) -> Option<Vec<&str>> {
    let lower = body.to_ascii_lowercase();
    if !lower.starts_with("first") || !word_boundary(body, 0, 5) {
        return None;
    }
    let mut parts = Vec::new();
    let mut start = 5;
    for &ordinal in &ORDINALS[1..] {
        let next = lower[start..]
            .match_indices(ordinal)
            .find_map(|(offset, _)| {
                let index = start + offset;
                word_boundary(body, index, index + ordinal.len()).then_some(index)
            });
        let Some(index) = next else { break };
        let item = trim_item(&body[start..index]);
        if item.is_empty() {
            return None;
        }
        parts.push(item);
        start = index + ordinal.len();
    }
    parts.push(trim_item(&body[start..]));
    if parts.len() < 2 || parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    // An incomplete/out-of-order sequence is not a trustworthy list boundary.
    if parts.iter().any(|part| {
        part.split_whitespace().any(|word| {
            ORDINALS.contains(
                &word
                    .trim_matches([',', '.', ':'])
                    .to_ascii_lowercase()
                    .as_str(),
            )
        })
    }) {
        return None;
    }
    Some(parts)
}

fn split_final_and<'a>(parts: &mut Vec<&'a str>) {
    let Some(last) = parts.last().copied() else {
        return;
    };
    let lower = last.to_ascii_lowercase();
    if lower.starts_with("and ") {
        *parts.last_mut().unwrap() = last[4..].trim();
        return;
    }
    if last.contains(['"', '“', '”', '(', ')', '[', ']']) {
        return;
    }
    // Conventional compound items keep their internal conjunction. Commas or
    // first/second markers always let a speaker make boundaries unambiguous.
    if [
        "mac and cheese",
        "macaroni and cheese",
        "peanut butter and jelly",
        "salt and pepper",
        "fish and chips",
        "research and development",
        "black and white",
        "arts and crafts",
        "health and safety",
        "bread and butter",
        "ham and cheese",
        "bacon and eggs",
        "rock and roll",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
    {
        return;
    }
    if lower.matches(" and ").count() == 1 {
        let index = lower.find(" and ").unwrap();
        let (left, right) = (last[..index].trim(), last[index + 5..].trim());
        if !left.is_empty() && !right.is_empty() && !clause(left) && !clause(right) {
            *parts.last_mut().unwrap() = left;
            parts.push(right);
        }
    }
}

/// Return a complete replacement only when an enumeration has clear boundaries.
/// Introductory prose and any following sentence stay outside the bullets.
pub fn automatic_list(transcript: &str) -> Option<String> {
    let text = transcript.trim();
    if text.is_empty() || text.len() > 64 * 1024 || text.contains(['\n', '\r', '•', '`']) {
        return None;
    }
    let (prefix, body, explicit) = if let Some((end, explicit)) = intro(text) {
        (
            text[..end].trim(),
            text[end..].trim_start_matches(|c: char| c.is_whitespace() || ":.,;-".contains(c)),
            explicit,
        )
    } else {
        ("", text, false)
    };
    if body.is_empty() {
        return None;
    }
    let mut suffix = "";
    let mut list_body = body;
    // Do not swallow a normal follow-up sentence into the final list item.
    for (index, _) in body.match_indices(". ") {
        if !is_abbreviation(&body[..index]) && clause(&body[index + 2..]) {
            list_body = &body[..index];
            suffix = body[index + 2..].trim();
            break;
        }
    }
    let ordered = ordered_items(list_body);
    let is_ordered = ordered.is_some();
    let mut items = match ordered {
        Some(items) => items,
        None => {
            let parts = split_items(list_body, false)?;
            if parts.len() < 2 && explicit {
                split_items(list_body, true)?
            } else {
                parts
            }
        }
    };
    if !is_ordered {
        split_final_and(&mut items);
    }
    let minimum = if prefix.is_empty() && !is_ordered {
        3
    } else {
        2
    };
    if items.len() < minimum || items.len() > 50 {
        return None;
    }
    if items.iter().any(|item| {
        item.is_empty()
            || item.len() > 1000
            || matches!(item.to_ascii_lowercase().as_str(), "and" | "or")
    }) {
        return None;
    }
    if !is_ordered
        && items.iter().any(|item| {
            item.split_whitespace().count() > if explicit { 16 } else { 7 }
                || clause(item)
                || item.contains(['?', '!'])
                || item.contains(". ") && !is_abbreviation(item.split(". ").next().unwrap_or(""))
        })
    {
        return None;
    }
    // Bare number chunks may be numeric entry or an address/postcode. Require
    // an introduction before interpreting those mixed comma groups as a list.
    if prefix.is_empty()
        && !is_ordered
        && items
            .iter()
            .any(|item| super::numbers::digit_sequence(item).is_some())
    {
        return None;
    }
    let mut output = String::new();
    if !prefix.is_empty() {
        output.push_str(prefix.trim_end_matches(['.', ',', ';', ':']));
        output.push_str(":\n");
    }
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        output.push_str("• ");
        output.push_str(item);
    }
    if !suffix.is_empty() {
        output.push_str("\n\n");
        output.push_str(suffix);
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grocery_example_keeps_intro_order_and_multiword_items() {
        assert_eq!(automatic_list("I'm going grocery shopping. I need these in order. Eggs, milk, bread, cheese pizza.").unwrap(),
            "I'm going grocery shopping. I need these in order:\n• Eggs\n• milk\n• bread\n• cheese pizza");
        assert_eq!(
            automatic_list("I need eggs, milk, bread and cheese pizza.").unwrap(),
            "I need:\n• eggs\n• milk\n• bread\n• cheese pizza"
        );
        assert_eq!(
            automatic_list("Eggs, milk, bread, cheese pizza.").unwrap(),
            "• Eggs\n• milk\n• bread\n• cheese pizza"
        );
    }
    #[test]
    fn lists_accept_punctuation_and_explicit_spoken_order() {
        for text in [
            "Grocery list: eggs; milk; cheese pizza.",
            "Grocery list. eggs. milk. cheese pizza.",
            "Grocery list first eggs second milk third cheese pizza.",
        ] {
            assert_eq!(
                automatic_list(text).unwrap(),
                "Grocery list:\n• eggs\n• milk\n• cheese pizza",
                "{text}"
            );
        }
        assert_eq!(automatic_list("First, call Alex about the invoice. Second, send the quote. Third, book the venue.").unwrap(), "• call Alex about the invoice\n• send the quote\n• book the venue");
    }
    #[test]
    fn preserves_compounds_quantities_names_and_repeated_items() {
        assert_eq!(
            automatic_list("I need eggs, milk, mac and cheese.").unwrap(),
            "I need:\n• eggs\n• milk\n• mac and cheese"
        );
        assert_eq!(
            automatic_list("I need 2 eggs, 1.5 liters of milk, and 3 cheese pizzas.").unwrap(),
            "I need:\n• 2 eggs\n• 1.5 liters of milk\n• 3 cheese pizzas"
        );
        assert_eq!(
            automatic_list("I need iPhone cases, café beans, and café beans.").unwrap(),
            "I need:\n• iPhone cases\n• café beans\n• café beans"
        );
        assert_eq!(
            automatic_list("Shopping list: eggs (large, organic), milk, bread.").unwrap(),
            "Shopping list:\n• eggs (large, organic)\n• milk\n• bread"
        );
        assert_eq!(
            automatic_list("I need eggs, milk, bread. I'll go after work.").unwrap(),
            "I need:\n• eggs\n• milk\n• bread\n\nI'll go after work."
        );
        assert_eq!(
            automatic_list("Shopping list: eggs, milk, \"red and blue\".").unwrap(),
            "Shopping list:\n• eggs\n• milk\n• \"red and blue\""
        );
        assert_eq!(
            automatic_list("I need 1,234 labels, 2 boxes, 1.5 liters of milk.").unwrap(),
            "I need:\n• 1,234 labels\n• 2 boxes\n• 1.5 liters of milk"
        );
    }
    #[test]
    fn ordinary_prose_ambiguous_boundaries_and_existing_lists_stay_unchanged() {
        for text in [
            "Hello, John",
            "Actually, I think, we should wait.",
            "I need to go home, but first I should call Alex.",
            "If you need eggs, milk, bread, just ask.",
            "My grocery list is empty.",
            "I need these in order eggs milk bread cheese pizza.",
            "I need eggs, milk, and",
            "I need eggs,, milk, bread",
            "I need eggs (large, milk, bread",
            "• eggs\n• milk",
            "1. eggs\n2. milk",
            "one, five, four",
            "First eggs third milk second bread",
            "New York, New York, 10001",
            "I need her, not him.",
        ] {
            assert!(
                automatic_list(text).is_none(),
                "unexpected list: {text}: {:?}",
                automatic_list(text)
            );
        }
    }
    #[test]
    fn formatting_is_idempotent() {
        let output = automatic_list("I need eggs, milk, bread.").unwrap();
        assert!(automatic_list(&output).is_none());
    }
}
