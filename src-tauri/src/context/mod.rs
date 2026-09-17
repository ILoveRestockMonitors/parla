// Context awareness v1: frontmost executable -> app_category for the §7.2
// envelope. Pure mapping below is unit-tested; the Win32 probe reads the
// foreground window's process image name.
pub mod target;
pub mod windows;

/// What to do with the first word of an insertion, given the text before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeamMode {
    Capitalize,
    Lowercase,
}

/// Decide seam handling from the characters preceding the caret (§7.4 rule 5,
/// applied deterministically instead of trusting the LLM).
pub fn seam_mode_for(text_before: Option<&str>) -> Option<SeamMode> {
    let before = text_before?;
    let trimmed = before.trim_end_matches([' ', '\t']);
    let last = trimmed.chars().last()?;
    match last {
        '.' | '!' | '?' | ':' => Some(SeamMode::Capitalize),
        ',' | ';' => Some(SeamMode::Lowercase),
        c if c.is_alphanumeric() => Some(SeamMode::Lowercase),
        _ => None, // open quote/paren/dash: leave whatever the LLM chose
    }
}

/// Apply a seam decision to the head of an insertion.
pub fn apply_seam(text: &str, mode: Option<SeamMode>) -> String {
    let mut out = text.to_string();
    match mode {
        None => {}
        Some(SeamMode::Capitalize) => {
            if let Some(c) = out.chars().next() {
                let upper: String = c.to_uppercase().collect();
                out.replace_range(0..c.len_utf8(), &upper);
            }
        }
        Some(SeamMode::Lowercase) => {
            if let Some(c) = out.chars().next() {
                let lower: String = c.to_lowercase().collect();
                out.replace_range(0..c.len_utf8(), &lower);
            }
        }
    }
    out
}

/// Keep only the trailing `max` chars (for stuffing into the LLM envelope).
pub fn tail_chars(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        s.to_string()
    } else {
        s.chars().skip(count - max).collect()
    }
}

/// Browser-wide insertion policy, independent of the formatter's categories.
/// Match image basenames so Electron apps such as Codex remain writing apps.
pub fn is_browser_exe(exe: &str) -> bool {
    let name = exe.rsplit(['\\', '/']).next().unwrap_or(exe);
    [
        "chrome.exe",
        "msedge.exe",
        "firefox.exe",
        "brave.exe",
        "opera.exe",
        "vivaldi.exe",
        "chromium.exe",
        "arc.exe",
        "zen.exe",
        "waterfox.exe",
        "floorp.exe",
        "librewolf.exe",
        "iexplore.exe",
    ]
    .iter()
    .any(|browser| name.eq_ignore_ascii_case(browser))
}

/// Known native terminal hosts. Match basenames, not command or window titles.
pub fn is_terminal_exe(exe: &str) -> bool {
    let name = exe.rsplit(['\\', '/']).next().unwrap_or(exe);
    ["WindowsTerminal.exe", "conhost.exe", "OpenConsole.exe"]
        .iter()
        .any(|terminal| name.eq_ignore_ascii_case(terminal))
}

/// Map an executable to the formatter's app category.
pub fn category_for_exe(exe: &str) -> &'static str {
    let e = exe.to_ascii_lowercase();

    // Code editors / IDEs first (VS Code is Electron too - must precede chat matches)
    if [
        "code.exe",
        "cursor.exe",
        "devenv.exe",
        "sublime_text.exe",
        "goland64.exe",
        "rider64.exe",
        "pycharm64.exe",
        "webstorm64.exe",
        "idea64.exe",
        "clion64.exe",
    ]
    .iter()
    .any(|k| e.ends_with(k))
    {
        return "code";
    }
    if ["vim.exe", "nvim.exe", "notepad++.exe"]
        .iter()
        .any(|k| e.contains(k))
    {
        return "code";
    }

    // Chat
    if [
        "slack.exe",
        "discord.exe",
        "teams.exe",
        "telegram.exe",
        "signal.exe",
        "whatsapp.exe",
    ]
    .iter()
    .any(|k| e.contains(k))
    {
        return "work_chat";
    }

    // Email clients
    if e.contains("outlook.exe") || e.contains("thunderbird.exe") || e.contains("mailbird.exe") {
        return "email";
    }

    // Docs
    if e.contains("winword.exe")
        || e.contains("notion.exe")
        || e.contains("obsidian.exe")
        || e.contains("typora.exe")
    {
        return "docs";
    }

    if e.contains("notepad.exe") {
        return "docs";
    }

    // Browsers and everything else
    "other"
}

#[cfg(test)]
mod tests {
    use super::{apply_seam, category_for_exe as cat, seam_mode_for, SeamMode};

    #[test]
    fn terminal_policy_uses_exact_host_image_names() {
        for exe in [
            "WindowsTerminal.exe",
            "CONHOST.EXE",
            "OpenConsole.exe",
            r"C:\Windows\System32\conhost.exe",
        ] {
            assert!(super::is_terminal_exe(exe), "{exe}");
        }
        for exe in [
            "Code.exe",
            "Codex.exe",
            "hermes.exe",
            "chrome.exe",
            "notconhost.exe",
            "",
            r"C:\conhost.exe\editor.exe",
        ] {
            assert!(!super::is_terminal_exe(exe), "{exe}");
        }
    }

    #[test]
    fn browser_policy_matches_exact_image_names_and_paths() {
        for exe in [
            "chrome.exe",
            "MSEDGE.EXE",
            "firefox.exe",
            "brave.exe",
            "opera.exe",
            "vivaldi.exe",
            "chromium.exe",
            "Arc.exe",
            "zen.exe",
            "waterfox.exe",
            "floorp.exe",
            "librewolf.exe",
            "iexplore.exe",
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            "C:/Program Files/Mozilla Firefox/firefox.exe",
        ] {
            assert!(super::is_browser_exe(exe), "{exe}");
        }
        for exe in [
            "Codex.exe",
            "Code.exe",
            "electron.exe",
            "notchrome.exe",
            "",
            r"C:\chrome.exe\Codex.exe",
        ] {
            assert!(!super::is_browser_exe(exe), "{exe}");
        }
    }

    #[test]
    fn seam_rules() {
        assert_eq!(
            seam_mode_for(Some("end of sentence. ")),
            Some(SeamMode::Capitalize)
        );
        assert_eq!(seam_mode_for(Some("wait:")), Some(SeamMode::Capitalize));
        assert_eq!(seam_mode_for(Some("and then, ")), Some(SeamMode::Lowercase));
        assert_eq!(seam_mode_for(Some("mid word")), Some(SeamMode::Lowercase));
        assert_eq!(seam_mode_for(Some("open quote \"")), None);
        assert_eq!(seam_mode_for(Some("")), None);
        assert_eq!(seam_mode_for(None), None);
    }

    #[test]
    fn seam_apply() {
        assert_eq!(
            apply_seam("the quick", Some(SeamMode::Capitalize)),
            "The quick"
        );
        assert_eq!(
            apply_seam("Quick brown", Some(SeamMode::Lowercase)),
            "quick brown"
        );
        assert_eq!(apply_seam("unchanged", None), "unchanged");
    }

    #[test]
    fn tail_chars_works() {
        assert_eq!(super::tail_chars("abcdef", 3), "def");
        assert_eq!(super::tail_chars("ab", 3), "ab");
    }

    #[test]
    fn known_apps_map() {
        assert_eq!(cat("Code.exe"), "code");
        assert_eq!(cat("CURSOR.EXE"), "code");
        assert_eq!(cat("Slack.exe"), "work_chat");
        assert_eq!(cat("Discord.exe"), "work_chat");
        assert_eq!(cat("OUTLOOK.EXE"), "email");
        assert_eq!(cat("Obsidian.exe"), "docs");
        assert_eq!(cat("Notepad.exe"), "docs");
    }

    #[test]
    fn unknown_and_browsers_are_other() {
        assert_eq!(cat("chrome.exe"), "other");
        assert_eq!(cat("msedge.exe"), "other");
        assert_eq!(cat("firefox.exe"), "other");
        assert_eq!(cat("totally_new_app.exe"), "other");
        assert_eq!(cat(""), "other");
    }

    #[test]
    fn full_paths_reduce_to_name() {
        // callers may pass full image paths; contains-matching still works
        assert_eq!(
            cat(r"C:\Users\x\AppData\Local\slack\slack.exe"),
            "work_chat"
        );
        assert_eq!(cat(r"D:\tools\Microsoft VS Code\Code.exe"), "code");
    }
}
