/// RAG Hitlist Catalog — numbered specialist categories.
///
/// Each category defines a theme that models are tested against.
/// The optimization engine scores models per-category across 10 runs
/// to determine which model excels at each specialist theme.
///
/// RAG instruction documents (.json) for each category live in
/// `<model_dir>/rag/` and are loaded by the packet loader. This
/// catalog defines the *canonical set* of categories the system
/// recognises, independent of whether the .json documents exist yet.

use serde::{Deserialize, Serialize};

/// A single hitlist entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitlistEntry {
    /// Numeric code, e.g. 1, 2, 3…
    pub code: u16,
    /// Machine-readable slug used in scoring, manifest, and RAG packets.
    pub slug: String,
    /// Human-readable display name.
    pub name: String,
    /// Short description of what this category detects.
    pub description: String,
    /// true = available for runs now; false = future / not yet instrumented.
    pub active: bool,
}

/// Return the full hitlist catalog.
///
/// Categories 1–6 are from the Module A design spec.
/// Categories 7+ are future expansions (active = false).
pub fn catalog() -> Vec<HitlistEntry> {
    vec![
        HitlistEntry {
            code: 1,
            slug: "fallacies".into(),
            name: "Logical Fallacies".into(),
            description: "Strawman, ad hominem, slippery slope, false dichotomy, etc.".into(),
            active: true,
        },
        HitlistEntry {
            code: 2,
            slug: "weaponized_language".into(),
            name: "Weaponized Language".into(),
            description: "Language designed to coerce, shame, or silence.".into(),
            active: true,
        },
        HitlistEntry {
            code: 3,
            slug: "euphemisms".into(),
            name: "Euphemisms & Dysphemisms".into(),
            description: "Softened or loaded substitutions that obscure meaning.".into(),
            active: true,
        },
        HitlistEntry {
            code: 4,
            slug: "conceptual_mismatches".into(),
            name: "Conceptual Mismatches".into(),
            description: "Covered by FAL-23 (False Equivalence) in the fallacies packet.".into(),
            active: false,
        },
        HitlistEntry {
            code: 5,
            slug: "tone_patterns".into(),
            name: "Tone Patterns".into(),
            description: "Authoritative, dismissive, patronising, or emotionally charged tone.".into(),
            active: false,
        },
        HitlistEntry {
            code: 6,
            slug: "ambiguous_framing".into(),
            name: "Ambiguous Framing".into(),
            description: "Open-ended claims with no direction, meandering thought, zero-evidence assertions expecting belief, and self-victimization framing.".into(),
            active: true,
        },
        // ── Future categories (active = false) ──────────────────
        HitlistEntry {
            code: 7,
            slug: "racism_intolerance".into(),
            name: "Racism & Intolerance".into(),
            description: "Discriminatory language, stereotyping, coded prejudice, and dehumanizing rhetoric targeting race, ethnicity, religion, gender, or identity.".into(),
            active: true,
        },
        HitlistEntry {
            code: 8,
            slug: "philosophical_filters".into(),
            name: "Philosophical Filters".into(),
            description: "Hidden ideological or philosophical bias in argumentation.".into(),
            active: false,
        },
        HitlistEntry {
            code: 9,
            slug: "nlp_techniques".into(),
            name: "NLP Techniques".into(),
            description: "Neuro-Linguistic Programming patterns: presuppositions, embedded commands, pacing-and-leading, double binds, Milton Model violations, anchoring, and future pacing.".into(),
            active: true,
        },
        HitlistEntry {
            code: 10,
            slug: "humor_detection".into(),
            name: "Humor Detection".into(),
            description: "Jokes, irony, satire, absurdist humour — flag when humour is used to smuggle a serious claim past critical scrutiny.".into(),
            active: false,
        },
        HitlistEntry {
            code: 11,
            slug: "sarcasm_detection".into(),
            name: "Sarcasm Detection".into(),
            description: "Sarcastic tone, mock agreement, rhetorical contempt — detect when surface meaning inverts intended meaning to belittle or dismiss.".into(),
            active: false,
        },
    ]
}

/// Return only the active hitlist slugs (used as `categories_active` in the manifest).
pub fn active_slugs() -> Vec<String> {
    catalog()
        .into_iter()
        .filter(|e| e.active)
        .map(|e| e.slug)
        .collect()
}

/// Return only the active entries.
pub fn active_entries() -> Vec<HitlistEntry> {
    catalog().into_iter().filter(|e| e.active).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_entries() {
        let c = catalog();
        assert!(c.len() >= 6, "expected at least 6 hitlist categories");
    }

    #[test]
    fn active_slugs_non_empty() {
        let slugs = active_slugs();
        assert!(!slugs.is_empty());
        assert!(slugs.contains(&"fallacies".to_string()));
    }

    #[test]
    fn codes_unique() {
        let c = catalog();
        let mut codes: Vec<u16> = c.iter().map(|e| e.code).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), c.len(), "hitlist codes must be unique");
    }

    #[test]
    fn slugs_unique() {
        let c = catalog();
        let mut slugs: Vec<String> = c.iter().map(|e| e.slug.clone()).collect();
        slugs.sort();
        slugs.dedup();
        assert_eq!(slugs.len(), c.len(), "hitlist slugs must be unique");
    }
}
