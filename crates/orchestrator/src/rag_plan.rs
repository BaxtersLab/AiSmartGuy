//! RAG prompt budgeting — fit an arbitrarily large packet library into a
//! model's context without ever dropping a rule.
//!
//! ## Why this exists
//!
//! The shipped default library is 69 rules across 5 packets and renders to
//! ~20.8k tokens. The operator's smallest lane model has an 8192-token native
//! context. Those numbers do not reconcile, and before this module the run
//! simply died:
//!
//! ```text
//! CONTEXT_TOO_SMALL: hardware-capped context is 8192 tokens but the RAG
//! packets (20833) + generation headroom (2048) + safety (819) leave only 0
//! for chapter text (minimum 1024).
//! ```
//!
//! The error blamed hardware and offered three fixes, none of which worked:
//! the throttle was already at 100%, a smaller model does not raise a *native*
//! context limit, and disabling packet categories means silently analysing the
//! book against fewer rules.
//!
//! ## What it does instead
//!
//! Three tiers, best first — the first that fits wins:
//!
//! 1. **Full detail, one pass.** Every field including explanations.
//! 2. **Compact, one pass.** Explanations dropped (54% of the bytes in the
//!    default library); every rule id, name, severity and pattern list kept.
//! 3. **Compact, N passes.** Rules are packed into as many prompts as it takes.
//!    Each chapter is then analysed once per batch and the outputs merge
//!    downstream exactly like any other per-chapter output.
//!
//! Tier 3 costs wall-clock time and nothing else: **every rule is still applied
//! to every chapter.** Packing is at rule granularity rather than packet
//! granularity so that a budget smaller than the largest single packet still
//! works — packet granularity would have made the operator's 8192 case
//! unsatisfiable, since one packet alone renders to ~2950 compact tokens.
//!
//! A rule so large it cannot fit the budget alone is emitted in its own batch
//! and named in `oversized_rules`, never discarded. That keeps the guarantee
//! total: `rules_covered() == total_rules`, always.

use rag_engine::{PromptDetail, RagPacket, RagRule};

/// Rough chars-per-token estimate. Deliberately conservative: real tokenizers
/// average 1.5–2.5 chars/token, and under-estimating prompt size is what
/// overflows a context window.
pub const CHARS_PER_TOKEN: usize = 2;

/// Estimate the token cost of a rendered string.
pub fn est_tokens(text: &str) -> usize {
    text.len() / CHARS_PER_TOKEN
}

/// One RAG pass: a rendered system prompt plus the rules it carries.
#[derive(Debug, Clone)]
pub struct RagBatch {
    /// The rendered system-prompt block for this pass.
    pub prompt: String,
    /// Estimated token cost of `prompt`.
    pub tokens: usize,
    /// Rule ids carried by this pass, in order.
    pub rule_ids: Vec<String>,
}

/// How one model's RAG library will be delivered across passes.
#[derive(Debug, Clone)]
pub struct RagPlan {
    /// Detail level the batches were rendered at.
    pub detail: PromptDetail,
    /// One entry per inference pass. Empty means "no RAG packets at all" —
    /// the caller still makes a single pass, with no system prompt.
    pub batches: Vec<RagBatch>,
    /// Total rules in the source library.
    pub total_rules: usize,
    /// Rules that exceed the budget on their own. They are still emitted (in a
    /// batch of their own); this names them so the run can say so out loud.
    pub oversized_rules: Vec<String>,
}

impl RagPlan {
    /// Number of inference passes per chapter. Always at least 1 — a model with
    /// no RAG packets still analyses the chapter.
    pub fn passes(&self) -> usize {
        self.batches.len().max(1)
    }

    /// The system prompt for pass `i` (empty string when there are no packets).
    pub fn prompt_for(&self, i: usize) -> &str {
        self.batches.get(i).map(|b| b.prompt.as_str()).unwrap_or("")
    }

    /// The largest single pass, in tokens. This — not the whole library — is
    /// what the chapter budget must make room for.
    pub fn max_batch_tokens(&self) -> usize {
        self.batches.iter().map(|b| b.tokens).max().unwrap_or(0)
    }

    /// How many rules the plan actually delivers. Invariant: equals
    /// `total_rules`. Batching never drops a rule; that is the whole point.
    pub fn rules_covered(&self) -> usize {
        self.batches.iter().map(|b| b.rule_ids.len()).sum()
    }

    /// One-line summary for the run log.
    pub fn describe(&self) -> String {
        if self.batches.is_empty() {
            return "no RAG packets — 1 pass, no system prompt".to_string();
        }
        let detail = match self.detail {
            PromptDetail::Full => "full detail",
            PromptDetail::Compact => "compact (explanations dropped)",
        };
        let mut s = format!(
            "{}, {} pass(es), max {}tok/pass, {}/{} rules covered",
            detail,
            self.batches.len(),
            self.max_batch_tokens(),
            self.rules_covered(),
            self.total_rules
        );
        if !self.oversized_rules.is_empty() {
            s.push_str(&format!(
                " — WARNING {} rule(s) exceed the per-pass budget alone and were sent anyway: {}",
                self.oversized_rules.len(),
                self.oversized_rules.join(", ")
            ));
        }
        s
    }
}

/// Plan how to deliver `packets` to a model that can spare `budget_tokens` of
/// context for RAG instructions.
///
/// Never drops a rule. See the module docs for the three tiers.
pub fn plan_rag(packets: &[RagPacket], budget_tokens: usize) -> RagPlan {
    let total_rules: usize = packets.iter().map(|p| p.rules.len()).sum();

    if total_rules == 0 {
        return RagPlan {
            detail: PromptDetail::Full,
            batches: Vec::new(),
            total_rules: 0,
            oversized_rules: Vec::new(),
        };
    }

    // Tier 1 — everything, full detail, one pass.
    let full = render(packets, PromptDetail::Full);
    if est_tokens(&full) <= budget_tokens {
        return single_batch(packets, full, PromptDetail::Full, total_rules);
    }

    // Tier 2 — everything, compact, one pass.
    let compact = render(packets, PromptDetail::Compact);
    if est_tokens(&compact) <= budget_tokens {
        return single_batch(packets, compact, PromptDetail::Compact, total_rules);
    }

    // Tier 3 — compact, packed across as many passes as it takes.
    pack_compact(packets, budget_tokens, total_rules)
}

/// Chapter capacity below this share of usable context is treated as too
/// cramped to be worth paying for full detail — chapters that small mean many
/// more of them, and the extra passes cost more than the explanations buy.
const COMFORTABLE_CHAPTER_SHARE: usize = 4; // i.e. one quarter of usable

/// Choose how much of a model's usable context to spend on RAG, then plan.
///
/// Returns the plan and the chapter capacity it leaves behind.
///
/// **Detail is not traded for speed silently.** If the whole library fits at
/// full detail in one pass and still leaves a comfortable chapter budget, that
/// wins outright — explanations make the analysis better, and a work metric
/// alone would quietly drop them just to gain chapter room the run does not
/// need.
///
/// Only when full detail cannot be delivered that way does the share get
/// swept, and then it is swept rather than hardcoded because the optimum is
/// interior. Total inference work for a book is proportional to
/// `passes / chapter_capacity`: a larger RAG share buys fewer passes but
/// shrinks chapters, so more of them are needed. Both extremes are bad — all
/// context to RAG leaves no room to read, none to RAG means a pass per rule.
///
/// For the operator's 8192-token lane model (usable ≈ 5325, compact library
/// ≈ 9496) the sweep lands near a 55% share: 4 passes over ~2100-token
/// chapters, versus the 10 passes a 20% share would cost.
pub fn plan_rag_for_context(
    packets: &[RagPacket],
    usable_tokens: usize,
    min_chapter_tokens: usize,
) -> (RagPlan, usize) {
    // No room to read at all — nothing to optimise. Give RAG nothing and let
    // the caller's own floor check report the real problem.
    if usable_tokens <= min_chapter_tokens {
        let plan = plan_rag(packets, 0);
        let capacity = usable_tokens.saturating_sub(plan.max_batch_tokens());
        return (plan, capacity);
    }

    let max_share = usable_tokens - min_chapter_tokens;

    // Quality first: full detail in a single pass, if it leaves room to read.
    let full_tokens = est_tokens(&render(packets, PromptDetail::Full));
    let full_capacity = usable_tokens.saturating_sub(full_tokens);
    let comfortable = (usable_tokens / COMFORTABLE_CHAPTER_SHARE).max(min_chapter_tokens);
    if full_tokens <= max_share && full_capacity >= comfortable {
        return (plan_rag(packets, full_tokens), full_capacity);
    }

    let mut best: Option<(RagPlan, usize, f64)> = None;

    // 10%..=90% in 5-point steps, never exceeding what leaves a readable chapter.
    for pct in (10..=90).step_by(5) {
        let budget = (usable_tokens * pct / 100).min(max_share);
        if budget == 0 {
            continue;
        }

        let plan = plan_rag(packets, budget);
        let capacity = usable_tokens.saturating_sub(plan.max_batch_tokens());
        if capacity < min_chapter_tokens {
            continue;
        }

        let score = plan.passes() as f64 / capacity as f64;
        let better = match &best {
            None => true,
            Some((best_plan, _, best_score)) => {
                // Lower work wins; on a tie prefer fewer passes, which keeps
                // more rules in front of the model at once.
                score < *best_score - f64::EPSILON
                    || ((score - *best_score).abs() <= f64::EPSILON
                        && plan.passes() < best_plan.passes())
            }
        };
        if better {
            best = Some((plan, capacity, score));
        }
    }

    match best {
        Some((plan, capacity, _)) => (plan, capacity),
        // Every share left too little to read — fall back to the largest share
        // that respects the chapter floor and let the caller decide.
        None => {
            let plan = plan_rag(packets, max_share);
            let capacity = usable_tokens.saturating_sub(plan.max_batch_tokens());
            (plan, capacity)
        }
    }
}

fn render(packets: &[RagPacket], detail: PromptDetail) -> String {
    rag_engine::build_system_prompt_with_detail(packets, detail)
}

fn all_rule_ids(packets: &[RagPacket]) -> Vec<String> {
    packets
        .iter()
        .flat_map(|p| p.rules.iter().map(|r| r.rule_id.clone()))
        .collect()
}

fn single_batch(
    packets: &[RagPacket],
    prompt: String,
    detail: PromptDetail,
    total_rules: usize,
) -> RagPlan {
    RagPlan {
        detail,
        batches: vec![RagBatch {
            tokens: est_tokens(&prompt),
            prompt,
            rule_ids: all_rule_ids(packets),
        }],
        total_rules,
        oversized_rules: Vec::new(),
    }
}

/// Greedy first-fit packing at rule granularity.
///
/// Rules are visited in library order, so a batch holds a contiguous run and
/// related rules stay together — a pass sees whole categories wherever the
/// budget allows, rather than an arbitrary scatter.
fn pack_compact(packets: &[RagPacket], budget_tokens: usize, total_rules: usize) -> RagPlan {
    let mut batches: Vec<RagBatch> = Vec::new();
    let mut oversized: Vec<String> = Vec::new();
    let mut current: Vec<RagPacket> = Vec::new();

    for packet in packets {
        for rule in &packet.rules {
            push_rule(&mut current, packet, rule);

            if est_tokens(&render(&current, PromptDetail::Compact)) <= budget_tokens {
                continue;
            }

            if rule_count(&current) == 1 {
                // A single rule that will not fit on its own. Splitting further
                // is not possible, so send it and say so rather than drop it.
                oversized.push(rule.rule_id.clone());
                flush(&mut current, &mut batches);
                continue;
            }

            // Back the rule out, close the batch without it, reopen with it.
            pop_rule(&mut current);
            flush(&mut current, &mut batches);
            push_rule(&mut current, packet, rule);

            // The reopened batch holds exactly this one rule — it may itself
            // be over budget, in which case it is oversized on its own.
            if est_tokens(&render(&current, PromptDetail::Compact)) > budget_tokens {
                oversized.push(rule.rule_id.clone());
                flush(&mut current, &mut batches);
            }
        }
    }

    flush(&mut current, &mut batches);

    RagPlan {
        detail: PromptDetail::Compact,
        batches,
        total_rules,
        oversized_rules: oversized,
    }
}

/// Append `rule` to the working subset, reusing the trailing packet when the
/// rule belongs to it so a packet header is not repeated within one batch.
fn push_rule(current: &mut Vec<RagPacket>, packet: &RagPacket, rule: &RagRule) {
    if let Some(last) = current.last_mut() {
        if last.packet_id == packet.packet_id {
            last.rules.push(rule.clone());
            return;
        }
    }
    let mut shell = packet.clone();
    shell.rules = vec![rule.clone()];
    current.push(shell);
}

/// Remove the most recently pushed rule, dropping its packet shell if that
/// leaves the shell empty.
fn pop_rule(current: &mut Vec<RagPacket>) {
    if let Some(last) = current.last_mut() {
        last.rules.pop();
        if last.rules.is_empty() {
            current.pop();
        }
    }
}

fn rule_count(current: &[RagPacket]) -> usize {
    current.iter().map(|p| p.rules.len()).sum()
}

fn flush(current: &mut Vec<RagPacket>, batches: &mut Vec<RagBatch>) {
    if rule_count(current) == 0 {
        current.clear();
        return;
    }
    let prompt = render(current, PromptDetail::Compact);
    batches.push(RagBatch {
        tokens: est_tokens(&prompt),
        rule_ids: all_rule_ids(current),
        prompt,
    });
    current.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use rag_engine::{
        HitConditions, InjectLocation, ModelBehavior, PatternType, RagPacket, RagRule, Severity,
    };
    use std::collections::HashSet;
    use std::path::PathBuf;

    /// A rule whose rendered size is driven by `pad` — `explanation` is the
    /// field `Compact` drops, `patterns` the field it keeps, so padding both
    /// separately lets a test target either rendering.
    fn rule(id: &str, pattern_pad: usize, explanation_pad: usize) -> RagRule {
        RagRule {
            rule_id: id.to_string(),
            name: format!("rule {}", id),
            pattern_type: PatternType::Linguistic,
            patterns: vec!["p".repeat(pattern_pad)],
            severity: Severity::Medium,
            explanation: "e".repeat(explanation_pad),
        }
    }

    fn packet(id: u16, category: &str, rules: Vec<RagRule>) -> RagPacket {
        RagPacket {
            packet_id: id,
            category: category.to_string(),
            description: String::new(),
            version: "1.0".to_string(),
            rules,
            hit_conditions: HitConditions {
                min_pattern_matches: 1,
                confidence_weight: 0.5,
            },
            model_behavior: ModelBehavior {
                inject_as: InjectLocation::SystemPrompt,
                priority: 1,
            },
            source_path: PathBuf::new(),
        }
    }

    /// A library shaped like the real default set: several packets, several
    /// rules each, explanations dominating the byte count.
    fn library() -> Vec<RagPacket> {
        vec![
            packet(
                1,
                "fallacies",
                (0..8).map(|i| rule(&format!("FAL-{:02}", i), 200, 400)).collect(),
            ),
            packet(
                2,
                "weaponized_language",
                (0..6).map(|i| rule(&format!("WPN-{:02}", i), 200, 400)).collect(),
            ),
            packet(
                6,
                "ambiguous_framing",
                (0..4).map(|i| rule(&format!("AMB-{:02}", i), 200, 400)).collect(),
            ),
        ]
    }

    fn covered_ids(plan: &RagPlan) -> Vec<String> {
        plan.batches
            .iter()
            .flat_map(|b| b.rule_ids.iter().cloned())
            .collect()
    }

    // ── Tier selection ──────────────────────────────────────────────────────

    #[test]
    fn prefers_full_detail_when_the_whole_library_fits() {
        let lib = library();
        let full_tokens = est_tokens(&render(&lib, PromptDetail::Full));
        let plan = plan_rag(&lib, full_tokens + 100);

        assert_eq!(plan.detail, PromptDetail::Full);
        assert_eq!(plan.passes(), 1);
        assert!(plan.prompt_for(0).contains("Explanation:"));
    }

    #[test]
    fn falls_back_to_compact_before_it_starts_batching() {
        let lib = library();
        let full_tokens = est_tokens(&render(&lib, PromptDetail::Full));
        let compact_tokens = est_tokens(&render(&lib, PromptDetail::Compact));
        assert!(compact_tokens < full_tokens, "test library must compress");

        // Budget between the two: too small for full, room enough for compact.
        let plan = plan_rag(&lib, compact_tokens + 10);

        assert_eq!(plan.detail, PromptDetail::Compact);
        assert_eq!(plan.passes(), 1, "one pass should still suffice");
        assert!(!plan.prompt_for(0).contains("Explanation:"));
    }

    #[test]
    fn batches_only_when_compact_alone_is_not_enough() {
        let lib = library();
        let compact_tokens = est_tokens(&render(&lib, PromptDetail::Compact));
        let plan = plan_rag(&lib, compact_tokens / 3);

        assert_eq!(plan.detail, PromptDetail::Compact);
        assert!(plan.passes() > 1, "expected multiple passes, got {}", plan.passes());
    }

    // ── The guarantee: no rule is ever lost ─────────────────────────────────

    /// This is the property the whole module exists to provide. Before
    /// batching, a library this size against a budget this small produced
    /// CONTEXT_TOO_SMALL and the run died having analysed nothing.
    #[test]
    fn every_rule_survives_batching() {
        let lib = library();
        let total: usize = lib.iter().map(|p| p.rules.len()).sum();

        // Sweep a range of budgets, including brutally small ones.
        for divisor in [2, 3, 4, 6, 8, 12, 20] {
            let compact_tokens = est_tokens(&render(&lib, PromptDetail::Compact));
            let budget = (compact_tokens / divisor).max(1);
            let plan = plan_rag(&lib, budget);

            let ids = covered_ids(&plan);
            assert_eq!(
                ids.len(),
                total,
                "budget {}: delivered {} rules, library has {}",
                budget,
                ids.len(),
                total
            );
            assert_eq!(
                plan.rules_covered(),
                plan.total_rules,
                "budget {}: rules_covered must equal total_rules",
                budget
            );

            let unique: HashSet<&String> = ids.iter().collect();
            assert_eq!(unique.len(), ids.len(), "budget {}: a rule was duplicated", budget);
        }
    }

    #[test]
    fn every_rule_appears_in_the_rendered_prompts_not_just_the_index() {
        let lib = library();
        let compact_tokens = est_tokens(&render(&lib, PromptDetail::Compact));
        let plan = plan_rag(&lib, compact_tokens / 5);

        let all_prompts = plan
            .batches
            .iter()
            .map(|b| b.prompt.clone())
            .collect::<Vec<_>>()
            .join("\n");

        for p in &lib {
            for r in &p.rules {
                assert!(
                    all_prompts.contains(&r.rule_id),
                    "rule {} never reached a prompt",
                    r.rule_id
                );
            }
        }
    }

    #[test]
    fn batches_respect_the_budget_when_rules_individually_fit() {
        let lib = library();
        let compact_tokens = est_tokens(&render(&lib, PromptDetail::Compact));
        let budget = compact_tokens / 4;
        let plan = plan_rag(&lib, budget);

        assert!(plan.oversized_rules.is_empty(), "no rule should be oversized here");
        for (i, b) in plan.batches.iter().enumerate() {
            assert!(
                b.tokens <= budget,
                "batch {} is {}tok, over budget {}",
                i,
                b.tokens,
                budget
            );
        }
        assert_eq!(plan.max_batch_tokens(), plan.batches.iter().map(|b| b.tokens).max().unwrap());
    }

    // ── Edge cases ──────────────────────────────────────────────────────────

    /// A rule too big for the budget cannot be split, but dropping it would
    /// silently narrow the analysis. It is sent anyway and named.
    #[test]
    fn oversized_rule_is_kept_and_reported_never_dropped() {
        let lib = vec![packet(1, "huge", vec![rule("BIG-01", 4000, 4000), rule("SML-01", 10, 10)])];
        let plan = plan_rag(&lib, 100);

        let ids = covered_ids(&plan);
        assert!(ids.contains(&"BIG-01".to_string()), "oversized rule was dropped");
        assert!(ids.contains(&"SML-01".to_string()));
        assert_eq!(plan.rules_covered(), 2);
        assert!(plan.oversized_rules.contains(&"BIG-01".to_string()));
        assert!(plan.describe().contains("BIG-01"));
    }

    #[test]
    fn empty_library_still_yields_one_pass_with_no_prompt() {
        let plan = plan_rag(&[], 4096);
        assert_eq!(plan.total_rules, 0);
        assert!(plan.batches.is_empty());
        assert_eq!(plan.passes(), 1, "a model with no packets still reads the chapter");
        assert_eq!(plan.prompt_for(0), "");
        assert_eq!(plan.max_batch_tokens(), 0);
    }

    #[test]
    fn zero_budget_does_not_hang_or_drop() {
        let lib = library();
        let total: usize = lib.iter().map(|p| p.rules.len()).sum();
        let plan = plan_rag(&lib, 0);
        assert_eq!(plan.rules_covered(), total);
        assert_eq!(plan.oversized_rules.len(), total, "every rule is oversized at budget 0");
    }

    #[test]
    fn a_batch_never_repeats_a_packet_header_for_contiguous_rules() {
        let lib = vec![packet(
            1,
            "fallacies",
            (0..4).map(|i| rule(&format!("FAL-{:02}", i), 10, 10)).collect(),
        )];
        let plan = plan_rag(&lib, 100_000);
        assert_eq!(plan.passes(), 1);
        assert_eq!(
            plan.prompt_for(0).matches("[RAG-PACKET").count(),
            1,
            "four rules of one packet should share one header"
        );
    }

    // ── plan_rag_for_context ────────────────────────────────────────────────

    /// The operator's failing case, reduced to its numbers: an 8192-token model
    /// against a library far larger than its whole window. Before batching this
    /// returned a negative budget and the run aborted with CONTEXT_TOO_SMALL
    /// having analysed nothing.
    #[test]
    fn small_context_model_gets_a_workable_plan_instead_of_failing() {
        let lib = library();
        // 8192 ctx − 2048 generation − 10% safety ≈ 5325 usable.
        let usable = 5325;
        let (plan, capacity) = plan_rag_for_context(&lib, usable, 1024);

        assert!(capacity >= 1024, "chapter capacity {} below the floor", capacity);
        assert!(capacity <= usable);
        assert_eq!(plan.rules_covered(), plan.total_rules, "no rule may be dropped");
        assert!(plan.max_batch_tokens() + capacity <= usable);
    }

    #[test]
    fn generous_context_gets_full_detail_in_one_pass() {
        let lib = library();
        let full_tokens = est_tokens(&render(&lib, PromptDetail::Full));
        let (plan, capacity) = plan_rag_for_context(&lib, full_tokens * 4, 1024);

        assert_eq!(plan.detail, PromptDetail::Full);
        assert_eq!(plan.passes(), 1);
        assert!(capacity >= 1024);
    }

    /// The sweep exists because the interior optimum beats both extremes.
    #[test]
    fn chosen_share_beats_a_naive_fixed_split() {
        let lib = library();
        let usable = 5325;
        let (plan, capacity) = plan_rag_for_context(&lib, usable, 1024);
        let chosen = plan.passes() as f64 / capacity as f64;

        for pct in [10usize, 20, 80, 90] {
            let naive = plan_rag(&lib, usable * pct / 100);
            let naive_cap = usable.saturating_sub(naive.max_batch_tokens());
            if naive_cap < 1024 {
                continue;
            }
            let naive_score = naive.passes() as f64 / naive_cap as f64;
            assert!(
                chosen <= naive_score + f64::EPSILON,
                "sweep picked {:.6} work/token, {}% split would have been {:.6}",
                chosen,
                pct,
                naive_score
            );
        }
    }

    #[test]
    fn context_too_small_to_read_still_returns_a_plan_not_a_panic() {
        let lib = library();
        let (plan, capacity) = plan_rag_for_context(&lib, 500, 1024);
        // The caller reports the failure; the planner must not panic or lie.
        assert!(capacity <= 500);
        assert_eq!(plan.rules_covered(), plan.total_rules);
    }

    #[test]
    fn max_batch_tokens_is_what_the_chapter_budget_must_reserve() {
        let lib = library();
        let compact_tokens = est_tokens(&render(&lib, PromptDetail::Compact));
        let budget = compact_tokens / 4;
        let plan = plan_rag(&lib, budget);

        // The whole library is far larger than any single pass — this is
        // precisely the saving that makes an 8K model viable.
        assert!(
            plan.max_batch_tokens() < compact_tokens,
            "max batch {} should be well under the whole library {}",
            plan.max_batch_tokens(),
            compact_tokens
        );
    }
}
