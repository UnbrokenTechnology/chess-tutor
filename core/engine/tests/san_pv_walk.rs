//! Regression: rendering a principal variation as SAN must format each
//! move from the board it's actually played on, and the public formatters
//! must never mutate the caller's position.
//!
//! The original bug: a `format_on(&mut pos, mv)` that callers looped over a
//! PV formatted every move from the *root*, so a deep move whose
//! from-square was empty at the root panicked "from must be occupied".
//! From the user's game, clicking `7.Ne6+` crashed walking its best line.
//! The walk now lives in `san::pv_to_san` (which owns an internal scratch);
//! `san::format` is the non-mutating single-move formatter.

use std::time::Duration;

use chess_tutor_engine::engine::{Engine, SearchParams};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;

#[test]
fn pv_to_san_walks_the_line_and_never_panics() {
    let opening = [
        "e4", "e5", "Qh5", "Nc6", "Bc4", "Nh6", "Nc3", "Bd6", "Nd5", "b5", "Nxc7+", "Kf8",
    ];
    let mut pos = Position::startpos();
    for m in opening {
        let mv = san::parse(&mut pos, m).unwrap_or_else(|e| panic!("parse {m}: {e}"));
        pos.do_move(mv);
    }
    let pre = pos.clone();
    let played = san::parse(&mut pos.clone(), "Ne6+").expect("Ne6+ legal");

    let mut engine = Engine::new(16);
    engine.new_game();
    let mut root = pre.clone();
    let analyses = chess_tutor_engine::analysis::analyze_position(
        &mut engine,
        &mut root,
        SearchParams {
            max_depth: 10,
            max_nodes: Some(100_000_000),
            max_time: Some(Duration::from_millis(10_000)),
            multi_pv: 2,
            force_include: vec![played],
            threads: 1,
            ..Default::default()
        },
    );

    for a in &analyses {
        let sans = san::pv_to_san(&pre, &a.pv);
        assert_eq!(sans.len(), a.pv.len(), "every PV move is formatted");
    }
}

#[test]
fn format_does_not_mutate_the_position() {
    // Contract guard: the public single-move formatter must leave the
    // caller's position untouched.
    let pos = Position::startpos();
    let e4 = san::parse(&mut pos.clone(), "e4").unwrap();
    let before = pos.side_to_move();
    let san = san::format(&pos, e4);
    assert_eq!(san, "e4");
    assert_eq!(pos.side_to_move(), before, "format must not advance the position");
}
