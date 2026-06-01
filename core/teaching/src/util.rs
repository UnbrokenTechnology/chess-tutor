//! Shared formatting + SAN helpers used across retrospective
//! narration modules. All `pub(crate)` so sibling narrators can
//! reuse them; nothing in this module is intended to leave the
//! `retrospective` subtree.

use chess_tutor_engine::analysis::{MoveVerdict, PieceLocation};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::types::{Move, PieceType, Value};

/// Full English name for a piece type, lower-cased for inline use
/// in phrases.
pub(crate) fn piece_name(pt: PieceType) -> &'static str {
    match pt {
        PieceType::Pawn => "pawn",
        PieceType::Knight => "knight",
        PieceType::Bishop => "bishop",
        PieceType::Rook => "rook",
        PieceType::Queen => "queen",
        PieceType::King => "king",
    }
}

/// Render the list of attackers as `"attacked by the e3 pawn"` or
/// `"attacked by the e3 pawn and b5 bishop"` or for 3+ attackers
/// `"attacked by the e3 pawn, b5 bishop, and d1 queen"`. The Oxford
/// comma is there on purpose — multiple attackers are rare enough
/// that clarity wins over brevity.
pub(crate) fn format_attackers(attackers: &[PieceLocation]) -> String {
    if attackers.is_empty() {
        // Shouldn't happen — a hanging piece has ≥ 1 enemy attacker
        // by construction. Render defensively rather than panic.
        return "attacked".to_string();
    }
    let labels: Vec<String> = attackers
        .iter()
        .map(|a| format!("{} {}", a.square.to_algebraic(), piece_name(a.piece)))
        .collect();
    let joined = match labels.as_slice() {
        [one] => one.clone(),
        [first, second] => format!("{first} and {second}"),
        many => {
            let last = many.last().unwrap().clone();
            let lead: Vec<String> = many[..many.len() - 1].to_vec();
            format!("{}, and {}", lead.join(", "), last)
        }
    };
    format!("attacked by the {joined}")
}

/// Render the first `through_ply + 1` moves of `pv` as SAN, each
/// formatted relative to the position it's played from.
pub(crate) fn pv_to_san_through(root: &Position, pv: &[Move], through_ply: usize) -> Vec<String> {
    let limit = (through_ply + 1).min(pv.len());
    let mut out = Vec::with_capacity(limit);
    let mut scratch = root.clone();
    for mv in &pv[..limit] {
        out.push(san::format_on(&mut scratch, *mv));
        scratch.do_move(*mv);
    }
    out
}

/// Format a shelter score (engine-cp midgame component) as
/// pawn-equivalents with a leading sign — `+0.85`, `-0.30`. Matches
/// the `{:+.2}` convention the secondary-terms list uses. Also
/// reused for mobility deltas since both render as mg-scale cp.
pub(crate) fn format_shelter_pawns(cp: i32) -> String {
    format!("{:+.2}", cp as f32 / 100.0)
}

/// Format a `Value` score in pawn-equivalents. Mate scores render
/// as `#5` / `-#3` rather than huge raw cp.
pub(crate) fn format_score_pawns(score: Value) -> String {
    let abs = score.0.abs();
    let mate_threshold = Value::MATE.0 - Value::MAX_PLY;
    if abs >= mate_threshold {
        let plies = Value::MATE.0 - abs;
        let moves = (plies + 1) / 2;
        if score.0 >= 0 {
            format!("#{}", moves)
        } else {
            format!("-#{}", moves)
        }
    } else {
        // Engine score → chess.com-aligned pawns. The score is raw
        // engine-cp (tapered pawn 128→213); divide by PAWN_EG to match
        // the CLI's `units.rs` and the GUI's headline.
        format!("{:+.2}", score.0 as f32 / Value::PAWN_EG.0 as f32)
    }
}

/// Format a delta (user_score - best_score) in pawn-equivalents, on
/// the same PAWN_EG scale as [`format_score_pawns`].
pub(crate) fn format_delta_pawns(delta_cp: i32) -> String {
    format!("{:+.2}", delta_cp as f32 / Value::PAWN_EG.0 as f32)
}

/// Human-readable label for a [`MoveVerdict`].
pub(crate) fn verdict_label(v: MoveVerdict) -> &'static str {
    match v {
        MoveVerdict::Best => "Best",
        MoveVerdict::Good => "Good",
        MoveVerdict::Inaccuracy => "Inaccuracy",
        MoveVerdict::Mistake => "Mistake",
        MoveVerdict::Blunder => "Blunder",
        MoveVerdict::Miss => "Miss",
        MoveVerdict::BestAvailable => "Best available",
    }
}

/// Traditional chess-annotation suffix for a move's SAN.
/// - `??` for a Blunder, `?` for a Mistake — the classic bad-move
///   annotations.
/// - `!` when the caller identifies the move as "sharp" (verdict is
///   Best or Good AND the surprise classifier says `LooksBadButGood`
///   — i.e. the move looked risky to a shallow reader but the
///   deeper engine sees through).
/// - Empty string for a plain Good / Inaccuracy / Best /
///   BestAvailable / Miss. A Miss carries no SAN suffix — the move
///   itself was sound (it didn't hang material), so a `?`/`??` glyph
///   would mislead; the "Miss" verdict label carries the meaning.
pub(crate) fn sharp_or_verdict_annotation(v: MoveVerdict, is_sharp: bool) -> &'static str {
    if is_sharp {
        return "!";
    }
    match v {
        MoveVerdict::Blunder => "??",
        MoveVerdict::Mistake => "?",
        MoveVerdict::Best => "",
        MoveVerdict::Good => "",
        MoveVerdict::Inaccuracy => "",
        MoveVerdict::Miss => "",
        MoveVerdict::BestAvailable => "",
    }
}

/// Render the "Engine preferred X (+Y)" line, optionally annotated
/// when the engine's preferred move is itself sharp
/// (LooksBadButGood from root STM's POV). Split out as a pure
/// helper so the prose is unit-testable.
pub(crate) fn format_engine_preferred_line(
    best_san: &str,
    best_score_str: &str,
    is_sharp: bool,
) -> String {
    if is_sharp {
        format!(
            "Engine preferred {best_san}! ({best_score_str}) — \
             a sharp move that looks risky but pays off in the longer line."
        )
    } else {
        format!("Engine preferred {best_san} ({best_score_str}).")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::types::{PieceType, Square};

    fn pl(square: Square, piece: PieceType) -> PieceLocation {
        PieceLocation { square, piece }
    }

    // ---- verdict_label + annotation + delta -------------------------

    #[test]
    fn verdict_labels_cover_every_variant() {
        for v in [
            MoveVerdict::Best,
            MoveVerdict::Good,
            MoveVerdict::Inaccuracy,
            MoveVerdict::Mistake,
            MoveVerdict::Blunder,
            MoveVerdict::Miss,
            MoveVerdict::BestAvailable,
        ] {
            assert!(!verdict_label(v).is_empty());
        }
    }

    #[test]
    fn format_delta_pawns_signs_and_rounds() {
        // Engine-cp → pawns on the PAWN_EG (213) scale, matching the
        // CLI's units.rs. One engine pawn (PAWN_EG) reads as 1.00.
        assert_eq!(format_delta_pawns(0), "+0.00");
        assert_eq!(format_delta_pawns(Value::PAWN_EG.0), "+1.00");
        assert_eq!(format_delta_pawns(-2 * Value::PAWN_EG.0), "-2.00");
    }

    #[test]
    fn format_score_pawns_uses_pawn_eg_scale() {
        assert_eq!(format_score_pawns(Value(Value::PAWN_EG.0)), "+1.00");
        assert_eq!(format_score_pawns(Value(0)), "+0.00");
    }

    #[test]
    fn verdict_annotation_tags_only_extremes() {
        assert_eq!(
            sharp_or_verdict_annotation(MoveVerdict::Blunder, false),
            "??"
        );
        assert_eq!(
            sharp_or_verdict_annotation(MoveVerdict::Mistake, false),
            "?"
        );
        assert_eq!(sharp_or_verdict_annotation(MoveVerdict::Best, false), "");
        assert_eq!(sharp_or_verdict_annotation(MoveVerdict::Good, false), "");
        assert_eq!(
            sharp_or_verdict_annotation(MoveVerdict::Inaccuracy, false),
            ""
        );
        assert_eq!(
            sharp_or_verdict_annotation(MoveVerdict::BestAvailable, false),
            ""
        );
    }

    #[test]
    fn sharp_annotation_overrides_verdict() {
        assert_eq!(sharp_or_verdict_annotation(MoveVerdict::Best, true), "!");
        assert_eq!(sharp_or_verdict_annotation(MoveVerdict::Good, true), "!");
        assert_eq!(sharp_or_verdict_annotation(MoveVerdict::Mistake, true), "!");
    }

    // ---- format_engine_preferred_line --------------------------------

    #[test]
    fn engine_preferred_line_plain_when_not_sharp() {
        let line = format_engine_preferred_line("Nf3", "+0.15", false);
        assert_eq!(line, "Engine preferred Nf3 (+0.15).");
    }

    #[test]
    fn engine_preferred_line_annotated_when_sharp() {
        let line = format_engine_preferred_line("Qxh7", "+4.23", true);
        assert!(line.starts_with("Engine preferred Qxh7! (+4.23)"));
        assert!(line.contains("sharp move"));
        assert!(line.contains("pays off"));
    }

    // ---- format_attackers --------------------------------------------

    #[test]
    fn format_attackers_single() {
        let attackers = vec![pl(Square::E3, PieceType::Pawn)];
        assert_eq!(format_attackers(&attackers), "attacked by the e3 pawn");
    }

    #[test]
    fn format_attackers_two_joins_with_and() {
        let attackers = vec![
            pl(Square::E3, PieceType::Pawn),
            pl(Square::B5, PieceType::Bishop),
        ];
        assert_eq!(
            format_attackers(&attackers),
            "attacked by the e3 pawn and b5 bishop"
        );
    }

    #[test]
    fn format_attackers_three_uses_oxford_comma() {
        let attackers = vec![
            pl(Square::E3, PieceType::Pawn),
            pl(Square::B5, PieceType::Bishop),
            pl(Square::D1, PieceType::Queen),
        ];
        assert_eq!(
            format_attackers(&attackers),
            "attacked by the e3 pawn, b5 bishop, and d1 queen"
        );
    }

    #[test]
    fn format_attackers_empty_falls_back_to_generic() {
        assert_eq!(format_attackers(&[]), "attacked");
    }
}
