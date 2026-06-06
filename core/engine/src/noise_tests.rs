use super::*;
use crate::engine::SearchLine;
use crate::position::Position;
use crate::types::{Move, Square, Value};

/// Stub line with the given score and an empty PV. Used by tests that
/// exercise branches which don't classify material (variety, mate-guard
/// suppression, determinism): with an empty PV the material delta is
/// zero, which is fine because those tests never expect the blunder/miss
/// branch to *fire*.
fn line(score_cp: i32) -> SearchLine {
    SearchLine {
        pv: Vec::<Move>::new(),
        score: Value(score_cp),
        depth: 1,
        ply_traces: Vec::new(),
        settled_ply: None,
    }
}

/// Line with a real PV so the material classifier has something to walk.
fn mat_line(score_cp: i32, pv: Vec<Move>, settled: Option<usize>) -> SearchLine {
    SearchLine {
        pv,
        score: Value(score_cp),
        depth: 8,
        ply_traces: Vec::new(),
        settled_ply: settled,
    }
}

/// Position used by the material-classifier and miss/blunder tests.
/// White: Ke1, Qd4, Pe3. Black: Ke8, Qd5. White to move. White's `Qxd5`
/// wins the black queen outright (undefended); `e4` then `…Qxe4` hangs
/// the e-pawn; `Kf1` is a quiet, material-neutral move.
fn mat_root() -> Position {
    Position::from_fen("4k3/8/8/3q4/3Q4/4P3/8/4K3 w - - 0 1").unwrap()
}

fn any_root() -> Position {
    Position::startpos()
}

// Named moves on `mat_root`.
fn qxd5() -> Move {
    Move::normal(Square::D4, Square::D5)
}
fn kf1() -> Move {
    Move::normal(Square::E1, Square::F1)
}
// White hangs the e-pawn: e3-e4 then Black …Qxe4.
fn hang_pawn_pv() -> Vec<Move> {
    vec![Move::normal(Square::E3, Square::E4), Move::normal(Square::D5, Square::E4)]
}

/// Position with a *non-capture* material win for the miss-branch tests:
/// the win comes from a knight fork, so its first move is quiet. White:
/// Ke1, Ne4, Pb2. Black: Ke8, Qd5. `Nf6+` forks king and queen; after the
/// forced king move `Nxd5` wins the queen — a win that opens with a quiet
/// move, which the gated miss branch may decline. `b2-b3` hangs the b-pawn
/// to `…Qxb3` (a one-pawn, in-band slip). `Ke2` is quiet and neutral.
fn fork_root() -> Position {
    Position::from_fen("4k3/8/8/3q4/4N3/8/1P6/4K3 w - - 0 1").unwrap()
}
// The non-capture queen-winning fork: Nf6+, Kf8, Nxd5 (capture at ply 2).
fn fork_win_pv() -> Vec<Move> {
    vec![
        Move::normal(Square::E4, Square::F6),
        Move::normal(Square::E8, Square::F8),
        Move::normal(Square::F6, Square::D5),
    ]
}
// White hangs the b-pawn: b2-b3 then …Qxb3.
fn fork_hang_pawn_pv() -> Vec<Move> {
    vec![Move::normal(Square::B2, Square::B3), Move::normal(Square::D5, Square::B3)]
}
// Quiet, material-neutral white king move on `fork_root`.
fn fork_quiet() -> Move {
    Move::normal(Square::E1, Square::E2)
}

/// Position with a *capture-first sacrifice* — the case the simple
/// `!first_move_is_capture` gate would have mis-classified as an obvious
/// grab. White: Ke1, Ng4, Bb3. Black: Ke8, Rf7, Pd6, Pe5. White's winning
/// line opens with the capture `Nxe5` (+1), is recaptured `…dxe5` (−3, so
/// **down 2 pawns after two plies**), then `Bxf7` wins the rook (+5) — net
/// +3 at the settled end. First move is a capture, yet it's a combination,
/// so the 2-ply read keeps it missable.
fn sac_root() -> Position {
    Position::from_fen("4k3/5r2/3p4/4p3/6N1/1B6/8/4K3 w - - 0 1").unwrap()
}
// Nxe5 (+1), …dxe5 (−3), Bxf7 (+5): capture-first, down at 2 plies, wins.
fn sac_win_pv() -> Vec<Move> {
    vec![
        Move::normal(Square::G4, Square::E5),
        Move::normal(Square::D6, Square::E5),
        Move::normal(Square::B3, Square::F7),
    ]
}
// Quiet, material-neutral white king move on `sac_root`.
fn sac_quiet() -> Move {
    Move::normal(Square::E1, Square::E2)
}

// ---- off / degenerate inputs -------------------------------------

#[test]
fn off_profile_always_picks_first() {
    let noise = NoiseProfile::default();
    let root = any_root();
    let lines = vec![line(50), line(40), line(30), line(20)];
    for ply in 0..20 {
        assert_eq!(pick(&noise, 0xCAFE, ply, &root, &lines), NoisePick::Line(0));
    }
}

#[test]
fn single_line_blunder_has_nothing_to_pick() {
    let noise = NoiseProfile {
        avg_move_rank: 4.0,
        blunder_chance: 1.0,
        ..Default::default()
    };
    let root = mat_root();
    let lines = vec![mat_line(10, vec![kf1()], None)];
    // Only one line → blunder pool (i >= 1) is empty → best.
    assert_eq!(pick(&noise, 0xCAFE, 1, &root, &lines), NoisePick::Line(0));
}

#[test]
fn empty_lines_picks_zero() {
    let noise = NoiseProfile::default();
    let root = any_root();
    let lines: Vec<SearchLine> = Vec::new();
    assert_eq!(pick(&noise, 0, 0, &root, &lines), NoisePick::Line(0));
}

#[test]
fn variety_floor_always_picks_first() {
    // avg_move_rank at the 1.0 floor → zero spread → always #1.
    let noise = NoiseProfile {
        avg_move_rank: 1.0,
        ..Default::default()
    };
    let root = any_root();
    let lines = vec![line(0), line(-10), line(-20)];
    for ply in 0..10 {
        assert_eq!(pick(&noise, 0xBEEF, ply, &root, &lines), NoisePick::Line(0));
    }
}

#[test]
fn variety_stays_within_available_lines() {
    // High centre, but only 4 lines: every pick must be a valid index.
    let noise = NoiseProfile {
        avg_move_rank: 8.0,
        ..Default::default()
    };
    let root = any_root();
    let lines = vec![line(20), line(15), line(10), line(-200)];
    for ply in 0..200 {
        match pick(&noise, 0xABCD, ply, &root, &lines) {
            NoisePick::Line(idx) => assert!(idx < lines.len(), "variety out of range: {idx}"),
            other => panic!("non-variety pick at ply {ply}: {other:?}"),
        }
    }
}

#[test]
fn variety_centres_near_the_dial() {
    // Centre 3 over 8 lines: the average played rank should sit roughly
    // around 3 (2-based tolerance), and it must visit more than one rank.
    let noise = NoiseProfile {
        avg_move_rank: 3.0,
        ..Default::default()
    };
    let root = any_root();
    let lines: Vec<_> = (0..8).map(|i| line(-i * 10)).collect();
    let mut seen = std::collections::HashSet::new();
    let mut sum_rank = 0usize;
    let n = 400;
    for ply in 0..n {
        match pick(&noise, 0xDEAD, ply, &root, &lines) {
            NoisePick::Line(idx) => {
                seen.insert(idx);
                sum_rank += idx + 1; // 1-based rank
            }
            other => panic!("non-variety pick: {other:?}"),
        }
    }
    assert!(seen.len() >= 3, "variety barely moved: {seen:?}");
    let avg = sum_rank as f32 / n as f32;
    assert!((2.0..=4.0).contains(&avg), "average rank {avg} off-centre from 3.0");
}

// ---- material classifier (pure) ----------------------------------

#[test]
fn standard_piece_values_are_the_chart() {
    use crate::types::PieceType::*;
    assert_eq!(standard_piece_value_cp(Pawn), 100);
    assert_eq!(standard_piece_value_cp(Knight), 300);
    assert_eq!(standard_piece_value_cp(Bishop), 300);
    assert_eq!(standard_piece_value_cp(Rook), 500);
    assert_eq!(standard_piece_value_cp(Queen), 900);
    assert_eq!(standard_piece_value_cp(King), 0);
}

#[test]
fn line_material_delta_reads_pv_outcome() {
    let root = mat_root();
    let stm = root.side_to_move();
    // Winning a queen.
    let win = mat_line(900, vec![qxd5()], None);
    assert_eq!(line_material_delta_cp(&root, &win, stm), 900);
    // Quiet move, no captures.
    let quiet = mat_line(0, vec![kf1()], None);
    assert_eq!(line_material_delta_cp(&root, &quiet, stm), 0);
    // Hang a pawn: e4 then …Qxe4.
    let hang = mat_line(-150, hang_pawn_pv(), Some(1));
    assert_eq!(line_material_delta_cp(&root, &hang, stm), -100);
    // Empty PV is materially neutral.
    let empty = mat_line(0, vec![], None);
    assert_eq!(line_material_delta_cp(&root, &empty, stm), 0);
}

#[test]
fn two_ply_material_separates_grab_from_combination() {
    // Obvious grab: an immediate hanging-queen capture is up +900 at 2 plies
    // (the single-move PV stops at ply 0).
    let mr = mat_root();
    let grab = mat_line(900, vec![qxd5()], None);
    assert_eq!(two_ply_material_cp(&mr, &grab, mr.side_to_move()), 900);

    // Combination (quiet first move): a knight fork is materially flat after
    // two plies (Nf6+, …Kf8 — no captures yet) though it settles +900.
    let fr = fork_root();
    let fork = mat_line(900, fork_win_pv(), Some(2));
    assert_eq!(two_ply_material_cp(&fr, &fork, fr.side_to_move()), 0);
    assert_eq!(line_material_delta_cp(&fr, &fork, fr.side_to_move()), 900);

    // Combination (sacrifice): capture-first but down −200 after the
    // recapture, even though the settled line wins +300.
    let sr = sac_root();
    let sac = mat_line(300, sac_win_pv(), Some(2));
    assert_eq!(two_ply_material_cp(&sr, &sac, sr.side_to_move()), -200);
    assert_eq!(line_material_delta_cp(&sr, &sac, sr.side_to_move()), 300);
}

#[test]
fn material_blunder_pool_selects_in_band_only() {
    // deltas[0] is best (never a blunder). Losses: -, 0, 100, 300, 900.
    // In band [100,400]: indices 2 (100) and 3 (300); 900 is too big.
    let deltas = [0, 0, -100, -300, -900];
    assert_eq!(material_blunder_pool(&deltas, 100, 400), vec![2, 3]);
}

#[test]
fn material_blunder_pool_empty_when_no_in_band_loss() {
    // Only a queen-hang available, band tops out at a minor → empty.
    assert!(material_blunder_pool(&[0, -900], 100, 300).is_empty());
    // Winning / neutral lines are never blunder candidates.
    assert!(material_blunder_pool(&[0, 200, 0], 100, 400).is_empty());
}

// ---- blunder branch (material) -----------------------------------

#[test]
fn blunder_picks_an_in_band_material_loss() {
    let noise = NoiseProfile {
        blunder_chance: 1.0,
        blunder_min_material_cp: 100,
        blunder_max_material_cp: 400,
        guaranteed_mate_in: 0,
        ..Default::default()
    };
    let root = mat_root();
    // #1 best quiet (neutral), #2 hangs a pawn (-100, in band).
    let lines = vec![mat_line(0, vec![kf1()], None), mat_line(-150, hang_pawn_pv(), Some(1))];
    for ply in 0..30 {
        assert_eq!(
            pick(&noise, 0xABCD, ply, &root, &lines),
            NoisePick::Blunder(1),
            "should hang the in-band pawn",
        );
    }
}

#[test]
fn blunder_does_not_fire_when_only_hang_is_out_of_band() {
    let noise = NoiseProfile {
        blunder_chance: 1.0,
        blunder_min_material_cp: 100,
        blunder_max_material_cp: 300, // a pawn-hang would be in band, but…
        guaranteed_mate_in: 0,
        ..Default::default()
    };
    let root = mat_root();
    // #1 quiet (Kd1); #2 walks the king to f1 and lets …Qxd4 take the
    // whole queen (-900, above the 300 cap) — the only non-best line.
    // Gated-on-existence: no in-band hang → no blunder → plays best.
    let lines = vec![
        mat_line(0, vec![Move::normal(Square::E1, Square::D1)], None),
        mat_line(
            -800,
            vec![Move::normal(Square::E1, Square::F1), Move::normal(Square::D5, Square::D4)],
            Some(1),
        ),
    ];
    for ply in 0..10 {
        assert_eq!(pick(&noise, 0xCAFE, ply, &root, &lines), NoisePick::Line(0));
    }
}

#[test]
fn blunder_suppressed_when_mate_guarded() {
    let noise = NoiseProfile {
        blunder_chance: 1.0,
        blunder_min_material_cp: 100,
        blunder_max_material_cp: 900,
        guaranteed_mate_in: 3,
        ..Default::default()
    };
    let root = mat_root();
    let mate_in_2 = Value::MATE.0 - 3;
    let lines = vec![
        mat_line(mate_in_2, vec![kf1()], None),
        mat_line(-150, hang_pawn_pv(), Some(1)),
    ];
    for ply in 0..20 {
        assert_eq!(pick(&noise, 0xFACE, ply, &root, &lines), NoisePick::Line(0));
    }
}

#[test]
fn blunder_allowed_for_mate_beyond_guarantee() {
    let noise = NoiseProfile {
        blunder_chance: 1.0,
        blunder_min_material_cp: 100,
        blunder_max_material_cp: 900,
        guaranteed_mate_in: 3,
        ..Default::default()
    };
    let root = mat_root();
    let mate_in_5 = Value::MATE.0 - 9;
    let lines = vec![
        mat_line(mate_in_5, vec![kf1()], None),
        mat_line(-150, hang_pawn_pv(), Some(1)),
    ];
    let mut saw_blunder = false;
    for ply in 0..20 {
        if matches!(pick(&noise, 0xFACE, ply, &root, &lines), NoisePick::Blunder(_)) {
            saw_blunder = true;
            break;
        }
    }
    assert!(saw_blunder, "blunder branch never fired against mate-in-5");
}

#[test]
fn variety_suppressed_when_mate_guarded() {
    // Regression for the reported bug: an `avg_move_rank > 1` bot demoted
    // itself off a mate-in-1 even with `guaranteed_mate_in = 1`, because
    // the variety branch wasn't checked against the mate guard. A mate
    // within the guarantee must always play #0, every ply.
    let noise = NoiseProfile {
        avg_move_rank: 4.0,
        guaranteed_mate_in: 1,
        ..Default::default()
    };
    let root = mat_root();
    let mate_in_1 = Value::MATE.0 - 1;
    let lines = vec![line(mate_in_1), line(50), line(40), line(30), line(20)];
    for ply in 0..30 {
        assert_eq!(pick(&noise, 0xBEEF, ply, &root, &lines), NoisePick::Line(0));
    }
}

#[test]
fn variety_allowed_for_mate_beyond_guarantee() {
    // A mate deeper than the guarantee is NOT protected — the bot "can't
    // see that far," so the variety branch may still demote off it.
    let noise = NoiseProfile {
        avg_move_rank: 4.0,
        guaranteed_mate_in: 1,
        ..Default::default()
    };
    let root = mat_root();
    let mate_in_5 = Value::MATE.0 - 9;
    let lines = vec![line(mate_in_5), line(50), line(40), line(30), line(20)];
    let mut saw_demotion = false;
    for ply in 0..30 {
        if !matches!(pick(&noise, 0xBEEF, ply, &root, &lines), NoisePick::Line(0)) {
            saw_demotion = true;
            break;
        }
    }
    assert!(saw_demotion, "variety never demoted off an unprotected mate-in-5");
}

// ---- material easing (capture rescue) ----------------------------

/// Fraction of plies on which `pick` returns the engine's best line.
fn grab_rate(noise: &NoiseProfile, root: &Position, lines: &[SearchLine], plies: u64) -> f64 {
    let hits = (0..plies)
        .filter(|&p| matches!(pick(noise, 0xABCD, p, root, lines), NoisePick::Line(0)))
        .count();
    hits as f64 / plies as f64
}

#[test]
fn capture_rescue_grabs_hanging_material() {
    // #0 grabs the hanging black queen (Qxd5); #1 is a quiet king move that
    // throws it away. Even a rank-4 bot — which without the easing almost
    // never plays #0 — should still grab the queen a meaningful share of the
    // time, whereas a *non-capture* best move gets no such rescue.
    let root = mat_root();
    let r4 = NoiseProfile { avg_move_rank: 4.0, ..Default::default() };
    let capture = vec![
        mat_line(2000, vec![qxd5()], Some(0)),
        mat_line(100, vec![kf1()], Some(0)),
    ];
    let quiet = vec![
        mat_line(2000, vec![kf1()], Some(0)),
        mat_line(100, vec![Move::normal(Square::E3, Square::E4)], Some(0)),
    ];
    let cap = grab_rate(&r4, &root, &capture, 2000);
    let non = grab_rate(&r4, &root, &quiet, 2000);
    assert!(cap > 0.15, "rank-4 bot should still grab a hanging queen often, got {cap}");
    assert!(non < 0.10, "a non-capture best move gets no rescue, got {non}");
    assert!(cap > non + 0.10, "capture {cap} should beat non-capture {non} clearly");
}

#[test]
fn queen_always_grabbed_at_rank_two() {
    // Anchor: a hanging queen at rank 2.0 is taken essentially always
    // (P caps at 1 for V=9 / rank 2), so even a rank-2 bot doesn't leave a
    // free queen sitting — the believability floor.
    let root = mat_root();
    let r2 = NoiseProfile { avg_move_rank: 2.0, ..Default::default() };
    let lines = vec![
        mat_line(2000, vec![qxd5()], Some(0)),
        mat_line(100, vec![kf1()], Some(0)),
    ];
    let rate = grab_rate(&r2, &root, &lines, 2000);
    assert!(rate > 0.9, "queen at rank 2 should be grabbed ~always, got {rate}");
}

#[test]
fn capture_rescue_falls_with_rank() {
    // The weaker (higher-rank) bot misses the free queen more often.
    let root = mat_root();
    let lines = vec![
        mat_line(2000, vec![qxd5()], Some(0)),
        mat_line(100, vec![kf1()], Some(0)),
    ];
    let r15 = NoiseProfile { avg_move_rank: 1.5, ..Default::default() };
    let r4 = NoiseProfile { avg_move_rank: 4.0, ..Default::default() };
    let rate15 = grab_rate(&r15, &root, &lines, 2000);
    let rate4 = grab_rate(&r4, &root, &lines, 2000);
    assert!(
        rate15 > rate4 + 0.10,
        "lower rank should grab the queen more: r1.5={rate15}, r4={rate4}"
    );
}

// ---- miss branch -------------------------------------------------

#[test]
fn miss_declines_a_combination_winning_best_move() {
    let noise = NoiseProfile {
        miss_chance: 1.0,
        guaranteed_mate_in: 0,
        ..Default::default()
    };
    let root = fork_root();
    // #1 wins the queen by a knight fork (+900) whose first move is the quiet
    // Nf6+ — a combination you had to see; #2 is a quiet, non-winning move.
    let lines = vec![mat_line(900, fork_win_pv(), Some(2)), mat_line(0, vec![fork_quiet()], None)];
    for ply in 0..30 {
        assert_eq!(
            pick(&noise, 0x1234, ply, &root, &lines),
            NoisePick::Miss(1),
            "miss must decline the combination win and play the best non-winning line",
        );
    }
}

#[test]
fn miss_does_not_decline_an_immediate_capture() {
    // The gate: a winning move that just grabs a hanging piece (Qxd5) is an
    // *obvious* capture — handled by the variety branch's material easing, not
    // the miss branch. Miss only declines wins you had to calculate, so a
    // pure-capture best move is never turned into a Miss, even at 100% miss.
    let noise = NoiseProfile {
        miss_chance: 1.0,
        guaranteed_mate_in: 0,
        ..Default::default()
    };
    let root = mat_root();
    let lines = vec![mat_line(900, vec![qxd5()], None), mat_line(0, vec![kf1()], None)];
    for ply in 0..30 {
        assert!(
            !matches!(pick(&noise, 0x1234, ply, &root, &lines), NoisePick::Miss(_)),
            "an immediate capture must not be declined by the miss branch",
        );
    }
}

#[test]
fn miss_declines_a_capture_first_sacrifice() {
    // The 2-ply refinement: a win whose first move *is* a capture but which
    // is **down material after two plies** (Nxe5, …dxe5) is a sacrificial
    // combination, not an obvious grab — the simple `!first_move_is_capture`
    // gate would have wrongly exempted it. It must stay missable.
    let noise = NoiseProfile {
        miss_chance: 1.0,
        guaranteed_mate_in: 0,
        ..Default::default()
    };
    let root = sac_root();
    // #1 is the +3 sacrifice (settled), #2 a quiet, non-winning move.
    let lines = vec![mat_line(300, sac_win_pv(), Some(2)), mat_line(0, vec![sac_quiet()], None)];
    for ply in 0..30 {
        assert_eq!(
            pick(&noise, 0x1234, ply, &root, &lines),
            NoisePick::Miss(1),
            "a capture-first sacrifice is a combination and must stay missable",
        );
    }
}

#[test]
fn miss_inert_when_best_move_wins_no_material() {
    let noise = NoiseProfile {
        miss_chance: 1.0,
        guaranteed_mate_in: 0,
        ..Default::default()
    };
    let root = mat_root();
    // Best move is quiet (no material on offer) → nothing to miss.
    let lines = vec![mat_line(0, vec![kf1()], None), mat_line(-150, hang_pawn_pv(), Some(1))];
    for ply in 0..20 {
        assert_eq!(pick(&noise, 0x1234, ply, &root, &lines), NoisePick::Line(0));
    }
}

#[test]
fn miss_suppressed_when_mate_guarded() {
    let noise = NoiseProfile {
        miss_chance: 1.0,
        guaranteed_mate_in: 3,
        ..Default::default()
    };
    let root = mat_root();
    let mate_in_2 = Value::MATE.0 - 3;
    let lines = vec![mat_line(mate_in_2, vec![qxd5()], None), mat_line(0, vec![kf1()], None)];
    for ply in 0..20 {
        assert_eq!(pick(&noise, 0xFACE, ply, &root, &lines), NoisePick::Line(0));
    }
}

// ---- precedence + determinism ------------------------------------

#[test]
fn miss_takes_precedence_over_blunder() {
    // Both rolls would fire; miss is evaluated first.
    let noise = NoiseProfile {
        miss_chance: 1.0,
        blunder_chance: 1.0,
        blunder_min_material_cp: 100,
        blunder_max_material_cp: 900,
        guaranteed_mate_in: 0,
        ..Default::default()
    };
    let root = fork_root();
    // #1 wins the queen by a (non-capture) fork — miss-eligible; #2 hangs a
    // pawn (in blunder band). Both rolls would fire; miss is evaluated first.
    let lines = vec![mat_line(900, fork_win_pv(), Some(2)), mat_line(-150, fork_hang_pawn_pv(), Some(1))];
    for ply in 0..20 {
        assert_eq!(pick(&noise, 0xBEEF, ply, &root, &lines), NoisePick::Miss(1));
    }
}


#[test]
fn pick_is_deterministic_for_same_inputs() {
    let noise = NoiseProfile {
        avg_move_rank: 4.0,
        blunder_chance: 0.3,
        miss_chance: 0.3,
        blunder_min_material_cp: 80,
        blunder_max_material_cp: 900,
        guaranteed_mate_in: 1,
    };
    let root = mat_root();
    let lines = vec![mat_line(900, vec![qxd5()], None), mat_line(-150, hang_pawn_pv(), Some(1))];
    for ply in 0..20 {
        let a = pick(&noise, 0xABCD, ply, &root, &lines);
        let b = pick(&noise, 0xABCD, ply, &root, &lines);
        assert_eq!(a, b, "same inputs gave different picks at ply {ply}");
    }
}

#[test]
fn pick_varies_with_seed() {
    let noise = NoiseProfile {
        avg_move_rank: 4.0,
        ..Default::default()
    };
    let root = any_root();
    let lines = vec![line(0), line(-20), line(-40), line(-80)];
    let seq_a: Vec<_> = (0..50).map(|p| pick(&noise, 0x1111_2222, p, &root, &lines)).collect();
    let seq_b: Vec<_> = (0..50).map(|p| pick(&noise, 0xAAAA_BBBB, p, &root, &lines)).collect();
    assert_ne!(seq_a, seq_b, "seed didn't affect the pick sequence");
}
