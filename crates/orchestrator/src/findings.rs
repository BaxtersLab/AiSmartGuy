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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

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
/// Quote is clamped to this many chars on render (grammar targets ≤200).
const QUOTE_CLAMP: usize = 200;

/// Aggregated findings for one rule across the whole book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleAggregate {
    pub rule_id: String,
    /// Total number of findings for this rule across all chapters.
    pub count: usize,
    /// Highest severity seen.
    pub max_severity: Severity,
    /// Distinct per-chapter locations (dedup'd, in first-seen order).
    pub locations: Vec<String>,
    /// Up to [`MAX_EXEMPLARS`] representative quotes (near-duplicates collapsed).
    pub exemplars: Vec<String>,
}

/// The compact, lossless-on-findings aggregate for the whole book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingsAggregate {
    /// Per-rule aggregates, sorted by `rule_id` for deterministic output.
    pub rules: Vec<RuleAggregate>,
    /// Total findings across all rules.
    pub total_findings: usize,
    /// How many chapter outputs parsed successfully.
    pub chapters_parsed: usize,
}

impl FindingsAggregate {
    /// rule_id → count. Feeds `optimization` real per-rule hit counts.
    pub fn per_rule_counts(&self) -> BTreeMap<String, u32> {
        self.rules
            .iter()
            .map(|r| (r.rule_id.clone(), r.count as u32))
            .collect()
    }

    /// Convert per-rule counts to per-**category** counts via `rule_category`
    /// (rule_id → category, from the loaded RAG packets). Rules with no mapping
    /// are dropped (unknown / inactive category). This is what feeds
    /// `optimization::compute_scores_from_counts` the real per-category hits.
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

/// Merge every chapter's output into one aggregate. `chapters` is
/// `(chapter_label, raw_output)` in deterministic order.
pub fn merge_findings(chapters: &[(String, String)]) -> MergeResult {
    // rule_id → working aggregate. BTreeMap keeps rule_id order deterministic.
    let mut acc: BTreeMap<String, RuleAggregate> = BTreeMap::new();
    let mut parse_failures: Vec<(String, String)> = Vec::new();
    let mut chapters_parsed = 0usize;
    let mut total = 0usize;

    for (label, raw) in chapters {
        match parse_findings(raw) {
            Some(findings) => {
                chapters_parsed += 1;
                for f in findings {
                    total += 1;
                    let sev = Severity::parse(&f.severity);
                    let entry = acc.entry(f.rule_id.clone()).or_insert_with(|| RuleAggregate {
                        rule_id: f.rule_id.clone(),
                        count: 0,
                        max_severity: Severity::Unknown,
                        locations: Vec::new(),
                        exemplars: Vec::new(),
                    });
                    entry.count += 1;
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
                    add_exemplar(&mut entry.exemplars, &f.quote);
                }
            }
            None => {
                // Keep the prose for the degraded-mode fold.
                if !raw.trim().is_empty() {
                    parse_failures.push((label.clone(), raw.clone()));
                }
            }
        }
    }

    let rules: Vec<RuleAggregate> = acc.into_values().collect();
    MergeResult {
        aggregate: FindingsAggregate {
            rules,
            total_findings: total,
            chapters_parsed,
        },
        parse_failures,
    }
}

/// Add `quote` to `exemplars` unless a near-identical one is already present, and
/// only up to [`MAX_EXEMPLARS`]. Near-identical = normalized substring containment
/// either way (handles the same excerpt with different surrounding whitespace or a
/// slightly longer/shorter clip).
fn add_exemplar(exemplars: &mut Vec<String>, quote: &str) {
    let q = quote.trim();
    if q.is_empty() || exemplars.len() >= MAX_EXEMPLARS {
        return;
    }
    let qn = normalize(q);
    if qn.is_empty() {
        return;
    }
    for existing in exemplars.iter() {
        let en = normalize(existing);
        if en.contains(&qn) || qn.contains(&en) {
            return; // near-duplicate
        }
    }
    exemplars.push(q.to_string());
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

/// Render the aggregate as a Markdown findings table — the compact input to the
/// single final synthesis pass (and a human artifact written to the run dir).
pub fn render_findings_table(agg: &FindingsAggregate) -> String {
    if agg.rules.is_empty() {
        return "# Findings\n\n_No structured findings were produced._\n".to_string();
    }
    let mut out = String::new();
    out.push_str("# Findings\n\n");
    out.push_str(&format!(
        "{} finding(s) across {} rule(s), from {} chapter(s).\n\n",
        agg.total_findings,
        agg.rules.len(),
        agg.chapters_parsed
    ));
    for r in &agg.rules {
        out.push_str(&format!(
            "## {} — {} occurrence(s), max severity {}\n\n",
            r.rule_id,
            r.count,
            r.max_severity.as_str()
        ));
        if !r.locations.is_empty() {
            out.push_str("- Locations: ");
            out.push_str(&r.locations.join("; "));
            out.push('\n');
        }
        for ex in &r.exemplars {
            out.push_str(&format!("- Quote: \"{}\"\n", clamp_quote(ex)));
        }
        out.push('\n');
    }
    out
}

/// Serialize the aggregate to JSON for the run-dir `.json` artifact.
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
            .map(|e| json_string(&clamp_quote(e)))
            .collect::<Vec<_>>()
            .join(",");
        items.push(format!(
            "{{\"rule_id\":{},\"count\":{},\"max_severity\":{},\"locations\":[{}],\"exemplars\":[{}]}}",
            json_string(&r.rule_id),
            r.count,
            json_string(r.max_severity.as_str()),
            locs,
            exs
        ));
    }
    format!(
        "{{\"total_findings\":{},\"chapters_parsed\":{},\"rules\":[{}]}}",
        agg.total_findings,
        agg.chapters_parsed,
        items.join(",")
    )
}

fn json_string(s: &str) -> String {
    // Reuse serde for correct escaping.
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let chapters = vec![
            ("ch1".to_string(), f("F-01", "quote A", "p1", "low")),
            ("ch2".to_string(), f("F-01", "quote B is different", "p9", "high")),
        ];
        let res = merge_findings(&chapters);
        assert_eq!(res.aggregate.rules.len(), 1);
        let r = &res.aggregate.rules[0];
        assert_eq!(r.rule_id, "F-01");
        assert_eq!(r.count, 2);
        assert_eq!(r.max_severity, Severity::High); // max(low, high)
        assert_eq!(r.locations.len(), 2);
        assert_eq!(res.aggregate.total_findings, 2);
        assert_eq!(res.aggregate.chapters_parsed, 2);
        assert!(res.parse_failures.is_empty());
    }

    #[test]
    fn merge_dedupes_near_identical_quotes() {
        let chapters = vec![
            ("ch1".to_string(), f("F-05", "the manipulative phrase here", "p1", "low")),
            // same quote with extra whitespace + surrounding text → near-dup
            ("ch2".to_string(), f("F-05", "  the   manipulative phrase here  ", "p2", "low")),
        ];
        let res = merge_findings(&chapters);
        let r = &res.aggregate.rules[0];
        assert_eq!(r.count, 2, "both count");
        assert_eq!(r.exemplars.len(), 1, "near-duplicate quote collapsed to one exemplar");
    }

    #[test]
    fn merge_caps_exemplars_at_three() {
        let chapters: Vec<(String, String)> = (0..6)
            .map(|i| (format!("ch{i}"), f("F-07", &format!("distinct quote number {i}"), "p", "low")))
            .collect();
        let res = merge_findings(&chapters);
        let r = &res.aggregate.rules[0];
        assert_eq!(r.count, 6);
        assert_eq!(r.exemplars.len(), MAX_EXEMPLARS);
    }

    #[test]
    fn merge_routes_parse_failures_to_fallback() {
        let chapters = vec![
            ("ch1".to_string(), f("F-01", "q", "p", "low")),
            ("ch2".to_string(), "prose that is not JSON at all".to_string()),
        ];
        let res = merge_findings(&chapters);
        assert_eq!(res.aggregate.chapters_parsed, 1);
        assert_eq!(res.parse_failures.len(), 1);
        assert_eq!(res.parse_failures[0].0, "ch2");
    }

    #[test]
    fn rules_are_sorted_deterministically() {
        let chapters = vec![
            ("ch1".to_string(), f("F-09", "q", "p", "low")),
            ("ch2".to_string(), f("F-01", "q", "p", "low")),
            ("ch3".to_string(), f("F-05", "q", "p", "low")),
        ];
        let res = merge_findings(&chapters);
        let ids: Vec<&str> = res.aggregate.rules.iter().map(|r| r.rule_id.as_str()).collect();
        assert_eq!(ids, vec!["F-01", "F-05", "F-09"]);
    }

    #[test]
    fn per_rule_counts_match_aggregate() {
        let chapters = vec![
            ("ch1".to_string(), f("F-01", "q1", "p", "low")),
            ("ch2".to_string(), f("F-01", "q2 different", "p", "low")),
            ("ch3".to_string(), f("F-02", "q3", "p", "low")),
        ];
        let counts = merge_findings(&chapters).aggregate.per_rule_counts();
        assert_eq!(counts.get("F-01"), Some(&2));
        assert_eq!(counts.get("F-02"), Some(&1));
    }

    #[test]
    fn per_category_counts_maps_rules_to_categories() {
        use std::collections::HashMap;
        // F-01,F-02 → "fallacy"; F-09 → "tone"; F-99 has no mapping (dropped).
        let mut rule_cat: HashMap<String, String> = HashMap::new();
        rule_cat.insert("F-01".into(), "fallacy".into());
        rule_cat.insert("F-02".into(), "fallacy".into());
        rule_cat.insert("F-09".into(), "tone".into());
        let chapters = vec![
            ("ch1".to_string(), f("F-01", "q1", "p", "low")),
            ("ch2".to_string(), f("F-02", "q2", "p", "low")),
            ("ch3".to_string(), f("F-09", "q3", "p", "low")),
            ("ch4".to_string(), f("F-99", "q4", "p", "low")), // unmapped → dropped
        ];
        let agg = merge_findings(&chapters).aggregate;
        let cats = agg.per_category_counts(&rule_cat);
        assert_eq!(cats.get("fallacy"), Some(&2), "F-01 + F-02 → fallacy = 2");
        assert_eq!(cats.get("tone"), Some(&1), "F-09 → tone = 1");
        assert_eq!(cats.get("unknown"), None, "unmapped rule contributes nothing");
        assert_eq!(cats.len(), 2);
    }

    #[test]
    fn render_table_contains_rule_and_severity() {
        let chapters = vec![("ch1".to_string(), f("F-01", "the quote", "p1", "high"))];
        let table = render_findings_table(&merge_findings(&chapters).aggregate);
        assert!(table.contains("F-01"));
        assert!(table.contains("high"));
        assert!(table.contains("the quote"));
    }

    #[test]
    fn render_json_is_valid_and_stable() {
        let chapters = vec![("ch1".to_string(), f("F-01", "the quote", "p1", "high"))];
        let agg = merge_findings(&chapters).aggregate;
        let json = render_findings_json(&agg);
        // Round-trips as valid JSON.
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["total_findings"], 1);
        assert_eq!(parsed["rules"][0]["rule_id"], "F-01");
        assert_eq!(parsed["rules"][0]["max_severity"], "high");
    }

    #[test]
    fn quote_is_clamped_on_render() {
        let long = "x".repeat(500);
        let chapters = vec![("ch1".to_string(), f("F-01", &long, "p", "low"))];
        let table = render_findings_table(&merge_findings(&chapters).aggregate);
        assert!(table.contains('…'), "over-long quote must be clamped with an ellipsis");
    }

    #[test]
    fn empty_aggregate_renders_placeholder() {
        let res = merge_findings(&[]);
        let table = render_findings_table(&res.aggregate);
        assert!(table.contains("No structured findings"));
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

