//! Pretty-print an [`EvalTrace`] as a per-term table.
//!
//! This is the teaching tool's bare-bones UI: for every evaluation
//! term, show white's mg/eg contribution, black's mg/eg contribution,
//! and the net (white − black) in mg/eg. Per-term scores are the raw
//! [`Score`] values captured in [`EvalTrace`]; they are pre-taper and
//! pre-side-flip. The final single-number `final_value` at the bottom
//! is the fully-tapered, side-to-move-signed result.
//!
//! The pawn-structure and per-piece-positional terms are rendered as
//! an aggregate row followed by indented sub-term rows (isolated pawn,
//! knight outpost, etc.), mirroring the granular
//! [`chess_tutor_engine::eval::PawnsBreakdown`] and
//! [`chess_tutor_engine::eval::PiecesBreakdown`] the teaching analysis
//! pipeline consumes.

use std::fmt::Write;

use chess_tutor_engine::analysis::TermId;
use chess_tutor_engine::eval::{
    EvalTrace, KingBreakdown, MobilityBreakdown, PassedBreakdown, PawnsBreakdown, PiecesBreakdown,
    ThreatsBreakdown,
};
use chess_tutor_engine::types::Score;

use crate::glossary;

/// Render an [`EvalTrace`] as a multi-line report.
pub fn render(trace: &EvalTrace) -> String {
    let mut out = String::new();
    // Every number below this point is **engine-internal cp** (SF11
    // scale: PawnEG = 213, PawnMG = 128). That's deliberate — the
    // tapered eval combines per-term `(mg, eg)` Score components
    // pre-conversion, and the only honest way to render the
    // decomposition is at the scale the engine actually uses. The
    // headline summary above the table already shows pawns (= chess.com
    // scale) for the human-comparable read.
    writeln!(out, "phase:        {} / 128", trace.phase).unwrap();
    writeln!(out, "scale factor: {} / 64", trace.scale_factor).unwrap();
    writeln!(
        out,
        "tempo:        {:+} cp  (engine-internal, PawnEG=213 scale)",
        trace.tempo.0
    )
    .unwrap();
    writeln!(
        out,
        "final value:  {:+} cp  (engine-internal, side-to-move POV)",
        trace.final_value.0
    )
    .unwrap();
    writeln!(out).unwrap();

    writeln!(
        out,
        "{:<24}  {:>9}  {:>9}  {:>9}  {:>9}  {:>9}  {:>9}",
        "term", "white mg", "white eg", "black mg", "black eg", "net mg", "net eg"
    )
    .unwrap();
    writeln!(out, "{}", "-".repeat(24 + 2 + 9 * 6 + 2 * 5)).unwrap();

    // Pawn-structure: aggregate row plus indented sub-terms with gloss.
    let pw = &trace.pawns[0];
    let pb = &trace.pawns[1];
    write_pair_row(&mut out, "pawns", pw.total(), pb.total(), None);
    for (name, id, w, b) in pawns_sub_rows(pw, pb) {
        write_pair_row(&mut out, &format!("  {}", name), w, b, Some(id));
    }

    // Per-piece positional.
    let mw = &trace.pieces[0];
    let mb = &trace.pieces[1];
    write_pair_row(&mut out, "pieces", mw.total(), mb.total(), None);
    for (name, id, w, b) in pieces_sub_rows(mw, mb) {
        write_pair_row(&mut out, &format!("  {}", name), w, b, Some(id));
    }

    // Mobility.
    let mbw = &trace.mobility[0];
    let mbb = &trace.mobility[1];
    write_pair_row(&mut out, "mobility", mbw.total(), mbb.total(), None);
    for (name, id, w, b) in mobility_sub_rows(mbw, mbb) {
        write_pair_row(&mut out, &format!("  {}", name), w, b, Some(id));
    }

    // Threats.
    let tw = &trace.threats[0];
    let tb = &trace.threats[1];
    write_pair_row(&mut out, "threats", tw.total(), tb.total(), None);
    for (name, id, w, b) in threats_sub_rows(tw, tb) {
        write_pair_row(&mut out, &format!("  {}", name), w, b, Some(id));
    }

    // King safety.
    let kw = &trace.king[0];
    let kb = &trace.king[1];
    write_pair_row(&mut out, "king", kw.total(), kb.total(), None);
    for (name, id, w, b) in king_sub_rows(kw, kb) {
        write_pair_row(&mut out, &format!("  {}", name), w, b, Some(id));
    }

    // Passed pawns.
    let paw = &trace.passed[0];
    let pab = &trace.passed[1];
    write_pair_row(&mut out, "passed", paw.total(), pab.total(), None);
    for (name, id, w, b) in passed_sub_rows(paw, pab) {
        write_pair_row(&mut out, &format!("  {}", name), w, b, Some(id));
    }

    // Space is still an aggregate single-row term (one TermId, one row).
    write_pair_row(&mut out, "space", trace.space[0], trace.space[1], Some(TermId::Space));

    // Single-sided (already net) terms — show only the net columns.
    // `material` is shown as an aggregate row above the two split
    // sub-rows (piece value + PSQ positional) so the eval report
    // mirrors the run-time decomposition without losing the legacy
    // single-row sum that older snapshots compared against.
    let singles: &[(&str, Score, Option<TermId>)] = &[
        ("material", trace.material.total(), None),
        (
            "  piece-value",
            trace.material.piece_value,
            Some(TermId::MaterialPieceValue),
        ),
        (
            "  psq-positional",
            trace.material.psq_positional,
            Some(TermId::MaterialPsqPositional),
        ),
        ("imbalance", trace.imbalance, Some(TermId::Imbalance)),
        ("initiative", trace.initiative, Some(TermId::Initiative)),
        ("TOTAL", trace.total, None),
    ];
    writeln!(out).unwrap();
    writeln!(out, "{:<24}  {:>9}  {:>9}  description", "term (net)", "mg", "eg").unwrap();
    writeln!(out, "{}", "-".repeat(24 + 2 + 9 * 2 + 2 + 80)).unwrap();
    for (name, score, id) in singles {
        let gloss_str = match id {
            Some(t) => format!("  // {}", glossary::description(*t)),
            None => String::new(),
        };
        writeln!(
            out,
            "{:<24}  {:>9}  {:>9}{}",
            name,
            score.mg().0,
            score.eg().0,
            gloss_str,
        )
        .unwrap();
    }
    out
}

fn write_pair_row(out: &mut String, name: &str, w: Score, b: Score, id: Option<TermId>) {
    let net = w - b;
    // The gloss prints as a trailing `  // description` so a regex /
    // column-parser can ignore everything past the first `//` and still
    // parse the numeric block deterministically.
    let gloss_str = match id {
        Some(t) => format!("  // {}", glossary::description(t)),
        None => String::new(),
    };
    writeln!(
        out,
        "{:<24}  {:>9}  {:>9}  {:>9}  {:>9}  {:>9}  {:>9}{}",
        name,
        w.mg().0,
        w.eg().0,
        b.mg().0,
        b.eg().0,
        net.mg().0,
        net.eg().0,
        gloss_str,
    )
    .unwrap();
}

// Each sub_rows iterator yields `(short-label, TermId, white_score,
// black_score)`. The TermId feeds the gloss lookup so `write_pair_row`
// can append a `// description` trailer.

fn king_sub_rows<'a>(
    w: &'a KingBreakdown,
    b: &'a KingBreakdown,
) -> impl Iterator<Item = (&'static str, TermId, Score, Score)> + 'a {
    [
        ("pawn-shield", TermId::KingPawnShield, w.pawn_shield, b.pawn_shield),
        ("pawn-storm", TermId::KingPawnStorm, w.pawn_storm, b.pawn_storm),
        ("pawn-distance", TermId::KingPawnDistance, w.king_pawn_distance, b.king_pawn_distance),
        ("danger", TermId::KingDanger, w.danger, b.danger),
        ("pawnless-flank", TermId::KingPawnlessFlank, w.pawnless_flank, b.pawnless_flank),
        ("flank-attacks", TermId::KingFlankAttacks, w.flank_attacks, b.flank_attacks),
    ]
    .into_iter()
}

fn passed_sub_rows<'a>(
    w: &'a PassedBreakdown,
    b: &'a PassedBreakdown,
) -> impl Iterator<Item = (&'static str, TermId, Score, Score)> + 'a {
    [
        ("rank-bonus", TermId::PassedRankBonus, w.rank_bonus, b.rank_bonus),
        ("king-proximity", TermId::PassedKingProximity, w.king_proximity, b.king_proximity),
        ("free-advance", TermId::PassedFreeAdvance, w.free_advance, b.free_advance),
        ("stopper-penalty", TermId::PassedStopperPenalty, w.stopper_penalty, b.stopper_penalty),
    ]
    .into_iter()
}

fn mobility_sub_rows<'a>(
    w: &'a MobilityBreakdown,
    b: &'a MobilityBreakdown,
) -> impl Iterator<Item = (&'static str, TermId, Score, Score)> + 'a {
    [
        ("knight", TermId::MobilityKnight, w.knight, b.knight),
        ("bishop", TermId::MobilityBishop, w.bishop, b.bishop),
        ("rook", TermId::MobilityRook, w.rook, b.rook),
        ("queen", TermId::MobilityQueen, w.queen, b.queen),
    ]
    .into_iter()
}

fn threats_sub_rows<'a>(
    w: &'a ThreatsBreakdown,
    b: &'a ThreatsBreakdown,
) -> impl Iterator<Item = (&'static str, TermId, Score, Score)> + 'a {
    [
        ("by-minor", TermId::ThreatsByMinor, w.by_minor, b.by_minor),
        ("by-rook", TermId::ThreatsByRook, w.by_rook, b.by_rook),
        ("by-king", TermId::ThreatsByKing, w.by_king, b.by_king),
        ("hanging", TermId::ThreatsHanging, w.hanging, b.hanging),
        ("restricted", TermId::ThreatsRestricted, w.restricted, b.restricted),
        ("by-safe-pawn", TermId::ThreatsBySafePawn, w.by_safe_pawn, b.by_safe_pawn),
        ("by-pawn-push", TermId::ThreatsByPawnPush, w.by_pawn_push, b.by_pawn_push),
        ("knight-on-queen", TermId::ThreatsKnightOnQueen, w.knight_on_queen, b.knight_on_queen),
        ("slider-on-queen", TermId::ThreatsSliderOnQueen, w.slider_on_queen, b.slider_on_queen),
    ]
    .into_iter()
}

fn pawns_sub_rows<'a>(
    w: &'a PawnsBreakdown,
    b: &'a PawnsBreakdown,
) -> impl Iterator<Item = (&'static str, TermId, Score, Score)> + 'a {
    [
        ("connected", TermId::PawnsConnected, w.connected, b.connected),
        ("isolated", TermId::PawnsIsolated, w.isolated, b.isolated),
        ("backward", TermId::PawnsBackward, w.backward, b.backward),
        ("doubled", TermId::PawnsDoubled, w.doubled, b.doubled),
        ("weak-unopposed", TermId::PawnsWeakUnopposed, w.weak_unopposed, b.weak_unopposed),
        ("weak-lever", TermId::PawnsWeakLever, w.weak_lever, b.weak_lever),
    ]
    .into_iter()
}

fn pieces_sub_rows<'a>(
    w: &'a PiecesBreakdown,
    b: &'a PiecesBreakdown,
) -> impl Iterator<Item = (&'static str, TermId, Score, Score)> + 'a {
    [
        ("outposts", TermId::PiecesOutposts, w.outposts, b.outposts),
        ("reachable-outposts", TermId::PiecesReachableOutposts, w.reachable_outposts, b.reachable_outposts),
        ("minor-behind-pawn", TermId::PiecesMinorBehindPawn, w.minor_behind_pawn, b.minor_behind_pawn),
        ("king-protector", TermId::PiecesKingProtector, w.king_protector, b.king_protector),
        ("bishop-pawns", TermId::PiecesBishopPawns, w.bishop_pawns, b.bishop_pawns),
        ("long-diagonal-bishop", TermId::PiecesLongDiagonalBishop, w.long_diagonal_bishop, b.long_diagonal_bishop),
        ("rook-on-queen-file", TermId::PiecesRookOnQueenFile, w.rook_on_queen_file, b.rook_on_queen_file),
        ("rook-on-open-file", TermId::PiecesRookOnOpenFile, w.rook_on_open_file, b.rook_on_open_file),
        ("rook-on-semiopen-file", TermId::PiecesRookOnSemiopenFile, w.rook_on_semiopen_file, b.rook_on_semiopen_file),
        ("trapped-rook", TermId::PiecesTrappedRook, w.trapped_rook, b.trapped_rook),
        ("weak-queen", TermId::PiecesWeakQueen, w.weak_queen, b.weak_queen),
    ]
    .into_iter()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::eval::evaluate_with_trace;
    use chess_tutor_engine::position::Position;

    #[test]
    fn renders_startpos_trace_without_panic() {
        let pos = Position::startpos();
        let (_v, trace) = evaluate_with_trace(&pos);
        let out = render(&trace);
        assert!(out.contains("material"));
        assert!(out.contains("mobility"));
        assert!(out.contains("TOTAL"));
        assert!(out.contains("final value:"));
    }

    #[test]
    fn renders_pawns_and_pieces_sub_terms() {
        // After the Phase-0 refactor, the report surfaces sub-term rows
        // under the aggregate pawns/pieces headings. Spot-check a few
        // representative labels so future refactors that collapse them
        // back fail loudly.
        let pos = Position::startpos();
        let (_v, trace) = evaluate_with_trace(&pos);
        let out = render(&trace);
        assert!(out.contains("isolated"));
        assert!(out.contains("connected"));
        assert!(out.contains("outposts"));
        assert!(out.contains("rook-on-open-file"));
        assert!(out.contains("weak-queen"));
    }

    #[test]
    fn renders_mobility_sub_terms_by_piece_type() {
        // Parallel to renders_pawns_and_pieces_sub_terms but for the
        // mobility split. Each per-piece-type row should surface under
        // the aggregate "mobility" heading.
        let pos = Position::startpos();
        let (_v, trace) = evaluate_with_trace(&pos);
        let out = render(&trace);
        assert!(out.contains("mobility"));
        assert!(out.contains("knight"));
        assert!(out.contains("bishop"));
        assert!(out.contains("rook"));
        assert!(out.contains("queen"));
    }

    #[test]
    fn renders_passed_sub_terms() {
        // Passed split into 4 sub-terms — spot-check each surfaces
        // under the aggregate "passed" heading.
        let pos = Position::startpos();
        let (_v, trace) = evaluate_with_trace(&pos);
        let out = render(&trace);
        assert!(out.contains("passed"));
        assert!(out.contains("rank-bonus"));
        assert!(out.contains("king-proximity"));
        assert!(out.contains("free-advance"));
        assert!(out.contains("stopper-penalty"));
    }

    #[test]
    fn renders_king_sub_terms() {
        // King split into 6 sub-terms — pawn-shield, pawn-storm,
        // pawn-distance (= former `shelter` aggregate), plus danger,
        // pawnless-flank, flank-attacks. Spot-check each surfaces
        // under the aggregate "king" heading.
        let pos = Position::startpos();
        let (_v, trace) = evaluate_with_trace(&pos);
        let out = render(&trace);
        assert!(out.contains("king"));
        assert!(out.contains("pawn-shield"));
        assert!(out.contains("pawn-storm"));
        assert!(out.contains("pawn-distance"));
        assert!(out.contains("danger"));
        assert!(out.contains("pawnless-flank"));
        assert!(out.contains("flank-attacks"));
    }

    #[test]
    fn renders_threats_sub_terms() {
        // Threats split into 9 sub-terms — spot-check representative
        // labels so future refactors that collapse them back fail
        // loudly.
        let pos = Position::startpos();
        let (_v, trace) = evaluate_with_trace(&pos);
        let out = render(&trace);
        assert!(out.contains("threats"));
        assert!(out.contains("by-minor"));
        assert!(out.contains("hanging"));
        assert!(out.contains("restricted"));
        assert!(out.contains("by-safe-pawn"));
        assert!(out.contains("knight-on-queen"));
        assert!(out.contains("slider-on-queen"));
    }
}
