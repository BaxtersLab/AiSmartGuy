use manifest::Manifest;
use crate::aggregator::{aggregate_scores, compute_win_counts};
use crate::consensus::determine_winner;
use crate::errors::{OptimizationError, OptimizationResult};
use crate::mapping::build_best_model_map;
use crate::penalties::apply_penalties;
use crate::types::{BookScore, ScoreHistory};

/// Update the manifest's `optimization_state` with data from a new book pass.
///
/// Lifecycle:
/// 1. Append `book_score` to `history`.
/// 2. Apply false-positive penalties to all scores in the book.
/// 3. Increment `books_completed`.
/// 4. If `books_completed` >= 10:
///    - aggregate across history
///    - run consensus per category
///    - build best-model map
///    - if all categories have stable winners → set `optimized_available = true`
pub fn update_optimization_state(
    manifest: &mut Manifest,
    book_score: BookScore,
    history: &mut ScoreHistory,
) -> OptimizationResult<()> {
    if book_score.model_scores.is_empty() {
        return Err(OptimizationError::InvalidInput(
            "book_score contains no model scores".to_string(),
        ));
    }

    // Apply penalties to all scores in this book before storing.
    let mut penalized_book = book_score;
    for s in &mut penalized_book.model_scores {
        apply_penalties(s);
    }

    history.push(penalized_book);

    manifest.optimization_state.books_completed += 1;

    let books_completed = manifest.optimization_state.books_completed;

    // Only run consensus after 10 books.
    if books_completed < 10 {
        return Ok(());
    }

    // Keep only the last 10 books to bound memory growth.
    const WINDOW_SIZE: usize = 10;
    if history.len() > WINDOW_SIZE {
        history.drain(..history.len() - WINDOW_SIZE);
    }
    let window = &history[..];

    let aggregated = aggregate_scores(window);
    let win_counts = compute_win_counts(window);

    let categories: Vec<String> = aggregated.keys().cloned().collect();
    let mut winners = Vec::new();

    for category in &categories {
        if let Some(scores) = aggregated.get(category) {
            if let Some(winner) = determine_winner(category, scores, &win_counts, books_completed) {
                winners.push(winner);
            }
        }
    }

    let best_map = build_best_model_map(&winners);

    // Stable winners for all known categories = optimization available.
    let all_stable = !categories.is_empty() && best_map.len() == categories.len();

    manifest.optimization_state.optimized_available = all_stable;

    manifest.optimization_state.best_model_per_category =
        if best_map.is_empty() { None } else { Some(best_map) };

    eprintln!(
        "[optimization][INFO] books_completed={} stable_categories={}/{} optimized_available={}",
        books_completed,
        manifest.optimization_state.best_model_per_category.as_ref().map_or(0, |m| m.len()),
        categories.len(),
        manifest.optimization_state.optimized_available
    );

    Ok(())
}
