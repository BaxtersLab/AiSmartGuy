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

use unicode_normalization::char::{canonical_combining_class, decompose_compatible};
use unicode_normalization::{is_nfkc_quick, IsNormalized, UnicodeNormalization};

use crate::casefold_table::CASEFOLD;

/// The label for a quote found in the chapter it cites.
pub const VERIFIED: &str = "Quote verified in chapter";
/// The label for a quote that is not in the chapter it cites.
pub const NOT_FOUND: &str = "Quote not found in chapter";

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
    /// text (`chapter.body().chars()`), not byte offsets.
    Verified { start: usize, end: usize },
    NotFound,
}

impl QuoteCheck {
    pub fn label(&self) -> &'static str {
        match self {
            QuoteCheck::Verified { .. } => VERIFIED,
            QuoteCheck::NotFound => NOT_FOUND,
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

    /// Check one quote. The first occurrence that begins and ends on whole
    /// source characters wins. An empty quote verifies nothing.
    pub fn check(&self, quote: &str) -> QuoteCheck {
        let q = normalize_mapped(quote).chars;
        let c = &self.mapped.chars;
        let spans = &self.mapped.spans;
        if q.is_empty() || q.len() > c.len() {
            return QuoteCheck::NotFound;
        }
        for i in 0..=c.len() - q.len() {
            let j = i + q.len();
            if c[i] != q[0] || c[i..j] != q[..] {
                continue;
            }
            let starts_whole = i == 0 || spans[i - 1] != spans[i];
            let ends_whole = j == c.len() || spans[j] != spans[j - 1];
            if starts_whole && ends_whole {
                return QuoteCheck::Verified { start: spans[i].0, end: spans[j - 1].1 };
            }
        }
        QuoteCheck::NotFound
    }
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

    fn slice(text: &str, start: usize, end: usize) -> String {
        text.chars().skip(start).take(end - start).collect()
    }

    /// Every verified match: the original slice normalises to the quote.
    fn assert_verified(quote: &str, chapter: &str) -> (usize, usize) {
        match check_quote(quote, chapter) {
            QuoteCheck::Verified { start, end } => {
                assert_eq!(normalize(&slice(chapter, start, end)), normalize(quote),
                    "the slice at {start}..{end} must normalise to the quote");
                (start, end)
            }
            QuoteCheck::NotFound => panic!("{quote:?} should verify"),
        }
    }

    #[test]
    fn labels_are_the_ruling_s_words() {
        assert_eq!(VERIFIED, "Quote verified in chapter");
        assert_eq!(NOT_FOUND, "Quote not found in chapter");
        assert_eq!(QuoteCheck::Verified { start: 0, end: 1 }.label(), VERIFIED);
        assert_eq!(QuoteCheck::NotFound.label(), NOT_FOUND);
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
        assert_verified("the author's claim", "Consider the author\u{2019}s claim first.");
        assert_verified("the author\u{2019}s claim", "Consider the author's claim first.");
        // A line break inside the quote is only whitespace.
        assert_verified("minutes said. Nobody asked", CHAPTER_1);
    }

    #[test]
    fn nfkc_and_full_case_folding_apply() {
        // A ligature in the book, plain letters in the quote.
        assert_verified("the fish", "Then the \u{FB01}sh swam away.");
        // Full folding, not lowercase: U+00DF folds to "ss".
        assert_verified("STRASSE", "Die Stra\u{00DF}e war leer.");
        // Fullwidth letters and a no-break space are NFKC compatibility forms.
        assert_verified("ABC def", "x \u{FF21}\u{FF22}\u{FF23}\u{00A0}def y");
        // Final sigma folds to sigma.
        assert_verified("\u{039F}\u{0394}\u{039F}\u{03A3}", "\u{03BF}\u{03B4}\u{03BF}\u{03C2} end");
    }

    #[test]
    fn whitespace_around_a_quote_is_trimmed() {
        // The passage opens the chapter, so nothing precedes it: a kept
        // leading space could never match there.
        let (s, e) = assert_verified("  \n It was the best of times \t", CHAPTER_1);
        assert_eq!((s, e), (0, 24));
    }

    #[test]
    fn soft_hyphens_and_zero_width_characters_are_removed() {
        assert_verified("unanimously", "voted una\u{00AD}nimously today");
        assert_verified("unanimously", "voted una\u{200B}nimous\u{FEFF}ly today");
        assert_verified("voted today", "voted \u{200D} today");
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
        assert_eq!(check_quote("it was the best of time", CHAPTER_1), QuoteCheck::Verified { start: 0, end: 23 },
            "a prefix of the text IS in the text; matching is substring, not word");
        assert_eq!(check_quote("It were the best of times", CHAPTER_1), QuoteCheck::NotFound);
    }

    #[test]
    fn catalog_phrases_are_not_found() {
        // The small models' failure: the rule catalog's own example wording,
        // presented as a quote from the book.
        for phrase in ["appeal to emotion", "everyone knows that", "loaded language"] {
            assert_eq!(check_quote(phrase, CHAPTER_1), QuoteCheck::NotFound, "{phrase}");
        }
    }

    #[test]
    fn empty_quotes_are_not_found() {
        for q in ["", "   ", "\u{200B}", "\u{00AD}\n\t"] {
            assert_eq!(check_quote(q, CHAPTER_1), QuoteCheck::NotFound, "{q:?}");
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
        // "ish" is inside the NFKC expansion of the ligature, not in the text.
        assert_eq!(check_quote("ish", "\u{FB01}sh"), QuoteCheck::NotFound);
        // A later whole-character occurrence still verifies.
        let t = "\u{FB01}sh and a dish";
        let (s, e) = assert_verified("ish", t);
        assert_eq!(slice(t, s, e), "ish");
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
