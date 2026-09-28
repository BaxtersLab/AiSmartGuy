//! ASG-Q3: every quote a model gives is checked against the chapter it cites.
//!
//! The manager ruled (a) on 2026-09-27; the supervisor's reading fixes the
//! details, so nothing here is a guess:
//! - The quote is labelled, never dropped: [`VERIFIED`] or [`NOT_FOUND`].
//! - Matching is exact after one fixed normalisation, applied identically to
//!   the quote and the chapter: NFKC, then full Unicode case folding; soft
//!   hyphens and zero-width characters removed; curly and prime quote marks
//!   and apostrophes mapped to ASCII `'` and `"`; every whitespace run
//!   collapsed to one space; trimmed. Nothing else: no stemming, no edit
//!   distance. A PDF line-break hyphen ("exam-\nple") therefore stays
//!   unverified, and is counted as such.
//! - Only the cited chapter's own text is searched (`pdf_io::Chapter::body`),
//!   never the preamble carried from the previous section.
//! - A verified quote records start and end CHARACTER offsets into that
//!   original chapter text, so a reader can find the passage. A match must
//!   begin and end on whole source characters: "ish" does not verify inside
//!   the ligature of "ﬁsh".
//!
//! The manager's follow-up rulings (2026-09-28):
//! - F2: the exact match comes first. Only if it fails, at most one trailing
//!   `. , ; : ! ?` is removed from the MODEL'S quote and the match retried.
//!   The chapter text is never touched; the original quote is kept, and a
//!   retry's offsets are recorded like any other.
//! - F3: a quote of fewer than four lexical words after normalisation is
//!   [`TOO_SHORT`], an unsupported inference, never evidence. Four or more
//!   words still need the exact match: length alone verifies nothing.

use unicode_normalization::char::{canonical_combining_class, decompose_compatible};
use unicode_normalization::{is_nfkc_quick, IsNormalized, UnicodeNormalization};

use crate::casefold_table::CASEFOLD;

/// The label for a quote found in the chapter it cites.
pub const VERIFIED: &str = "Quote verified in chapter";
/// The label for a quote that is not in the chapter it cites.
pub const NOT_FOUND: &str = "Quote not found in chapter";
/// The label for a quote too short to mean anything if found (F3).
pub const TOO_SHORT: &str = "Quote too short to verify";

/// Fewer lexical words than this is too short to verify (F3).
pub const MIN_WORDS: usize = 4;
/// The one trailing mark the retry may remove from the model's quote (F2).
const TRAILING_MARKS: [char; 6] = ['.', ',', ';', ':', '!', '?'];

/// Removed before matching: the soft hyphen and the zero-width characters.
const INVISIBLE: [char; 6] = ['\u{00AD}', '\u{200B}', '\u{200C}', '\u{200D}', '\u{2060}', '\u{FEFF}'];
/// Curly and prime single quote marks and apostrophes, mapped to `'`.
const SINGLE_QUOTES: [char; 7] = [
    '\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}', '\u{2032}', '\u{2035}', '\u{02BC}',
];
/// Curly and prime double quote marks, mapped to `"`. NFKC runs first and
/// splits U+2033 and U+2036 into two single primes, so those two end up as
/// `''`; they are listed here only so the intent is on record.
const DOUBLE_QUOTES: [char; 9] = [
    '\u{201C}', '\u{201D}', '\u{201E}', '\u{201F}', '\u{2033}', '\u{2036}', '\u{301D}', '\u{301E}',
    '\u{301F}',
];

/// The outcome of checking one quote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteCheck {
    /// Found. `start..end` are character offsets into the chapter's original
    /// text (`chapter.body().chars()`), not byte offsets. `trailing_mark_removed`
    /// says the match needed the F2 retry.
    Verified { start: usize, end: usize, trailing_mark_removed: bool },
    NotFound,
    /// Fewer than [`MIN_WORDS`] lexical words: not checked, never evidence.
    TooShort,
}

impl QuoteCheck {
    pub fn label(&self) -> &'static str {
        match self {
            QuoteCheck::Verified { .. } => VERIFIED,
            QuoteCheck::NotFound => NOT_FOUND,
            QuoteCheck::TooShort => TOO_SHORT,
        }
    }
    pub fn is_verified(&self) -> bool {
        matches!(self, QuoteCheck::Verified { .. })
    }
}

/// Normalised text, with each normalised character's source span: the
/// character range of the original text it came from.
struct Mapped {
    chars: Vec<char>,
    spans: Vec<(usize, usize)>,
}

/// NFKC can be applied piecewise at a character whose decomposition starts
/// with a starter that never composes with what precedes it.
fn boundary_before(c: char) -> bool {
    let mut first = None;
    decompose_compatible(c, |d| {
        if first.is_none() {
            first = Some(d);
        }
    });
    let d = first.unwrap_or(c);
    canonical_combining_class(d) == 0
        && !matches!(is_nfkc_quick(std::iter::once(d)), IsNormalized::Maybe)
}

fn casefold(c: char, mut emit: impl FnMut(char)) {
    match CASEFOLD.binary_search_by_key(&c, |&(k, _)| k) {
        Ok(i) => CASEFOLD[i].1.chars().for_each(emit),
        Err(_) => emit(c),
    }
}

fn normalize_mapped(text: &str) -> Mapped {
    let src: Vec<char> = text.chars().collect();
    // 1-3: NFKC per segment, case folding, removals and quote marks.
    let mut folded: Vec<(char, (usize, usize))> = Vec::with_capacity(src.len());
    let mut start = 0;
    for end in 1..=src.len() {
        if end < src.len() && !boundary_before(src[end]) {
            continue;
        }
        let segment: String = src[start..end].iter().collect();
        for c in segment.nfkc() {
            casefold(c, |f| {
                if INVISIBLE.contains(&f) {
                    return;
                }
                let m = if SINGLE_QUOTES.contains(&f) {
                    '\''
                } else if DOUBLE_QUOTES.contains(&f) {
                    '"'
                } else {
                    f
                };
                folded.push((m, (start, end)));
            });
        }
        start = end;
    }
    // 4: whitespace runs become one space, spanning the whole run.
    let mut chars = Vec::with_capacity(folded.len());
    let mut spans: Vec<(usize, usize)> = Vec::with_capacity(folded.len());
    for (c, span) in folded {
        if c.is_whitespace() {
            if chars.last() == Some(&' ') {
                spans.last_mut().expect("a space was pushed").1 = span.1;
                continue;
            }
            chars.push(' ');
        } else {
            chars.push(c);
        }
        spans.push(span);
    }
    // 5: trim.
    let lead = chars.iter().take_while(|&&c| c == ' ').count();
    let trail = chars.iter().rev().take_while(|&&c| c == ' ').count().min(chars.len() - lead);
    chars.truncate(chars.len() - trail);
    spans.truncate(spans.len() - trail);
    chars.drain(..lead);
    spans.drain(..lead);
    Mapped { chars, spans }
}

/// The Unicode versions behind the normalisation, for the run's findings
/// JSON: NFKC comes from the unicode-normalization crate, case folding from
/// the generated table. They can differ; the report says so.
pub fn unicode_versions() -> String {
    let (a, b, c) = unicode_normalization::UNICODE_VERSION;
    format!(
        "NFKC Unicode {a}.{b}.{c}; case folding Unicode {}",
        crate::casefold_table::UNICODE_VERSION
    )
}

/// The normalisation both sides of a match go through.
pub fn normalize(text: &str) -> String {
    normalize_mapped(text).chars.into_iter().collect()
}

/// A chapter's own text, normalised once and searched for many quotes.
pub struct ChapterText {
    mapped: Mapped,
}

impl ChapterText {
    pub fn new(body: &str) -> ChapterText {
        ChapterText { mapped: normalize_mapped(body) }
    }

    /// Check one quote (F3 length rule, exact match, then the F2 retry). The
    /// first occurrence that begins and ends on whole source characters wins.
    pub fn check(&self, quote: &str) -> QuoteCheck {
        let q = normalize_mapped(quote).chars;
        if lexical_words(&q) < MIN_WORDS {
            return QuoteCheck::TooShort;
        }
        if let Some((start, end)) = self.find(&q) {
            return QuoteCheck::Verified { start, end, trailing_mark_removed: false };
        }
        if let Some(last) = q.last() {
            if TRAILING_MARKS.contains(last) {
                let mut shorter = q[..q.len() - 1].to_vec();
                while shorter.last() == Some(&' ') {
                    shorter.pop();
                }
                if let Some((start, end)) = self.find(&shorter) {
                    return QuoteCheck::Verified { start, end, trailing_mark_removed: true };
                }
            }
        }
        QuoteCheck::NotFound
    }

    fn find(&self, q: &[char]) -> Option<(usize, usize)> {
        let c = &self.mapped.chars;
        let spans = &self.mapped.spans;
        if q.is_empty() || q.len() > c.len() {
            return None;
        }
        for i in 0..=c.len() - q.len() {
            let j = i + q.len();
            if c[i] != q[0] || c[i..j] != q[..] {
                continue;
            }
            let starts_whole = i == 0 || spans[i - 1] != spans[i];
            let ends_whole = j == c.len() || spans[j] != spans[j - 1];
            if starts_whole && ends_whole {
                return Some((spans[i].0, spans[j - 1].1));
            }
        }
        None
    }
}

/// Words with at least one letter or digit in them: a lone dash or mark is
/// not a word.
fn lexical_words(normalized: &[char]) -> usize {
    normalized
        .split(|&c| c == ' ')
        .filter(|w| w.iter().any(|c| c.is_alphanumeric()))
        .count()
}

/// Check one quote against one chapter's own text.
pub fn check_quote(quote: &str, chapter_body: &str) -> QuoteCheck {
    ChapterText::new(chapter_body).check(quote)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAPTER_1: &str = "It was the best of times, it was the worst of times.\n\
        The committee \u{201C}voted unanimously\u{201D} \u{2014} or so the minutes said.\n\
        Nobody asked why the vote hap-\npened at midnight.";
    const CHAPTER_2: &str = "In the second chapter the author admits the figures were estimates.";
    /// Paine, Common Sense: the sentence Hermes-7B quoted with an added full stop.
    const PAINE: &str = "The more simple any thing is, the less liable it is to be disordered; \
        and the easier repaired when disordered.";

    fn slice(text: &str, start: usize, end: usize) -> String {
        text.chars().skip(start).take(end - start).collect()
    }

    /// An exact match: the original slice normalises to the quote.
    fn assert_verified(quote: &str, chapter: &str) -> (usize, usize) {
        match check_quote(quote, chapter) {
            QuoteCheck::Verified { start, end, trailing_mark_removed: false } => {
                assert_eq!(normalize(&slice(chapter, start, end)), normalize(quote),
                    "the slice at {start}..{end} must normalise to the quote");
                (start, end)
            }
            other => panic!("{quote:?} should verify exactly, got {other:?}"),
        }
    }

    #[test]
    fn labels_are_the_rulings_words() {
        assert_eq!(VERIFIED, "Quote verified in chapter");
        assert_eq!(NOT_FOUND, "Quote not found in chapter");
        assert_eq!(TOO_SHORT, "Quote too short to verify");
        let v = QuoteCheck::Verified { start: 0, end: 1, trailing_mark_removed: false };
        assert_eq!(v.label(), VERIFIED);
        assert_eq!(QuoteCheck::NotFound.label(), NOT_FOUND);
        assert_eq!(QuoteCheck::TooShort.label(), TOO_SHORT);
        assert!(!QuoteCheck::TooShort.is_verified() && !QuoteCheck::NotFound.is_verified());
    }

    #[test]
    fn a_real_quote_verifies_with_offsets_into_the_original() {
        let (s, e) = assert_verified("it was the worst of times", CHAPTER_1);
        assert_eq!(slice(CHAPTER_1, s, e), "it was the worst of times");
        // Offsets are characters, not bytes: the text before has none wider
        // than a byte here, so compare with a chapter that has.
        let wide = "\u{00E9}t\u{00E9} \u{2014} it was the worst of times";
        let (s, e) = assert_verified("it was the worst of times", wide);
        assert_eq!((s, e), (6, 31));
    }

    #[test]
    fn case_whitespace_and_quote_marks_are_normalised() {
        assert_verified("IT WAS THE BEST   OF\ttimes", CHAPTER_1);
        // ASCII quotes in the model's quote, curly ones in the book.
        assert_verified("The committee \"voted unanimously\"", CHAPTER_1);
        // An apostrophe: curly in the book, ASCII in the quote, and the reverse.
        assert_verified("consider the author's claim", "Consider the author\u{2019}s claim first.");
        assert_verified("consider the author\u{2019}s claim", "Consider the author's claim first.");
        // A line break inside the quote is only whitespace.
        assert_verified("minutes said. Nobody asked", CHAPTER_1);
    }

    #[test]
    fn whitespace_around_a_quote_is_trimmed() {
        // The passage opens the chapter, so nothing precedes it: a kept
        // leading space could never match there.
        let (s, e) = assert_verified("  \n It was the best of times \t", CHAPTER_1);
        assert_eq!((s, e), (0, 24));
    }

    #[test]
    fn nfkc_and_full_case_folding_apply() {
        // A ligature in the book, plain letters in the quote.
        assert_verified("then the fish swam", "Then the \u{FB01}sh swam away.");
        // Full folding, not lowercase: U+00DF folds to "ss".
        assert_verified("die STRASSE war leer", "Die Stra\u{00DF}e war leer.");
        // Fullwidth letters and a no-break space are NFKC compatibility forms.
        assert_verified("x ABC def y", "x \u{FF21}\u{FF22}\u{FF23}\u{00A0}def y");
        // Final sigma folds to sigma.
        assert_verified("the word \u{039F}\u{0394}\u{039F}\u{03A3} ends", "the word \u{03BF}\u{03B4}\u{03BF}\u{03C2} ends here");
    }

    #[test]
    fn soft_hyphens_and_zero_width_characters_are_removed() {
        assert_verified("all voted unanimously today", "we all voted una\u{00AD}nimously today");
        assert_verified("all voted unanimously today", "we all voted una\u{200B}nimous\u{FEFF}ly today");
        assert_verified("we all voted today", "we all voted \u{200D} today");
    }

    #[test]
    fn a_pdf_line_break_hyphen_stays_unverified() {
        // "hap-\npened" is a hyphen and a line break, not a soft hyphen. No
        // rule rejoins it, so the quote the model meant is not found.
        assert_eq!(check_quote("the vote happened at midnight", CHAPTER_1), QuoteCheck::NotFound);
        // Control: the text as printed does verify.
        assert_verified("the vote hap- pened at midnight", CHAPTER_1);
    }

    #[test]
    fn near_miss_paraphrases_are_not_found() {
        assert_eq!(check_quote("it was the worst of all times", CHAPTER_1), QuoteCheck::NotFound);
        assert_eq!(check_quote("the committee voted unanimously", CHAPTER_1), QuoteCheck::NotFound,
            "dropping the quote marks is a change the ruling does not normalise");
        assert!(check_quote("it was the best of time", CHAPTER_1).is_verified(),
            "a prefix of the text IS in the text; matching is substring, not word");
        assert_eq!(check_quote("It were the best of times", CHAPTER_1), QuoteCheck::NotFound);
    }

    #[test]
    fn catalog_phrases_are_not_found() {
        // The small models' failure: the rule catalog's own example wording,
        // presented as a quote from the book.
        for phrase in ["appeal to emotion used here", "everyone knows that this is", "it is what it is"] {
            assert_eq!(check_quote(phrase, CHAPTER_1), QuoteCheck::NotFound, "{phrase}");
        }
    }

    #[test]
    fn empty_quotes_are_too_short() {
        for q in ["", "   ", "\u{200B}", "\u{00AD}\n\t"] {
            assert_eq!(check_quote(q, CHAPTER_1), QuoteCheck::TooShort, "{q:?}");
        }
    }

    #[test]
    fn text_only_in_another_chapter_is_not_found() {
        let q = "the figures were estimates";
        assert_verified(q, CHAPTER_2);
        assert_eq!(check_quote(q, CHAPTER_1), QuoteCheck::NotFound);
    }

    #[test]
    fn a_match_must_cover_whole_source_characters() {
        // "ish" starts inside the NFKC expansion of the ligature.
        let q = "ish swam by the river";
        assert_eq!(check_quote(q, "a \u{FB01}sh swam by the river"), QuoteCheck::NotFound);
        // A later whole-character occurrence still verifies.
        let t = "a \u{FB01}sh swam by the river, a dish swam by the river";
        let (s, e) = assert_verified(q, t);
        assert_eq!(slice(t, s, e), q);
    }

    // ── F3: fewer than four lexical words is too short ─────────────────

    #[test]
    fn three_words_are_too_short_even_when_they_are_in_the_chapter() {
        assert_eq!(check_quote("to be disordered", PAINE), QuoteCheck::TooShort);
        // Punctuation and dashes are not words.
        assert_eq!(check_quote("to be \u{2014} disordered !", PAINE), QuoteCheck::TooShort);
        assert_eq!(check_quote("THE MORE simple", PAINE), QuoteCheck::TooShort);
    }

    #[test]
    fn four_words_are_checked_and_length_alone_verifies_nothing() {
        assert_verified("is to be disordered", PAINE);
        assert_eq!(check_quote("is to be ordered", PAINE), QuoteCheck::NotFound);
    }

    // ── F2: one trailing mark, from the model's quote, after the exact match ──

    #[test]
    fn each_allowed_trailing_mark_is_removed_once() {
        let body = "the less liable it is to be disordered";
        for mark in ['.', ',', ':', '!', '?'] {
            let quote = format!("{body}{mark}");
            match check_quote(&quote, PAINE) {
                QuoteCheck::Verified { start, end, trailing_mark_removed: true } => {
                    assert_eq!(slice(PAINE, start, end), body, "{mark}");
                }
                other => panic!("{mark}: {other:?}"),
            }
        }
        // ';' is what the book has, so the exact match already succeeds.
        let (s, e) = assert_verified(&format!("{body};"), PAINE);
        assert_eq!(slice(PAINE, s, e), format!("{body};"));
    }

    #[test]
    fn two_trailing_marks_are_never_removed() {
        // ";." would verify, correctly: one mark goes, and ';' is the book's own.
        for tail in ["..", "!?", ".;", ". ."] {
            let quote = format!("the less liable it is to be disordered{tail}");
            assert_eq!(check_quote(&quote, PAINE), QuoteCheck::NotFound, "{tail:?}");
        }
    }

    #[test]
    fn the_exact_match_comes_before_the_retry() {
        // Without its full stop the quote occurs first at the very start; with
        // it, only at the end. The exact match must win.
        let chapter = "We went home late and slept. Later that year we went home late.";
        match check_quote("we went home late.", chapter) {
            QuoteCheck::Verified { start, end, trailing_mark_removed: false } => {
                assert_eq!(slice(chapter, start, end), "we went home late.");
                assert!(start > 0);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn punctuation_is_never_removed_from_the_chapter() {
        let chapter = "and the easier repaired, when disordered.";
        // The book has a comma the quote lacks: no source-side stripping.
        assert_eq!(check_quote("the easier repaired when disordered", chapter), QuoteCheck::NotFound);
        // Only the quote's own trailing mark goes, never one inside it.
        assert_eq!(check_quote("the easier repaired; when disordered", chapter), QuoteCheck::NotFound);
        // A mark before a closing quote is not the last character.
        assert_eq!(check_quote("\"the easier repaired, when disordered.\"", chapter), QuoteCheck::NotFound);
        // Control: the quote's added mark removed, the book's punctuation intact.
        match check_quote("the easier repaired, when disordered!", chapter) {
            QuoteCheck::Verified { start, end, trailing_mark_removed: true } => {
                assert_eq!(slice(chapter, start, end), "the easier repaired, when disordered");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn double_primes_become_two_apostrophes_as_the_order_dictates() {
        // NFKC splits U+2033 into two primes before quote marks are mapped.
        assert_eq!(normalize("6\u{2033}"), "6''");
        assert_eq!(normalize("\u{301D}x\u{301E}"), "\"x\"");
    }

    #[test]
    fn piecewise_nfkc_equals_whole_string_nfkc() {
        // The index map applies NFKC per segment. These strings cross every
        // segment rule: combining marks, Hangul jamo that compose across
        // starters, a Tibetan vowel whose decomposition starts with a
        // non-starter, compatibility forms and ligatures.
        let cases = [
            "e\u{0301}a\u{0308}\u{0323}o",
            "\u{1100}\u{1161}\u{11A8} \u{1100}\u{1161} \u{AC00}\u{11A8}",
            "\u{0F40}\u{0F73}\u{0F71}\u{0F72}",
            "\u{FB01}\u{FB03} \u{FF21}\u{00A0}\u{2460} \u{2033}\u{00BD}",
            "A\u{030A}\u{212B}\u{0041}\u{0301}\u{0327}",
        ];
        for s in cases {
            let src: Vec<char> = s.chars().collect();
            let mut pieces = String::new();
            let mut start = 0;
            for end in 1..=src.len() {
                if end < src.len() && !boundary_before(src[end]) {
                    continue;
                }
                pieces.extend(src[start..end].iter().collect::<String>().nfkc());
                start = end;
            }
            assert_eq!(pieces, s.nfkc().collect::<String>(), "{s:?}");
        }
    }

    #[test]
    fn the_casefold_table_is_sorted_and_versioned() {
        assert!(CASEFOLD.windows(2).all(|w| w[0].0 < w[1].0), "binary search needs order");
        assert_eq!(crate::casefold_table::UNICODE_VERSION, "16.0.0");
        assert_eq!(unicode_versions(), "NFKC Unicode 17.0.0; case folding Unicode 16.0.0");
        let fold = |c: char| {
            let mut s = String::new();
            casefold(c, |f| s.push(f));
            s
        };
        assert_eq!(fold('\u{00DF}'), "ss");
        assert_eq!(fold('\u{212A}'), "k");
        assert_eq!(fold('\u{0130}'), "i\u{0307}");
        assert_eq!(fold('a'), "a");
    }
}
