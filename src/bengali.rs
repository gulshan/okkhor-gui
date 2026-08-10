//! Working out what to send to rewrite the on-screen preview.
//!
//! This module is deliberately free of any Win32 dependency, so the part that
//! decides how much to erase can be unit tested on its own.

/// The keystrokes needed to turn `old` into `new` on the target's screen.
#[derive(Debug, PartialEq, Eq)]
pub struct Replacement {
    pub backspaces: usize,
    pub text: String,
}

/// Compute the minimal replacement: keep the shared prefix, erase the rest of
/// `old`, then type the rest of `new`.
///
/// One backspace erases one UTF-16 code unit. Every target measured deletes at
/// that granularity — Win32 edit controls, WinForms `TextBox` and
/// `RichTextBox`, WPF `TextBox`, and Chromium, the last checked end to end by
/// typing into Edge 148 and reading the field back.
///
/// The prefix is a plain code-unit comparison, and deliberately so. Bangla
/// invites the assumption that this has to be segmentation-aware, but it does
/// not: a lone combining mark can be emitted safely, because the base it
/// attaches to is already on screen and already correct. Rewriting `আম` to
/// `আমি` sends no backspaces and types `ি`, which lands on the `ম` sitting
/// there. Adding segmentation here would only make every rewrite larger.
///
/// If a target ever seems to erase more than this, measure it by typing into
/// the real application. In a browser, `Selection.modify('extend','backward',
/// 'character')` looks like the way to check and is not — it walks visible
/// characters and gave the opposite answer for Chromium.
pub fn diff(old: &str, new: &str) -> Replacement {
    let mut at = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(x, y)| x == y)
        .count();

    while at > 0 && !old.is_char_boundary(at) {
        at -= 1;
    }

    Replacement {
        backspaces: old[at..].encode_utf16().count(),
        text: new[at..].to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_strings_produce_no_work() {
        assert_eq!(
            diff("আমি", "আমি"),
            Replacement {
                backspaces: 0,
                text: String::new()
            }
        );
    }

    #[test]
    fn appending_needs_no_backspaces() {
        assert_eq!(
            diff("আ", "আম"),
            Replacement {
                backspaces: 0,
                text: "ম".into()
            }
        );
    }

    #[test]
    fn a_vowel_sign_lands_on_the_base_already_on_screen() {
        // The ম stays put and only the vowel sign is sent after it.
        assert_eq!(
            diff("আম", "আমি"),
            Replacement {
                backspaces: 0,
                text: "ি".into()
            }
        );
    }

    #[test]
    fn a_reanalysed_conjunct_rewrites_only_its_tail() {
        // ক -> ক্ষ keeps the ক and appends the hasant and ষ.
        assert_eq!(
            diff("ক", "ক্ষ"),
            Replacement {
                backspaces: 0,
                text: "\u{09CD}\u{09B7}".into()
            }
        );

        // ক্ষ -> ক্স shares ক্ and swaps only the final consonant.
        assert_eq!(
            diff("ক্ষ", "ক্স"),
            Replacement {
                backspaces: 1,
                text: "\u{09B8}".into()
            }
        );
    }

    #[test]
    fn erasing_counts_utf16_units() {
        assert_eq!(
            diff("ক্ষ", ""),
            Replacement {
                backspaces: 3,
                text: String::new()
            }
        );
        assert_eq!(
            diff("", "ক্ষ"),
            Replacement {
                backspaces: 0,
                text: "ক্ষ".into()
            }
        );
    }
}

/// End-to-end tests over the real okkhor parser.
///
/// These stand in for the parts of the keyboard hook that cannot be tested
/// without a desktop: a fake "screen" applies each [`Replacement`] the way a
/// text control would, and every step asserts that the screen still equals
/// what the parser says the word should look like.
#[cfg(test)]
mod pipeline {
    use super::*;
    use okkhor::parser::Parser;

    /// Type `keys` one at a time, tracking what the target would display.
    fn type_word(keys: &str) -> String {
        let parser = Parser::new_phonetic();
        let mut raw = String::new();
        let mut screen = String::new();

        for key in keys.chars() {
            raw.push(key);
            let expected = parser.convert(&raw);
            let step = diff(&screen, &expected);

            let mut chars: Vec<char> = screen.chars().collect();
            chars.truncate(chars.len() - step.backspaces);
            screen = chars.into_iter().collect::<String>() + &step.text;

            assert_eq!(screen, expected, "after typing {key:?} of {keys:?}");
        }

        screen
    }

    const WORDS: [&str; 8] = [
        "ami",
        "banglay",
        "gan",
        "gai",
        "kkhoma",
        "kxoma",
        "rikx",
        "bangladesh",
    ];

    #[test]
    fn live_preview_converges() {
        for word in WORDS {
            let expected = Parser::new_phonetic().convert(word);
            assert_eq!(type_word(word), expected);
        }
    }

    #[test]
    fn backspacing_unwinds_the_word() {
        let parser = Parser::new_phonetic();
        let mut raw = String::from("kkhoma");
        let mut screen = type_word(&raw);

        while !raw.is_empty() {
            raw.pop();
            let expected = parser.convert(&raw);
            let step = diff(&screen, &expected);

            let mut chars: Vec<char> = screen.chars().collect();
            chars.truncate(chars.len() - step.backspaces);
            screen = chars.into_iter().collect::<String>() + &step.text;

            assert_eq!(screen, expected, "after erasing down to {raw:?}");
        }

        assert!(screen.is_empty());
    }

    /// Punctuation okkhor converts. `keyboard::punctuation` has to route every
    /// one of these into the buffer; if any is treated as a word break instead,
    /// the conversion is silently lost and the raw ASCII reaches the target.
    #[test]
    fn punctuation_conversions() {
        let parser = Parser::new_phonetic();

        assert_eq!(parser.convert("."), "\u{0964}"); // danda ।
        assert_eq!(parser.convert(".."), "\u{0964}\u{0964}"); // ।।
        assert_eq!(parser.convert("..."), "..."); // ellipsis stays literal
        assert_eq!(parser.convert("ami."), "আমি\u{0964}");

        assert_eq!(parser.convert(":"), "\u{0983}"); // visarga ঃ
        assert_eq!(parser.convert("^"), "\u{0981}"); // candrabindu ঁ
        assert_eq!(parser.convert(",,"), "\u{09CD}\u{200C}"); // hasant + ZWNJ
        assert_eq!(parser.convert("$"), "\u{09F3}"); // taka ৳
        assert_eq!(parser.convert(","), ",");

        // The backtick escapes each of them back to the plain character, which
        // only works because the backtick is buffered alongside.
        assert_eq!(parser.convert(".`"), ".");
        assert_eq!(parser.convert(":`"), ":");
        assert_eq!(parser.convert("^`"), "^");
    }

    /// A dot in front of a digit stays a dot, so decimals survive. Because the
    /// digit only arrives on a later keystroke, the preview necessarily shows
    /// the danda first and then corrects itself.
    #[test]
    fn decimal_point_corrects_itself_mid_word() {
        let parser = Parser::new_phonetic();
        assert_eq!(parser.convert("3."), "৩\u{0964}");
        assert_eq!(parser.convert("3.14"), "৩.১৪");
        assert_eq!(type_word("3.14"), "৩.১৪");
    }

    #[test]
    fn punctuation_survives_the_live_preview() {
        for keys in ["ami.", "ami..", "bhalo:", "3.14", "100$", "ka,,kha"] {
            let expected = Parser::new_phonetic().convert(keys);
            assert_eq!(type_word(keys), expected, "typing {keys:?}");
        }
    }

    /// Avro is case sensitive, and the distinction carries real consonants —
    /// not stylistic variants. The hook therefore must not treat Shift as a
    /// word break, and must fold Shift with Caps Lock correctly.
    #[test]
    fn case_selects_different_consonants() {
        let parser = Parser::new_phonetic();
        assert_eq!(parser.convert("s"), "স");
        assert_eq!(parser.convert("S"), "শ");
        assert_eq!(parser.convert("t"), "ত");
        assert_eq!(parser.convert("T"), "ট");
        assert_eq!(parser.convert("d"), "দ");
        assert_eq!(parser.convert("D"), "ড");
        assert_eq!(parser.convert("n"), "ন");
        assert_eq!(parser.convert("N"), "ণ");
        assert_eq!(parser.convert("r"), "র");
        // Escaped: this is the precomposed U+09DC, not ড + U+09BC NUKTA. The
        // two are indistinguishable in a source literal and okkhor emits the
        // precomposed form, exactly as it does for য়.
        assert_eq!(parser.convert("R"), "\u{09DC}");
        assert_eq!(parser.convert("Dhaka"), "ঢাকা");
    }

    /// `kx` is the clearest example of a spelling whose value depends on an
    /// earlier letter: the x only becomes ষ because a k precedes it. Split the
    /// two apart and you get ক followed by এক্স — which is exactly what a
    /// dropped buffer produced before the focus-event fix in `winevent.rs`.
    #[test]
    fn later_letters_reinterpret_earlier_ones() {
        let parser = Parser::new_phonetic();
        assert_eq!(parser.convert("k"), "ক");
        assert_eq!(parser.convert("x"), "এক্স");
        assert_eq!(parser.convert("kx"), "ক্ষ");
        assert_eq!(parser.convert("kkh"), "ক্ষ");
        assert_eq!(parser.convert("kxoma"), "ক্ষমা");

        // Losing the buffer between the two keystrokes gives this instead, and
        // it is what the desktop test in scripts/e2e-focus-noise.ps1 pins down.
        assert_eq!(
            format!("{}{}", parser.convert("k"), parser.convert("x")),
            "কএক্স"
        );
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
        assert_eq!(parser.convert("kkhoma"), "ক্ষমা");
        // Digits become Bangla numerals, which is why they extend a word
        // rather than breaking it.
        assert_eq!(parser.convert("123"), "১২৩");
    }
}
