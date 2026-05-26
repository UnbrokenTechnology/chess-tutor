//! Secondary terms (Helped / Hurt fallback) card builder.
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::{
    cumulative_prefix,
    MoveAnalysis, TermId,
};
use chess_tutor_engine::types::Color;

use crate::view::{
    RetrospectiveCategory,
    RetrospectiveItem, Sentiment,
};


// ---------------------------------------------------------------------
// Secondary terms (Helped / Hurt fallback)
// ---------------------------------------------------------------------

const RETROSPECTIVE_TOP_PERCENT: f32 = 50.0;

pub(super) fn build_secondary_item(
    user: &MoveAnalysis,
    root_stm: Color,
    skip: &[TermId],
    show_all: bool,
) -> Option<RetrospectiveItem> {
    // show_all bypasses the 50%-coverage trim so every residual term
    // with a non-zero delta appears as a row. The GUI's collapsible
    // card keeps the noise out of the way until the user expands.
    let percent = if show_all { 100.0 } else { RETROSPECTIVE_TOP_PERCENT };
    let prefix = cumulative_prefix(&user.term_deltas, percent);
    let sign = if root_stm == Color::White { 1 } else { -1 };
    let rows: Vec<(TermId, i32)> = prefix
        .iter()
        .filter(|d| !skip.contains(&d.term) && d.delta_tapered != 0)
        .map(|d| (d.term, d.delta_tapered * sign))
        .collect();
    if rows.is_empty() {
        return None;
    }
    let (helped, hurt): (Vec<_>, Vec<_>) = rows.into_iter().partition(|(_, cp)| *cp > 0);
    let mut detail_lines = Vec::new();
    if !helped.is_empty() {
        detail_lines.push(format!(
            "Also helped: {}",
            format_term_list(&helped)
        ));
    }
    if !hurt.is_empty() {
        detail_lines.push(format!(
            "Also hurt: {}",
            format_term_list(&hurt)
        ));
    }
    let net: i32 = helped.iter().map(|(_, cp)| *cp).sum::<i32>()
        + hurt.iter().map(|(_, cp)| *cp).sum::<i32>();
    let sentiment = if net > 0 {
        Sentiment::Positive
    } else if net < 0 {
        Sentiment::Negative
    } else {
        Sentiment::Mixed
    };
    let summary = if !helped.is_empty() && !hurt.is_empty() {
        format!(
            "{} helped, {} hurt",
            helped.len(),
            hurt.len()
        )
    } else if !helped.is_empty() {
        format!("{} helped", helped.len())
    } else {
        format!("{} hurt", hurt.len())
    };
    Some(RetrospectiveItem {
        category: RetrospectiveCategory::Secondary,
        heading: "Other shifts".to_string(),
        summary,
        detail: detail_lines.join("\n"),
        score_delta_pawns: Some(net as f32 / 100.0),
        sentiment,
        annotations: Vec::new(),
    })
}

pub(super) fn format_term_list(rows: &[(TermId, i32)]) -> String {
    let mut sorted: Vec<&(TermId, i32)> = rows.iter().collect();
    sorted.sort_by_key(|(_, cp)| std::cmp::Reverse(cp.abs()));
    sorted
        .iter()
        .map(|(term, cp)| format!("{} {:+.2}", term.pretty_label(), *cp as f32 / 100.0))
        .collect::<Vec<_>>()
        .join(", ")
}

