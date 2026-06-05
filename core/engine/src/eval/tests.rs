use super::*;
use crate::opponent::EvalCategory;

// ---- Tempo is the only thing distinguishing side-to-move ----------

#[test]
fn startpos_evaluates_to_tempo_plus_any_asymmetry_from_white_pov() {
    // The starting position is perfectly symmetric, so the signed
    // evaluation before tempo is zero. With white to move we get
    // +tempo; with black to move we get +tempo too (side-to-move
    // flip then add).
    let p = Position::startpos();
    let v = evaluate(&p);
    assert_eq!(v, TEMPO);
}

#[test]
fn startpos_with_black_to_move_also_tempo() {
    let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b KQkq - 0 1";
    let p = Position::from_fen(fen).unwrap();
    let v = evaluate(&p);
    assert_eq!(v, TEMPO);
}

// ---- Material preponderance --------------------------------------

#[test]
fn extra_queen_favours_owning_side() {
    // White has an extra queen on d1 — evaluation from white's POV
    // should be clearly positive.
    let p = Position::from_fen("4k3/8/8/8/8/8/8/3QK3 w - - 0 1").unwrap();
    let v = evaluate(&p);
    assert!(
        v.0 > 500,
        "extra queen should yield a big positive eval, got {}",
        v.0
    );
}

#[test]
fn extra_queen_for_black_is_negative_from_whites_turn() {
    // Black has an extra queen. With white to move we should be
    // deeply negative (minus queen material plus tempo).
    let p = Position::from_fen("3qk3/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
    let v = evaluate(&p);
    assert!(
        v.0 < -500,
        "down a queen should yield a big negative eval, got {}",
        v.0
    );
}

// ---- Determinism --------------------------------------------------

#[test]
fn empty_mask_matches_unmasked_evaluate() {
    // Sanity for the hot path: the mask-aware entry point with
    // an empty mask must produce bit-identical values to the
    // bare `evaluate(pos)` it replaced. Any divergence here is
    // a bug in the mask plumbing (probably an `else` branch
    // that picks up the masked value instead of the unmasked
    // sum).
    let positions = [
        "r1bqkb1r/ppp2ppp/2np1n2/4p3/2B1P3/2N2N2/PPPP1PPP/R1BQK2R w KQkq - 0 5",
        "r3k2r/pppq1ppp/2n2n2/3pp3/3PP3/2N2N2/PPPQ1PPP/R3K2R w KQkq - 0 1",
        "4k3/8/8/8/8/8/8/4K2R w K - 0 1",
    ];
    for fen in positions {
        let p = Position::from_fen(fen).expect("test FEN");
        let mut cache = pawns::Table::new();
        let bare = evaluate(&p);
        let masked = evaluate_with_pawn_cache(
            &p,
            &mut cache,
            EvalMask::EMPTY,
            crate::endgame::EndgameSkill::Full,
        );
        assert_eq!(
            bare, masked,
            "empty mask should match bare evaluate at {fen}",
        );
    }
}

#[test]
fn disabling_king_safety_changes_eval_when_king_safety_was_contributing() {
    // Pick a position where the king is genuinely exposed so the
    // KingSafety term has a non-zero contribution. With the
    // category masked off, the resulting score must differ.
    // (A null assertion would be too weak — we want to know the
    // gate actually short-circuits the += line, not just that
    // some line ran.)
    let p =
        Position::from_fen("r1bqkb1r/ppp2ppp/2np1n2/4p3/2B1P3/2N2N2/PPPP1PPP/R1BQK2R w KQkq - 0 5")
            .expect("test FEN");
    let mut cache = pawns::Table::new();
    let full = evaluate_with_pawn_cache(
        &p,
        &mut cache,
        EvalMask::EMPTY,
        crate::endgame::EndgameSkill::Full,
    );
    let mut mask = EvalMask::EMPTY;
    mask.disable(EvalCategory::KingSafety);
    let mut cache2 = pawns::Table::new();
    let masked =
        evaluate_with_pawn_cache(&p, &mut cache2, mask, crate::endgame::EndgameSkill::Full);
    assert_ne!(
        full, masked,
        "masking off KingSafety should change the score in a real midgame position",
    );
}

#[test]
fn evaluate_is_pure() {
    let p =
        Position::from_fen("r1bqkb1r/ppp2ppp/2np1n2/4p3/2B1P3/2N2N2/PPPP1PPP/R1BQK2R w KQkq - 0 5")
            .unwrap();
    let a = evaluate(&p);
    let b = evaluate(&p);
    assert_eq!(a, b);
}

// ---- Mirror symmetry ---------------------------------------------

#[test]
fn mirrored_positions_evaluate_to_symmetric_values() {
    // White's side to move evaluation of position A should equal
    // black's side to move evaluation of the colour-flipped mirror,
    // up to sign. Concrete test: Italian Game mirrored.
    let white_pov =
        Position::from_fen("r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 2 3")
            .unwrap();
    let black_pov =
        Position::from_fen("rnbqk2r/pppp1ppp/5n2/2b1p3/4P3/2N2N2/PPPP1PPP/R1BQKB1R b KQkq - 2 3")
            .unwrap();
    let v1 = evaluate(&white_pov);
    let v2 = evaluate(&black_pov);
    assert_eq!(
        v1, v2,
        "mirrored positions should give equal side-to-move evals"
    );
}

// ---- EvalTrace ---------------------------------------------------

#[test]
fn evaluate_with_trace_final_value_matches_evaluate() {
    // The trace's `final_value` must match what `evaluate`
    // returns on the same position — for positions that *don't*
    // trigger the lazy-eval short-circuit. The lazy gate
    // intentionally lets the untraced path bail early when
    // material is lopsided, so untraced and traced values
    // diverge there by design (see `lazy_eval_diverges_when_…`
    // for the explicit assertion of that divergence).
    let fens = [
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b KQkq - 0 1",
        "r1bqkb1r/ppp2ppp/2np1n2/4p3/2B1P3/2N2N2/PPPP1PPP/R1BQK2R w KQkq - 0 5",
    ];
    for fen in fens {
        let p = Position::from_fen(fen).unwrap();
        let direct = evaluate(&p);
        let (traced, trace) = evaluate_with_trace(&p);
        assert_eq!(direct, traced, "values must agree for {}", fen);
        assert_eq!(
            trace.final_value, direct,
            "trace.final_value must match for {}",
            fen
        );
        assert_eq!(trace.tempo, TEMPO);
    }
}

#[test]
fn lazy_eval_diverges_from_traced_on_lopsided_positions() {
    // Documents the intentional contract break introduced by
    // lazy eval: the untraced `evaluate()` is allowed to
    // short-circuit on positions whose material + imbalance +
    // pawn-structure score is already lopsided past
    // `LazyThreshold`, while `evaluate_with_trace()` always
    // runs the full breakdown. The untraced value is a
    // conservative approximation of the full one and is
    // typically a few hundred cp away — used by the search's
    // pruning where exact value isn't load-bearing. The
    // teaching layer always goes through the traced path so
    // user-visible numbers come from the full eval.
    let p = Position::from_fen("4k3/1p6/8/8/8/8/P7/3QK3 w - - 0 1").unwrap();
    let direct = evaluate(&p);
    let (traced, _) = evaluate_with_trace(&p);
    assert_ne!(
        direct, traced,
        "lazy eval must short-circuit this lopsided KQP-vs-KP position"
    );
}

#[test]
fn evaluate_with_trace_endgame_path_reports_final_value() {
    // KXK endgame short-circuits the classical breakdown. The
    // trace still ends up with `final_value` == `evaluate(pos)`;
    // per-term fields are left at zero by design (the eval didn't
    // come from classical terms).
    let p = Position::from_fen("4k3/8/8/8/8/8/8/3QK3 w - - 0 1").unwrap();
    let direct = evaluate(&p);
    let (traced, trace) = evaluate_with_trace(&p);
    assert_eq!(direct, traced);
    assert_eq!(trace.final_value, direct);
}

#[test]
fn trace_material_captures_psq_score() {
    // `trace.material.total()` is the PSQT score (material +
    // positional), pre-taper. For startpos this is exactly zero
    // by symmetry; both component fields are zero too.
    let p = Position::startpos();
    let (_, trace) = evaluate_with_trace(&p);
    assert_eq!(trace.material.piece_value, Score::ZERO);
    assert_eq!(trace.material.psq_positional, Score::ZERO);
    assert_eq!(trace.material.total(), Score::ZERO);
    assert_eq!(trace.material.total(), p.psq_score());
}

#[test]
fn trace_material_is_nonzero_when_material_is_imbalanced() {
    // Extra white queen — PSQT should skew heavily positive.
    // Include pawns so the KXK endgame driver doesn't short-circuit
    // past the classical trace.
    let p = Position::from_fen("4k3/1p6/8/8/8/8/P7/3QK3 w - - 0 1").unwrap();
    let (_, trace) = evaluate_with_trace(&p);
    assert_ne!(trace.material.total(), Score::ZERO);
    // Mg component of an extra queen is strongly positive for white;
    // the queen's raw piece value lives in `piece_value`.
    assert!(trace.material.piece_value.mg().0 > 500);
}

#[test]
fn trace_material_split_sums_to_psq_score() {
    // After any move sequence, piece_value + psq_positional must
    // equal pos.psq_score(). The split is exact, not an
    // approximation.
    let positions = [
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1",
        "r1bqkb1r/ppp2ppp/2np1n2/4p3/2B1P3/2N2N2/PPPP1PPP/R1BQK2R w KQkq - 0 5",
    ];
    for fen in positions {
        let p = Position::from_fen(fen).unwrap();
        let (_, trace) = evaluate_with_trace(&p);
        assert_eq!(
            trace.material.total(),
            p.psq_score(),
            "split must sum to psq_score for {fen}",
        );
    }
}

#[test]
fn trace_has_phase_and_scale_factor_in_valid_ranges() {
    let p =
        Position::from_fen("r1bqkb1r/ppp2ppp/2np1n2/4p3/2B1P3/2N2N2/PPPP1PPP/R1BQK2R w KQkq - 0 5")
            .unwrap();
    let (_, trace) = evaluate_with_trace(&p);
    assert!(
        (0..=128).contains(&trace.phase),
        "phase out of range: {}",
        trace.phase
    );
    assert!(
        trace.scale_factor > 0,
        "scale factor must be positive, got {}",
        trace.scale_factor
    );
}
