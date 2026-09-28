//! Phase 2 — structured findings pipeline (the lossless merge).
//!
//! Per-chapter model output is a JSON array of [`Finding`]s (shape enforced by
//! `assets/grammars/findings.gbnf`). Instead of folding *prose* through repeated
//! lossy LLM summarization, we parse each chapter's findings and **mechanically
//! merge** them — no model, zero loss — into a compact [`FindingsAggregate`] that
//! always fits the context window. One final LLM pass then writes the review from
//! the rendered table.
//!
//! Any chapter output that fails to parse is NOT discarded: it is returned in
//! [`MergeResult::parse_failures`] so the caller can route it through the existing
//! prose fold (`fusion.rs`) — the degraded-mode path, since some models fight
//! grammars.
//!
//! Every quote is checked against the chapter it cites (ASG-Q3, see
//! [`crate::quote_check`]). A finding whose quote is not there stays in the
//! report as an unsupported model inference; it is never scored or shown as
//! evidence.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

use crate::quote_check::{ChapterText, QuoteCheck};

/// The GBNF grammar, embedded at compile time (materialized to the run dir for
/// `--grammar-file`). Mirrors `assets/rag_defaults` embedding in `src-tauri`.
pub const FINDINGS_GBNF: &str = include_str!("../../../assets/grammars/findings.gbnf");

/// Per-chapter analysis instruction for structured-findings mode. The RAG rule
/// catalog stays in the system prompt; this tells the model to emit ONLY the
/// findings JSON array (empty `[]` when nothing is found).
pub const FINDINGS_INSTRUCTION: &str = "\
Analyze the passage above for manipulation tactics, logical fallacies, and \
rhetorical framing using the rule catalog. Respond with ONLY a JSON array of \
findings, nothing else. Each element must be:\n\
{\"rule_id\": <a rule id from the catalog>, \"quote\": <verbatim excerpt ≤200 \
chars>, \"location\": <page/paragraph hint>, \"severity\": \"low\"|\"medium\"|\"high\", \
\"note\": <≤160-char rationale>}\n\
If nothing is found, respond with exactly: []";

/// Materialize the embedded grammar into `run_dir/findings.gbnf` and return its
/// path (for `InferenceRequest::grammar_file` / llama `--grammar-file`).
pub fn write_grammar_file(run_dir: &Path) -> std::io::Result<PathBuf> {
    let path = run_dir.join("findings.gbnf");
    std::fs::write(&path, FINDINGS_GBNF)?;
    Ok(path)
}

/// A single structured finding emitted by a model for one chapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// Must match a loaded RAG rule id (`rag_engine::RagRule.rule_id`).
    pub rule_id: String,
    /// Verbatim excerpt (grammar caps it; we also clamp on render).
    pub quote: String,
    /// Page/paragraph hint.
    pub location: String,
    /// "low" | "medium" | "high".
    pub severity: String,
    /// Short rationale.
    pub note: String,
}

/// Ordered severity for max-severity aggregation. Unknown strings sort lowest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Unknown = 0,
    Low = 1,
    Medium = 2,
    High = 3,
}

impl Severity {
    pub fn parse(s: &str) -> Severity {
        match s.trim().to_ascii_lowercase().as_str() {
            "low" => Severity::Low,
            "medium" | "med" => Severity::Medium,
            "high" => Severity::High,
            _ => Severity::Unknown,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Unknown => "unknown",
            Severity::Low => "low",
            Severity::Medium => "medium",
            Severity::High => "high",
        }
    }
}

/// Max exemplar quotes kept per rule.
const MAX_EXEMPLARS: usize = 3;
/// Quote is clamped to this many chars in the Markdown table (grammar targets
/// ≤200). The JSON keeps every quote exactly as the model gave it.
const QUOTE_CLAMP: usize = 200;

/// A verified quote, kept as evidence for its rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exemplar {
    /// "chapter N": the chapter the quote was verified in.
    pub chapter: String,
    /// The model's quote, exactly as given.
    pub quote: String,
    /// Character offsets of the passage in that chapter's original text.
    pub start: usize,
    pub end: usize,
}

/// One finding with its quote checked against the chapter it cites (ASG-Q3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedFinding {
    /// "chapter N": the chapter the finding cites, the one the model analysed.
    pub chapter: String,
    /// The model's finding, its quote exactly as given.
    pub finding: Finding,
    pub check: QuoteCheck,
}

/// Aggregated findings for one rule across the whole book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleAggregate {
    pub rule_id: String,
    /// Findings backed by a quote verified in the chapter they cite: the
    /// evidence. Scores use this count, and only this.
    pub count: usize,
    /// Findings whose quote is not in the chapter they cite. They stay in the
    /// report as unsupported model inferences, never as evidence.
    pub unsupported: usize,
    /// Highest severity among the evidenced findings.
    pub max_severity: Severity,
    /// Distinct locations of the evidenced findings, in first-seen order.
    pub locations: Vec<String>,
    /// Up to [`MAX_EXEMPLARS`] verified quotes (near-duplicates collapsed).
    pub exemplars: Vec<Exemplar>,
}

/// The compact, lossless-on-findings aggregate for the whole book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingsAggregate {
    /// Per-rule aggregates, sorted by `rule_id` for deterministic output.
    pub rules: Vec<RuleAggregate>,
    /// Every finding in merge order, each with its quote check.
    pub findings: Vec<CheckedFinding>,
    /// Total findings across all rules, evidenced and unsupported.
    pub total_findings: usize,
    /// How many chapter outputs parsed successfully.
    pub chapters_parsed: usize,
}

impl FindingsAggregate {
    /// Quotes found in the chapter they cite.
    pub fn quotes_verified(&self) -> usize {
        self.findings.iter().filter(|f| f.check.is_verified()).count()
    }

    /// Every quote checked: one per finding, empty ones included.
    pub fn quotes_total(&self) -> usize {
        self.findings.len()
    }

    /// The report's header line.
    pub fn quotes_header(&self) -> String {
        format!("Quotes verified: {} of {}", self.quotes_verified(), self.quotes_total())
    }

    /// rule_id → evidenced count. Feeds `optimization` real per-rule hit counts.
    pub fn per_rule_counts(&self) -> BTreeMap<String, u32> {
        self.rules
            .iter()
            .map(|r| (r.rule_id.clone(), r.count as u32))
            .collect()
    }

    /// Convert per-rule evidenced counts to per-**category** counts via
    /// `rule_category` (rule_id → category, from the loaded RAG packets). Rules
    /// with no mapping are dropped (unknown / inactive category). This is what
    /// feeds `optimization::compute_scores_from_counts` the real per-category
    /// hits. An unsupported finding scores nothing.
    pub fn per_category_counts(
        &self,
        rule_category: &std::collections::HashMap<String, String>,
    ) -> std::collections::HashMap<String, u32> {
        let mut out: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for r in &self.rules {
            if let Some(cat) = rule_category.get(&r.rule_id) {
                *out.entry(cat.clone()).or_insert(0) += r.count as u32;
            }
        }
        out
    }
}

/// Result of merging every chapter's output.
#[derive(Debug, Clone)]
pub struct MergeResult {
    pub aggregate: FindingsAggregate,
    /// `(chapter_label, raw_prose)` for chapters whose output did not parse —
    /// route these through the prose fold so nothing is silently dropped.
    pub parse_failures: Vec<(String, String)>,
}

/// Parse a model output into findings. Tries strict JSON first, then a lenient
/// extraction of the first balanced `[ ... ]` array (tolerating leading/trailing
/// noise the grammar shouldn't produce but non-grammar models might). Returns
/// `None` only if no findings array can be recovered — the caller then treats the
/// output as prose and routes it to the fold.
pub fn parse_findings(raw: &str) -> Option<Vec<Finding>> {
    let trimmed = raw.trim();
    if let Ok(v) = serde_json::from_str::<Vec<Finding>>(trimmed) {
        return Some(v);
    }
    // Lenient backstop: extract the first top-level [ ... ] slice and retry.
    let slice = extract_json_array(trimmed)?;
    serde_json::from_str::<Vec<Finding>>(slice).ok()
}

/// Return the substring covering the first balanced top-level JSON array, or None.
/// Respects string literals + escapes so brackets inside quotes don't confuse it.
fn extract_json_array(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let start = bytes.iter().position(|&b| b == b'[')?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    for i in start..bytes.len() {
        let b = bytes[i];
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Merge every chapter output into one aggregate, checking each quote against
/// the chapter it cites (ASG-Q3). `outputs` is `(chapter_index, raw_output)`
/// in deterministic order, the index 0-based. `sources[i]` is chapter i's own
/// text (`pdf_io::Chapter::body`). An output whose chapter has no source
/// verifies nothing: all its quotes are "not found".
pub fn merge_findings(outputs: &[(usize, String)], sources: &[&str]) -> MergeResult {
    // rule_id → working aggregate. BTreeMap keeps rule_id order deterministic.
    let mut acc: BTreeMap<String, RuleAggregate> = BTreeMap::new();
    let mut checked: Vec<CheckedFinding> = Vec::new();
    let mut texts: BTreeMap<usize, ChapterText> = BTreeMap::new();
    let mut parse_failures: Vec<(String, String)> = Vec::new();
    let mut chapters_parsed = 0usize;

    for (ch, raw) in outputs {
        let label = format!("chapter {}", ch + 1);
        let Some(findings) = parse_findings(raw) else {
            // Keep the prose for the degraded-mode fold.
            if !raw.trim().is_empty() {
                parse_failures.push((label, raw.clone()));
            }
            continue;
        };
        chapters_parsed += 1;
        for f in findings {
            let check = match sources.get(*ch) {
                Some(body) => texts
                    .entry(*ch)
                    .or_insert_with(|| ChapterText::new(body))
                    .check(&f.quote),
                None => QuoteCheck::NotFound,
            };
            let entry = acc.entry(f.rule_id.clone()).or_insert_with(|| RuleAggregate {
                rule_id: f.rule_id.clone(),
                count: 0,
                unsupported: 0,
                max_severity: Severity::Unknown,
                locations: Vec::new(),
                exemplars: Vec::new(),
            });
            if let QuoteCheck::Verified { start, end } = check {
                entry.count += 1;
                let sev = Severity::parse(&f.severity);
                if sev > entry.max_severity {
                    entry.max_severity = sev;
                }
                let loc = if f.location.trim().is_empty() {
                    label.clone()
                } else {
                    format!("{}: {}", label, f.location.trim())
                };
                if !entry.locations.iter().any(|l| l == &loc) {
                    entry.locations.push(loc);
                }
                add_exemplar(&mut entry.exemplars, Exemplar {
                    chapter: label.clone(),
                    quote: f.quote.clone(),
                    start,
                    end,
                });
            } else {
                entry.unsupported += 1;
            }
            checked.push(CheckedFinding { chapter: label.clone(), finding: f, check });
        }
    }

    let rules: Vec<RuleAggregate> = acc.into_values().collect();
    MergeResult {
        aggregate: FindingsAggregate {
            rules,
            total_findings: checked.len(),
            findings: checked,
            chapters_parsed,
        },
        parse_failures,
    }
}

/// Add `ex` to `exemplars` unless a near-identical quote is already present, and
/// only up to [`MAX_EXEMPLARS`]. Near-identical = normalized substring containment
/// either way (handles the same excerpt with different surrounding whitespace or a
/// slightly longer/shorter clip).
fn add_exemplar(exemplars: &mut Vec<Exemplar>, ex: Exemplar) {
    if exemplars.len() >= MAX_EXEMPLARS {
        return;
    }
    let qn = normalize(&ex.quote);
    if qn.is_empty() {
        return;
    }
    for existing in exemplars.iter() {
        let en = normalize(&existing.quote);
        if en.contains(&qn) || qn.contains(&en) {
            return; // near-duplicate
        }
    }
    exemplars.push(ex);
}

/// Lowercase + collapse all whitespace runs to single spaces + trim.
fn normalize(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn clamp_quote(q: &str) -> String {
    let q = q.trim();
    if q.chars().count() <= QUOTE_CLAMP {
        return q.to_string();
    }
    let clipped: String = q.chars().take(QUOTE_CLAMP).collect();
    format!("{}…", clipped)
}

/// Render the aggregate as a Markdown findings table: the human artifact
/// written to the run dir, and the report body when no synthesis ran. Every
/// quote carries its label. An unsupported finding is listed as a model
/// inference, and its quote shown only as the model's claim, outside the
/// evidence.
pub fn render_findings_table(agg: &FindingsAggregate) -> String {
    render_table(agg, true)
}

/// The same table for the one final synthesis pass. The claimed quote of an
/// unsupported finding is left out, so the synthesis model is never handed
/// it to present as evidence.
pub fn render_synthesis_table(agg: &FindingsAggregate) -> String {
    render_table(agg, false)
}

fn render_table(agg: &FindingsAggregate, with_claimed_quotes: bool) -> String {
    let mut out = String::from("# Findings\n\n");
    out.push_str(&agg.quotes_header());
    out.push_str("\n\n");
    if agg.rules.is_empty() {
        out.push_str("_No structured findings were produced._\n");
        return out;
    }
    let verified = agg.quotes_verified();
    out.push_str(&format!(
        "{} finding(s) across {} rule(s), from {} chapter(s). {} are backed by a quote \
         verified in the chapter they cite; {} are unsupported model inferences, their \
         quotes not found there.\n\n",
        agg.total_findings,
        agg.rules.len(),
        agg.chapters_parsed,
        verified,
        agg.total_findings - verified
    ));
    for r in &agg.rules {
        if r.count > 0 {
            out.push_str(&format!(
                "## {} — {} evidenced occurrence(s), max severity {}\n\n",
                r.rule_id,
                r.count,
                r.max_severity.as_str()
            ));
        } else {
            out.push_str(&format!("## {} — no evidenced occurrence\n\n", r.rule_id));
        }
        if !r.locations.is_empty() {
            out.push_str("- Locations: ");
            out.push_str(&r.locations.join("; "));
            out.push('\n');
        }
        for ex in &r.exemplars {
            out.push_str(&format!(
                "- {} {} (characters {}–{}): \"{}\"\n",
                crate::quote_check::VERIFIED,
                ex.chapter.trim_start_matches("chapter "),
                ex.start,
                ex.end,
                clamp_quote(&ex.quote)
            ));
        }
        for cf in agg.findings.iter().filter(|cf| cf.finding.rule_id == r.rule_id && !cf.check.is_verified()) {
            out.push_str(&format!(
                "- Unsupported model inference ({}, severity {}): {}\n",
                cf.chapter,
                Severity::parse(&cf.finding.severity).as_str(),
                cf.finding.note.trim()
            ));
            if with_claimed_quotes {
                out.push_str(&format!(
                    "  - {} {}. Model's claimed quote (not found in chapter): \"{}\"\n",
                    crate::quote_check::NOT_FOUND,
                    cf.chapter.trim_start_matches("chapter "),
                    clamp_quote(&cf.finding.quote)
                ));
            }
        }
        out.push('\n');
    }
    out
}

/// Serialize the aggregate to JSON for the run-dir `.json` artifact. Every
/// finding is listed with its quote exactly as given, its label, and, when
/// verified, the passage's character offsets in the cited chapter.
pub fn render_findings_json(agg: &FindingsAggregate) -> String {
    // Hand-rolled stable JSON (RuleAggregate isn't Serialize to keep Severity as
    // an ordered enum). Deterministic field order.
    let mut items: Vec<String> = Vec::with_capacity(agg.rules.len());
    for r in &agg.rules {
        let locs = r
            .locations
            .iter()
            .map(|l| json_string(l))
            .collect::<Vec<_>>()
            .join(",");
        let exs = r
            .exemplars
            .iter()
            .map(|e| format!(
                "{{\"chapter\":{},\"quote\":{},\"start\":{},\"end\":{}}}",
                json_string(&e.chapter), json_string(&e.quote), e.start, e.end
            ))
            .collect::<Vec<_>>()
            .join(",");
        items.push(format!(
            "{{\"rule_id\":{},\"count\":{},\"unsupported\":{},\"max_severity\":{},\"locations\":[{}],\"exemplars\":[{}]}}",
            json_string(&r.rule_id),
            r.count,
            r.unsupported,
            json_string(r.max_severity.as_str()),
            locs,
            exs
        ));
    }
    let findings: Vec<String> = agg
        .findings
        .iter()
        .map(|cf| {
            let offsets = match cf.check {
                QuoteCheck::Verified { start, end } => format!("{{\"start\":{start},\"end\":{end}}}"),
                QuoteCheck::NotFound => "null".to_string(),
            };
            format!(
                "{{\"chapter\":{},\"rule_id\":{},\"severity\":{},\"location\":{},\"note\":{},\"quote\":{},\"quote_check\":{},\"source_offsets\":{}}}",
                json_string(&cf.chapter),
                json_string(&cf.finding.rule_id),
                json_string(&cf.finding.severity),
                json_string(&cf.finding.location),
                json_string(&cf.finding.note),
                json_string(&cf.finding.quote),
                json_string(cf.check.label()),
                offsets
            )
        })
        .collect();
    format!(
        "{{\"quotes_verified\":{},\"quotes_total\":{},\"quote_normalisation\":{},\"total_findings\":{},\"chapters_parsed\":{},\"rules\":[{}],\"findings\":[{}]}}",
        agg.quotes_verified(),
        agg.quotes_total(),
        json_string(&crate::quote_check::unicode_versions()),
        agg.total_findings,
        agg.chapters_parsed,
        items.join(","),
        findings.join(",")
    )
}

fn json_string(s: &str) -> String {
    // Reuse serde for correct escaping.
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quote_check::{NOT_FOUND, VERIFIED};

    const CH1: &str = "The minister said \u{201C}every family will be better off\u{201D} and left the room.";
    const CH2: &str = "Later the figures were revised down. Everyone knows the minister was wrong.";

    fn f(rule: &str, quote: &str, loc: &str, sev: &str) -> String {
        serde_json::to_string(&vec![Finding {
            rule_id: rule.into(),
            quote: quote.into(),
            location: loc.into(),
            severity: sev.into(),
            note: "n".into(),
        }])
        .unwrap()
    }

    fn merge(outputs: Vec<(usize, String)>) -> MergeResult {
        merge_findings(&outputs, &[CH1, CH2])
    }

    fn slice(text: &str, start: usize, end: usize) -> String {
        text.chars().skip(start).take(end - start).collect()
    }

    #[test]
    fn parses_valid_findings_array() {
        let v = parse_findings(&f("F-01", "some quote", "p1", "high")).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].rule_id, "F-01");
        assert_eq!(v[0].severity, "high");
    }

    #[test]
    fn parses_empty_array_as_no_findings() {
        let v = parse_findings("[]").unwrap();
        assert!(v.is_empty());
    }

    #[test]
    fn lenient_extractor_recovers_array_from_noise() {
        let noisy = "Here are the findings:\n[{\"rule_id\":\"F-02\",\"quote\":\"q\",\"location\":\"l\",\"severity\":\"low\",\"note\":\"n\"}]\nDone.";
        let v = parse_findings(noisy).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].rule_id, "F-02");
    }

    #[test]
    fn extractor_ignores_brackets_inside_strings() {
        let s = "[{\"rule_id\":\"F-03\",\"quote\":\"a ] bracket [ inside\",\"location\":\"l\",\"severity\":\"medium\",\"note\":\"n\"}]";
        let v = parse_findings(s).unwrap();
        assert_eq!(v.len(), 1);
        assert!(v[0].quote.contains("] bracket ["));
    }

    #[test]
    fn garbage_returns_none_for_fallback() {
        assert!(parse_findings("this is just prose, not findings").is_none());
        assert!(parse_findings("").is_none());
    }

    #[test]
    fn merge_counts_and_max_severity() {
        let res = merge(vec![
            (0, f("F-01", "every family will be better off", "p1", "low")),
            (1, f("F-01", "the figures were revised down", "p9", "high")),
        ]);
        assert_eq!(res.aggregate.rules.len(), 1);
        let r = &res.aggregate.rules[0];
        assert_eq!(r.rule_id, "F-01");
        assert_eq!(r.count, 2);
        assert_eq!(r.unsupported, 0);
        assert_eq!(r.max_severity, Severity::High); // max(low, high)
        assert_eq!(r.locations, vec!["chapter 1: p1", "chapter 2: p9"]);
        assert_eq!(res.aggregate.total_findings, 2);
        assert_eq!(res.aggregate.chapters_parsed, 2);
        assert!(res.parse_failures.is_empty());
    }

    #[test]
    fn merge_dedupes_near_identical_quotes() {
        let res = merge(vec![
            (0, f("F-05", "every family will be better off", "p1", "low")),
            // same quote with extra whitespace → near-dup
            (0, f("F-05", "  every   family will be better off  ", "p2", "low")),
        ]);
        let r = &res.aggregate.rules[0];
        assert_eq!(r.count, 2, "both count");
        assert_eq!(r.exemplars.len(), 1, "near-duplicate quote collapsed to one exemplar");
    }

    #[test]
    fn merge_caps_exemplars_at_three() {
        let book: String = (0..6).map(|i| format!("Sentence number {i} is here. ")).collect();
        let outputs: Vec<(usize, String)> = (0..6)
            .map(|i| (0, f("F-07", &format!("sentence number {i} is here"), "p", "low")))
            .collect();
        let res = merge_findings(&outputs, &[&book]);
        let r = &res.aggregate.rules[0];
        assert_eq!(r.count, 6);
        assert_eq!(r.exemplars.len(), MAX_EXEMPLARS);
    }

    #[test]
    fn merge_routes_parse_failures_to_fallback() {
        let res = merge(vec![
            (0, f("F-01", "every family", "p", "low")),
            (1, "prose that is not JSON at all".to_string()),
        ]);
        assert_eq!(res.aggregate.chapters_parsed, 1);
        assert_eq!(res.parse_failures.len(), 1);
        assert_eq!(res.parse_failures[0].0, "chapter 2");
    }

    #[test]
    fn rules_are_sorted_deterministically() {
        let res = merge(vec![
            (0, f("F-09", "q", "p", "low")),
            (0, f("F-01", "q", "p", "low")),
            (1, f("F-05", "q", "p", "low")),
        ]);
        let ids: Vec<&str> = res.aggregate.rules.iter().map(|r| r.rule_id.as_str()).collect();
        assert_eq!(ids, vec!["F-01", "F-05", "F-09"]);
    }

    #[test]
    fn per_rule_counts_match_aggregate() {
        let counts = merge(vec![
            (0, f("F-01", "every family", "p", "low")),
            (1, f("F-01", "the figures were revised", "p", "low")),
            (1, f("F-02", "the minister was wrong", "p", "low")),
        ])
        .aggregate
        .per_rule_counts();
        assert_eq!(counts.get("F-01"), Some(&2));
        assert_eq!(counts.get("F-02"), Some(&1));
    }

    fn rule_categories() -> std::collections::HashMap<String, String> {
        // F-01,F-02 → "fallacy"; F-09 → "tone"; F-99 has no mapping (dropped).
        [("F-01", "fallacy"), ("F-02", "fallacy"), ("F-09", "tone")]
            .into_iter()
            .map(|(r, c)| (r.to_string(), c.to_string()))
            .collect()
    }

    #[test]
    fn per_category_counts_maps_rules_to_categories() {
        let agg = merge(vec![
            (0, f("F-01", "every family", "p", "low")),
            (1, f("F-02", "the figures were revised", "p", "low")),
            (1, f("F-09", "the minister was wrong", "p", "low")),
            (0, f("F-99", "left the room", "p", "low")), // unmapped → dropped
        ])
        .aggregate;
        let cats = agg.per_category_counts(&rule_categories());
        assert_eq!(cats.get("fallacy"), Some(&2), "F-01 + F-02 → fallacy = 2");
        assert_eq!(cats.get("tone"), Some(&1), "F-09 → tone = 1");
        assert_eq!(cats.get("unknown"), None, "unmapped rule contributes nothing");
        assert_eq!(cats.len(), 2);
    }

    #[test]
    fn an_unverified_quote_is_not_evidence_and_scores_nothing() {
        let agg = merge(vec![
            (0, f("F-01", "every family will be better off", "p1", "low")),
            // The rule catalog's own wording, presented as a quote: not in the book.
            (0, f("F-01", "appeal to emotion", "p2", "high")),
            (1, f("F-09", "an invented line", "p3", "high")),
        ])
        .aggregate;
        let r = &agg.rules[0];
        assert_eq!((r.rule_id.as_str(), r.count, r.unsupported), ("F-01", 1, 1));
        assert_eq!(r.max_severity, Severity::Low, "the unsupported 'high' raises nothing");
        assert_eq!(r.locations, vec!["chapter 1: p1"]);
        assert_eq!(r.exemplars.len(), 1);
        assert_eq!(r.exemplars[0].quote, "every family will be better off");
        // A rule with only unsupported findings stays listed, with no evidence.
        let r9 = &agg.rules[1];
        assert_eq!((r9.rule_id.as_str(), r9.count, r9.unsupported), ("F-09", 0, 1));
        assert_eq!(r9.max_severity, Severity::Unknown);
        assert!(r9.exemplars.is_empty() && r9.locations.is_empty());
        // Scores count evidence only.
        assert_eq!(agg.per_rule_counts().get("F-01"), Some(&1));
        assert_eq!(agg.per_rule_counts().get("F-09"), Some(&0));
        let cats = agg.per_category_counts(&rule_categories());
        assert_eq!(cats.get("fallacy"), Some(&1));
        assert_eq!(cats.get("tone"), Some(&0));
        // Every finding is kept.
        assert_eq!(agg.total_findings, 3);
        assert_eq!(agg.findings.len(), 3);
    }

    #[test]
    fn a_quote_found_only_in_another_chapter_is_not_found() {
        let q = "the figures were revised down";
        let agg = merge(vec![(0, f("F-01", q, "p", "low"))]).aggregate;
        assert_eq!(agg.findings[0].check, QuoteCheck::NotFound, "it is in chapter 2, not 1");
        // Control: cited correctly, it verifies.
        let agg = merge(vec![(1, f("F-01", q, "p", "low"))]).aggregate;
        assert!(agg.findings[0].check.is_verified());
    }

    #[test]
    fn an_output_with_no_source_verifies_nothing() {
        let agg = merge(vec![(5, f("F-01", "every family", "p", "low"))]).aggregate;
        assert_eq!(agg.findings[0].check, QuoteCheck::NotFound);
        assert_eq!(agg.findings[0].chapter, "chapter 6");
    }

    #[test]
    fn an_empty_quote_is_an_unsupported_inference() {
        let agg = merge(vec![(0, f("F-01", "", "p", "high"))]).aggregate;
        assert_eq!((agg.rules[0].count, agg.rules[0].unsupported), (0, 1));
        assert_eq!(agg.quotes_header(), "Quotes verified: 0 of 1");
    }

    #[test]
    fn the_original_quote_is_kept_and_its_offsets_find_the_passage() {
        let quote = "said  \"Every family will be better off\"";
        let agg = merge(vec![(0, f("F-01", quote, "p", "low"))]).aggregate;
        let cf = &agg.findings[0];
        assert_eq!(cf.finding.quote, quote, "kept exactly as the model gave it");
        let QuoteCheck::Verified { start, end } = cf.check else { panic!("should verify") };
        assert_eq!(slice(CH1, start, end), "said \u{201C}every family will be better off\u{201D}");
        assert_eq!(crate::quote_check::normalize(&slice(CH1, start, end)),
                   crate::quote_check::normalize(quote));
        let json: serde_json::Value = serde_json::from_str(&render_findings_json(&agg)).unwrap();
        assert_eq!(json["findings"][0]["quote"], quote);
        assert_eq!(json["findings"][0]["source_offsets"]["start"], start);
        assert_eq!(json["findings"][0]["source_offsets"]["end"], end);
        assert_eq!(json["rules"][0]["exemplars"][0]["quote"], quote);
        assert_eq!(json["rules"][0]["exemplars"][0]["start"], start);
    }

    fn mixed() -> FindingsAggregate {
        merge(vec![
            (0, f("F-01", "every family will be better off", "p1", "low")),
            (0, f("F-01", "appeal to emotion", "p2", "high")),
        ])
        .aggregate
    }

    #[test]
    fn counts_and_labels_are_prominent() {
        let agg = mixed();
        assert_eq!(agg.quotes_header(), "Quotes verified: 1 of 2");
        let table = render_findings_table(&agg);
        assert!(table.starts_with("# Findings\n\nQuotes verified: 1 of 2\n"), "{table}");
        assert!(table.contains(&format!("- {VERIFIED} 1 (characters 19–50): \"every family will be better off\"")), "{table}");
        assert!(table.contains(&format!("{NOT_FOUND} 1. Model's claimed quote (not found in chapter): \"appeal to emotion\"")), "{table}");
        let json: serde_json::Value = serde_json::from_str(&render_findings_json(&agg)).unwrap();
        assert_eq!(json["quotes_verified"], 1);
        assert_eq!(json["quotes_total"], 2);
        assert_eq!(json["quote_normalisation"], crate::quote_check::unicode_versions());
        assert_eq!(json["findings"][0]["quote_check"], VERIFIED);
        assert_eq!(json["findings"][1]["quote_check"], NOT_FOUND);
        assert!(json["findings"][1]["source_offsets"].is_null());
        assert_eq!(json["rules"][0]["count"], 1);
        assert_eq!(json["rules"][0]["unsupported"], 1);
    }

    #[test]
    fn the_claimed_quote_is_never_shown_as_evidence() {
        let agg = mixed();
        let table = render_findings_table(&agg);
        for line in table.lines().filter(|l| l.contains("appeal to emotion")) {
            assert!(line.contains("Model's claimed quote (not found in chapter)"), "{line}");
            assert!(!line.contains(VERIFIED), "{line}");
        }
        assert!(table.contains("- Unsupported model inference (chapter 1, severity high): n"), "{table}");
        // The synthesis pass keeps the inference but is never handed the claim.
        let synth = render_synthesis_table(&agg);
        assert!(!synth.contains("appeal to emotion"), "{synth}");
        assert!(synth.contains("- Unsupported model inference (chapter 1, severity high): n"), "{synth}");
        assert!(synth.contains("every family will be better off"), "evidence still goes in");
        assert!(synth.starts_with("# Findings\n\nQuotes verified: 1 of 2\n"));
    }

    #[test]
    fn render_table_contains_rule_and_severity() {
        let agg = merge(vec![(0, f("F-01", "left the room", "p1", "high"))]).aggregate;
        let table = render_findings_table(&agg);
        assert!(table.contains("F-01"));
        assert!(table.contains("high"));
        assert!(table.contains("left the room"));
    }

    #[test]
    fn render_json_is_valid_and_stable() {
        let agg = merge(vec![(0, f("F-01", "left the room", "p1", "high"))]).aggregate;
        let json = render_findings_json(&agg);
        // Round-trips as valid JSON.
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["total_findings"], 1);
        assert_eq!(parsed["rules"][0]["rule_id"], "F-01");
        assert_eq!(parsed["rules"][0]["max_severity"], "high");
        assert_eq!(json, render_findings_json(&agg));
    }

    #[test]
    fn quote_is_clamped_on_render_but_kept_whole_in_json() {
        let long = "x".repeat(500);
        let agg = merge(vec![(0, f("F-01", &long, "p", "low"))]).aggregate;
        let table = render_findings_table(&agg);
        assert!(table.contains('…'), "over-long quote must be clamped with an ellipsis");
        let json: serde_json::Value = serde_json::from_str(&render_findings_json(&agg)).unwrap();
        assert_eq!(json["findings"][0]["quote"], long.as_str());
    }

    #[test]
    fn empty_aggregate_renders_placeholder() {
        let res = merge_findings(&[], &[]);
        let table = render_findings_table(&res.aggregate);
        assert!(table.contains("No structured findings"));
        assert!(table.contains("Quotes verified: 0 of 0"));
    }
}

#[cfg(test)]
mod grammar_tests {
    use super::FINDINGS_GBNF;

    /// Lines that continue a rule outside parentheses. llama.cpp's GBNF parser
    /// ends a rule at a newline unless it is inside parentheses, so such a
    /// line is read as a new rule with no name. Real failure, 2026-09-27:
    /// `error parsing grammar: expecting name at "\"rule_id\""`. The in-app
    /// b10238 build aborted, and the archive 8681 build dropped the grammar
    /// and ran unconstrained. Phase 2 had never run against a real llama.
    fn continuations_outside_parens(grammar: &str) -> Vec<String> {
        let mut bad = Vec::new();
        let mut depth = 0i32;
        for line in grammar.lines() {
            let body = line.trim();
            let starts_rule = body.split_once("::=").map_or(false, |(name, _)| {
                let n = name.trim();
                !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            });
            if depth == 0 && !body.is_empty() && !body.starts_with('#') && !starts_rule {
                bad.push(line.to_string());
            }
            let (mut in_str, mut in_class, mut esc) = (false, false, false);
            for c in line.chars() {
                if esc { esc = false; continue; }
                match c {
                    '\\' if in_str || in_class => esc = true,
                    '"' if !in_class => in_str = !in_str,
                    '[' if !in_str => in_class = true,
                    ']' if in_class => in_class = false,
                    '#' if !in_str && !in_class => break,
                    '(' if !in_str && !in_class => depth += 1,
                    ')' if !in_str && !in_class => depth -= 1,
                    _ => {}
                }
            }
        }
        if depth != 0 {
            bad.push(format!("unbalanced parentheses (depth {depth} at the end)"));
        }
        bad
    }

    #[test]
    fn every_line_of_the_findings_grammar_parses_in_llama_cpp() {
        assert_eq!(continuations_outside_parens(FINDINGS_GBNF), Vec::<String>::new());
    }

    /// Whitespace between tokens is bounded, so a model cannot spend the pass
    /// on blank lines. Read from the grammar itself: the ws rule's class must
    /// carry an explicit {0,N} bound, never `*` or `+`.
    #[test]
    fn whitespace_in_the_findings_grammar_is_bounded() {
        let ws = FINDINGS_GBNF.lines()
            .find(|l| l.trim_start().starts_with("ws ") || l.trim_start().starts_with("ws\t"))
            .expect("a ws rule");
        let body = ws.split_once("::=").unwrap().1.trim();
        assert!(body.ends_with('}') && body.contains("{0,"), "unbounded whitespace: {body}");
        let bound: usize = body.rsplit_once(',').unwrap().1.trim_end_matches('}').parse().unwrap();
        assert!(bound <= 32, "bound too loose: {bound}");
    }

    /// Controls: the checker catches the broken shape, and passes the fixed one.
    #[test]
    fn a_rule_that_runs_on_outside_parentheses_is_caught() {
        assert_eq!(continuations_outside_parens("a ::= \"x\" ws\n      \"y\"\nws ::= [ \\t]*\n"),
                   vec!["      \"y\"".to_string()]);
        assert!(continuations_outside_parens("a ::= \"x\" (\n      \"(y\" [)]\n  ) \"z\"\n# c (\n").is_empty());
    }
}

