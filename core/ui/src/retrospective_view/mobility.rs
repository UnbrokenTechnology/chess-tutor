//! Mobility card builders, incl. per-piece highlights.
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::{
    MobilityOutcome, PieceMobility,
};
use chess_tutor_engine::eval::MobilityBreakdown;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Color, PieceType, Square};

use crate::view::{
    AnnotationKind, BoardAnnotation, RetrospectiveCategory,
    RetrospectiveItem, Sentiment,
};


// ---------------------------------------------------------------------
// Mobility
// ---------------------------------------------------------------------

const MOBILITY_DELTA_THRESHOLD_CP: i32 = 20;

/// A per-square delta tells us *which* piece's activity actually
/// moved when the per-piece-type aggregate shifted. Pieces sit on
/// different squares pre vs post when the piece moved itself; for
/// stationary pieces (e.g. both bishops after 1.e4), the same
/// square appears in both snapshots and the delta is `post - pre`.
const PER_PIECE_HIGHLIGHT_THRESHOLD_CP: i32 = 15;

pub(super) fn build_mobility_items(
    outcome: &MobilityOutcome,
    _post_pos: &Position,
    _root_stm: Color,
    show_all: bool,
) -> Vec<RetrospectiveItem> {
    // show_all drops the per-piece floor from 50 cp to 1 cp so a
    // bishop's 12→13 reach surfaces. Without it, the default 50 cp
    // gate hides knock-on shifts from pawn pushes that didn't really
    // change the piece's role on the board.
    let threshold = if show_all { 1 } else { MOBILITY_DELTA_THRESHOLD_CP };
    let mut items = Vec::new();

    for (label, piece_type, delta, pre, post) in
        mobility_all_shifts(&outcome.ours_pre, &outcome.ours_post, threshold)
    {
        let (heading, sentiment) = if delta < 0 {
            (format!("Your {label} activity dropped"), Sentiment::Negative)
        } else {
            (format!("Your {label} activity improved"), Sentiment::Positive)
        };
        let annotations = highlight_specific_pieces(
            &outcome.ours_per_piece_pre,
            &outcome.ours_per_piece_post,
            piece_type,
            sentiment,
        );
        items.push(RetrospectiveItem {
            category: RetrospectiveCategory::Mobility,
            heading,
            summary: format!(
                "{:+.2} → {:+.2}",
                pre as f32 / 100.0,
                post as f32 / 100.0
            ),
            detail: format!(
                "Stockfish's mobility term weights the squares this piece type attacks \
                 inside its safe-area bitmap. A {label} climbing from {:+.2} to {:+.2} \
                 typically means it found a more active diagonal, file, or outpost.",
                pre as f32 / 100.0,
                post as f32 / 100.0
            ),
            score_delta_pawns: Some(delta as f32 / 100.0),
            sentiment,
            annotations,
        });
    }

    for (label, piece_type, delta, pre, post) in
        mobility_all_shifts(&outcome.theirs_pre, &outcome.theirs_post, threshold)
    {
        let (heading, sentiment) = if delta < 0 {
            (
                format!("You restricted the opponent's {label}"),
                Sentiment::Positive,
            )
        } else {
            (
                format!("The opponent's {label} got more active"),
                Sentiment::Negative,
            )
        };
        let annotations = highlight_specific_pieces(
            &outcome.theirs_per_piece_pre,
            &outcome.theirs_per_piece_post,
            piece_type,
            sentiment,
        );
        items.push(RetrospectiveItem {
            category: RetrospectiveCategory::Mobility,
            heading,
            summary: format!(
                "{:+.2} → {:+.2}",
                pre as f32 / 100.0,
                post as f32 / 100.0
            ),
            detail: format!(
                "The opponent's {label} mobility shifted from {:+.2} to {:+.2}. \
                 Restricting an opponent's piece is just as valuable as activating \
                 your own — it tends to lock in long-term advantages.",
                pre as f32 / 100.0,
                post as f32 / 100.0
            ),
            score_delta_pawns: Some(-delta as f32 / 100.0),
            sentiment,
            annotations,
        });
    }

    items
}

/// All per-piece-type mobility shifts whose `|delta_mg|` clears
/// `threshold_cp`, sorted biggest-first. Returns up to four entries:
/// (label, piece_type, delta, pre_mg, post_mg).
pub(super) fn mobility_all_shifts(
    pre: &MobilityBreakdown,
    post: &MobilityBreakdown,
    threshold_cp: i32,
) -> Vec<(&'static str, PieceType, i32, i32, i32)> {
    let candidates: [(&'static str, PieceType, i32, i32); 4] = [
        ("knight", PieceType::Knight, pre.knight.mg().0, post.knight.mg().0),
        ("bishop", PieceType::Bishop, pre.bishop.mg().0, post.bishop.mg().0),
        ("rook", PieceType::Rook, pre.rook.mg().0, post.rook.mg().0),
        ("queen", PieceType::Queen, pre.queen.mg().0, post.queen.mg().0),
    ];
    let mut shifts: Vec<_> = candidates
        .into_iter()
        .map(|(label, pt, pre_mg, post_mg)| (label, pt, post_mg - pre_mg, pre_mg, post_mg))
        .filter(|(_, _, delta, _, _)| delta.abs() >= threshold_cp)
        .collect();
    shifts.sort_by_key(|(_, _, delta, _, _)| std::cmp::Reverse(delta.abs()));
    shifts
}

/// Pick the *specific* pieces of `piece_type` whose mobility shifted
/// in the direction `sentiment` calls out. Pre/post snapshots are
/// keyed by square — for pieces that didn't move themselves the same
/// square appears in both and the per-square delta tells us whose
/// activity actually changed. When a piece moved between pre and
/// post (different from-square / to-square), the post entry stands
/// in for "the piece that just moved here" so its new square gets
/// the highlight.
///
/// For each highlighted piece we also emit `NewMobility` /
/// `LostMobility` square highlights for the squares that piece
/// newly attacks (or no longer attacks), so the student sees what
/// "activity improved" actually means on the board. The moved piece
/// has no same-square pre snapshot — every square it attacks counts
/// as newly available for the improved case.
///
/// Threshold filters out the always-on rocking that happens when
/// any pawn push reshapes the mobility bitmap by a handful of cp.
pub(super) fn highlight_specific_pieces(
    pre_pieces: &[PieceMobility],
    post_pieces: &[PieceMobility],
    piece_type: PieceType,
    sentiment: Sentiment,
) -> Vec<BoardAnnotation> {
    let piece_kind = match sentiment {
        Sentiment::Positive => AnnotationKind::GoodPiece,
        Sentiment::Negative => AnnotationKind::BadPiece,
        _ => AnnotationKind::Highlight,
    };

    // For the overall change to be "improved", per-square deltas
    // pointing the same direction are the ones to surface. Per-piece
    // deltas pointing the *opposite* direction are noise (one piece
    // gained mobility, another lost some) — they'd confuse the
    // teaching story.
    let want_positive = matches!(sentiment, Sentiment::Positive);

    // Build a map of pre-move per-piece records keyed by square (only
    // for pieces of the requested type).
    use std::collections::HashMap;
    let mut pre_by_sq: HashMap<Square, &PieceMobility> = HashMap::new();
    for pm in pre_pieces {
        if pm.piece == piece_type {
            pre_by_sq.insert(pm.square, pm);
        }
    }

    // Squares where the piece exists post-move with a meaningful
    // per-square delta in the surfaced direction.
    let mut hits: Vec<(&PieceMobility, i32)> = Vec::new();
    for pm in post_pieces {
        if pm.piece != piece_type {
            continue;
        }
        // If the piece was on the same square pre-move, use the
        // per-square delta. If it just landed here (the moved piece),
        // treat the "delta" as its full post-move mobility — it's
        // the piece that produced the most obvious activity change.
        let prev = pre_by_sq.get(&pm.square).copied();
        let delta = match prev {
            Some(p) => pm.mg - p.mg,
            None => pm.mg,
        };
        let aligned = (want_positive && delta > 0) || (!want_positive && delta < 0);
        if aligned && delta.abs() >= PER_PIECE_HIGHLIGHT_THRESHOLD_CP {
            hits.push((pm, delta.abs()));
        }
    }

    // If nothing crossed the threshold, fall back to whichever
    // post-move piece had the largest aligned delta — students
    // still want *some* visual when the card says "activity moved."
    if hits.is_empty() {
        let mut best: Option<(&PieceMobility, i32)> = None;
        for pm in post_pieces {
            if pm.piece != piece_type {
                continue;
            }
            let prev = pre_by_sq.get(&pm.square).copied();
            let delta = match prev {
                Some(p) => pm.mg - p.mg,
                None => pm.mg,
            };
            let aligned = (want_positive && delta > 0) || (!want_positive && delta < 0);
            if !aligned {
                continue;
            }
            match best {
                Some((_, b)) if delta.abs() <= b => {}
                _ => best = Some((pm, delta.abs())),
            }
        }
        if let Some((pm, _)) = best {
            let mut anns = vec![BoardAnnotation::SquareHighlight {
                square: pm.square,
                kind: piece_kind,
            }];
            push_mobility_square_highlights(
                &mut anns,
                pm,
                pre_by_sq.get(&pm.square).copied(),
                want_positive,
            );
            return anns;
        }
        return Vec::new();
    }

    // Sort descending by magnitude so the biggest swing is visually
    // dominant (renderers paint in order; later highlights overdraw
    // earlier ones, but with same alpha that's a non-issue here).
    hits.sort_by_key(|(_, d)| std::cmp::Reverse(*d));
    let mut anns = Vec::new();
    for (pm, _) in &hits {
        anns.push(BoardAnnotation::SquareHighlight {
            square: pm.square,
            kind: piece_kind,
        });
    }
    for (pm, _) in &hits {
        push_mobility_square_highlights(
            &mut anns,
            pm,
            pre_by_sq.get(&pm.square).copied(),
            want_positive,
        );
    }
    anns
}

/// Highlight the squares this piece's mobility footprint gained
/// (positive sentiment) or lost (negative sentiment).
///
/// When `prev` is `Some`, the piece was on the same square pre and
/// post and the diff between the two `mobility_squares` bitboards
/// names what actually changed. When `prev` is `None`, this is the
/// piece that just moved here — every square it now attacks is
/// "newly available from this piece" in the improved case; the
/// dropped case has no analogue (its from-square footprint lives
/// at a different square entirely), so we paint nothing.
pub(super) fn push_mobility_square_highlights(
    out: &mut Vec<BoardAnnotation>,
    post: &PieceMobility,
    prev: Option<&PieceMobility>,
    want_positive: bool,
) {
    let (squares, kind) = match (prev, want_positive) {
        (Some(p), true) => (
            post.mobility_squares & !p.mobility_squares,
            AnnotationKind::NewMobility,
        ),
        (Some(p), false) => (
            p.mobility_squares & !post.mobility_squares,
            AnnotationKind::LostMobility,
        ),
        (None, true) => (post.mobility_squares, AnnotationKind::NewMobility),
        (None, false) => return,
    };
    for sq in squares {
        out.push(BoardAnnotation::SquareHighlight { square: sq, kind });
    }
}

