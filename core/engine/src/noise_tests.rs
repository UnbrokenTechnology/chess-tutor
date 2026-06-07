use super::*;
use crate::engine::SearchLine;
use crate::position::Position;
use crate::types::{Move, Square, Value};

/// Stub line with the given score and an empty PV. Used by tests that
/// exercise branches which don't classify material (variety, mate-guard
/// suppression, determinism): with an empty PV the material delta is
/// zero, which is fine because those tests never expect the easing to
/// *fire*.
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

/// Position used by the material-classifier and easing tests.
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

/// The user-reported KP-vs-K wiggle position: Black (us) has a pawn on g2
/// one step from queening with the king on f3 guarding g1; White's king is
/// far away on b3. Black to move — `g2-g1=Q` is the obvious win the engine
/// ranks #1, but the rank lever used to demote off it for ten king-shuffle
/// moves before finally promoting (promotions weren't material-eased).
fn promo_root() -> Position {
    Position::from_fen("8/8/8/8/8/1K3k2/6p1/8 b - - 0 1").unwrap()
}
fn g1_queen() -> Move {
    Move::promotion(Square::G2, Square::G1, PieceType::Queen)
}
fn wiggle_kf2() -> Move {
    Move::normal(Square::F3, Square::F2)
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
fn single_line_always_picks_it() {
    let noise = NoiseProfile {
        avg_move_rank: 4.0,
        ..Default::default()
    };
    let root = mat_root();
    let lines = vec![mat_line(10, vec![kf1()], None)];
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
fn material_delta_counts_promotion() {
    let root = promo_root();
    let stm = root.side_to_move();
    // g1=Q upgrades a pawn to a queen: +800 (900 - 100), no capture.
    let promo = mat_line(2000, vec![g1_queen()], Some(0));
    assert_eq!(line_material_delta_cp(&root, &promo, stm), 800);
}

#[test]
fn material_delta_counts_capture_promotion() {
    // gxh8=Q captures a rook AND promotes: +500 (rook) + 800 (promo) = 1300.
    let root = Position::from_fen("7r/6P1/8/8/8/8/8/K6k w - - 0 1").unwrap();
    let stm = root.side_to_move();
    let cap_promo =
        mat_line(2000, vec![Move::promotion(Square::G7, Square::H8, PieceType::Queen)], Some(0));
    assert_eq!(line_material_delta_cp(&root, &cap_promo, stm), 1300);
}

// ---- mate guard ----------------------------------------------------

#[test]
fn variety_suppressed_when_mate_guarded() {
    // Regression for the reported bug: an `avg_move_rank > 1` bot demoted
    // itself off a mate-in-1 even with `guaranteed_mate_in = 1`, because
    // the variety branch wasn't checked against the mate guard. A mate
    // within the guarantee must always play #0, every ply.
    let noise = NoiseProfile {
        avg_move_rank: 4.0,
        guaranteed_mate_in: 1,
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

// ---- self-hang guard ---------------------------------------------

/// Position 2 from the t600 vs Martin game: Black Qb1; White Qd8/Kd5/Ph5.
/// `Qg6` (b1-g6) drops the queen to `hxg6`; `Qd1+` is the safe (winning) move.
/// The filter reads the loss off the line's PV, so the hang must carry the
/// recapture (`[Qg6, hxg6]`) to be detected — a bare `[Qg6]` models a bot
/// that never saw `hxg6` (qsearch-0 / perception-pruned) and is NOT filtered.
fn queen_hang_root() -> Position {
    Position::from_fen("3Q4/5k2/8/3K3P/8/8/8/1q6 b - - 0 1").unwrap()
}
fn qg6_hang() -> Move {
    Move::normal(Square::B1, Square::G6)
}
fn hxg6_recapture() -> Move {
    Move::normal(Square::H5, Square::G6)
}
fn qd1_safe() -> Move {
    Move::normal(Square::B1, Square::D1)
}

/// Black Ka8, Rd8; White Kh1, Pe4. `Rd5` drops the rook to `exd5`; `Rd7` is
/// safe. Used to check the guard *scales* with value (a rook isn't always
/// saved, unlike a queen).
fn rook_hang_root() -> Position {
    Position::from_fen("k2r4/8/8/8/4P3/8/8/7K b - - 0 1").unwrap()
}
fn rd5_hang() -> Move {
    Move::normal(Square::D8, Square::D5)
}
fn exd5_recapture() -> Move {
    Move::normal(Square::E4, Square::D5)
}
fn rd7_safe() -> Move {
    Move::normal(Square::D8, Square::D7)
}

/// Black Kg8, Qb4 (already attacked by the white a3 pawn); White Ka1, Pa3.
/// The bug this whole change fixes: `Kg7` keeps the *moved* piece (the king)
/// safe but ABANDONS the queen to `axb4`. The old SEE check only looked at
/// the moved piece's landing square, so it sailed through; the PV-delta gate
/// catches it. `Qc5` retreats the queen to safety.
fn abandon_queen_root() -> Position {
    Position::from_fen("6k1/8/8/8/1q6/P7/8/K7 b - - 0 1").unwrap()
}
fn kg7_abandons() -> Move {
    Move::normal(Square::G8, Square::G7)
}
fn axb4_grabs() -> Move {
    Move::normal(Square::A3, Square::B4)
}
fn qc5_saves() -> Move {
    Move::normal(Square::B4, Square::C5)
}

#[test]
fn self_hang_saves_a_perceived_queen_drop() {
    // The hang's PV carries the recapture (`[Qg6, hxg6]`), so the line settles
    // a full queen down — the bot *saw* the loss. A rank-4 bot samples the
    // demoted Qg6 almost always; the queen anchor (P=1 through rank 4) must
    // reject it every ply and fall back to the safe line.
    let root = queen_hang_root();
    let noise = NoiseProfile { avg_move_rank: 4.0, ..Default::default() };
    let lines = vec![
        mat_line(2000, vec![qd1_safe()], None),
        mat_line(100, vec![qg6_hang(), hxg6_recapture()], Some(1)),
    ];
    for ply in 0..300 {
        assert_eq!(
            pick(&noise, 0xABCD, ply, &root, &lines),
            NoisePick::Line(0),
            "must never knowingly drop the queen (recapture is in the PV)",
        );
    }
}

#[test]
fn self_hang_ignores_an_unperceived_hang() {
    // Same Qg6 move, but the PV is bare (`[Qg6]`) — the bot never saw `hxg6`
    // (qsearch-0, or perception pruned the recapture). The line reads as
    // materially neutral, so the filter does NOT fire and the bot commits the
    // realistic, geometry-shaped blunder. This is the behavior the perception
    // era wants and the old full-strength SEE check destroyed.
    let root = queen_hang_root();
    let noise = NoiseProfile { avg_move_rank: 4.0, ..Default::default() };
    let lines = vec![
        mat_line(2000, vec![qd1_safe()], None),
        mat_line(100, vec![qg6_hang()], None),
    ];
    let hung = (0..400)
        .filter(|&p| matches!(pick(&noise, 0xABCD, p, &root, &lines), NoisePick::Line(1)))
        .count();
    assert!(hung > 200, "an unperceived hang must be played often, got {hung}/400");
}

#[test]
fn self_hang_filters_an_abandoned_piece() {
    // The headline fix: the moved piece (the king) is perfectly safe, but the
    // line abandons the queen to `axb4` — and that capture is in the PV. The
    // old SEE check looked only at the king's landing square and missed it;
    // the PV-delta gate saves the queen (P=1 through rank 4).
    let root = abandon_queen_root();
    let noise = NoiseProfile { avg_move_rank: 4.0, ..Default::default() };
    let lines = vec![
        mat_line(2000, vec![qc5_saves()], None),
        mat_line(100, vec![kg7_abandons(), axb4_grabs()], Some(1)),
    ];
    for ply in 0..300 {
        assert_eq!(
            pick(&noise, 0xABCD, ply, &root, &lines),
            NoisePick::Line(0),
            "abandoning the queen must be filtered even though the king is safe",
        );
    }
}

#[test]
fn self_hang_scales_with_piece_value() {
    // A rook is saved ~5 / (3·3) ≈ 56% at rank 4 — often, but NOT always
    // (smaller material still hangs believably). Contrast the queen's
    // always-save above. The recapture (`exd5`) is in the PV so the loss is
    // perceived.
    let root = rook_hang_root();
    let noise = NoiseProfile { avg_move_rank: 4.0, ..Default::default() };
    let lines = vec![
        mat_line(2000, vec![rd7_safe()], None),
        mat_line(100, vec![rd5_hang(), exd5_recapture()], Some(1)),
    ];
    let saved = (0..3000)
        .filter(|&p| matches!(pick(&noise, 0xBEEF, p, &root, &lines), NoisePick::Line(0)))
        .count() as f64
        / 3000.0;
    assert!((0.40..0.72).contains(&saved), "rook saved ~56%, got {saved}");
}

#[test]
fn promotion_is_always_grabbed() {
    // Regression for the reported KP-vs-K bug: a rank-4 bot shuffled its king
    // for ten moves instead of pushing g2-g1=Q. Unlike a capture (value-curve,
    // missable by weak bots), a material-gaining promotion is rescued with
    // P=1 — every bot queens every time, at any rank.
    let root = promo_root();
    let r4 = NoiseProfile { avg_move_rank: 4.0, ..Default::default() };
    let lines = vec![
        mat_line(2000, vec![g1_queen()], Some(0)),
        mat_line(100, vec![wiggle_kf2()], Some(0)),
    ];
    for ply in 0..200 {
        assert_eq!(
            pick(&r4, 0xABCD, ply, &root, &lines),
            NoisePick::Line(0),
            "a free promotion must always be played, never demoted to a king move",
        );
    }
}

// ---- determinism ---------------------------------------------------

#[test]
fn pick_is_deterministic_for_same_inputs() {
    let noise = NoiseProfile {
        avg_move_rank: 4.0,
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
