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

use chess_tutor_engine::eval::{
    EvalTrace, KingBreakdown, MobilityBreakdown, PassedBreakdown, PawnsBreakdown, PiecesBreakdown,
    ThreatsBreakdown,
};
use chess_tutor_engine::types::Score;

/// Render an [`EvalTrace`] as a multi-line report.
pub fn render(trace: &EvalTrace) -> String {
    let mut out = String::new();
    writeln!(out, "phase:        {} / 128", trace.phase).unwrap();
    writeln!(out, "scale factor: {} / 64", trace.scale_factor).unwrap();
    writeln!(out, "tempo:        {}", trace.tempo.0).unwrap();
    writeln!(
        out,
        "final value:  {} cp (side-to-move POV)",
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

    // Pawn-structure: aggregate row plus indented sub-terms.
    let pw = &trace.pawns[0];
    let pb = &trace.pawns[1];
    write_pair_row(&mut out, "pawns", pw.total(), pb.total());
    for (name, w, b) in pawns_sub_rows(pw, pb) {
        write_pair_row(&mut out, &format!("  {}", name), w, b);
    }

    // Per-piece positional: same shape — aggregate row plus indented
    // sub-terms.
    let mw = &trace.pieces[0];
    let mb = &trace.pieces[1];
    write_pair_row(&mut out, "pieces", mw.total(), mb.total());
    for (name, w, b) in pieces_sub_rows(mw, mb) {
        write_pair_row(&mut out, &format!("  {}", name), w, b);
    }

    // Mobility: aggregate row plus indented per-piece-type sub-terms.
    let mbw = &trace.mobility[0];
    let mbb = &trace.mobility[1];
    write_pair_row(&mut out, "mobility", mbw.total(), mbb.total());
    for (name, w, b) in mobility_sub_rows(mbw, mbb) {
        write_pair_row(&mut out, &format!("  {}", name), w, b);
    }

    // Threats: aggregate row plus indented per-sub-term rows.
    let tw = &trace.threats[0];
    let tb = &trace.threats[1];
    write_pair_row(&mut out, "threats", tw.total(), tb.total());
    for (name, w, b) in threats_sub_rows(tw, tb) {
        write_pair_row(&mut out, &format!("  {}", name), w, b);
    }

    // King safety: aggregate row plus indented per-sub-term rows.
    let kw = &trace.king[0];
    let kb = &trace.king[1];
    write_pair_row(&mut out, "king", kw.total(), kb.total());
    for (name, w, b) in king_sub_rows(kw, kb) {
        write_pair_row(&mut out, &format!("  {}", name), w, b);
    }

    // Passed pawns: aggregate row plus indented per-sub-term rows.
    let paw = &trace.passed[0];
    let pab = &trace.passed[1];
    write_pair_row(&mut out, "passed", paw.total(), pab.total());
    for (name, w, b) in passed_sub_rows(paw, pab) {
        write_pair_row(&mut out, &format!("  {}", name), w, b);
    }

    // Space is still an aggregate single-row term.
    let aggregates: &[(&str, [Score; 2])] = &[("space", trace.space)];
    for (name, scores) in aggregates {
        write_pair_row(&mut out, name, scores[0], scores[1]);
    }

    // Single-sided (already net) terms — show only the net columns.
    // `material` is shown as an aggregate row above the two split
    // sub-rows (piece value + PSQ positional) so the eval report
    // mirrors the run-time decomposition without losing the legacy
    // single-row sum that older snapshots compared against.
    let singles: &[(&str, Score)] = &[
        ("material", trace.material.total()),
        ("  piece-value", trace.material.piece_value),
        ("  psq-positional", trace.material.psq_positional),
        ("imbalance", trace.imbalance),
        ("initiative", trace.initiative),
        ("TOTAL", trace.total),
    ];
    writeln!(out).unwrap();
    writeln!(out, "{:<24}  {:>9}  {:>9}", "term (net)", "mg", "eg").unwrap();
    writeln!(out, "{}", "-".repeat(24 + 2 + 9 * 2 + 2)).unwrap();
    for (name, score) in singles {
        writeln!(
            out,
            "{:<24}  {:>9}  {:>9}",
            name,
            score.mg().0,
            score.eg().0
        )
        .unwrap();
    }
    out
}

fn write_pair_row(out: &mut String, name: &str, w: Score, b: Score) {
    let net = w - b;
    writeln!(
        out,
        "{:<24}  {:>9}  {:>9}  {:>9}  {:>9}  {:>9}  {:>9}",
        name,
        w.mg().0,
        w.eg().0,
        b.mg().0,
        b.eg().0,
        net.mg().0,
        net.eg().0,
    )
    .unwrap();
}

fn king_sub_rows<'a>(
    w: &'a KingBreakdown,
    b: &'a KingBreakdown,
) -> impl Iterator<Item = (&'static str, Score, Score)> + 'a {
    [
        ("pawn-shield", w.pawn_shield, b.pawn_shield),
        ("pawn-storm", w.pawn_storm, b.pawn_storm),
        ("pawn-distance", w.king_pawn_distance, b.king_pawn_distance),
        ("danger", w.danger, b.danger),
        ("pawnless-flank", w.pawnless_flank, b.pawnless_flank),
        ("flank-attacks", w.flank_attacks, b.flank_attacks),
    ]
    .into_iter()
}

fn passed_sub_rows<'a>(
    w: &'a PassedBreakdown,
    b: &'a PassedBreakdown,
) -> impl Iterator<Item = (&'static str, Score, Score)> + 'a {
    [
        ("rank-bonus", w.rank_bonus, b.rank_bonus),
        ("king-proximity", w.king_proximity, b.king_proximity),
        ("free-advance", w.free_advance, b.free_advance),
        ("stopper-penalty", w.stopper_penalty, b.stopper_penalty),
    ]
    .into_iter()
}

fn mobility_sub_rows<'a>(
    w: &'a MobilityBreakdown,
    b: &'a MobilityBreakdown,
) -> impl Iterator<Item = (&'static str, Score, Score)> + 'a {
    [
        ("knight", w.knight, b.knight),
        ("bishop", w.bishop, b.bishop),
        ("rook", w.rook, b.rook),
        ("queen", w.queen, b.queen),
    ]
    .into_iter()
}

fn threats_sub_rows<'a>(
    w: &'a ThreatsBreakdown,
    b: &'a ThreatsBreakdown,
) -> impl Iterator<Item = (&'static str, Score, Score)> + 'a {
    [
        ("by-minor", w.by_minor, b.by_minor),
        ("by-rook", w.by_rook, b.by_rook),
        ("by-king", w.by_king, b.by_king),
        ("hanging", w.hanging, b.hanging),
        ("restricted", w.restricted, b.restricted),
        ("by-safe-pawn", w.by_safe_pawn, b.by_safe_pawn),
        ("by-pawn-push", w.by_pawn_push, b.by_pawn_push),
        ("knight-on-queen", w.knight_on_queen, b.knight_on_queen),
        ("slider-on-queen", w.slider_on_queen, b.slider_on_queen),
    ]
    .into_iter()
}

fn pawns_sub_rows<'a>(
    w: &'a PawnsBreakdown,
    b: &'a PawnsBreakdown,
) -> impl Iterator<Item = (&'static str, Score, Score)> + 'a {
    [
        ("connected", w.connected, b.connected),
        ("isolated", w.isolated, b.isolated),
        ("backward", w.backward, b.backward),
        ("doubled", w.doubled, b.doubled),
        ("weak-unopposed", w.weak_unopposed, b.weak_unopposed),
        ("weak-lever", w.weak_lever, b.weak_lever),
    ]
    .into_iter()
}

fn pieces_sub_rows<'a>(
    w: &'a PiecesBreakdown,
    b: &'a PiecesBreakdown,
) -> impl Iterator<Item = (&'static str, Score, Score)> + 'a {
    [
        ("outposts", w.outposts, b.outposts),
        (
            "reachable-outposts",
            w.reachable_outposts,
            b.reachable_outposts,
        ),
        (
            "minor-behind-pawn",
            w.minor_behind_pawn,
            b.minor_behind_pawn,
        ),
        ("king-protector", w.king_protector, b.king_protector),
        ("bishop-pawns", w.bishop_pawns, b.bishop_pawns),
        (
            "long-diagonal-bishop",
            w.long_diagonal_bishop,
            b.long_diagonal_bishop,
        ),
        (
            "rook-on-queen-file",
            w.rook_on_queen_file,
            b.rook_on_queen_file,
        ),
        (
            "rook-on-open-file",
            w.rook_on_open_file,
            b.rook_on_open_file,
        ),
        (
            "rook-on-semiopen-file",
            w.rook_on_semiopen_file,
            b.rook_on_semiopen_file,
        ),
        ("trapped-rook", w.trapped_rook, b.trapped_rook),
        ("weak-queen", w.weak_queen, b.weak_queen),
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
