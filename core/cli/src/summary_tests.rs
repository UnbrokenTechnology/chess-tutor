//! Sibling tests for [`super`] (`summary.rs`).

use super::*;
use chess_tutor_engine::position::Position;

#[test]
fn startpos_summary_text_is_well_formed() {
    let pos = Position::startpos();
    let summary = build(&pos, ScoreSource::Static, None);
    let text = render_text(&summary);

    assert!(text.contains("position: "));
    assert!(text.contains("to move:  White"));
    assert!(text.contains("in check: no"));
    assert!(text.contains("material: even"));
    assert!(text.contains("legal:    20 moves"));
    assert!(text.contains("[static]"));
    // Score should be near zero — within 0.5 pawns either way.
    assert!(
        text.contains("+0.0") || text.contains("-0.0") || text.contains("+0.1") || text.contains("-0.1"),
        "expected near-zero pawns: {text}",
    );
}

#[test]
fn black_to_move_summary_uses_white_pov_for_score() {
    // After 1.e4, black is to move. The static eval from black's
    // POV is slightly negative (white has the better position); the
    // summary should re-express it as positive for white.
    let pos = Position::from_fen("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1")
        .unwrap();
    let summary = build(&pos, ScoreSource::Static, None);
    assert_eq!(summary.to_move, "Black");
    // White-POV: should NOT start with '-' (Black is to move and
    // worse, so white-POV is non-negative). Tolerate a tiny ε.
    let pawns = summary.score.pawns_white_pov.as_str();
    assert!(
        !pawns.starts_with('-') || pawns == "-0.00",
        "expected white-POV positive (or zero), got {pawns}",
    );
}

#[test]
fn search_score_source_carries_depth_tag() {
    use chess_tutor_engine::types::Value;
    let pos = Position::startpos();
    let summary = build(&pos, ScoreSource::Search { depth: 14 }, Some(Value(50)));
    let text = render_text(&summary);
    assert!(text.contains("[search d=14]"), "{text}");
    // External value 50 → +0.23 pawns; the headline shows that plus
    // engine-cp +50 stm so both scales are visible without
    // mistaking one for the other.
    assert!(text.contains("+0.23 pawns"), "{text}");
    assert!(text.contains("engine-cp: +50 stm"), "{text}");
}

#[test]
fn checkmate_position_flags_terminal() {
    // Minimal queen-and-king mate: white Q on a7 defended by Kb6
    // checks the black king on a8 with no escape (b7/b8 covered, a-file
    // and rank 7 covered, Qa7 defended so Kxa7 illegal).
    let pos = Position::from_fen("k7/Q7/1K6/8/8/8/8/8 b - - 0 1").unwrap();
    let summary = build(&pos, ScoreSource::Static, None);
    assert_eq!(summary.legal_move_count, 0);
    assert!(summary.in_check);
    assert!(summary.terminal.unwrap().contains("checkmate"));
}

#[test]
fn material_block_renders_piece_codes() {
    let pos = Position::startpos();
    let summary = build(&pos, ScoreSource::Static, None);
    assert_eq!(summary.material.white_summary, "Q+2R+2B+2N+8P");
    assert_eq!(summary.material.black_summary, "Q+2R+2B+2N+8P");
    assert_eq!(summary.material.white_points, 9 + 10 + 6 + 6 + 8);
    assert_eq!(summary.material.balance, "even");
}

#[test]
fn material_imbalance_describes_advantage() {
    // King + extra rook for white vs lone king.
    let pos = Position::from_fen("7k/8/8/8/8/8/8/R3K3 w Q - 0 1").unwrap();
    let summary = build(&pos, ScoreSource::Static, None);
    assert_eq!(summary.material.balance, "white +5");
    assert_eq!(summary.material.white_summary, "R");
    assert_eq!(summary.material.black_summary, "K only");
}

#[test]
fn json_round_trips_summary() {
    let pos = Position::startpos();
    let summary = build(&pos, ScoreSource::Static, None);
    let json = serde_json::to_string(&summary).unwrap();
    assert!(json.contains("\"to_move\":\"White\""));
    assert!(json.contains("\"win_pct_white\":"));
    // Engine-cp is explicit in the field name so callers can't confuse
    // it with the conv-cp field. Both must be present.
    assert!(json.contains("\"engine_cp_stm\":"));
    assert!(json.contains("\"engine_cp_white_pov\":"));
    assert!(json.contains("\"conv_cp_white_pov\":"));
}
