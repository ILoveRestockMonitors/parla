//! Browsers and terminals use prose: Shift+Enter can navigate or submit input.
use crate::context::{category_for_exe, is_browser_exe, is_terminal_exe};
use std::borrow::Cow;

pub fn automatic_text(text: &str, app: Option<&str>) -> Option<String> {
    if let Some(digits) = super::numbers::digit_sequence(text) {
        return Some(digits);
    }
    if app.is_some_and(|exe| {
        is_browser_exe(exe) || is_terminal_exe(exe) || category_for_exe(exe) == "code"
    }) {
        return None;
    }
    super::lists::automatic_list(text)
}

/// Apply after polishing and again to the actual insertion target. This also
/// covers recovered text and explicit commands, which can bypass formatting.
pub fn for_app<'a>(text: &'a str, app: Option<&str>) -> Cow<'a, str> {
    if app.is_some_and(|exe| is_browser_exe(exe) || is_terminal_exe(exe)) {
        single_line(text)
    } else {
        Cow::Borrowed(text)
    }
}

fn line_separator(c: char) -> bool {
    c.is_whitespace() && c != ' '
}

/// Preserve ordinary sentences exactly; flatten paragraphs and turn bullet
/// lines into comma-separated items without changing their wording or order.
pub fn single_line(text: &str) -> Cow<'_, str> {
    if !text.contains(line_separator) {
        return Cow::Borrowed(text);
    }
    let mut output = String::new();
    let mut previous_item = false;
    for line in text
        .split(line_separator)
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let item = line
            .strip_prefix('•')
            .map(str::trim_start)
            .or_else(|| line.strip_prefix("- "))
            .or_else(|| line.strip_prefix("* "));
        if !output.is_empty() {
            if previous_item && item.is_some() && !output.ends_with([',', ';', '.', '!', '?']) {
                output.push(',');
            }
            output.push(' ');
        }
        output.push_str(item.unwrap_or(line));
        previous_item = item.is_some();
    }
    // Preserve the commit layer's trailing word separator.
    if !output.is_empty() && text.ends_with(char::is_whitespace) {
        output.push(' ');
    }
    Cow::Owned(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    const SPOKEN: &str = "I need these in order. Cheese pizza, eggs, broccoli, potatoes.";
    const LIST: &str = "I need these in order:\n• Cheese pizza\n• Eggs\n• Broccoli\n• Potatoes";

    #[test]
    fn terminals_keep_prose_and_digits_and_flatten_recovered_lists() {
        for app in ["WindowsTerminal.exe", "conhost.exe", "OpenConsole.exe"] {
            assert_eq!(automatic_text(SPOKEN, Some(app)), None);
            assert_eq!(for_app(SPOKEN, Some(app)), SPOKEN);
            assert_eq!(
                automatic_text("zero zero seven", Some(app)).as_deref(),
                Some("007")
            );
            assert_eq!(
                for_app(LIST, Some(app)),
                "I need these in order: Cheese pizza, Eggs, Broccoli, Potatoes"
            );
        }
    }

    #[test]
    fn browsers_keep_grocery_sentences_and_numeric_entry() {
        for app in ["chrome.exe", "msedge.exe", "firefox.exe", "brave.exe"] {
            assert_eq!(automatic_text(SPOKEN, Some(app)), None);
            assert_eq!(for_app(SPOKEN, Some(app)), SPOKEN);
            assert_eq!(
                automatic_text("sixty seven two four zero nine eight", Some(app)).as_deref(),
                Some("6724098")
            );
        }
    }

    #[test]
    fn codex_and_writing_apps_keep_bullets_while_code_stays_unchanged() {
        for app in [Some("Codex.exe"), Some("notepad.exe"), None] {
            assert!(automatic_text(SPOKEN, app)
                .unwrap()
                .contains("\n• Cheese pizza"));
            assert_eq!(for_app(LIST, app), LIST);
        }
        assert_eq!(automatic_text(SPOKEN, Some("Code.exe")), None);
    }

    #[test]
    fn polished_or_recovered_bullets_become_one_line_in_browsers() {
        assert_eq!(
            for_app(LIST, Some("chrome.exe")),
            "I need these in order: Cheese pizza, Eggs, Broccoli, Potatoes"
        );
        assert_eq!(
            single_line("Shopping list:\r\n- eggs\r\n- milk\r\n- cheese pizza "),
            "Shopping list: eggs, milk, cheese pizza "
        );
        assert_eq!(
            single_line("First sentence.\n\nSecond sentence."),
            "First sentence. Second sentence."
        );
        assert_eq!(
            single_line("Café\t🛒\u{85}eggs\u{2028}milk\u{2029}bread"),
            "Café 🛒 eggs milk bread"
        );
        assert_eq!(
            single_line("I need 2 eggs, 1.5 liters of milk. "),
            "I need 2 eggs, 1.5 liters of milk. "
        );
    }
}
