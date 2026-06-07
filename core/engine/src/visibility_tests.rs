use super::*;

use crate::san;

fn parse(fen: &str) -> Position {
    Position::from_fen(fen).expect("test FEN must parse")
}

fn mv(pos: &Position, s: &str) -> Move {
    let mut scratch = pos.clone();
    san::parse(&mut scratch, s).expect("test SAN must be legal")
}

fn v(pos: &Position, s: &str, last_move_to: Option<Square>) -> f64 {
    let ctx = VisibilityContext::at_node(pos, last_move_to);
    visibility(pos, mv(pos, s), &ctx)
}

const EPS: f64 = 1e-9;

// ---- archetype positions (the worked examples in PLAN-perception.md) ----

#[test]
fn normal_forward_quiet_move_has_no_difficulty() {
    // 1. e4 from the start position: quiet, forward, pawn, no occlusion,
    // no attention context — V must be exactly 1.0.
    let pos = Position::startpos();
    assert!((v(&pos, "e4", None) - 1.0).abs() < EPS);
}

#[test]
fn adjacent_recapture_is_fully_visible() {
    // After 1.e4 d5 (locus d5): exd5 captures ON the last-move square
    // from one square away. Forward pawn capture, no occlusion, both
    // endpoints within 2 of the locus -> V = 1.0: never declined.
    let pos = parse("rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq d6 0 2");
    let d5 = Square::from_algebraic("d5").unwrap();
    assert!((v(&pos, "exd5", Some(d5)) - 1.0).abs() < EPS);
}

#[test]
fn distant_recapture_pays_attention_and_threading_costs() {
    // After 1.e4 d5 2.exd5 (locus d5): ...Qxd5 recaptures, but (a) the
    // queen starts 3 squares from the locus -> one A_NEAR endpoint
    // factor, and (b) the d8->d5 slide threads past the home-rank
    // clutter (c7/e7 pawns + c8/e8 pieces flank the d6/d7 interior) ->
    // heavy threading. The two-endpoint design intentionally prices
    // "you saw the capture square but must notice the distant piece
    // reaches it"; the home-nest threading hit is a FEEL-TEST flag —
    // if real play shows queen sorties over-missed, narrow the
    // threading neighbourhood.
    let pos = parse("rnbqkbnr/ppp1pppp/8/3P4/8/8/PPPP1PPP/RNBQKB1R b KQkq - 0 2");
    let d5 = Square::from_algebraic("d5").unwrap();
    let got = v(&pos, "Qxd5", Some(d5));
    assert!((got - A_NEAR * O_THREAD_HEAVY).abs() < EPS, "got {got}");
}

#[test]
fn backward_queen_move_carries_the_direction_penalty() {
    // The Qe1-classroom shape: a backward quiet queen move. White queen
    // on e7 retreating to e1 (kings parked far away, nothing else
    // applies): V = D_BACKWARD exactly.
    let pos = parse("k7/4Q3/8/8/8/8/8/K7 w - - 0 1");
    let got = v(&pos, "Qe1", None);
    assert!((got - D_BACKWARD).abs() < EPS, "got {got}");
}

#[test]
fn sideways_move_carries_the_sideways_penalty() {
    let pos = parse("k7/4Q3/8/8/8/8/8/K7 w - - 0 1");
    let got = v(&pos, "Qa7+", None); // e7 -> a7, same rank
    assert!((got - D_SIDEWAYS).abs() < EPS, "got {got}");
}

#[test]
fn knight_moves_stack_with_direction() {
    // Knight on e5 (white): Nf7 is forward (knight factor only);
    // Nf3 is backward (knight x backward).
    let pos = parse("k7/8/8/4N3/8/8/8/K7 w - - 0 1");
    let fwd = v(&pos, "Nf7", None);
    let back = v(&pos, "Nf3", None);
    assert!((fwd - K_KNIGHT).abs() < EPS, "forward knight: {fwd}");
    assert!((back - K_KNIGHT * D_BACKWARD).abs() < EPS, "backward knight: {back}");
}

#[test]
fn en_passant_and_castling_carry_rule_penalties() {
    // En passant: after ...d7d5 next to a white e5 pawn.
    let pos = parse("k7/8/8/3pP3/8/8/8/K7 w - d6 0 2");
    let ep = v(&pos, "exd6", None);
    // exd6 e.p. is forward; only the salience penalty applies.
    assert!((ep - S_EN_PASSANT).abs() < EPS, "ep: {ep}");

    // Castling kingside from a legal castling position.
    let pos = parse("k7/8/8/8/8/8/8/4K2R w K - 0 1");
    let castle = v(&pos, "O-O", None);
    // King e1->g1 is sideways; castling salience x sideways direction.
    assert!(
        (castle - S_CASTLING * D_SIDEWAYS).abs() < EPS,
        "castle: {castle}"
    );
}

#[test]
fn underpromotion_is_nearly_invisible_queen_promotion_is_not() {
    let pos = parse("8/4P3/8/8/8/2k5/8/4K3 w - - 0 1");
    let queen = v(&pos, "e8=Q", None);
    let knight = v(&pos, "e8=N", None);
    assert!((queen - 1.0).abs() < EPS, "queen promo: {queen}");
    assert!((knight - S_UNDERPROMOTION).abs() < EPS, "underpromo: {knight}");
}

#[test]
fn discovered_attack_vehicle_is_penalized() {
    // White bishop a1 aims at the black king on h8 through a white
    // knight on d4. Moving the knight OFF the ray (Nb5) unveils the
    // attack -> vehicle penalty; moving ALONG the diagonal would not,
    // but a knight never stays on a ray, so contrast with a non-blocker
    // knight move in a twin position without the bishop.
    let pos = parse("7k/8/8/8/3N4/8/8/B6K w - - 0 1");
    let vehicle = v(&pos, "Nb5", None);
    assert!(
        (vehicle - K_KNIGHT * O_VEHICLE).abs() < EPS,
        "vehicle knight: {vehicle}"
    );

    let no_bishop = parse("7k/8/8/8/3N4/8/8/7K w - - 0 1");
    let plain = v(&no_bishop, "Nb5", None);
    assert!((plain - K_KNIGHT).abs() < EPS, "plain knight: {plain}");
}

#[test]
fn threaded_slider_path_is_penalized() {
    // White bishop a1 -> f6: the a1-f6 diagonal (interior b2,c3,d4,e5)
    // stays empty, but four pawns sit immediately adjacent to it
    // (b3, d3, c4, e4) -> traffic >= 4 -> heavy threading penalty.
    let pos = parse("k7/8/8/8/2p1p3/1P1P4/8/B6K w - - 0 1");
    let got = v(&pos, "Bf6", None);
    assert!(
        (got - O_THREAD_HEAVY).abs() < EPS,
        "threaded bishop: {got}"
    );

    // Same move on an empty board: no traffic, no penalty.
    let empty = parse("k7/8/8/8/8/8/8/B6K w - - 0 1");
    let clear = v(&empty, "Bf6", None);
    assert!((clear - 1.0).abs() < EPS, "clear bishop: {clear}");
}

#[test]
fn attention_penalizes_both_far_endpoints_independently() {
    // White rook a1 -> a8 (forward, clear file). Locus at h1: the rook's
    // from-square a1 is 7 away (far) and to-square a8 is 7 away (far)
    // -> A_FAR twice. Locus at a2: from is adjacent (1.0), to is 6 away
    // -> A_FAR once.
    let pos = parse("k7/8/8/8/8/8/8/R3K3 w - - 0 1");
    let h1 = Square::from_algebraic("h1").unwrap();
    let a2 = Square::from_algebraic("a2").unwrap();
    let both_far = v(&pos, "Ra8+", Some(h1));
    let one_far = v(&pos, "Ra8+", Some(a2));
    assert!((both_far - A_FAR * A_FAR).abs() < EPS, "both far: {both_far}");
    assert!((one_far - A_FAR).abs() < EPS, "one far: {one_far}");
}

// ---- the margin curve ----

#[test]
fn curve_hits_the_plan_reference_points() {
    // The PLAN-perception.md reference table.
    let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
    // V = 1.0: always seen, any perception.
    assert!(close(p_see(1.0, 0.0), 1.0));
    assert!(close(p_see(1.0, 0.7), 1.0));
    // V = 0.4 at p = 0.7: margin .1 -> .8 + .2*(1/3) = .8667
    assert!(close(p_see(0.4, 0.7), 0.8 + 0.2 / 3.0));
    // V = 0.4 at p = 0.4: margin -.2 -> .8*(1 - .2/.45)^2 ~= .2469
    assert!(close(p_see(0.4, 0.4), 0.8 * (1.0 - 0.2 / 0.45_f64).powi(2)));
    // V = 0.4 at p = 0: margin -.6 below -CLIFF -> literal zero.
    assert!(close(p_see(0.4, 0.0), 0.0));
    // V = 0.55 at p = 0: margin -.45 == -CLIFF -> exactly zero.
    assert!(close(p_see(0.55, 0.0), 0.0));
    // Margin >= RAMP -> deterministic 1.0.
    assert!(close(p_see(0.9, 0.4), 1.0));
    // Plateau floor exactly at margin 0.
    assert!(close(p_see(0.6, 0.4), PLATEAU));
}

#[test]
fn curve_is_monotone_in_both_arguments() {
    let vs = [0.05, 0.2, 0.4, 0.55, 0.7, 0.85, 0.99];
    let ps = [0.0, 0.1, 0.3, 0.5, 0.7, 0.9, 0.99];
    for w in vs.windows(2) {
        for &p in &ps {
            assert!(p_see(w[0], p) <= p_see(w[1], p) + EPS);
        }
    }
    for &vv in &vs {
        for w in ps.windows(2) {
            assert!(p_see(vv, w[0]) <= p_see(vv, w[1]) + EPS);
        }
    }
}

// ---- the deterministic roll ----

#[test]
fn rolls_are_deterministic_and_distinct_per_move() {
    let m1 = Move::normal(
        Square::from_algebraic("e2").unwrap(),
        Square::from_algebraic("e4").unwrap(),
    );
    let m2 = Move::normal(
        Square::from_algebraic("d2").unwrap(),
        Square::from_algebraic("d4").unwrap(),
    );
    // Same inputs -> same outcome, always.
    for _ in 0..3 {
        assert_eq!(sees(42, 0xDEAD_BEEF, m1, 0.5), sees(42, 0xDEAD_BEEF, m1, 0.5));
    }
    // Across many (seed,key) pairs the two moves must sometimes
    // disagree (independent streams) and the hit rate must track p.
    let mut hits = 0;
    let mut disagree = 0;
    let n = 10_000;
    for i in 0..n {
        let seed = 0x5EED + i as u64;
        let key = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        if sees(seed, key, m1, 0.3) {
            hits += 1;
        }
        if sees(seed, key, m1, 0.3) != sees(seed, key, m2, 0.3) {
            disagree += 1;
        }
    }
    let rate = hits as f64 / n as f64;
    assert!((rate - 0.3).abs() < 0.02, "hit rate {rate} != 0.3");
    assert!(disagree > n / 10, "moves' streams look correlated");
}

#[test]
fn extreme_probabilities_skip_the_roll() {
    let m = Move::normal(
        Square::from_algebraic("e2").unwrap(),
        Square::from_algebraic("e4").unwrap(),
    );
    assert!(sees(1, 2, m, 1.0));
    assert!(!sees(1, 2, m, 0.0));
}

// ---- line findability ----

#[test]
fn forcing_line_stays_findable_quiet_subtle_line_compounds_down() {
    // Forcing checks line from the material_settled test: every link
    // is a normal forward/sideways-free move? Rd8+ is forward for
    // white; Rd7+ backward... use a simple forward-capture line
    // instead: 1.e4 d5 2.exd5 (all forward, all normal).
    let pos = Position::startpos();
    let line = [
        mv(&pos, "e4"),
        {
            let mut p2 = pos.clone();
            p2.do_move(mv(&pos, "e4"));
            san::parse(&mut p2.clone(), "d5").unwrap()
        },
    ];
    let f = line_findability(&pos, &line, Color::White, 1, 0.3);
    // White's only move in range (e4) is V = 1.0 -> findability 1.0
    // regardless of perception.
    assert!((f - 1.0).abs() < EPS, "got {f}");

    // A line whose mover-ply is a backward queen move compounds below 1.
    let qpos = parse("k7/4Q3/8/8/8/8/8/K7 w - - 0 1");
    let qline = [mv(&qpos, "Qe1")];
    let f = line_findability(&qpos, &qline, Color::White, 0, 0.3);
    let expect = p_see(D_BACKWARD, 0.3);
    assert!((f - expect).abs() < EPS, "got {f}, expect {expect}");
}
