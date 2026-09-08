// Deterministic replacement rules applied BEFORE the LLM (§7.6 place 1),
// plus the preferred-spellings block injected INTO the prompt (place 2).
use super::{Entry, VocabularySnapshot};

/// Single pass, personal entries outrank shared, one replacement per term
/// (spec failure-mode notes: no rule loops, no chaining).
pub fn apply_replacements(transcript: &str, entries: &[Entry]) -> String {
    let snapshot = snapshot_from_entries(entries);
    apply_snapshot(transcript, &snapshot)
}

pub fn snapshot_from_entries(entries: &[Entry]) -> VocabularySnapshot {
    let mut canonical = Vec::new();
    let mut aliases = Vec::new();
    for e in entries {
        if e.snippet.is_some() {
            continue;
        }
        let target = e.replacement.as_deref().unwrap_or(&e.term).trim();
        if target.is_empty() {
            continue;
        }
        if !canonical
            .iter()
            .any(|x: &String| x.eq_ignore_ascii_case(target))
        {
            canonical.push(target.to_string());
        }
        if e.term != target {
            aliases.push((e.term.clone(), target.to_string()));
        }
    }
    VocabularySnapshot {
        version: 0,
        canonical,
        aliases,
    }
}

/// Replace aliases in one left-to-right pass over the source.  Matching is
/// case insensitive, requires token boundaries, and prefers the longest
/// alias. Generated replacements are never rescanned.
pub fn apply_snapshot(transcript: &str, snapshot: &VocabularySnapshot) -> String {
    let chars: Vec<char> = transcript.chars().collect();
    let mut rules: Vec<(Vec<char>, &str)> = snapshot
        .aliases
        .iter()
        .filter_map(|(alias, target)| {
            let lowered: Vec<char> = alias.chars().flat_map(|c| c.to_lowercase()).collect();
            (lowered.len() == alias.chars().count()).then_some((lowered, target.as_str()))
        })
        .collect();
    for term in &snapshot.canonical {
        let lowered: Vec<char> = term.chars().flat_map(|c| c.to_lowercase()).collect();
        if lowered.len() == term.chars().count() {
            rules.push((lowered, term.as_str()));
        }
    }
    let mut out = String::with_capacity(transcript.len());
    let mut i = 0;
    while i < chars.len() {
        let mut best: Option<(&str, usize)> = None;
        for (a, target) in &rules {
            if a.is_empty() || i + a.len() > chars.len() {
                continue;
            }
            if !boundary_before(&chars, i) || !boundary_after(&chars, i + a.len()) {
                continue;
            }
            if a.iter()
                .enumerate()
                .all(|(n, c)| chars[i + n].to_lowercase().eq(c.to_lowercase()))
            {
                if best.map_or(true, |(_, n)| a.len() > n) {
                    best = Some((target, a.len()));
                }
            }
        }
        if let Some((target, len)) = best {
            out.push_str(target);
            i += len;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn boundary_before(chars: &[char], i: usize) -> bool {
    i == 0 || !chars[i - 1].is_alphanumeric() && chars[i - 1] != '_'
}
fn boundary_after(chars: &[char], i: usize) -> bool {
    i == chars.len() || !chars[i].is_alphanumeric() && chars[i] != '_'
}

/// "Preferred spellings: Supabase, Okafor, Parla" block for the envelope.
pub fn preferred_spellings_block(entries: &[Entry]) -> Option<Vec<String>> {
    let mut terms = Vec::new();
    for e in entries.iter().filter(|e| e.snippet.is_none()) {
        let target = e.replacement.as_deref().unwrap_or(&e.term).trim();
        if !target.is_empty()
            && !terms
                .iter()
                .any(|x: &String| x.eq_ignore_ascii_case(target))
        {
            terms.push(target.to_string());
        }
    }
    if terms.is_empty() {
        None
    } else {
        Some(terms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_rule_terms() {
        let entries = vec![Entry {
            term: "super base".into(),
            replacement: Some("Supabase".into()),
            snippet: None,
        }];
        assert_eq!(
            apply_replacements("email okafor about super base", &entries),
            "email okafor about Supabase"
        );
    }

    #[test]
    fn no_loops_on_identity() {
        let entries = vec![Entry {
            term: "foo".into(),
            replacement: Some("foo".into()),
            snippet: None,
        }];
        assert_eq!(apply_replacements("foo foo", &entries), "foo foo");
    }

    #[test]
    fn longest_case_insensitive_boundary_match_is_non_cascading() {
        let entries = vec![
            Entry {
                term: "cloud".into(),
                replacement: Some("Claude".into()),
                snippet: None,
            },
            Entry {
                term: "cloud nine".into(),
                replacement: Some("Cloud Nine".into()),
                snippet: None,
            },
        ];
        assert_eq!(
            apply_replacements("CLOUD NINE; cloudy", &entries),
            "Cloud Nine; cloudy"
        );
    }
    #[test]
    fn canonical_terms_and_unicode_boundaries() {
        let entries = vec![
            Entry {
                term: "Claude".into(),
                replacement: None,
                snippet: None,
            },
            Entry {
                term: "Parakeet".into(),
                replacement: None,
                snippet: None,
            },
            Entry {
                term: "claw ed".into(),
                replacement: Some("Claude".into()),
                snippet: None,
            },
            Entry {
                term: "İ".into(),
                replacement: Some("Iota".into()),
                snippet: None,
            },
        ];
        let s = snapshot_from_entries(&entries);
        assert_eq!(
            apply_snapshot("claude PARAKEET claw ed cloudy", &s),
            "Claude Parakeet Claude cloudy"
        );
        assert_eq!(
            preferred_spellings_block(&entries).unwrap(),
            vec!["Claude", "Parakeet", "Iota"]
        );
    }
}
