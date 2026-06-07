use super::*;

use chess_tutor_engine::san;

fn pv_from_san(pos: &Position, sans: &[&str]) -> Vec<Move> {
    let mut scratch = pos.clone();
    let mut pv = Vec::new();
    for s in sans {
        let mv = san::parse(&mut scratch, s).expect("test SAN must be legal");
        pv.push(mv);
        scratch.do_move(mv);
    }
    pv
}

#[test]
fn quiet_line_settles_at_zero_with_no_material() {
    let pos = Position::startpos();
    let pv = pv_from_san(&pos, &["e4", "e5", "Nf3", "Nc6", "Bc4"]);
    let facts = walk_pv(&pos, &pv);
    assert!(facts.iter().all(|f| !f.forcing && f.event_cp == 0));
    let (settled, net) = prototype_first_resolution(&facts);
    assert_eq!(settled, 0);
    assert_eq!(net, 0);
}

#[test]
fn immediate_exchange_settles_at_recapture() {
    // 1. e4 e5 2. Nf3 Nc6 3. d4: exd4 Nxd4 then quiet moves — the
    // resolution is the two captures at plies 0 and 1; the quiet tail
    // must not extend the settled window.
    let pos = Position::from_fen(
        "r1bqkbnr/pppp1ppp/2n5/4p3/3PP3/5N2/PPP2PPP/RNBQKB1R b KQkq - 0 3",
    )
    .unwrap();
    let pv = pv_from_san(&pos, &["exd4", "Nxd4", "Nf6", "Nc3", "Bb4"]);
    let facts = walk_pv(&pos, &pv);
    assert!(facts[0].forcing && facts[0].event_cp == 100); // exd4: we take a pawn
    assert!(facts[1].forcing && facts[1].event_cp == -100); // Nxd4: they take ours back
    let (settled, net) = prototype_first_resolution(&facts);
    assert_eq!(settled, 1, "settles on the recapture");
    assert_eq!(net, 0, "even trade nets zero");
}

#[test]
fn quiet_gap_inside_a_tactic_does_not_settle_early() {
    // Forcing event at ply 0, two quiet plies, then another capture at
    // ply 3: a 2-quiet-ply gap (fork-shaped) must stay inside one
    // resolution window under QUIET_RUN_LEN = 3.
    let facts = vec![
        PlyFacts { forcing: true, event_cp: 100 },
        PlyFacts { forcing: false, event_cp: 0 },
        PlyFacts { forcing: false, event_cp: 0 },
        PlyFacts { forcing: true, event_cp: 300 },
        PlyFacts { forcing: false, event_cp: 0 },
    ];
    let (settled, net) = prototype_first_resolution(&facts);
    assert_eq!(settled, 3);
    assert_eq!(net, 400);
}

#[test]
fn three_quiet_plies_end_the_window() {
    // Capture at ply 0, then three quiet plies, then a speculative
    // deep-tail capture: the tail capture must NOT count.
    let facts = vec![
        PlyFacts { forcing: true, event_cp: 100 },
        PlyFacts { forcing: false, event_cp: 0 },
        PlyFacts { forcing: false, event_cp: 0 },
        PlyFacts { forcing: false, event_cp: 0 },
        PlyFacts { forcing: true, event_cp: -900 },
    ];
    let (settled, net) = prototype_first_resolution(&facts);
    assert_eq!(settled, 0, "window closed at the first capture");
    assert_eq!(net, 100, "deep-tail queen trade excluded");
}

#[test]
fn promotion_counts_as_event_and_upgrade() {
    // White pawn promotes with check-free quiet surroundings.
    let pos = Position::from_fen("8/4P3/8/8/8/2k5/8/4K3 w - - 0 1").unwrap();
    let pv = pv_from_san(&pos, &["e8=Q"]);
    let facts = walk_pv(&pos, &pv);
    assert!(facts[0].forcing);
    assert_eq!(facts[0].event_cp, 800, "queen minus the pawn it replaces");
}

#[test]
fn checks_keep_the_window_open_without_material() {
    let facts = vec![
        PlyFacts { forcing: true, event_cp: 100 },
        PlyFacts { forcing: false, event_cp: 0 },
        PlyFacts { forcing: false, event_cp: 0 },
        PlyFacts { forcing: true, event_cp: 0 }, // a check
        PlyFacts { forcing: false, event_cp: 0 },
        PlyFacts { forcing: false, event_cp: 0 },
        PlyFacts { forcing: true, event_cp: 500 },
    ];
    let (settled, net) = prototype_first_resolution(&facts);
    assert_eq!(settled, 6, "the check bridged the gaps");
    assert_eq!(net, 600);
}

#[test]
fn classify_uses_the_noise_threshold() {
    assert_eq!(classify(WIN_MATERIAL_CP), MatClass::Win);
    assert_eq!(classify(WIN_MATERIAL_CP - 1), MatClass::Neutral);
    assert_eq!(classify(-WIN_MATERIAL_CP), MatClass::Loss);
    assert_eq!(classify(0), MatClass::Neutral);
}
