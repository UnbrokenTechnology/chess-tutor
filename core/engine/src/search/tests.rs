//! Unit tests for the `search` module (moved verbatim from the former
//! inline `#[cfg(test)] mod tests` block).

use super::*;
use crate::eval::{evaluate_with_trace, EvalTrace};
use crate::movepick::BUTTERFLY_HISTORY_BOUND;
use crate::position::{Position, StateInfo};
use crate::tt::TranspositionTable;
use crate::types::{Move, Square, Value};
use crate::engine::{SearchLine, SearchParams};
use crate::engine::Engine;

fn search_to_depth(pos: &mut Position, depth: u32) -> SearchLine {
    let mut engine = Engine::new(1);
    let params = SearchParams {
        max_depth: depth,
        ..Default::default()
    };
    let mut lines = engine.search(pos, params);
    assert!(!lines.is_empty(), "search returned no lines");
    lines.remove(0)
}

#[test]
fn search_returns_a_legal_root_move() {
    let mut pos = Position::startpos();
    let line = search_to_depth(&mut pos, 2);
    assert!(!line.pv.is_empty());
    let first = line.pv[0];
    let legal = crate::movegen::pseudo_legal_moves_vec(&pos);
    assert!(legal.contains(&first));
}

#[test]
fn search_finds_mate_in_one() {
    // Classic K+Q mate: white K f6, Q g6, black K h8. White plays
    // Qg7#. The queen is supported by the white king, so black can't
    // capture; g8 and h7 are both covered by the queen.
    let mut pos = Position::from_fen("7k/8/5KQ1/8/8/8/8/8 w - - 0 1").unwrap();
    let line = search_to_depth(&mut pos, 3);
    assert_eq!(line.pv[0], Move::normal(Square::G6, Square::G7));
    assert!(
        line.score.0 >= Value::MATE.0 - Value::MAX_PLY,
        "expected mate score, got {}",
        line.score.0
    );
}

#[test]
fn search_drives_home_kxk_endgame() {
    // White K + Q vs lone black king on the edge. With the KXK
    // evaluator in place, search should find *some* progress-making
    // move rather than shuffling. Specifically: the engine's score
    // should exceed plain queen value at even a modest depth,
    // because PushToEdges / PushClose add ~100–200 on top.
    let mut pos = Position::from_fen("7k/8/5KQ1/8/8/8/8/8 w - - 0 1").unwrap();
    let line = search_to_depth(&mut pos, 4);
    assert!(!line.pv.is_empty());
    assert!(
        line.score.0 > Value::QUEEN_MG.0,
        "KXK endgame should score above raw queen value; got {}",
        line.score.0
    );
}

#[test]
fn search_completes_depth_six_from_startpos() {
    // End-to-end smoke test: the full pruning stack must survive a
    // real opening position at a non-trivial depth. Doesn't assert
    // the best move (that's tuning-sensitive) — just that we get a
    // non-empty PV and a sane score.
    let mut pos = Position::startpos();
    let line = search_to_depth(&mut pos, 6);
    assert!(!line.pv.is_empty());
    assert!(
        line.score.0.abs() < Value::MATE.0 - Value::MAX_PLY,
        "opening eval should not be a mate score, got {}",
        line.score.0
    );
    assert_eq!(line.depth, 6);
}

#[test]
fn search_line_leaf_trace_matches_pv_leaf_static_eval() {
    // After the per-ply refactor, the leaf trace is
    // `ply_traces.last()`. It must still equal a fresh
    // `evaluate_with_trace` at the PV's final position.
    let mut pos = Position::startpos();
    let line = search_to_depth(&mut pos, 3);
    let mut replay = pos.clone();
    let mut states: Vec<StateInfo> = Vec::with_capacity(line.pv.len());
    for mv in &line.pv {
        states.push(replay.do_move(*mv));
    }
    let (_, trace) = evaluate_with_trace(&replay);
    assert_eq!(
        line.ply_traces.last().unwrap(),
        &trace,
        "leaf trace must match a fresh evaluate_with_trace at the PV end"
    );
}

#[test]
fn value_to_from_tt_roundtrip_preserves_non_mate_values() {
    let v = Value(42);
    assert_eq!(value_from_tt(value_to_tt(v, 5), 5), v);
}

#[test]
fn value_to_from_tt_handles_mate_values() {
    let v = Value::mate_in(3);
    assert_eq!(value_from_tt(value_to_tt(v, 3), 3), v);
}

#[test]
fn lmr_reduction_matches_sf11_at_sample_points() {
    // Sample points hand-computed from SF11's formula:
    // `r = Reductions[d] * Reductions[mn]; (r + 511) / 1024 + (!i && r > 1007)`
    // with `Reductions[i] = int(23.4 * ln(i))`.
    //   R[5]=37, R[8]=48, R[10]=53, R[20]=70
    // d=8, mc=5, improving=true:  r=48*37=1776,  (1776+511)/1024 = 2
    assert_eq!(lmr_reduction(8, 5, true), 2);
    // d=10, mc=10, improving=true: r=53*53=2809, (2809+511)/1024 = 3
    assert_eq!(lmr_reduction(10, 10, true), 3);
    // d=20, mc=20, improving=true: r=70*70=4900, (4900+511)/1024 = 5
    assert_eq!(lmr_reduction(20, 20, true), 5);
}

#[test]
fn lmr_reduction_grows_with_depth_and_count() {
    let r_small = lmr_reduction(4, 5, true);
    let r_big = lmr_reduction(10, 20, true);
    assert!(r_big >= r_small);
}

#[test]
fn lmr_reduction_increases_when_not_improving_above_r_gate() {
    // SF11's `!improving && r > 1007` bonus only kicks in once
    // `r = R[d]*R[mn] > 1007`. At (d=10, mc=20) that's
    // 53*70=3710 > 1007, so non-improving adds +1.
    let r_improving = lmr_reduction(10, 20, true);
    let r_not_improving = lmr_reduction(10, 20, false);
    assert_eq!(r_not_improving, r_improving + 1);
}

#[test]
fn history_bonus_respects_butterfly_bound() {
    for d in 1..=20 {
        let b = history_bonus(d);
        assert!((0..=BUTTERFLY_HISTORY_BOUND).contains(&b));
    }
}

#[test]
fn recursion_bails_at_max_ply_without_panicking() {
    // Regression: `pv_length` was sized MAX_PLY and indexed at `ply`
    // before any bail check, and `negamax` had no ply-cap at all.
    // A check-rich position that fed check extensions past MAX_PLY
    // recursion levels crashed with "index out of bounds".
    let tt = TranspositionTable::new(1);
    let mut worker = crate::engine::WorkerState::new();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut search = Search::new(&tt, &mut worker, stop);
    let mut pos = Position::startpos();

    // Both entry points must survive being called at the cap.
    let _ = search.qsearch(&mut pos, -Value::INFINITE, Value::INFINITE, MAX_PLY, 0);
    let _ = search.negamax(
        &mut pos,
        -Value::INFINITE,
        Value::INFINITE,
        1,
        MAX_PLY,
        false,
        false,
        None,
        false,
    );
    // Parent read path: child at MAX_PLY must leave pv_length[MAX_PLY]
    // = 0 so a parent calling update_pv sees an empty child PV.
    assert_eq!(search.pv_length[MAX_PLY], 0);
}

#[test]
fn is_repetition_detects_matches_against_seeded_path_keys() {
    // Direct test of the detection logic: the repetition check
    // compares the current position's key against entries in
    // `path_keys` within the `halfmove_clock` window (positions
    // before the last pawn move / capture can't physically
    // repeat). Seeding `path_keys` with real game history (as
    // `SearchParams::game_history` will do) must make in-tree
    // positions that match that history fire as draws.
    //
    // Using a FEN with halfmove_clock=4 so the scan window
    // actually covers the 2-entry gap between seeded repetitions
    // below. The bit-layout is identical to startpos (the key
    // matches) but the clock honestly reflects "four reversible
    // plies have preceded this position."
    let tt = TranspositionTable::new(1);
    let mut worker = crate::engine::WorkerState::new();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut search = Search::new(&tt, &mut worker, stop);
    let pos =
        Position::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 4 3").unwrap();

    // Earlier key unrelated to `pos` → not a repetition.
    search.path_keys.clear();
    search.path_keys.push(0xDEAD_BEEF);
    search.path_keys.push(pos.key());
    assert!(!search.is_repetition(&pos));

    // Earlier key equal to `pos.key()` → repetition.
    search.path_keys.clear();
    search.path_keys.push(pos.key());
    search.path_keys.push(0xABCD);
    search.path_keys.push(pos.key());
    assert!(search.is_repetition(&pos));
}

#[test]
fn search_scores_known_repetition_as_draw() {
    // End-to-end: construct a game history where the current
    // position appears twice already (1st in game start, 2nd as the
    // search root). Any move that returns the position to an
    // earlier key is a 3rd occurrence — strictly a draw. With the
    // history seeded, moves in the engine's PV that would cycle
    // back must score as 0 cp.
    //
    // Concrete setup: at the startpos, play Nf3 Nf6 Ng1 Ng8 — four
    // moves that return both sides to the initial position. Feed
    // the engine the keys of every intermediate position as
    // `game_history`, then search. Replaying the knight cycle
    // would detect each of those keys mid-tree and return DRAW.
    // The engine must prefer a non-cycling move (e.g. d4 / e4 /
    // c4), so the score is strictly positive (tempo + whatever the
    // engine normally finds for white).
    let mut pos = Position::startpos();
    let k0 = pos.key();
    pos.do_move(Move::normal(Square::G1, Square::F3));
    let k1 = pos.key();
    pos.do_move(Move::normal(Square::G8, Square::F6));
    let k2 = pos.key();
    pos.do_move(Move::normal(Square::F3, Square::G1));
    let k3 = pos.key();
    pos.do_move(Move::normal(Square::F6, Square::G8));
    // After the cycle we're back at the startpos bit-layout, but
    // `halfmove_clock == 4` now — exactly what the bounded
    // repetition scan needs to see the seeded history. (Undoing
    // back to startpos here would reset hmc to 0 and the scan
    // would never look far enough back to find the repeats.)
    assert_eq!(pos.key(), k0);
    assert_eq!(pos.halfmove_clock(), 4);

    let game_history = vec![k0, k1, k2, k3];

    let mut engine = Engine::new(1);
    let lines = engine.search(
        &mut pos,
        SearchParams {
            max_depth: 4,
            game_history,
            ..Default::default()
        },
    );
    let line = lines.into_iter().next().expect("search returned no lines");
    // Top move must not be Nf3 — that immediately lands on `k1`
    // which is in game history (→ draw by repetition).
    assert_ne!(
        line.pv[0],
        Move::normal(Square::G1, Square::F3),
        "engine should avoid Nf3 when game history makes it a repetition draw"
    );
    // And the score should be positive — the engine found a non-
    // drawing continuation.
    assert!(
        line.score.0 > 0,
        "expected a positive score with a non-repeating continuation, got {}",
        line.score.0
    );
}

// ---- MultiPV ----------------------------------------------------

fn multi_pv_search(pos: &mut Position, depth: u32, multi_pv: usize) -> Vec<SearchLine> {
    let mut engine = Engine::new(1);
    engine.search(
        pos,
        SearchParams {
            max_depth: depth,
            multi_pv,
            ..Default::default()
        },
    )
}

#[test]
fn multi_pv_returns_requested_number_of_lines_from_startpos() {
    // 20 legal moves at the start; asking for 3 must return 3.
    let mut pos = Position::startpos();
    let lines = multi_pv_search(&mut pos, 4, 3);
    assert_eq!(lines.len(), 3);
}

#[test]
fn multi_pv_lines_are_sorted_by_score_descending() {
    let mut pos = Position::startpos();
    let lines = multi_pv_search(&mut pos, 4, 5);
    assert_eq!(lines.len(), 5);
    for pair in lines.windows(2) {
        assert!(
            pair[0].score >= pair[1].score,
            "MultiPV must be sorted desc: {:?} then {:?}",
            pair[0].score,
            pair[1].score
        );
    }
}

#[test]
fn multi_pv_first_moves_are_distinct() {
    // Every PV slot is claimed by a distinct root move — no slot
    // ever duplicates another's first move.
    let mut pos = Position::startpos();
    let lines = multi_pv_search(&mut pos, 4, 5);
    let firsts: Vec<Move> = lines.iter().map(|l| l.pv[0]).collect();
    for i in 0..firsts.len() {
        for j in (i + 1)..firsts.len() {
            assert_ne!(
                firsts[i],
                firsts[j],
                "PVs #{} and #{} share first move {:?}",
                i + 1,
                j + 1,
                firsts[i]
            );
        }
    }
}

#[test]
fn multi_pv_clamps_to_legal_move_count() {
    // King + king + queen endgame — very few legal moves for black.
    // White played Qg7+, black's king on h8 is in check. Let's use
    // a position where there are just 2 legal replies but we ask
    // for 10.
    let mut pos = Position::from_fen("7k/6Q1/5K2/8/8/8/8/8 b - - 0 1").unwrap();
    // Black's king can step to g8 (attacked by K) — actually let's
    // not overthink: use a slightly-more-constrained position.
    let legal_count = crate::movegen::legal_moves_vec(&mut pos).len();
    let lines = multi_pv_search(&mut pos, 3, 10);
    assert_eq!(
        lines.len(),
        legal_count,
        "MultiPV should clamp to legal-move count ({} legal moves)",
        legal_count
    );
}

#[test]
fn multi_pv_returns_empty_on_terminal_position() {
    // Fool's-mate-style position: black king checkmated, it's
    // black to move, no legal moves. Return empty.
    //
    // Position: white queen on g7 (protected by Kg6), black king h8.
    // Actually simpler: known checkmate FEN.
    let mut pos = Position::from_fen("7k/5KQ1/8/8/8/8/8/8 b - - 0 1").unwrap();
    let legal = crate::movegen::legal_moves_vec(&mut pos);
    assert!(
        legal.is_empty(),
        "precondition: test FEN must be a terminal position"
    );
    let lines = multi_pv_search(&mut pos, 3, 5);
    assert!(lines.is_empty(), "terminal position should yield 0 PVs");
}

#[test]
fn multi_pv_first_line_matches_single_pv_first_line() {
    // Whether the caller asked for 1 PV or 5, the leading line
    // should agree on the best move. Note: this property is
    // approximate at shallow depths because MultiPV's slot-1..N
    // work at earlier IDS depths leaves extra TT entries that
    // single-PV never produces, and pruning changes (reverse-
    // futility, statScore-driven LMR, NMP gating, CMP, ProbCut,
    // …) can amplify that small state difference into a
    // different move. The test uses a 1 MB TT (high collision
    // rate, amplifies sensitivity); the CLI's much larger
    // default TT typically converges at lower depths than this
    // test requires. Each time a new pruning feature lands the
    // convergence depth bumps; the test sits one step *above*
    // the divergence boundary, not at it. History: depth 4 →
    // 8 (reverse-futility) → 11 (statScore-LMR) → 13 (ProbCut)
    // → 14 (extension refinements).
    let mut pos = Position::startpos();
    let single = multi_pv_search(&mut pos, 14, 1);
    let multi = multi_pv_search(&mut pos, 14, 5);
    assert!(!single.is_empty());
    assert!(!multi.is_empty());
    assert_eq!(
        single[0].pv[0], multi[0].pv[0],
        "MultiPV slot 0's first move must match single-PV"
    );
}

#[test]
fn multi_pv_one_is_backwards_compatible_with_pre_refactor() {
    // Historical contract: multi_pv=1 returns exactly one line for
    // a non-terminal position, and its shape (non-empty PV,
    // non-mate score at a shallow depth) matches what the old
    // single-PV path returned.
    let mut pos = Position::startpos();
    let lines = multi_pv_search(&mut pos, 4, 1);
    assert_eq!(lines.len(), 1);
    let line = &lines[0];
    assert!(!line.pv.is_empty());
    assert!(
        line.score.0.abs() < Value::MATE.0 - Value::MAX_PLY,
        "opening eval shouldn't be mate, got {}",
        line.score.0
    );
}

// ---- Per-ply traces + settled-ply ----------------------------------

#[test]
fn ply_traces_length_matches_pv_length() {
    let mut pos = Position::startpos();
    let line = search_to_depth(&mut pos, 4);
    assert!(!line.pv.is_empty());
    assert_eq!(
        line.ply_traces.len(),
        line.pv.len(),
        "ply_traces must have exactly one entry per PV move"
    );
}

#[test]
fn ply_traces_agree_with_replay_at_each_index() {
    // For each index i, ply_traces[i] must match a fresh
    // evaluate_with_trace at the position reached by replaying
    // pv[0..=i]. Catches off-by-one errors in the walk.
    let mut pos = Position::startpos();
    let line = search_to_depth(&mut pos, 3);
    let mut replay = pos.clone();
    for (i, mv) in line.pv.iter().enumerate() {
        replay.do_move(*mv);
        let (_, expected) = evaluate_with_trace(&replay);
        assert_eq!(
            line.ply_traces[i], expected,
            "ply_traces[{}] must match a fresh evaluate_with_trace at that ply",
            i
        );
    }
}

#[test]
fn settled_ply_none_on_terminal_position() {
    // Checkmate from black's side: no legal moves, so no PV, so no
    // settled-ply to report.
    let mut pos = Position::from_fen("7k/5KQ1/8/8/8/8/8/8 b - - 0 1").unwrap();
    let lines = multi_pv_search(&mut pos, 3, 1);
    assert!(lines.is_empty());
}

#[test]
fn settled_ply_zero_when_single_move_pv() {
    // Constructed scenario: if the PV has length 1, there's no
    // adjacent delta to evaluate, so settled_ply == 0 trivially.
    // Direct unit-level check of the helper.
    use crate::types::Color;
    let trace = EvalTrace::zero();
    let result = compute_settled_ply(&[trace], Color::White);
    assert_eq!(result, Some(0));
}

#[test]
fn settled_ply_none_when_no_traces() {
    use crate::types::Color;
    let result = compute_settled_ply(&[], Color::White);
    assert_eq!(result, None);
}

#[test]
fn settled_ply_zero_when_every_delta_below_threshold() {
    // Hand-constructed trace sequence where the white-POV score
    // barely moves. Must settle at 0 regardless of length.
    use crate::types::{Color, Value};
    let mut traces = Vec::new();
    for i in 0..6 {
        let mut t = EvalTrace::zero();
        // Alternate sign on final_value per ply to mimic
        // side-to-move oscillation. With i % 2 == 0 meaning stm is
        // black, the white-POV converts to -t.final_value + tempo.
        // We want a stable white-POV of ~+5, so:
        //   - even i (black-to-move): final_value = -(5 - TEMPO) = TEMPO - 5.
        //   - odd  i (white-to-move): final_value = 5 + TEMPO.
        let tempo = t.tempo.0;
        let fv = if i % 2 == 0 { tempo - 5 } else { 5 + tempo };
        t.final_value = Value(fv);
        traces.push(t);
    }
    assert_eq!(compute_settled_ply(&traces, Color::White), Some(0));
}

/// Build a trace sequence with the given white-POV targets for
/// each ply, assuming `root_stm == White` (so the stm-after-ply
/// pattern is Black, White, Black, ...).
fn traces_from_white_pov(targets_white_pov: &[i32]) -> Vec<EvalTrace> {
    use crate::types::Value;
    let tempo = EvalTrace::zero().tempo.0;
    targets_white_pov
        .iter()
        .enumerate()
        .map(|(i, &w)| {
            let mut t = EvalTrace::zero();
            // Even i → stm is black (root is White, flipped once).
            // White-POV w means stm_unsigned = -w, and final_value
            // = stm_unsigned + tempo = -w + tempo.
            // Odd i → stm is white. final_value = w + tempo.
            let fv = if i % 2 == 0 { -w + tempo } else { w + tempo };
            t.final_value = Value(fv);
            t
        })
        .collect()
}

#[test]
fn settled_ply_filters_the_single_ply_sawtooth() {
    // A canonical sawtooth: alternating 20/300 white-POV values
    // with every 1-ply delta huge (280 cp) but every 2-ply delta
    // exactly zero. Must settle at 0 — the eval is actually stable
    // across complete exchanges, the 1-ply swings are just the
    // "I moved but you haven't responded yet" asymmetry.
    use crate::types::Color;
    let traces = traces_from_white_pov(&[20, 300, 20, 300, 20, 300]);
    assert_eq!(compute_settled_ply(&traces, Color::White), Some(0));
}

#[test]
fn settled_ply_detects_two_ply_shift_on_top_of_sawtooth() {
    // Same sawtooth as above for the first four plies, then a
    // 180-cp lift: ply 4 = 200 (same side as ply 2 = 20, diff
    // 180), ply 5 = 480 (same side as ply 3 = 300, diff 180).
    // Under 2-ply comparison both plies 4 and 5 show big deltas
    // against their same-side predecessor; scanning backward
    // finds the last unstable at ply 5. PV ends mid-shift (no
    // post-resolution ply available), so we land on 5 itself.
    use crate::types::Color;
    let traces = traces_from_white_pov(&[20, 300, 20, 300, 200, 480]);
    assert_eq!(compute_settled_ply(&traces, Color::White), Some(5));
}

#[test]
fn settled_ply_lands_on_post_resolution_when_available() {
    // Mid-exchange peak modelled on the Nf3 → Bxe6 fxe6 scenario:
    // ply 4 (white-side) shows white temporarily up a bishop
    // (white_pov 950, up from 50 two plies back — 900 cp jump,
    // unstable). Ply 5 (black-side post-recapture) restores
    // parity (white_pov 60, 10 cp from ply 3's 50 — stable).
    // Walking backward, the loop finds ply 4 unstable; with a
    // post-resolution ply available (5), settle there rather
    // than on the peak (4).
    use crate::types::Color;
    let traces = traces_from_white_pov(&[0, 0, 50, 50, 950, 60]);
    assert_eq!(compute_settled_ply(&traces, Color::White), Some(5));
}

#[test]
fn settled_ply_reports_zero_on_short_pv_below_two_plies() {
    // A 2-ply trace sequence cannot use the 2-ply comparison
    // (there's no index >= 2). Settles trivially at 0.
    use crate::types::Color;
    let traces = traces_from_white_pov(&[0, 100]);
    assert_eq!(compute_settled_ply(&traces, Color::White), Some(0));
}

#[test]
fn settled_ply_on_live_search_is_within_bounds() {
    // End-to-end: on a real search, settled_ply must be a valid
    // index into ply_traces (if Some) or None for an empty PV.
    let mut pos = Position::startpos();
    let lines = multi_pv_search(&mut pos, 4, 2);
    for line in &lines {
        match line.settled_ply {
            Some(i) => assert!(
                i < line.ply_traces.len(),
                "settled_ply {} out of bounds (ply_traces len {})",
                i,
                line.ply_traces.len()
            ),
            None => assert!(line.pv.is_empty()),
        }
    }
}

// ---- force_include ------------------------------------------------

/// Helper: run a search with forced moves and return the resulting lines.
fn search_with_forced(
    pos: &mut Position,
    depth: u32,
    multi_pv: usize,
    forced: Vec<Move>,
) -> Vec<SearchLine> {
    let mut engine = Engine::new(1);
    engine.search(
        pos,
        SearchParams {
            max_depth: depth,
            multi_pv,
            force_include: forced,
            ..Default::default()
        },
    )
}

/// Find a legal move that the search definitely won't pick in the
/// top-k at a given depth. We take the last legal move in the
/// generated order — from startpos, that's typically a rook or
/// knight retreat that can't possibly be best.
fn pick_uninteresting_move(pos: &mut Position) -> Move {
    let legal = crate::movegen::legal_moves_vec(pos);
    *legal.last().expect("startpos must have legal moves")
}

#[test]
fn force_include_empty_matches_plain_multi_pv() {
    // Empty force_include vector must be a no-op.
    let mut pos = Position::startpos();
    let plain = multi_pv_search(&mut pos, 4, 3);
    let forced = search_with_forced(&mut pos, 4, 3, Vec::new());
    assert_eq!(plain.len(), forced.len());
    for (p, f) in plain.iter().zip(forced.iter()) {
        assert_eq!(p.pv[0], f.pv[0], "first-move ordering must match");
    }
}

#[test]
fn force_include_adds_out_of_top_k_move() {
    // Take a startpos move that will not naturally appear in top-3
    // (the last-generated legal move, usually a knight moving to a
    // passive square) and force it into the output.
    let mut pos = Position::startpos();
    let victim = pick_uninteresting_move(&mut pos);

    let plain = multi_pv_search(&mut pos, 4, 3);
    let natural_first_moves: Vec<Move> = plain.iter().map(|l| l.pv[0]).collect();
    assert!(
        !natural_first_moves.contains(&victim),
        "test setup: victim must NOT naturally be in top-3; \
         if this fires, pick a different victim"
    );

    let forced = search_with_forced(&mut pos, 4, 3, vec![victim]);
    let forced_first_moves: Vec<Move> = forced.iter().map(|l| l.pv[0]).collect();
    assert!(
        forced_first_moves.contains(&victim),
        "forced move must appear in output; got {:?}",
        forced_first_moves
    );
}

#[test]
fn force_include_forced_slot_has_valid_score_and_pv() {
    // The forced slot must produce a real score (not -INFINITE) and
    // a PV of length > 1 at depth >= 2 — i.e. the search actually
    // ran, didn't just stub out a one-move PV.
    let mut pos = Position::startpos();
    let victim = pick_uninteresting_move(&mut pos);

    let forced = search_with_forced(&mut pos, 3, 1, vec![victim]);
    let slot = forced
        .iter()
        .find(|l| l.pv[0] == victim)
        .expect("forced move must appear");
    assert_ne!(
        slot.score,
        -Value::INFINITE,
        "forced slot must have real score"
    );
    assert!(slot.pv.len() > 1, "forced PV must extend past ply 1");
    assert_eq!(
        slot.ply_traces.len(),
        slot.pv.len(),
        "forced slot's ply_traces must align with its PV length"
    );
}

#[test]
fn force_include_skips_move_already_in_top_k() {
    // Forcing the natural best move should be a no-op — the output
    // shouldn't have a duplicate of the best move.
    let mut pos = Position::startpos();
    let plain = multi_pv_search(&mut pos, 3, 2);
    let natural_best = plain[0].pv[0];

    let forced = search_with_forced(&mut pos, 3, 2, vec![natural_best]);
    let duplicates = forced.iter().filter(|l| l.pv[0] == natural_best).count();
    assert_eq!(duplicates, 1, "natural best must appear exactly once");
    assert_eq!(forced.len(), plain.len(), "output size must not grow");
}

#[test]
fn force_include_ignores_illegal_moves_silently() {
    // A move that isn't legal at the root (e.g. Move::NONE, or a
    // fabricated move from a wrong-color piece) must be silently
    // dropped — not crash, not return anything extra.
    let mut pos = Position::startpos();
    let plain = multi_pv_search(&mut pos, 3, 2);
    let forced = search_with_forced(&mut pos, 3, 2, vec![Move::NONE]);
    assert_eq!(forced.len(), plain.len());
}

#[test]
fn force_include_deduplicates_within_its_list() {
    // The same forced move listed twice should still produce only
    // one extra output row.
    let mut pos = Position::startpos();
    let victim = pick_uninteresting_move(&mut pos);
    let forced = search_with_forced(&mut pos, 3, 2, vec![victim, victim, victim]);
    let victim_count = forced.iter().filter(|l| l.pv[0] == victim).count();
    assert_eq!(
        victim_count, 1,
        "duplicate forced moves must dedup to one slot"
    );
}

#[test]
fn force_include_multiple_distinct_moves_all_appear() {
    // Force in two distinct out-of-top-k moves; both must show.
    let mut pos = Position::startpos();
    let legal = crate::movegen::legal_moves_vec(&mut pos);
    // Take two tail moves that we expect to be out of top-1.
    let v1 = legal[legal.len() - 1];
    let v2 = legal[legal.len() - 2];

    let forced = search_with_forced(&mut pos, 3, 1, vec![v1, v2]);
    let first_moves: Vec<Move> = forced.iter().map(|l| l.pv[0]).collect();
    assert!(
        first_moves.contains(&v1),
        "v1 must appear: {:?}",
        first_moves
    );
    assert!(
        first_moves.contains(&v2),
        "v2 must appear: {:?}",
        first_moves
    );
}

#[test]
fn force_include_output_is_sorted_by_score_descending() {
    // After the final sort, the whole output (natural + forced)
    // should be monotonically non-increasing in score.
    let mut pos = Position::startpos();
    let victim = pick_uninteresting_move(&mut pos);
    let forced = search_with_forced(&mut pos, 4, 3, vec![victim]);
    for pair in forced.windows(2) {
        assert!(
            pair[0].score.0 >= pair[1].score.0,
            "output must be sorted descending by score; got {} then {}",
            pair[0].score.0,
            pair[1].score.0,
        );
    }
}

#[test]
fn force_include_preserves_natural_top_k() {
    // Forcing an extra move must not change which moves appear in
    // the natural top-k. (They may be reordered by the final sort,
    // but the SET of moves covering the natural top positions
    // plus the forced move should equal natural top-k ∪ {forced}.)
    let mut pos = Position::startpos();
    let victim = pick_uninteresting_move(&mut pos);
    let plain = multi_pv_search(&mut pos, 4, 2);
    let plain_moves: std::collections::HashSet<Move> = plain.iter().map(|l| l.pv[0]).collect();

    let forced = search_with_forced(&mut pos, 4, 2, vec![victim]);
    let forced_moves: std::collections::HashSet<Move> =
        forced.iter().map(|l| l.pv[0]).collect();

    // Everything natural is preserved; plus the victim is now in.
    for m in &plain_moves {
        assert!(
            forced_moves.contains(m),
            "natural move disappeared after force_include"
        );
    }
    assert!(forced_moves.contains(&victim));
}
