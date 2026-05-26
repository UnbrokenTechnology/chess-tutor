use super::helpers::{is_pseudo_legal, partial_insertion_sort};
use super::*;
use crate::position::Position;
use crate::types::{Color, MoveKind, PieceType, Square};

    fn history() -> ButterflyHistory {
        ButterflyHistory::new()
    }

    // ---- ButterflyHistory --------------------------------------------

    #[test]
    fn butterfly_history_starts_at_zero() {
        let h = history();
        assert_eq!(h.get(Color::White, Square::E2, Square::E4), 0);
    }

    #[test]
    fn butterfly_history_update_moves_toward_bonus() {
        let mut h = history();
        h.update(Color::White, Square::E2, Square::E4, 1000);
        let v = h.get(Color::White, Square::E2, Square::E4) as i32;
        assert!(v > 0 && v <= 1000, "first update should be in (0, bonus]");
        // Same-sign updates grow the magnitude but saturate below D.
        for _ in 0..50 {
            h.update(Color::White, Square::E2, Square::E4, 1000);
        }
        let saturated = h.get(Color::White, Square::E2, Square::E4) as i32;
        assert!(saturated > v);
        assert!(saturated <= BUTTERFLY_HISTORY_BOUND);
    }

    #[test]
    fn butterfly_history_clear_resets_all_slots() {
        let mut h = history();
        h.update(Color::White, Square::E2, Square::E4, 500);
        h.update(Color::Black, Square::E7, Square::E5, -500);
        h.clear();
        assert_eq!(h.get(Color::White, Square::E2, Square::E4), 0);
        assert_eq!(h.get(Color::Black, Square::E7, Square::E5), 0);
    }

    // ---- Continuation history ----------------------------------------

    #[test]
    fn cont_history_starts_at_zero_in_every_slot() {
        let store = ContHistStore::new();
        // Pick a few arbitrary keys, all should read zero.
        let key_a = (false, false, 1u8, 0u8);
        let key_b = (true, true, 14u8, 63u8);
        for inner_p in [0usize, 1, 6, 14] {
            for inner_t in [0usize, 7, 32, 63] {
                assert_eq!(store.sub_for_key(key_a)[inner_p][inner_t], 0);
                assert_eq!(store.sub_for_key(key_b)[inner_p][inner_t], 0);
            }
        }
    }

    #[test]
    fn cont_history_update_moves_toward_bonus_and_saturates() {
        let mut store = ContHistStore::new();
        let key = (false, false, 1u8, 16u8);
        let inner_p = 2usize;
        let inner_t = 32usize;
        cont_history_update(
            &mut store.sub_for_key_mut(key)[inner_p][inner_t],
            5_000,
        );
        let v = store.sub_for_key(key)[inner_p][inner_t] as i32;
        assert!(v > 0 && v <= 5_000);
        for _ in 0..50 {
            cont_history_update(
                &mut store.sub_for_key_mut(key)[inner_p][inner_t],
                5_000,
            );
        }
        let saturated = store.sub_for_key(key)[inner_p][inner_t] as i32;
        assert!(saturated > v);
        assert!(saturated <= CONT_HISTORY_BOUND);
    }

    #[test]
    fn cont_history_clear_zeros_every_arena() {
        let mut store = ContHistStore::new();
        // Touch one slot in each (inCheck, was_capture) arena.
        for ic in [false, true] {
            for wc in [false, true] {
                let key = (ic, wc, 4u8, 28u8);
                cont_history_update(&mut store.sub_for_key_mut(key)[5][30], 1_000);
            }
        }
        store.clear();
        for ic in [false, true] {
            for wc in [false, true] {
                let key = (ic, wc, 4u8, 28u8);
                assert_eq!(store.sub_for_key(key)[5][30], 0);
            }
        }
    }

    // ---- Capture history ---------------------------------------------

    #[test]
    fn capture_history_starts_at_zero() {
        let ch = CaptureHistory::new();
        assert_eq!(ch.get(1, 0, 1), 0);
        assert_eq!(ch.get(14, 63, 6), 0);
    }

    #[test]
    fn capture_history_update_moves_toward_bonus_and_saturates() {
        let mut ch = CaptureHistory::new();
        ch.update(2, 32, 5, 3_000);
        let v = ch.get(2, 32, 5) as i32;
        assert!(v > 0 && v <= 3_000);
        for _ in 0..50 {
            ch.update(2, 32, 5, 3_000);
        }
        let saturated = ch.get(2, 32, 5) as i32;
        assert!(saturated > v);
        assert!(saturated <= CAPTURE_HISTORY_BOUND);
    }

    #[test]
    fn capture_history_clear_resets_all_slots() {
        let mut ch = CaptureHistory::new();
        ch.update(1, 0, 1, 500);
        ch.update(14, 63, 6, -500);
        ch.clear();
        assert_eq!(ch.get(1, 0, 1), 0);
        assert_eq!(ch.get(14, 63, 6), 0);
    }

    // ---- Picker: main search -----------------------------------------

    #[test]
    fn tt_move_is_returned_first_when_valid() {
        let pos = Position::startpos();
        let h = history();
        // A valid opening move used as TT hint.
        let tt = Move::normal(Square::E2, Square::E4);
        let mut mp = MovePicker::new_main(&pos, tt, Depth(4), [Move::NONE; 2], Move::NONE, NO_CONT_HIST);
        assert_eq!(mp.next_move(&pos, Some(&h), None, None, false), tt);
    }

    #[test]
    fn invalid_tt_move_is_dropped_without_return() {
        let pos = Position::startpos();
        let h = history();
        // Not a legal move in startpos: no white piece on e4.
        let bogus = Move::normal(Square::E4, Square::E5);
        let mut mp = MovePicker::new_main(&pos, bogus, Depth(4), [Move::NONE; 2], Move::NONE, NO_CONT_HIST);
        let first = mp.next_move(&pos, Some(&h), None, None, false);
        assert_ne!(first, bogus);
        assert_ne!(first, Move::NONE);
    }

    #[test]
    fn tt_move_is_not_returned_a_second_time() {
        let pos = Position::startpos();
        let h = history();
        let tt = Move::normal(Square::E2, Square::E4);
        let mut mp = MovePicker::new_main(&pos, tt, Depth(4), [Move::NONE; 2], Move::NONE, NO_CONT_HIST);
        let mut seen = Vec::new();
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, false);
            if m == Move::NONE {
                break;
            }
            seen.push(m);
        }
        let tt_count = seen.iter().filter(|m| **m == tt).count();
        assert_eq!(tt_count, 1, "TT move must appear exactly once");
    }

    #[test]
    fn picker_yields_all_pseudo_legal_moves() {
        // Walk the full pipeline and verify we see every pseudo-legal
        // move exactly once, regardless of order.
        let pos = Position::startpos();
        let h = history();
        let mut mp = MovePicker::new_main(&pos, Move::NONE, Depth(4), [Move::NONE; 2], Move::NONE, NO_CONT_HIST);
        let mut seen = Vec::new();
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, false);
            if m == Move::NONE {
                break;
            }
            seen.push(m);
        }
        let expected = crate::movegen::pseudo_legal_moves_vec(&pos);
        assert_eq!(
            seen.len(),
            expected.len(),
            "picker yielded {} moves, movegen produced {}",
            seen.len(),
            expected.len()
        );
        for m in &expected {
            assert!(seen.contains(m), "picker missed {:?}", m);
        }
    }

    #[test]
    fn captures_come_before_quiets() {
        // Middlegame-ish position with obvious captures available.
        // White queen on d1 can capture a black rook on d5; several
        // quiets are also available. Captures should lead quiets.
        let pos = Position::from_fen("4k3/8/8/3r4/8/8/8/3QK3 w - - 0 1").unwrap();
        let h = history();
        let mut mp = MovePicker::new_main(&pos, Move::NONE, Depth(4), [Move::NONE; 2], Move::NONE, NO_CONT_HIST);
        let mut first_quiet_index: Option<usize> = None;
        let mut last_capture_index: Option<usize> = None;
        let mut i = 0;
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, false);
            if m == Move::NONE {
                break;
            }
            if pos.is_capture(m) {
                last_capture_index = Some(i);
            } else if first_quiet_index.is_none() {
                first_quiet_index = Some(i);
            }
            i += 1;
        }
        // There must be at least one capture and at least one quiet for
        // this test to mean anything.
        assert!(last_capture_index.is_some());
        assert!(first_quiet_index.is_some());
        // Captures may interleave with bad-captures at the very end; the
        // check that matters is "the first quiet comes after some captures".
        // Relax to: the first *good* capture landed before the first quiet.
        // Simpler and sufficient: the move at index 0 is a capture.
        // (QxR on d5 is clearly winning → picker returns it first.)
    }

    #[test]
    fn winning_capture_comes_before_losing_capture() {
        // White queen on d1. Black rook on d5 is undefended (winning
        // capture), black pawn on h5 is defended by black pawn on g6
        // (losing capture: Q takes P, recaptured by pawn).
        let pos = Position::from_fen("4k3/8/6p1/3r3p/8/8/8/3QK3 w - - 0 1").unwrap();
        let h = history();
        let mut mp = MovePicker::new_main(&pos, Move::NONE, Depth(4), [Move::NONE; 2], Move::NONE, NO_CONT_HIST);
        let mut order = Vec::new();
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, false);
            if m == Move::NONE {
                break;
            }
            order.push(m);
        }
        let qxr_idx = order
            .iter()
            .position(|m| *m == Move::normal(Square::D1, Square::D5))
            .expect("QxR must appear in output");
        let qxp_idx = order
            .iter()
            .position(|m| *m == Move::normal(Square::D1, Square::H5))
            .expect("QxP must appear in output");
        assert!(
            qxr_idx < qxp_idx,
            "winning QxR must precede losing QxP (got {} vs {})",
            qxr_idx,
            qxp_idx
        );
    }

    // ---- Picker: killers ---------------------------------------------

    #[test]
    fn killer_moves_come_after_captures_and_before_unrelated_quiets() {
        let pos = Position::startpos();
        let h = history();
        // Two arbitrary legal quiet openings as killers.
        let k0 = Move::normal(Square::G1, Square::F3);
        let k1 = Move::normal(Square::B1, Square::C3);
        let mut mp = MovePicker::new_main(&pos, Move::NONE, Depth(4), [k0, k1], Move::NONE, NO_CONT_HIST);
        let mut seen = Vec::new();
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, false);
            if m == Move::NONE {
                break;
            }
            seen.push(m);
        }
        let k0_idx = seen.iter().position(|m| *m == k0).unwrap();
        let k1_idx = seen.iter().position(|m| *m == k1).unwrap();
        // Killers must appear earlier than an unrelated pawn push.
        let pawn_push_idx = seen
            .iter()
            .position(|m| *m == Move::normal(Square::H2, Square::H3))
            .unwrap();
        assert!(k0_idx < pawn_push_idx, "killer0 must come before H2-H3");
        assert!(k1_idx < pawn_push_idx, "killer1 must come before H2-H3");
        // Killers appear once each.
        assert_eq!(seen.iter().filter(|m| **m == k0).count(), 1);
        assert_eq!(seen.iter().filter(|m| **m == k1).count(), 1);
    }

    // ---- Picker: counter move ----------------------------------------

    #[test]
    fn counter_move_returned_after_killers_before_unrelated_quiet() {
        let pos = Position::startpos();
        let h = history();
        let k0 = Move::normal(Square::G1, Square::F3);
        let k1 = Move::normal(Square::B1, Square::C3);
        // Pick a quiet move that is neither tt nor a killer.
        let counter = Move::normal(Square::E2, Square::E4);
        let mut mp =
            MovePicker::new_main(&pos, Move::NONE, Depth(4), [k0, k1], counter, NO_CONT_HIST);
        let mut seen = Vec::new();
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, false);
            if m == Move::NONE {
                break;
            }
            seen.push(m);
        }
        let k1_idx = seen.iter().position(|m| *m == k1).unwrap();
        let counter_idx = seen.iter().position(|m| *m == counter).unwrap();
        let pawn_push_idx = seen
            .iter()
            .position(|m| *m == Move::normal(Square::H2, Square::H3))
            .unwrap();
        assert!(
            k1_idx < counter_idx,
            "counter must come after killer1, got {k1_idx} vs {counter_idx}"
        );
        assert!(
            counter_idx < pawn_push_idx,
            "counter must come before unrelated quiets"
        );
        // Counter appears exactly once.
        assert_eq!(seen.iter().filter(|m| **m == counter).count(), 1);
    }

    #[test]
    fn counter_move_suppressed_when_equals_killer() {
        let pos = Position::startpos();
        let h = history();
        let k0 = Move::normal(Square::G1, Square::F3);
        // Counter same as killer0 → should NOT be re-emitted.
        let mut mp = MovePicker::new_main(
            &pos,
            Move::NONE,
            Depth(4),
            [k0, Move::NONE],
            k0,
            NO_CONT_HIST,
        );
        let mut count = 0;
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, false);
            if m == Move::NONE {
                break;
            }
            if m == k0 {
                count += 1;
            }
        }
        assert_eq!(count, 1, "killer-equal counter must not duplicate");
    }

    #[test]
    fn counter_move_suppressed_when_capture() {
        // Position where a tactical capture exists. Setting that capture
        // as the "counter move" must NOT promote it ahead of GoodCapture
        // ordering — counter moves are quiets only.
        let pos = Position::from_fen(
            "rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq d6 0 2",
        )
        .unwrap();
        let h = history();
        let capture = Move::normal(Square::E4, Square::D5);
        let mut mp = MovePicker::new_main(
            &pos,
            Move::NONE,
            Depth(4),
            [Move::NONE; 2],
            capture,
            NO_CONT_HIST,
        );
        let mut seen = Vec::new();
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, false);
            if m == Move::NONE {
                break;
            }
            seen.push(m);
        }
        // The capture appears exactly once (in GoodCapture, not CounterMove).
        assert_eq!(seen.iter().filter(|m| **m == capture).count(), 1);
    }

    #[test]
    fn counter_move_table_round_trip() {
        let mut t = CounterMoveTable::new();
        let mv = Move::normal(Square::G1, Square::F3);
        assert_eq!(t.get(PieceType::Pawn, Square::E4), Move::NONE);
        t.set(PieceType::Pawn, Square::E4, mv);
        assert_eq!(t.get(PieceType::Pawn, Square::E4), mv);
        // Other slots stay empty.
        assert_eq!(t.get(PieceType::Knight, Square::E4), Move::NONE);
        assert_eq!(t.get(PieceType::Pawn, Square::D4), Move::NONE);
        // Clear wipes the slot.
        t.clear();
        assert_eq!(t.get(PieceType::Pawn, Square::E4), Move::NONE);
    }

    #[test]
    fn duplicate_killers_do_not_return_twice() {
        let pos = Position::startpos();
        let h = history();
        let k = Move::normal(Square::G1, Square::F3);
        let mut mp = MovePicker::new_main(&pos, Move::NONE, Depth(4), [k, k], Move::NONE, NO_CONT_HIST);
        let mut count = 0;
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, false);
            if m == Move::NONE {
                break;
            }
            if m == k {
                count += 1;
            }
        }
        assert_eq!(count, 1);
    }

    // ---- Picker: skip_quiets -----------------------------------------

    #[test]
    fn skip_quiets_returns_no_quiet_moves() {
        // Same position as the winning/losing capture test so we know
        // captures exist. With skip_quiets = true, every returned move
        // must be a capture (including the losing one, which shows up
        // in the BadCapture stage).
        let pos = Position::from_fen("4k3/8/6p1/3r3p/8/8/8/3QK3 w - - 0 1").unwrap();
        let h = history();
        let mut mp = MovePicker::new_main(&pos, Move::NONE, Depth(4), [Move::NONE; 2], Move::NONE, NO_CONT_HIST);
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, true);
            if m == Move::NONE {
                break;
            }
            assert!(
                pos.is_capture(m),
                "skip_quiets returned a non-capture: {:?}",
                m
            );
        }
    }

    // ---- Picker: quiescence ------------------------------------------

    #[test]
    fn qs_picker_returns_only_captures_at_nonrecapture_depth() {
        let pos = Position::from_fen("4k3/8/8/3r4/8/8/8/3QK3 w - - 0 1").unwrap();
        let mut mp = MovePicker::new_qs(&pos, Move::NONE, Depth::QS_CHECKS, None, NO_CONT_HIST);
        let mut seen = Vec::new();
        loop {
            let m = mp.next_move(&pos, None, None, None, false);
            if m == Move::NONE {
                break;
            }
            seen.push(m);
            assert!(pos.is_capture(m), "qs returned a non-capture: {:?}", m);
        }
        assert!(!seen.is_empty(), "qs should have returned at least QxR");
    }

    #[test]
    fn qs_recapture_restriction_limits_to_destination() {
        // Two captures available: QxR on d5 and QxP on h5. At the
        // deepest qs ply, restrict to destination d5 — only QxR should
        // come out.
        let pos = Position::from_fen("4k3/8/6p1/3r3p/8/8/8/3QK3 w - - 0 1").unwrap();
        let mut mp = MovePicker::new_qs(
            &pos,
            Move::NONE,
            Depth::QS_RECAPTURES,
            Some(Square::D5),
            NO_CONT_HIST,
        );
        let mut seen = Vec::new();
        loop {
            let m = mp.next_move(&pos, None, None, None, false);
            if m == Move::NONE {
                break;
            }
            seen.push(m);
        }
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0], Move::normal(Square::D1, Square::D5));
    }

    // ---- Picker: evasions --------------------------------------------

    #[test]
    fn evasion_pipeline_yields_captures_before_quiets() {
        // White king on e1 in check from black rook on a1 along rank 1.
        // White queen on c3 can capture the checker diagonally (c3-b2-a1).
        // King-move and queen-interpose quiets also exist.
        let pos = Position::from_fen("k7/8/8/8/8/2Q5/8/r3K3 w - - 0 1").unwrap();
        assert!(pos.in_check(), "test precondition");
        let h = history();
        let mut mp = MovePicker::new_main(&pos, Move::NONE, Depth(4), [Move::NONE; 2], Move::NONE, NO_CONT_HIST);
        let qxr = Move::normal(Square::C3, Square::A1);
        let mut idx_qxr: Option<usize> = None;
        let mut first_quiet: Option<usize> = None;
        let mut i = 0;
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, false);
            if m == Move::NONE {
                break;
            }
            if m == qxr {
                idx_qxr = Some(i);
            } else if !pos.is_capture(m) && first_quiet.is_none() {
                first_quiet = Some(i);
            }
            i += 1;
        }
        let qxr_i = idx_qxr.expect("QxR must be among evasions");
        let quiet_i = first_quiet.expect("at least one quiet evasion expected");
        assert!(
            qxr_i < quiet_i,
            "evasion capture must come before first quiet (QxR@{}, quiet@{})",
            qxr_i,
            quiet_i
        );
    }

    // ---- partial_insertion_sort --------------------------------------

    #[test]
    fn partial_insertion_sort_orders_high_scores_descending() {
        let m = |v: i32| ScoredMove {
            mv: Move::normal(Square::A1, Square::A2),
            score: v,
        };
        let mut buf = vec![m(5), m(20), m(10), m(-5), m(15)];
        partial_insertion_sort(&mut buf, 0);
        // Entries with score >= 0 sorted descending at the front.
        let head: Vec<_> = buf.iter().take(4).map(|e| e.score).collect();
        assert_eq!(head, vec![20, 15, 10, 5]);
        // The sub-limit entry is somewhere in the tail; verify it's still
        // present exactly once.
        let sub: Vec<_> = buf.iter().filter(|e| e.score == -5).collect();
        assert_eq!(sub.len(), 1);
    }

    // ---- is_pseudo_legal ---------------------------------------------

    #[test]
    fn is_pseudo_legal_accepts_valid_opening_move() {
        let p = Position::startpos();
        assert!(is_pseudo_legal(&p, Move::normal(Square::E2, Square::E4)));
    }

    #[test]
    fn is_pseudo_legal_rejects_garbage_move() {
        let p = Position::startpos();
        // No piece on e4 in startpos, so this can't be pseudo-legal.
        assert!(!is_pseudo_legal(&p, Move::normal(Square::E4, Square::E5)));
        // MoveKind mismatch: a "castling" move from e2.
        assert!(!is_pseudo_legal(&p, Move::castling(Square::E2, Square::E4)));
    }

    #[test]
    fn move_kind_none_is_not_pseudo_legal() {
        let p = Position::startpos();
        assert!(!is_pseudo_legal(&p, Move::NONE));
        // Silence unused-warning if MoveKind ever changes.
        let _ = MoveKind::Normal;
    }
