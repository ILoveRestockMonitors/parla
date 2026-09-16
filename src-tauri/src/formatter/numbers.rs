//! Automatic numeric entry for utterances made entirely of number phrases.
//! Parse natural cardinal chunks, then concatenate them as a digit sequence.
//! Prose, ambiguous homophones, signs and decimals stay on the normal path.

#[derive(Clone)]
struct Number {
    digits: String,
    // Leading-zero groups and integers too large for arithmetic are still
    // valid code fragments. Preserve them verbatim instead of parsing them.
    value: Option<u64>,
    spoken: bool,
}

enum Token {
    Number(Number),
    Hundred,
    Scale(u64),
    And,
}

fn small_word(word: &str) -> Option<u64> {
    Some(match word {
        "zero" | "oh" => 0,
        "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        "eleven" => 11,
        "twelve" => 12,
        "thirteen" => 13,
        "fourteen" => 14,
        "fifteen" => 15,
        "sixteen" => 16,
        "seventeen" => 17,
        "eighteen" => 18,
        "nineteen" => 19,
        "twenty" => 20,
        "thirty" => 30,
        "forty" => 40,
        "fifty" => 50,
        "sixty" => 60,
        "seventy" => 70,
        "eighty" => 80,
        "ninety" => 90,
        _ => return None,
    })
}

fn is_tens(value: u64) -> bool {
    (20..=90).contains(&value) && value % 10 == 0
}

fn spoken_number(value: u64) -> Token {
    Token::Number(Number {
        digits: value.to_string(),
        value: Some(value),
        spoken: true,
    })
}

/// A natural tens/unit phrase forms one chunk ("sixty seven" -> 67).
/// Already numeric groups retain their boundaries ("60 7" / "60 seven" ->
/// 607). Only a spelled-out tens prefix can absorb the following unit.
fn small_chunk(tokens: &[Token], start: usize) -> Option<(Number, usize)> {
    let Token::Number(first) = tokens.get(start)? else {
        return None;
    };
    if let (Some(tens), Some(Token::Number(unit))) = (first.value, tokens.get(start + 1)) {
        if is_tens(tens) && unit.value.is_some_and(|v| (1..=9).contains(&v)) && first.spoken {
            let value = tens + unit.value?;
            return Some((
                Number {
                    digits: value.to_string(),
                    value: Some(value),
                    spoken: true,
                },
                start + 2,
            ));
        }
    }
    Some((first.clone(), start + 1))
}

/// Parse a small chunk with an optional hundred multiplier and remainder.
fn group(tokens: &[Token], start: usize) -> Option<(Number, usize)> {
    let (number, mut end) = small_chunk(tokens, start)?;
    if !matches!(tokens.get(end), Some(Token::Hundred)) {
        return Some((number, end));
    }
    let multiplier = number.value?;
    if !(1..=99).contains(&multiplier) {
        return None;
    }
    let mut value = multiplier * 100;
    end += 1;
    let conjunction = matches!(tokens.get(end), Some(Token::And));
    if conjunction {
        end += 1;
    }
    if matches!(tokens.get(end), Some(Token::Number(_))) {
        let (remainder, next) = small_chunk(tokens, end)?;
        let remainder = remainder.value?;
        if remainder >= 100 {
            return None;
        }
        value += remainder;
        end = next;
    } else if conjunction {
        return None;
    }
    Some((
        Number {
            digits: value.to_string(),
            value: Some(value),
            spoken: true,
        },
        end,
    ))
}

/// Descending scales form one cardinal: "sixty nine thousand four hundred
/// twenty" -> 69420. Invalid grammar is rejected, never partially converted.
fn cardinal_chunk(tokens: &[Token], start: usize) -> Option<(String, usize)> {
    let (mut current, mut end) = group(tokens, start)?;
    if !matches!(tokens.get(end), Some(Token::Scale(_))) {
        return Some((current.digits, end));
    }
    let mut total = 0u64;
    let mut previous_scale = u64::MAX;
    while let Some(Token::Scale(scale)) = tokens.get(end) {
        let value = current.value?;
        if *scale >= previous_scale || value == 0 || value >= *scale {
            return None;
        }
        total = total.checked_add(value.checked_mul(*scale)?)?;
        previous_scale = *scale;
        end += 1;
        let conjunction = matches!(tokens.get(end), Some(Token::And));
        if conjunction {
            end += 1;
        }
        if end == tokens.len() && !conjunction {
            return Some((total.to_string(), end));
        }
        (current, end) = group(tokens, end)?;
        if !matches!(tokens.get(end), Some(Token::Scale(_))) {
            let remainder = current.value?;
            if remainder >= previous_scale {
                return None;
            }
            total = total.checked_add(remainder)?;
            return Some((total.to_string(), end));
        }
    }
    None
}

/// Join number-only speech and ASR digit groups while keeping leading zeros.
/// "sixty seven two four" becomes 6724; "four twenty" becomes 420.
/// Commas/terminal punctuation may be added by ASR. A bare "oh" stays prose.
pub fn digit_sequence(transcript: &str) -> Option<String> {
    let text = transcript
        .trim()
        .trim_end_matches(['.', '!', '?'])
        .trim_end();
    let mut tokens = Vec::new();
    let mut unambiguous = false;
    for token in text
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|token| !token.is_empty())
    {
        let lower = token
            .to_ascii_lowercase()
            .replace(['\u{2010}', '\u{2011}'], "-");
        if let Some((tens, unit)) = lower.split_once('-') {
            // Only a spelled-out compound may contain a hyphen; never
            // collapse numeric ranges such as 1-5 or identifiers.
            let tens = small_word(tens)?;
            let unit = small_word(unit)?;
            if !is_tens(tens) || !(1..=9).contains(&unit) {
                return None;
            }
            tokens.push(spoken_number(tens + unit));
            unambiguous = true;
        } else if let Some(value) = small_word(&lower) {
            tokens.push(spoken_number(value));
            unambiguous |= lower != "oh";
        } else if token.bytes().all(|c| c.is_ascii_digit()) {
            tokens.push(Token::Number(Number {
                digits: token.into(),
                value: if token.len() > 1 && token.starts_with('0') {
                    None
                } else {
                    token.parse().ok()
                },
                spoken: false,
            }));
            unambiguous = true;
        } else {
            tokens.push(match lower.as_str() {
                "hundred" => Token::Hundred,
                "thousand" => Token::Scale(1_000),
                "million" => Token::Scale(1_000_000),
                "billion" => Token::Scale(1_000_000_000),
                "trillion" => Token::Scale(1_000_000_000_000),
                "and" => Token::And,
                _ => return None,
            });
        }
    }
    if !unambiguous {
        return None;
    }
    let mut digits = String::with_capacity(text.len());
    let mut position = 0;
    while position < tokens.len() {
        let (chunk, next) = cardinal_chunk(&tokens, position)?;
        digits.push_str(&chunk);
        position = next;
    }
    Some(digits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spoken_digits_become_one_exact_sequence() {
        for (raw, expected) in [
            ("One five four eight seven nine one three two", "154879132"),
            (
                "One, five, four, eight, seven, nine, one, three, two.",
                "154879132",
            ),
            (
                "zero one two three four five six seven eight nine",
                "0123456789",
            ),
            ("ZERO zero seven.", "007"),
            ("five", "5"),
            ("0", "0"),
            ("  one\tFIVE\nFour!  ", "154"),
        ] {
            assert_eq!(digit_sequence(raw).as_deref(), Some(expected), "{raw}");
        }
    }

    #[test]
    fn mixed_asr_digits_and_zero_alias_keep_every_digit() {
        for (raw, expected) in [
            ("one 54 eight 79 one three 2.", "154879132"),
            ("001 54 879132", "00154879132"),
            ("00154879132.", "00154879132"),
            ("oh oh seven", "007"),
            ("one oh five", "105"),
            ("0 oh", "00"),
        ] {
            assert_eq!(digit_sequence(raw).as_deref(), Some(expected), "{raw}");
            assert_eq!(digit_sequence(expected).as_deref(), Some(expected));
        }
        // Codes are strings: no integer overflow or leading-zero loss.
        let long = "zero nine ".repeat(100);
        assert_eq!(digit_sequence(&long), Some("09".repeat(100)));
    }

    #[test]
    fn reported_mixed_number_phrases_are_joined() {
        for (raw, expected) in [
            ("twenty", "20"),
            ("forty", "40"),
            ("sixty seven two four zero nine eight", "6724098"),
            ("Seven eight nine six nine four twenty", "78969420"),
            ("sixty nine four twenty", "69420"),
            ("sixty-nine thousand four hundred twenty", "69420"),
            ("Sixty nine thousand, four hundred and twenty.", "69420"),
            ("sixty nine thousand 420", "69420"),
            ("69 thousand four hundred 20", "69420"),
            ("789694 twenty", "78969420"),
            ("sixty 7 two 40 nine eight", "6724098"),
            ("60 seven two four zero nine eight", "60724098"),
            ("zero zero sixty-seven two", "00672"),
            ("00 sixty seven two", "00672"),
            ("oh forty two", "042"),
            ("69,420.", "69420"),
        ] {
            assert_eq!(digit_sequence(raw).as_deref(), Some(expected), "{raw}");
        }
    }

    #[test]
    fn every_english_number_under_one_hundred_is_supported() {
        let units = [
            "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
        ];
        let teens = [
            "ten",
            "eleven",
            "twelve",
            "thirteen",
            "fourteen",
            "fifteen",
            "sixteen",
            "seventeen",
            "eighteen",
            "nineteen",
        ];
        let tens = [
            "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
        ];
        for (value, word) in units.iter().chain(&teens).enumerate() {
            assert_eq!(digit_sequence(word), Some(value.to_string()), "{word}");
        }
        for (index, ten) in tens.iter().enumerate() {
            let value = (index + 2) * 10;
            assert_eq!(digit_sequence(ten), Some(value.to_string()));
            for (unit, word) in units.iter().enumerate().skip(1) {
                for separator in [" ", "-", "\u{2010}", "\u{2011}"] {
                    let phrase = format!("{ten}{separator}{word}");
                    assert_eq!(
                        digit_sequence(&phrase),
                        Some((value + unit).to_string()),
                        "{phrase}"
                    );
                }
            }
        }
    }

    #[test]
    fn cardinal_scales_and_conjunctions_preserve_value() {
        for (raw, expected) in [
            ("one hundred", "100"),
            ("one hundred five", "105"),
            ("one hundred and five", "105"),
            ("four hundred twenty", "420"),
            ("nine hundred ninety nine", "999"),
            ("nineteen hundred", "1900"),
            ("two thousand and forty", "2040"),
            (
                "one million two hundred thirty four thousand five hundred sixty seven",
                "1234567",
            ),
            (
                "two billion one million six thousand and eight",
                "2001006008",
            ),
            ("one trillion", "1000000000000"),
        ] {
            assert_eq!(digit_sequence(raw).as_deref(), Some(expected), "{raw}");
        }
    }

    #[test]
    fn numeric_code_groups_are_not_arithmetically_combined() {
        for (raw, expected) in [
            ("60 7", "607"),
            ("60 seven", "607"),
            ("40 nine", "409"),
            ("twenty zero", "200"),
            ("12 three", "123"),
            ("four twenty", "420"),
            ("060 seven", "0607"),
            ("sixty 07", "6007"),
            ("000 forty zero", "000400"),
            (
                "184467440737095516160000 forty",
                "18446744073709551616000040",
            ),
        ] {
            assert_eq!(digit_sequence(raw).as_deref(), Some(expected), "{raw}");
        }
    }

    #[test]
    fn malformed_or_ambiguous_cardinals_are_left_alone() {
        for raw in [
            "twenty and forty",
            "hundred",
            "and twenty",
            "one hundred and",
            "one thousand and",
            "one thousand million",
            "one thousand two thousand",
            "one hundred hundred",
            "zero hundred",
            "one million zero thousand",
            "one hundred 007",
            "one thousand 007",
            "18446744073709551616 thousand",
            "999999999999 trillion",
            "sixty-seven-eight",
            "one-five",
            "twenty-zero",
            "sixty-7",
            "60-seven",
            "sixty–seven",
            "twenty point five",
        ] {
            assert_eq!(digit_sequence(raw), None, "{raw}");
        }
    }

    #[test]
    fn prose_commands_and_ambiguous_content_are_not_rewritten() {
        for raw in [
            "",
            "   ",
            ",.!?",
            "Oh!",
            "oh oh",
            "I need one more.",
            "Paul had two apples.",
            "Paul ate two pandas and then ate three chickens.",
            "Paul had twenty apples.",
            "I am sixty seven years old.",
            "one five apples",
            "room one five four",
            "one and two",
            "someone",
            "one to three",
            "one for five",
            "won five",
            "one O five",
            "minus one",
            "one point five",
            "1.5",
            "-15",
            "+15",
            "1-5",
            "1/5",
            "one@example.com",
            "scratch that",
            "bullets",
            "one; five",
            "one: five",
            "one. Five.",
        ] {
            assert_eq!(digit_sequence(raw), None, "{raw}");
        }
    }
}
