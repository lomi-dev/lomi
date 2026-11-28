//! User vocabulary: phrase-boost terms for the engine and a correction pass
//! over the recognized text.
//!
//! The engine's boost cannot help once a term's first token was not chosen
//! ("co MIT" for "commit"), so recognized words are also matched against the
//! terms and their aliases: exactly after normalization (case, diacritics,
//! spacing, punctuation inside the word), or within one edit per five letters
//! for terms of at least five letters. Hesitation fillers ("yyy", "um") are
//! dropped.

/// Largest vocabulary accepted from the webview.
pub const MAX_ENTRIES: usize = 100;
const MAX_LINE: usize = 200;
/// Shorter keys match only exactly; fuzzy matching them hits real words.
const FUZZY_MIN: usize = 5;
/// Words a spoken term may be split into, beyond its own word count.
const EXTRA_WORDS: usize = 1;
const MAX_WORDS: usize = 4;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Vocabulary {
    entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    term: String,
    /// Normalized term first, then normalized aliases (matched exactly only).
    keys: Vec<String>,
    words: usize,
}

impl Vocabulary {
    /// One entry per line: `term` or `term = alias, alias`.
    pub fn parse(lines: &[String]) -> Result<Self, String> {
        let mut entries = Vec::new();
        for line in lines {
            if line.chars().count() > MAX_LINE {
                return Err("A vocabulary line is too long.".into());
            }
            let (term, aliases) = line.split_once('=').unwrap_or((line, ""));
            let term = term.trim();
            if term.is_empty() || term.chars().any(char::is_control) {
                continue;
            }
            let spoken = std::iter::once(term)
                .chain(aliases.split(',').map(str::trim))
                .filter(|s| !s.is_empty());
            let words = spoken
                .clone()
                .map(|s| s.split_whitespace().count())
                .max()
                .unwrap_or(1);
            let keys: Vec<String> = spoken.map(normalize).filter(|k| !k.is_empty()).collect();
            if keys.first().is_none_or(|k| *k != normalize(term)) {
                continue;
            }
            entries.push(Entry {
                term: term.to_string(),
                keys,
                words: (words + EXTRA_WORDS).min(MAX_WORDS),
            });
        }
        if entries.len() > MAX_ENTRIES {
            return Err("The vocabulary has too many entries.".into());
        }
        Ok(Self { entries })
    }

    /// Terms for the engine's phrase boost.
    pub fn terms(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.term.clone()).collect()
    }

    /// Replaces recognized spellings of vocabulary terms and drops fillers.
    pub fn correct(&self, text: &str) -> String {
        let tokens: Vec<Token> = text
            .split_whitespace()
            .map(Token::new)
            .filter(|t| !t.is_filler())
            .collect();
        let mut out: Vec<String> = Vec::with_capacity(tokens.len());
        let mut i = 0;
        while i < tokens.len() {
            match self.best_match(&tokens[i..]) {
                Some((len, term)) => {
                    let last = &tokens[i + len - 1];
                    out.push(format!("{}{}{}", tokens[i].lead, term, last.trail));
                    i += len;
                }
                None => {
                    out.push(tokens[i].raw.to_string());
                    i += 1;
                }
            }
        }
        out.join(" ")
    }

    /// Longest window starting at `tokens[0]` that spells a term; exact
    /// matches win over fuzzy ones.
    fn best_match(&self, tokens: &[Token]) -> Option<(usize, &str)> {
        let mut best: Option<(bool, usize, &str)> = None;
        for entry in &self.entries {
            let mut joined = String::new();
            for (index, token) in tokens.iter().take(entry.words).enumerate() {
                // A term never spans punctuation between words.
                if index > 0 && (!token.lead.is_empty() || !tokens[index - 1].trail.is_empty()) {
                    break;
                }
                joined.push_str(&token.key);
                if joined.is_empty() {
                    break;
                }
                let exact = entry.keys.contains(&joined);
                let close = !exact && fuzzy(&joined, &entry.keys[0]);
                if !exact && !close {
                    continue;
                }
                let candidate = (exact, index + 1, entry.term.as_str());
                if best.is_none_or(|(e, len, _)| (exact, index + 1) > (e, len)) {
                    best = Some(candidate);
                }
            }
        }
        best.map(|(_, len, term)| (len, term))
    }
}

struct Token<'a> {
    raw: &'a str,
    lead: &'a str,
    trail: &'a str,
    key: String,
}

impl<'a> Token<'a> {
    fn new(raw: &'a str) -> Self {
        let start = raw.find(char::is_alphanumeric).unwrap_or(raw.len());
        let end = raw.rfind(char::is_alphanumeric).map_or(start, |i| {
            i + raw[i..].chars().next().map_or(0, char::len_utf8)
        });
        Self {
            raw,
            lead: &raw[..start],
            trail: &raw[end.max(start)..],
            key: normalize(&raw[start..end.max(start)]),
        }
    }

    /// "yyy", "eee", "hmm", "um", "uh" and similar, standing alone.
    fn is_filler(&self) -> bool {
        let mut chars: Vec<char> = self.key.chars().collect();
        chars.dedup();
        let shape: String = chars.into_iter().collect();
        let repeated = self.key.chars().count() > shape.chars().count();
        matches!(
            shape.as_str(),
            "em" | "hm" | "mhm" | "um" | "uh" | "uhm" | "erm" | "ehm" | "eh" | "ym"
        ) && (repeated || shape != "eh")
            || (repeated && matches!(shape.as_str(), "y" | "e"))
    }
}

/// Lowercase letters and digits only, Polish diacritics folded.
fn normalize(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .filter_map(|c| match c {
            'ą' => Some('a'),
            'ć' => Some('c'),
            'ę' => Some('e'),
            'ł' => Some('l'),
            'ń' => Some('n'),
            'ó' => Some('o'),
            'ś' => Some('s'),
            'ź' | 'ż' => Some('z'),
            c if c.is_alphanumeric() => Some(c),
            _ => None,
        })
        .collect()
}

fn fuzzy(heard: &str, term: &str) -> bool {
    let len = term.chars().count();
    // An inflected form ("commitu", "commits") keeps its ending.
    let inflected = heard.starts_with(term) || term.starts_with(heard);
    len >= FUZZY_MIN && !inflected && distance(heard, term) <= len / 5
}

/// Levenshtein distance over chars.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let next = (diagonal + usize::from(ca != *cb))
                .min(row[j] + 1)
                .min(row[j + 1] + 1);
            diagonal = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab(lines: &[&str]) -> Vocabulary {
        Vocabulary::parse(&lines.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn joins_split_terms_and_fixes_case() {
        let v = vocab(&["commit", "main", "Lomi"]);
        assert_eq!(
            v.correct("Zrób co MIT i wypchnij zmianę na gałąź Main."),
            "Zrób commit i wypchnij zmianę na gałąź main."
        );
        assert_eq!(v.correct("Otwórz LOMI, proszę"), "Otwórz Lomi, proszę");
    }

    #[test]
    fn aliases_and_fuzzy_matches() {
        let v = vocab(&["Tauri = tałri, tory", "Claude Code"]);
        assert_eq!(v.correct("aplikacja w tory"), "aplikacja w Tauri");
        assert_eq!(
            v.correct("uruchom klaude code teraz"),
            "uruchom Claude Code teraz"
        );
        // Short keys never match fuzzily.
        assert_eq!(vocab(&["Lomi"]).correct("Lomy"), "Lomy");
    }

    #[test]
    fn leaves_other_words_and_punctuation_alone() {
        let v = vocab(&["commit"]);
        assert_eq!(v.correct("co, mit"), "co, mit");
        assert_eq!(v.correct("comment"), "comment");
        assert_eq!(v.correct("(comit)"), "(commit)");
        assert_eq!(v.correct("dodaj commitu"), "dodaj commitu");
        assert_eq!(Vocabulary::default().correct("a  b"), "a b");
    }

    #[test]
    fn drops_fillers_only() {
        let v = Vocabulary::default();
        assert_eq!(v.correct("Yyy, zrób eee to hmm teraz"), "zrób to teraz");
        assert_eq!(v.correct("um so uh yes"), "so yes");
        assert_eq!(v.correct("a i e y eh"), "a i e y eh");
    }

    #[test]
    fn parse_limits_and_terms() {
        let v = vocab(&["", "  Tauri = tory ", "=x"]);
        assert_eq!(v.terms(), ["Tauri"]);
        let many: Vec<String> = (0..=MAX_ENTRIES).map(|i| format!("t{i}")).collect();
        assert!(Vocabulary::parse(&many).is_err());
        assert!(Vocabulary::parse(&["x".repeat(MAX_LINE + 1)]).is_err());
    }
}
