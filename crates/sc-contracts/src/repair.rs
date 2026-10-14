//! Fixes for typical OCR confusions on the game's UI fonts.
//!
//! Only applied where the expected content is known (numbers, keywords, the `[!]` marker),
//! never to free text.

use strsim::levenshtein;

/// Reads a number written with OCR look-alikes: `O`->0, `B`->8, `l`/`I`->1, `S`->5...
/// Thousands separators (`,` `.` space) are dropped. `None` if no digit survives or a
/// character is not a plausible digit.
pub fn number(token: &str) -> Option<u64> {
    let mut digits = String::new();
    for ch in token.trim().chars() {
        let d = match ch {
            '0'..='9' => ch,
            'O' | 'o' | 'D' | 'Q' | 'U' => '0',
            'I' | 'l' | 'i' | '|' | '!' | 'L' | 'J' | ']' | '[' => '1',
            'Z' | 'z' => '2',
            'S' | 's' | '$' => '5',
            'G' | 'b' => '6',
            'T' => '7',
            'B' => '8',
            'g' | 'q' => '9',
            ',' | '.' | ' ' | '\'' => continue,
            _ => return None,
        };
        digits.push(d);
    }
    digits.parse().ok()
}

/// [`number`] narrowed to `u32`.
pub fn small_number(token: &str) -> Option<u32> {
    number(token).and_then(|n| u32::try_from(n).ok())
}

/// Splits a leading restricted-cargo marker (`[!]`, often read as `[1]`, `(!)`, `[I]`...)
/// off a commodity name.
pub fn restricted_marker(s: &str) -> (bool, &str) {
    let s = s.trim();
    let Some((first, rest)) = s.split_once(char::is_whitespace) else {
        return (false, s);
    };
    let marker_chars = |c: char| "[](){}!1Il|iLJ".contains(c);
    let has_bracket = first.chars().any(|c| "[](){}!".contains(c));
    if first.chars().count() <= 4 && has_bracket && first.chars().all(marker_chars) {
        (true, rest.trim())
    } else {
        (false, s)
    }
}

/// Unicode quotes to ASCII, whitespace collapsed.
pub fn clean_text(s: &str) -> String {
    let mapped: String = s
        .chars()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' | '\u{201B}' | '`' | '\u{00B4}' => '\'',
            '\u{201C}' | '\u{201D}' => '"',
            '\u{2013}' | '\u{2014}' => '-',
            c => c,
        })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A roman numeral read with look-alikes: `Ill` -> `III`, `lV` -> `IV`, `?I` -> `II`.
/// `None` unless the result is a numeral from I to X and the token is not plain digits.
pub fn roman(token: &str) -> Option<String> {
    if token.is_empty() || token.chars().count() > 4 || token.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mapped: String = token
        .chars()
        .map(|c| match c {
            'l' | '1' | '|' | '!' | '?' | 'i' | 'j' | ']' | '[' => 'I',
            'v' => 'V',
            'x' => 'X',
            c => c,
        })
        .collect();
    const NUMERALS: [&str; 10] = ["I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"];
    NUMERALS.contains(&mapped.as_str()).then_some(mapped)
}

/// Cleans a commodity or place name read by OCR: trailing junk after the last letter, digit
/// or `)` is dropped (`Center. :` -> `Center`), the stylised `&` (read as `6`, `G`, `e`,
/// `S`, `8`) is restored between two capitalised words (`Samson 6 Son's` -> `Samson & Son's`)
/// and roman numerals after a capitalised word are repaired (`Pyro Ill` -> `Pyro III`).
pub fn name(s: &str) -> String {
    let s = clean_text(s);
    let s = s
        .trim_start_matches(|c: char| !c.is_alphanumeric() && c != '[' && c != '(')
        .trim_end_matches(|c: char| !c.is_alphanumeric() && c != ')');
    let words: Vec<&str> = s.split(' ').collect();
    let capitalised = |w: &str| w.chars().next().is_some_and(char::is_uppercase);
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    for (i, w) in words.iter().enumerate() {
        let after_word = i > 0 && capitalised(words[i - 1]) && words[i - 1].chars().count() > 1;
        let between = after_word && i + 1 < words.len() && capitalised(words[i + 1]);
        if between && matches!(*w, "6" | "G" | "e" | "S" | "8" | "&" | "6&" | "&6") {
            out.push("&".to_owned());
        } else if let Some(numeral) = roman(w).filter(|_| after_word) {
            out.push(numeral);
        } else {
            out.push((*w).to_owned());
        }
    }
    out.join(" ")
}

/// Rewrites the first word of `s` to `keyword` when it is within a small edit distance
/// (`Dellver` -> `Deliver`), after dropping up to 3 junk characters before it (the bullet
/// diamonds are often read as `<>`, `O`, `o`...). `None` if `s` does not start with it.
pub fn leading_keyword(s: &str, keyword: &str) -> Option<String> {
    let s = s.trim();
    let mut starts: Vec<usize> = vec![0];
    starts.extend(
        s.char_indices()
            .filter(|(i, c)| *i > 0 && *i <= 4 && c.is_alphabetic())
            .map(|(i, _)| i),
    );
    for start in starts {
        let rest = &s[start..];
        let skipped = &s[..start];
        if skipped.chars().any(char::is_alphanumeric) && skipped.trim().chars().count() > 2 {
            continue;
        }
        let (word, tail) = rest.split_once(' ').unwrap_or((rest, ""));
        let lw = word.to_lowercase();
        let max = if keyword.len() >= 6 { 2 } else { 1 };
        if lw.len().abs_diff(keyword.len()) <= 1 && levenshtein(&lw, &keyword.to_lowercase()) <= max
        {
            let mut out = keyword.to_owned();
            if !tail.is_empty() {
                out.push(' ');
                out.push_str(tail);
            }
            return Some(out);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_with_lookalikes() {
        assert_eq!(number("535,000"), Some(535_000));
        assert_eq!(number("8O"), Some(80));
        assert_eq!(number("l68"), Some(168));
        assert_eq!(number("B"), Some(8));
        assert_eq!(number("abc"), None);
        assert_eq!(number(""), None);
    }

    #[test]
    fn restricted_markers() {
        for s in [
            "[!] Neon", "[1] Neon", "(!) Neon", "[I] Neon", "I!] Neon", "[l] Neon",
        ] {
            assert_eq!(restricted_marker(s), (true, "Neon"), "{s}");
        }
        assert_eq!(restricted_marker("Iron"), (false, "Iron"));
        assert_eq!(restricted_marker("I Neon"), (false, "I Neon"));
        assert_eq!(
            restricted_marker("Ship Ammunition"),
            (false, "Ship Ammunition")
        );
    }

    #[test]
    fn keywords() {
        assert_eq!(
            leading_keyword("<> Dellver 0/4 SCU", "Deliver").as_deref(),
            Some("Deliver 0/4 SCU")
        );
        assert_eq!(
            leading_keyword("o Collect Gold from X.", "Collect").as_deref(),
            Some("Collect Gold from X.")
        );
        assert_eq!(
            leading_keyword("Collect", "Collect").as_deref(),
            Some("Collect")
        );
        assert_eq!(leading_keyword("in Area18.", "Deliver"), None);
        assert_eq!(leading_keyword("Research Outpost", "Collect"), None);
    }

    #[test]
    fn cleans_names() {
        assert_eq!(
            name("Samson 6 Son's Salvage Center. :"),
            "Samson & Son's Salvage Center"
        );
        assert_eq!(name("Samson G Son's"), "Samson & Son's");
        assert_eq!(name("Aluminum (Ore)"), "Aluminum (Ore)");
        assert_eq!(name("Pyro 5b"), "Pyro 5b");
        assert_eq!(
            name("Seraphim Station above Crusader."),
            "Seraphim Station above Crusader"
        );
        assert_eq!(name("Sunset Mesa on Pyro ?I."), "Sunset Mesa on Pyro II");
        assert_eq!(name("Orbituary above Pyro Ill"), "Orbituary above Pyro III");
        assert_eq!(
            name("Teasa Spaceport in Lorville"),
            "Teasa Spaceport in Lorville"
        );
    }

    #[test]
    fn roman_numerals() {
        assert_eq!(roman("Ill").as_deref(), Some("III"));
        assert_eq!(roman("lV").as_deref(), Some("IV"));
        assert_eq!(roman("VI").as_deref(), Some("VI"));
        assert_eq!(roman("1"), None);
        assert_eq!(roman("in"), None);
        assert_eq!(roman("5b"), None);
    }

    #[test]
    fn cleans_text() {
        assert_eq!(
            clean_text(" Seer\u{2019}s   Canyon\n on "),
            "Seer's Canyon on"
        );
    }
}
