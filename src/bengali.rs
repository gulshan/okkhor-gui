//! Bangla grapheme-cluster segmentation and on-screen diffing.
//!
//! This module is deliberately free of any Win32 dependency so the tricky part
//! of the program — deciding how many backspaces to send — can be unit tested.

/// U+09CD BENGALI SIGN VIRAMA. Unlike the other combining marks it also binds
/// the *following* consonant into the same cluster (`ক` + `্` + `ষ` = `ক্ষ`).
const HASANT: char = '\u{09CD}';

/// How much text a single `VK_BACK` erases in the target application.
///
/// Applications disagree. Win32 edit controls, Notepad and VS Code delete one
/// code point at a time; Word and Chromium-based editors delete a whole
/// grapheme cluster, so a three-code-point conjunct like `ক্ষ` vanishes in one
/// press. Sending the wrong count leaves debris on screen, so this is
/// selectable per application.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum EraseMode {
    /// One backspace per UTF-16 code unit. Correct for most applications.
    #[default]
    CodePoint,
    /// One backspace per grapheme cluster. Correct for Word and Chromium.
    Cluster,
}

impl EraseMode {
    pub fn from_registry_str(s: &str) -> Self {
        if s.eq_ignore_ascii_case("cluster") {
            Self::Cluster
        } else {
            Self::CodePoint
        }
    }

    pub fn as_registry_str(self) -> &'static str {
        match self {
            Self::Cluster => "cluster",
            Self::CodePoint => "codepoint",
        }
    }
}

/// True if `c` attaches to the preceding base character instead of starting a
/// cluster of its own.
fn is_combining(c: char) -> bool {
    matches!(
        c as u32,
        0x0981..=0x0983     // candrabindu, anusvara, visarga
            | 0x09BC        // nukta
            | 0x09BE..=0x09CC // vowel signs aa .. au
            | 0x09CD        // hasant
            | 0x09D7        // au length mark
            | 0x09E2..=0x09E3 // vocalic l / ll vowel signs
            | 0x200C..=0x200D // ZWNJ, ZWJ
    )
}

/// Byte offsets at which each grapheme cluster of `s` starts, terminated by
/// `s.len()`. Always contains at least one element.
fn cluster_starts(s: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut chars = s.char_indices().peekable();

    while let Some((offset, first)) = chars.next() {
        starts.push(offset);
        let mut prev = first;
        while let Some(&(_, next)) = chars.peek() {
            // A combining mark always joins. A consonant joins only when the
            // previous character was a hasant.
            if is_combining(next) || prev == HASANT {
                prev = next;
                chars.next();
            } else {
                break;
            }
        }
    }

    starts.push(s.len());
    starts
}

pub fn cluster_count(s: &str) -> usize {
    cluster_starts(s).len() - 1
}

/// Number of `VK_BACK` presses needed to erase `s` from the target.
pub fn erase_units(s: &str, mode: EraseMode) -> usize {
    match mode {
        // Bangla lives entirely in the BMP so this is the same as counting
        // chars, but counting UTF-16 units is what the receiving edit control
        // actually measures.
        EraseMode::CodePoint => s.encode_utf16().count(),
        EraseMode::Cluster => cluster_count(s),
    }
}

/// Largest cluster start of `s` that is less than or equal to `at`.
fn snap_down(s: &str, at: usize) -> usize {
    cluster_starts(s)
        .into_iter()
        .rev()
        .find(|&start| start <= at)
        .unwrap_or(0)
}

/// Length in bytes of the longest common prefix of `a` and `b` that is a
/// grapheme-cluster boundary in *both*.
///
/// Snapping matters because the character that ends the common prefix may be
/// combining in one string and not the other — emitting a bare combining mark
/// would attach it to whatever is already on screen.
pub fn common_prefix(a: &str, b: &str) -> usize {
    let mut at = a
        .as_bytes()
        .iter()
        .zip(b.as_bytes())
        .take_while(|(x, y)| x == y)
        .count();

    while at > 0 && !a.is_char_boundary(at) {
        at -= 1;
    }

    // Each disagreeing round strictly lowers `at`, so this terminates at 0 in
    // the worst case, which is a cluster boundary in every string.
    loop {
        let in_a = snap_down(a, at);
        let in_b = snap_down(b, at);
        if in_a == in_b {
            return in_a;
        }
        at = in_a.min(in_b);
    }
}

/// The keystrokes needed to turn `old` into `new` on the target's screen.
#[derive(Debug, PartialEq, Eq)]
pub struct Replacement {
    pub backspaces: usize,
    pub text: String,
}

/// Compute the minimal replacement. Only the differing suffix is rewritten,
/// which keeps the flicker down and limits the blast radius of a wrong
/// [`EraseMode`] to the tail of the current word.
pub fn diff(old: &str, new: &str, mode: EraseMode) -> Replacement {
    let at = common_prefix(old, new);
    Replacement {
        backspaces: erase_units(&old[at..], mode),
        text: new[at..].to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_word_is_one_cluster_per_visible_unit() {
        // আমি = আ + ম + ি  -> "মি" is one cluster
        assert_eq!(cluster_count("আমি"), 2);
        assert_eq!(erase_units("আমি", EraseMode::CodePoint), 3);
        assert_eq!(erase_units("আমি", EraseMode::Cluster), 2);
    }

    #[test]
    fn hasant_binds_the_following_consonant() {
        // ক ্ ষ -> a single conjunct
        assert_eq!(cluster_count("ক্ষ"), 1);
        assert_eq!(erase_units("ক্ষ", EraseMode::CodePoint), 3);
        assert_eq!(erase_units("ক্ষ", EraseMode::Cluster), 1);

        // ক ্ ষ ্ ম -> still a single conjunct
        assert_eq!(cluster_count("ক্ষ্ম"), 1);
    }

    #[test]
    fn zwnj_does_not_start_a_cluster() {
        // ক ্ ZWNJ ষ -> the ZWNJ joins, then ষ starts fresh because the
        // character before it is no longer the hasant.
        assert_eq!(cluster_count("ক্\u{200C}ষ"), 2);
    }

    #[test]
    fn reph_stays_with_its_base() {
        // র ্ ক -> র্ক
        assert_eq!(cluster_count("র্ক"), 1);
    }

    #[test]
    fn empty_and_ascii() {
        assert_eq!(cluster_count(""), 0);
        assert_eq!(cluster_count("abc"), 3);
        assert_eq!(erase_units("", EraseMode::Cluster), 0);
    }

    #[test]
    fn common_prefix_never_splits_a_cluster() {
        // "আম" vs "আমি": the shared bytes cover all of "আম", but in the second
        // string the ম carries a vowel sign, so the boundary must back up to
        // before the ম.
        let at = common_prefix("আম", "আমি");
        assert_eq!(&"আম"[..at], "আ");
    }

    #[test]
    fn typing_ami_one_key_at_a_time() {
        // The exact sequence okkhor produces for a -> am -> ami.
        let step1 = diff("", "আ", EraseMode::CodePoint);
        assert_eq!(
            step1,
            Replacement {
                backspaces: 0,
                text: "আ".into()
            }
        );

        let step2 = diff("আ", "আম", EraseMode::CodePoint);
        assert_eq!(
            step2,
            Replacement {
                backspaces: 0,
                text: "ম".into()
            }
        );

        // ম must be re-emitted with its vowel sign attached.
        let step3 = diff("আম", "আমি", EraseMode::CodePoint);
        assert_eq!(
            step3,
            Replacement {
                backspaces: 1,
                text: "মি".into()
            }
        );
    }

    #[test]
    fn cluster_mode_erases_conjuncts_as_one_press() {
        let rep = diff("ক্ষ", "", EraseMode::Cluster);
        assert_eq!(
            rep,
            Replacement {
                backspaces: 1,
                text: String::new()
            }
        );

        let rep = diff("ক্ষ", "", EraseMode::CodePoint);
        assert_eq!(
            rep,
            Replacement {
                backspaces: 3,
                text: String::new()
            }
        );
    }

    #[test]
    fn identical_strings_produce_no_work() {
        assert_eq!(
            diff("আমি", "আমি", EraseMode::CodePoint),
            Replacement {
                backspaces: 0,
                text: String::new()
            }
        );
    }
}

/// End-to-end tests over the real okkhor parser.
///
/// These stand in for the parts of the keyboard hook that cannot be tested
/// without a desktop: a fake "screen" applies each [`Replacement`] the way a
/// text control would, and every step asserts that the screen still equals
/// what the parser says the word should look like. A wrong backspace count
/// shows up immediately as a divergence.
#[cfg(test)]
mod pipeline {
    use super::*;
    use okkhor::parser::Parser;

    /// Model of a text control receiving `units` backspaces.
    fn erase_from_end(text: &str, units: usize, mode: EraseMode) -> String {
        match mode {
            EraseMode::CodePoint => {
                let mut chars: Vec<char> = text.chars().collect();
                chars.truncate(chars.len() - units);
                chars.into_iter().collect()
            }
            EraseMode::Cluster => {
                let starts = cluster_starts(text);
                text[..starts[starts.len() - 1 - units]].to_string()
            }
        }
    }

    /// Type `keys` one at a time, tracking what the target would display.
    fn type_word(keys: &str, mode: EraseMode) -> String {
        let parser = Parser::new_phonetic();
        let mut raw = String::new();
        let mut screen = String::new();

        for key in keys.chars() {
            raw.push(key);
            let expected = parser.convert(&raw);
            let step = diff(&screen, &expected, mode);
            screen = erase_from_end(&screen, step.backspaces, mode) + &step.text;
            assert_eq!(
                screen, expected,
                "after typing {key:?} of {keys:?} in {mode:?}"
            );
        }

        screen
    }

    const WORDS: [&str; 6] = ["ami", "banglay", "gan", "gai", "kkhoma", "bangladesh"];

    #[test]
    fn live_preview_converges_in_codepoint_mode() {
        for word in WORDS {
            let expected = Parser::new_phonetic().convert(word);
            assert_eq!(type_word(word, EraseMode::CodePoint), expected);
        }
    }

    #[test]
    fn live_preview_converges_in_cluster_mode() {
        for word in WORDS {
            let expected = Parser::new_phonetic().convert(word);
            assert_eq!(type_word(word, EraseMode::Cluster), expected);
        }
    }

    #[test]
    fn backspacing_unwinds_the_word() {
        for mode in [EraseMode::CodePoint, EraseMode::Cluster] {
            let parser = Parser::new_phonetic();
            let mut raw = String::from("kkhoma");
            let mut screen = type_word(&raw, mode);

            while !raw.is_empty() {
                raw.pop();
                let expected = parser.convert(&raw);
                let step = diff(&screen, &expected, mode);
                screen = erase_from_end(&screen, step.backspaces, mode) + &step.text;
                assert_eq!(
                    screen, expected,
                    "after erasing down to {raw:?} in {mode:?}"
                );
            }

            assert!(screen.is_empty());
        }
    }

    #[test]
    fn known_conversions() {
        let parser = Parser::new_phonetic();
        assert_eq!(parser.convert("ami"), "আমি");
        // Fully escaped: okkhor emits the precomposed U+09DF for য়, not the
        // য + U+09BC NUKTA pair, and the two forms are easy to confuse in a
        // source literal.
        assert_eq!(
            parser.convert("banglay"),
            "\u{09AC}\u{09BE}\u{0982}\u{09B2}\u{09BE}\u{09DF}"
        );
        // বাং | লা | য়  — the anusvara joins the cluster before it.
        assert_eq!(cluster_count(&parser.convert("banglay")), 3);
        assert_eq!(parser.convert("kkhoma"), "ক্ষমা");
        // Digits become Bangla numerals, which is why they extend a word
        // rather than breaking it.
        assert_eq!(parser.convert("123"), "১২৩");
    }
}
