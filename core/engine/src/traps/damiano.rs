//! Damiano Defense refutation.
//!
//! After 1.e4 e5 2.Nf3 f6?? white sacrifices on e5. The main defense
//! Qe7 limits the loss to a pawn; recapturing with fxe5?? walks into
//! Qh5+ g6 Qxe5+ and drops a rook on h8 by fork.
//!
//! Tree layout:
//!
//! ```text
//! Nxe5 (root)
//! ├─ Qe7 (main defense — knight retreats, only a pawn lost)
//! │  └─ Nf3 (terminal, gain = +100)
//! └─ fxe5 (walks deeper — "ooh free knight" recapture)
//!    └─ Qh5+
//!       ├─ g6 (main defense — block is best in the lost position)
//!       │  └─ Qxe5+
//!       │     └─ Qe7 (main defense — only queen interposition)
//!       │        └─ Qxh8 (terminal, gain = +400, rook won)
//!       └─ Ke7 (walks deeper — king walks into a mate net)
//!          └─ terminal (gain ≈ +500; continuation is catastrophic
//!             but too position-specific to encode)
//! ```

use crate::bitboard::square_bb;
use crate::types::{Color, Piece, PieceType, Square};

use super::{DefenderOption, Invariant, InvariantKind, PunisherMove, TrapEntry, TriggerPattern};

// =========================================================================
// Refutation tree (leaves first, then inwards — each `PunisherMove` has
// to be defined before anything that references it).
// =========================================================================

static NF3_RETREAT: PunisherMove = PunisherMove {
    san: "Nf3",
    defender_options: &[],
    terminal_gain_cp: Some(100),
};

static QXH8_TERMINAL: PunisherMove = PunisherMove {
    san: "Qxh8",
    defender_options: &[],
    terminal_gain_cp: Some(400),
};

static QE7_INTERPOSE: DefenderOption = DefenderOption {
    san: "Qe7",
    is_main_defense: true,
    label: Some("interpose — the only move"),
    punisher_follow_up: Some(&QXH8_TERMINAL),
    terminal_gain_cp: None,
};

static QXE5_FORK: PunisherMove = PunisherMove {
    san: "Qxe5+",
    defender_options: &[QE7_INTERPOSE],
    terminal_gain_cp: None,
};

static G6_BLOCK: DefenderOption = DefenderOption {
    san: "g6",
    is_main_defense: true,
    label: Some("block — best available; still loses a rook"),
    punisher_follow_up: Some(&QXE5_FORK),
    terminal_gain_cp: None,
};

static KE7_WALK: DefenderOption = DefenderOption {
    san: "Ke7",
    is_main_defense: false,
    label: Some("king walks forward — loses the queen or gets mated"),
    // Library stops here. Qxe5+ Kf7 Bc4+ ... is too specific to encode
    // and the position is already catastrophic; the engine and normal
    // move evaluation take over from this point.
    punisher_follow_up: None,
    terminal_gain_cp: Some(500),
};

static QH5_CHECK: PunisherMove = PunisherMove {
    san: "Qh5+",
    defender_options: &[G6_BLOCK, KE7_WALK],
    terminal_gain_cp: None,
};

static FXE5_RECAPTURE: DefenderOption = DefenderOption {
    san: "fxe5",
    is_main_defense: false,
    label: Some("recapture — opens the h5-e8 diagonal; loses a rook"),
    punisher_follow_up: Some(&QH5_CHECK),
    terminal_gain_cp: None,
};

static QE7_AT_ROOT: DefenderOption = DefenderOption {
    san: "Qe7",
    is_main_defense: true,
    label: Some("best defense — only a pawn is lost"),
    punisher_follow_up: Some(&NF3_RETREAT),
    terminal_gain_cp: None,
};

static NXE5_ROOT: PunisherMove = PunisherMove {
    san: "Nxe5",
    defender_options: &[QE7_AT_ROOT, FXE5_RECAPTURE],
    terminal_gain_cp: None,
};

// =========================================================================
// Invariants — chess-exact predicates that must all hold in the post-
// trigger position for the trap to apply. Each doubles as teaching
// content: the `label` literally enumerates why the trap works.
// =========================================================================

static INVARIANTS: &[Invariant] = &[
    Invariant {
        kind: InvariantKind::AttackersEqual {
            color: Color::Black,
            square: Square::E5,
            mask: square_bb(Square::F6),
        },
        label: "e5 has exactly one defender — the pawn on f6",
    },
    Invariant {
        kind: InvariantKind::NotAttackedBy {
            color: Color::Black,
            square: Square::H5,
        },
        label: "h5 is not attacked — Qh5 is safe",
    },
    Invariant {
        kind: InvariantKind::PieceOn {
            square: Square::E8,
            piece: Piece::BlackKing,
        },
        label: "the black king is still on e8",
    },
    Invariant {
        kind: InvariantKind::RayClear {
            from: Square::H5,
            to: Square::E8,
        },
        label: "the h5-e8 diagonal is open — Qh5 delivers check",
    },
    Invariant {
        kind: InvariantKind::RayClear {
            from: Square::H5,
            to: Square::E5,
        },
        label: "the h5-e5 rank is open — Qh5 attacks e5 for the fork setup",
    },
    Invariant {
        kind: InvariantKind::RayClear {
            from: Square::E5,
            to: Square::E8,
        },
        label: "the e-file between e5 and e8 is open — Qxe5 delivers check and forks the h8 rook",
    },
    Invariant {
        kind: InvariantKind::AnyPieceOfColor {
            color: Color::Black,
            square: Square::F8,
        },
        label: "f8 is occupied — the king has no escape square",
    },
];

// =========================================================================
// Entry
// =========================================================================

pub static DAMIANO: TrapEntry = TrapEntry {
    name: "Damiano Defense refutation",
    description: "After 2...f6 the knight sacrifice on e5 wins material. \
                  Best defense (Qe7) limits the loss to a pawn; recapturing \
                  with fxe5 walks into Qh5+ and loses a rook on h8.",
    punisher: Color::White,
    trigger: TriggerPattern {
        mover: Color::Black,
        piece_type: PieceType::Pawn,
        to: Square::F6,
        from: None,
    },
    invariants: INVARIANTS,
    root: &NXE5_ROOT,
};

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::position::Position;
    use crate::san;
    use crate::types::{Color, PieceType, Square};

    /// Replay a sequence of SAN moves from the start position.
    fn play_sans(moves: &[&str]) -> Position {
        let mut pos = Position::startpos();
        for mv in moves {
            let m = san::parse(&mut pos, mv).unwrap_or_else(|e| panic!("illegal {mv}: {e}"));
            let _ = pos.do_move(m);
        }
        pos
    }

    #[test]
    fn fires_on_f6_after_1_e4_e5_2_nf3() {
        // Classic Damiano trigger position.
        let pos = play_sans(&["e4", "e5", "Nf3", "f6"]);
        let hits = scan_after_move(&pos, Color::Black, PieceType::Pawn, Square::F7, Square::F6);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].1.name.contains("Damiano"));
    }

    #[test]
    fn does_not_fire_without_the_trigger() {
        // Same "1.e4 e5 2.Nf3 f6" position, but we report a different
        // trigger move. The trigger-pattern gate should reject.
        let pos = play_sans(&["e4", "e5", "Nf3", "f6"]);
        let hits = scan_after_move(
            &pos,
            Color::Black,
            PieceType::Knight,
            Square::G8,
            Square::F6,
        );
        assert!(hits.is_empty());
    }

    #[test]
    fn does_not_fire_when_e5_has_a_second_defender() {
        // 1.e4 e5 2.Nf3 Nc6 3.Bc4 f6 — the knight on c6 defends e5,
        // so Nxe5 is met by Nxe5 (not fxe5) and black wins a piece.
        // The AttackersEqual invariant rejects this.
        let pos = play_sans(&["e4", "e5", "Nf3", "Nc6", "Bc4", "f6"]);
        let hits = scan_after_move(&pos, Color::Black, PieceType::Pawn, Square::F7, Square::F6);
        assert!(hits.is_empty(), "Nc6 defends e5; trap must not fire");
    }

    #[test]
    fn does_not_fire_when_white_cannot_reach_e5() {
        // Same pawn structure, but white's knight isn't on f3 (or
        // anywhere that attacks e5). Invariants may pass, but
        // main-line verification catches it: `Nxe5` has no legal
        // move to match.
        //
        // FEN: white's king-side knight is on h3 instead of f3. All
        // other pieces are as in the post-trigger Damiano position.
        let pos =
            Position::from_fen("rnbqkbnr/pppp2pp/5p2/4p3/4P3/7N/PPPP1PPP/RNBQKB1R w KQkq - 0 3")
                .expect("legal FEN");
        let hits = scan_after_move(&pos, Color::Black, PieceType::Pawn, Square::F7, Square::F6);
        assert!(hits.is_empty(), "no knight can reach e5 — main line fails");
    }

    #[test]
    fn pre_move_scan_flags_f6_when_black_is_about_to_play_it() {
        // Position after 1.e4 e5 2.Nf3 — black to move. Black's
        // candidate ...f6 should be flagged as a Damiano threat.
        let pos = play_sans(&["e4", "e5", "Nf3"]);
        let threats = scan_threats(&pos);
        assert_eq!(threats.len(), 1, "exactly one candidate triggers Damiano");
        assert_eq!(threats[0].candidate_uci, "f7f6");
        assert_eq!(threats[0].candidate_san, "f6");
        assert!(threats[0].hit.name.contains("Damiano"));
    }

    #[test]
    fn pre_move_scan_respects_side_to_move() {
        // Same underlying FEN but white to move. Even though black
        // could in principle play ...f6 eventually, it's not their
        // turn, so scan_threats must not warn now.
        let pos =
            Position::from_fen("rnbqkbnr/pppp1ppp/8/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 1 2")
                .expect("legal FEN");
        assert!(scan_threats(&pos).is_empty());
    }

    #[test]
    fn main_line_walks_to_qxh8_with_expected_material_gain() {
        // End-to-end: after the trigger, the hit's main line should be
        // Nxe5 Qe7 Nf3 (gain = +100). We can't easily reach the
        // fxe5 sub-branch through the main-line walker because that's
        // the *walks-deeper* branch; main-line follows is_main_defense
        // children.
        let pos = play_sans(&["e4", "e5", "Nf3", "f6"]);
        let hits = scan_after_move(&pos, Color::Black, PieceType::Pawn, Square::F7, Square::F6);
        assert_eq!(hits.len(), 1);
        let hit = &hits[0].1;
        assert_eq!(hit.main_line_san, vec!["Nxe5", "Qe7", "Nf3"]);
        assert_eq!(hit.main_line_gain_cp, 100);
        assert_eq!(hit.punisher, Color::White);
    }

    #[test]
    fn invariant_labels_describe_why_the_trap_works() {
        // Spot-check that each invariant carries a non-empty label —
        // the teaching surface depends on these being populated.
        for inv in super::INVARIANTS {
            assert!(
                !inv.label.is_empty(),
                "invariant missing label: {:?}",
                inv.kind
            );
        }
    }

    // ------------------------------------------------------------------
    // Pending-trap state machine
    // ------------------------------------------------------------------

    /// Seed a `PendingTrap` by replaying the trigger against the
    /// library, returning the (position after trigger, pending) pair.
    fn trigger_damiano() -> (Position, PendingTrap) {
        let pos = play_sans(&["e4", "e5", "Nf3", "f6"]);
        let hits = scan_after_move(&pos, Color::Black, PieceType::Pawn, Square::F7, Square::F6);
        assert_eq!(hits.len(), 1);
        let (entry, hit) = hits.into_iter().next().unwrap();
        (pos, PendingTrap::new(entry, hit))
    }

    /// Parse a SAN move in `pos`, advance the pending trap, then
    /// play the move and return the event + updated position.
    fn step(pending: &mut PendingTrap, pos: &mut Position, san_text: &str) -> (TrapEvent, bool) {
        let mv = san::parse(pos, san_text).unwrap_or_else(|e| panic!("illegal {san_text}: {e}"));
        let event = advance_pending(pending, pos, mv);
        let terminal = event.is_terminal();
        let _ = pos.do_move(mv);
        (event, terminal)
    }

    #[test]
    fn pending_starts_with_punisher_root_expected() {
        let (_pos, pending) = trigger_damiano();
        match pending.expectation {
            TrapExpectation::PunisherNext(node) => {
                assert_eq!(node.san, "Nxe5");
            }
            other => panic!("expected PunisherNext(Nxe5), got {other:?}"),
        }
    }

    #[test]
    fn main_line_path_nxe5_qe7_nf3_walks_to_tree_complete() {
        let (mut pos, mut pending) = trigger_damiano();

        let (ev, term) = step(&mut pending, &mut pos, "Nxe5");
        assert!(matches!(ev, TrapEvent::PunisherExecuted { .. }));
        assert!(!term);

        let (ev, term) = step(&mut pending, &mut pos, "Qe7");
        match ev {
            TrapEvent::DefenderInTree { option, .. } => {
                assert_eq!(option.san, "Qe7");
                assert!(option.is_main_defense);
            }
            other => panic!("expected DefenderInTree(Qe7), got {other:?}"),
        }
        assert!(!term);

        let (ev, term) = step(&mut pending, &mut pos, "Nf3");
        match ev {
            TrapEvent::TreeComplete { gain_cp, .. } => {
                assert_eq!(gain_cp, Some(100));
            }
            other => panic!("expected TreeComplete, got {other:?}"),
        }
        assert!(term);
    }

    #[test]
    fn walks_deeper_path_nxe5_fxe5_reports_blunder_and_continues() {
        let (mut pos, mut pending) = trigger_damiano();

        step(&mut pending, &mut pos, "Nxe5");

        let (ev, term) = step(&mut pending, &mut pos, "fxe5");
        match ev {
            TrapEvent::DefenderInTree { option, .. } => {
                assert_eq!(option.san, "fxe5");
                assert!(!option.is_main_defense, "fxe5 is an additional blunder");
                assert!(
                    option.punisher_follow_up.is_some(),
                    "fxe5 has a follow-up (Qh5+)",
                );
            }
            other => panic!("expected DefenderInTree(fxe5), got {other:?}"),
        }
        assert!(!term, "fxe5 has a follow-up; trap continues");

        // After fxe5 we expect Qh5+ next.
        match pending.expectation {
            TrapExpectation::PunisherNext(node) => assert_eq!(node.san, "Qh5+"),
            other => panic!("expected PunisherNext(Qh5+), got {other:?}"),
        }
    }

    #[test]
    fn full_walks_deeper_line_ends_at_qxh8_with_rook_gain() {
        // Black plays every walks-deeper / main-defense combo that
        // leads to the +400 cp terminal.
        let (mut pos, mut pending) = trigger_damiano();

        step(&mut pending, &mut pos, "Nxe5");
        step(&mut pending, &mut pos, "fxe5");
        step(&mut pending, &mut pos, "Qh5+");
        step(&mut pending, &mut pos, "g6");
        step(&mut pending, &mut pos, "Qxe5+");
        step(&mut pending, &mut pos, "Qe7");

        let (ev, term) = step(&mut pending, &mut pos, "Qxh8");
        match ev {
            TrapEvent::TreeComplete { gain_cp, .. } => {
                assert_eq!(gain_cp, Some(400));
            }
            other => panic!("expected TreeComplete, got {other:?}"),
        }
        assert!(term);
    }

    #[test]
    fn ke7_branch_cut_terminates_the_tree_mid_walk() {
        // After Nxe5 fxe5 Qh5+, black's `Ke7` is a scripted blunder
        // with `punisher_follow_up: None` — the library stops
        // tracking because the continuation is too position-
        // specific. The event must be terminal.
        let (mut pos, mut pending) = trigger_damiano();
        step(&mut pending, &mut pos, "Nxe5");
        step(&mut pending, &mut pos, "fxe5");
        step(&mut pending, &mut pos, "Qh5+");

        let (ev, term) = step(&mut pending, &mut pos, "Ke7");
        match ev {
            TrapEvent::DefenderInTree { option, .. } => {
                assert_eq!(option.san, "Ke7");
                assert!(!option.is_main_defense);
                assert!(option.punisher_follow_up.is_none());
            }
            other => panic!("expected DefenderInTree(Ke7), got {other:?}"),
        }
        assert!(term, "Ke7 branch is cut; trap ends");
    }

    #[test]
    fn punisher_missed_kills_the_trap() {
        // White plays any move other than Nxe5 after the trigger
        // fires.
        let (mut pos, mut pending) = trigger_damiano();
        let (ev, term) = step(&mut pending, &mut pos, "Nc3");
        match ev {
            TrapEvent::PunisherMissed { expected_san, .. } => {
                assert_eq!(expected_san, "Nxe5");
            }
            other => panic!("expected PunisherMissed, got {other:?}"),
        }
        assert!(term);
    }

    #[test]
    fn defender_escaped_when_black_plays_an_unscripted_reply() {
        // After Nxe5, scripted defender options are Qe7 and fxe5.
        // d6 is legal for black and not in the tree — an escape.
        let (mut pos, mut pending) = trigger_damiano();
        step(&mut pending, &mut pos, "Nxe5");
        let (ev, term) = step(&mut pending, &mut pos, "d6");
        assert!(matches!(ev, TrapEvent::DefenderEscaped { .. }));
        assert!(term);
    }
}
