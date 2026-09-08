// Dictionary management CLI: parla add <term> [= replacement] | list | remove.
// Operates on the SAME LOCALAPPDATA\Parla\dictionary.sqlite the live pipeline
// reads per-utterance, so changes take effect on the next dictation without a
// restart. Never touches mic, ASR server, or injection.

use crate::dictionary::{Dictionary, Entry};

pub fn dict_store_path() -> String {
    format!(
        "{}\\Parla\\dictionary.sqlite",
        std::env::var("LOCALAPPDATA").unwrap_or_default()
    )
}

/// Handle dictionary verbs. Returns Some(exit code) when `args` is a
/// dictionary command; None falls through to normal startup.
pub fn run(args: &[String]) -> Option<i32> {
    match args.get(1)?.as_str() {
        "list" => Some(cmd_list()),
        "add" => Some(cmd_add(&args[2..])),
        "remove" => Some(cmd_remove(args.get(2).map(String::as_str))),
        _ => None,
    }
}

/// Parse everything after `add`: words joined with spaces, split once on '='.
/// "super base = Supabase" -> ("super base", Some("Supabase")); bare term has
/// no replacement (hotword-only). Empty term -> None.
fn parse_add(rest: &[String]) -> Option<(String, Option<String>)> {
    let joined = rest.join(" ");
    let (term, repl) = match joined.split_once('=') {
        Some((t, r)) => (t.trim(), Some(r.trim().to_string())),
        None => (joined.trim(), None),
    };
    if term.is_empty() {
        None
    } else {
        Some((term.to_string(), repl))
    }
}

fn open_store() -> Result<Dictionary, i32> {
    Dictionary::open(&dict_store_path()).map_err(|e| {
        eprintln!("[parla] {e}");
        1
    })
}

fn cmd_add(rest: &[String]) -> i32 {
    let Some((term, replacement)) = parse_add(rest) else {
        eprintln!("usage: parla add <term> [= replacement]");
        return 2;
    };
    let dict = match open_store() {
        Ok(d) => d,
        Err(code) => return code,
    };
    match dict.add_entry(&Entry {
        term,
        replacement,
        snippet: None,
    }) {
        Ok(()) => {
            println!("[parla] added. {} entries now.", dict.all_entries().len());
            0
        }
        Err(e) => {
            eprintln!("[parla] {e}");
            1
        }
    }
}

fn cmd_list() -> i32 {
    let dict = match open_store() {
        Ok(d) => d,
        Err(code) => return code,
    };
    let entries = dict.all_entries();
    println!("[parla] {} dictionary entries:", entries.len());
    for e in entries {
        match (&e.replacement, &e.snippet) {
            (Some(r), _) => println!("  {} -> {}", e.term, r),
            (None, Some(s)) => println!("  {} [snippet]", s.chars().take(40).collect::<String>()),
            (None, None) => println!("  {}", e.term),
        }
    }
    0
}

fn cmd_remove(term: Option<&str>) -> i32 {
    let Some(term) = term else {
        eprintln!("usage: parla remove <term>");
        return 2;
    };
    let dict = match open_store() {
        Ok(d) => d,
        Err(code) => return code,
    };
    match dict.remove_entry(term) {
        Ok(true) => {
            println!("[parla] removed '{term}'.");
            0
        }
        Ok(false) => {
            eprintln!("[parla] '{term}' not found.");
            3
        }
        Err(e) => {
            eprintln!("[parla] {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parses_rule_with_quoted_multword_term() {
        // shell: parla add "super base" = Supabase
        let p = parse_add(&s(&["super base", "=", "Supabase"])).unwrap();
        assert_eq!(p, ("super base".to_string(), Some("Supabase".to_string())));
    }

    #[test]
    fn parses_bare_hotword_and_unquoted_rule() {
        assert_eq!(
            parse_add(&s(&["Supabase"])).unwrap(),
            ("Supabase".to_string(), None)
        );
        // unquoted still works if the term itself has no '=' inside
        assert_eq!(
            parse_add(&s(&["okafor", "=", "Okafor"])).unwrap(),
            ("okafor".to_string(), Some("Okafor".to_string()))
        );
    }

    #[test]
    fn rejects_empty_term_but_keeps_empty_replacement_guard() {
        assert!(parse_add(&[]).is_none());
        assert!(parse_add(&s(&["=", "X"])).is_none());
    }
}
