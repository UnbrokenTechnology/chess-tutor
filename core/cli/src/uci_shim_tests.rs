use super::*;

#[test]
fn build_position_startpos_no_moves() {
    let (pos, history, ply, last) = build_position("position startpos").unwrap();
    assert_eq!(pos.to_fen(), Position::startpos().to_fen());
    assert!(history.is_empty(), "no moves → no pre-root history");
    assert_eq!(ply, 0);
    assert_eq!(last, None, "no moves -> no attention locus");
}

#[test]
fn build_position_startpos_with_moves() {
    let (pos, history, ply, last) = build_position("position startpos moves e2e4 e7e5 g1f3").unwrap();
    // Root is after 3 half-moves; black... no, white to move? e4 e5 Nf3 → black to move.
    assert_eq!(pos.side_to_move(), chess_tutor_engine::types::Color::Black);
    assert_eq!(ply, 3);
    // Pre-root history excludes the root: keys before e4, before e5,
    // before Nf3 — three entries.
    assert_eq!(history.len(), 3);
    // The first history key is the start position's key.
    assert_eq!(history[0], Position::startpos().key());
    // History must not contain the root key (would be a phantom
    // repetition of the current position).
    assert!(!history.contains(&pos.key()));
    // The attention locus is the destination of the LAST applied move.
    assert_eq!(
        last,
        chess_tutor_engine::types::Square::from_algebraic("f3"),
    );
}

#[test]
fn build_position_from_fen() {
    // No contentious en-passant square: the engine canonicalises the EP
    // field to `-` when no EP capture is actually legal, so a FEN with a
    // live `c6` would not round-trip. This one round-trips exactly.
    let fen = "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3";
    let (pos, history, ply, _last) = build_position(&format!("position fen {fen}")).unwrap();
    assert_eq!(pos.to_fen(), fen);
    assert!(history.is_empty());
    assert_eq!(ply, 0);
}

#[test]
fn build_position_from_fen_with_moves() {
    let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    let (pos, history, ply, last) = build_position(&format!("position fen {fen} moves e2e4")).unwrap();
    assert_eq!(ply, 1);
    assert_eq!(history.len(), 1);
    assert_eq!(history[0], Position::startpos().key());
    assert_eq!(pos.side_to_move(), chess_tutor_engine::types::Color::Black);
    assert_eq!(
        last,
        chess_tutor_engine::types::Square::from_algebraic("e4"),
    );
}

#[test]
fn build_position_rejects_garbage_spec() {
    assert!(build_position("position banana").is_err());
}

#[test]
fn build_position_rejects_illegal_move() {
    // e2e5 is not a legal first move.
    assert!(build_position("position startpos moves e2e5").is_err());
}

#[test]
fn parse_go_depth_variants() {
    assert_eq!(parse_go_depth("go depth 12"), Some(12));
    assert_eq!(parse_go_depth("go wtime 1000 btime 1000 depth 8"), Some(8));
    assert_eq!(parse_go_depth("go infinite"), None);
    assert_eq!(parse_go_depth("go"), None);
    assert_eq!(parse_go_depth("go depth notanumber"), None);
}

#[test]
fn mix_seed_is_deterministic_and_varies_per_game() {
    assert_eq!(mix_seed(42, 0), mix_seed(42, 0), "same inputs → same seed");
    assert_ne!(mix_seed(42, 0), mix_seed(42, 1), "game index changes the seed");
    assert_ne!(mix_seed(1, 5), mix_seed(2, 5), "base seed changes the seed");
}

#[test]
fn uci_score_renders_cp_in_conventional_scale() {
    use chess_tutor_engine::types::Value;
    // PAWN_EG engine-cp == exactly one conventional pawn == 100 cp.
    assert_eq!(uci_score(Value::PAWN_EG), "cp 100");
    assert_eq!(uci_score(Value(0)), "cp 0");
    // Negative (side to move is worse) survives the sign.
    assert_eq!(uci_score(Value(-Value::PAWN_EG.0)), "cp -100");
}

#[test]
fn uci_score_renders_signed_mate() {
    use chess_tutor_engine::types::Value;
    // MATE - 3 plies => mate in 2 full moves, stm delivering.
    assert_eq!(uci_score(Value(Value::MATE.0 - 3)), "mate 2");
    // Getting mated is negative.
    assert_eq!(uci_score(Value(-(Value::MATE.0 - 3))), "mate -2");
}
