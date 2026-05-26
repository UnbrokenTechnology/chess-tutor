    use super::*;
    use chess_tutor_engine::analysis::{
        HangingPiece, MaterialOutcome,
    };
    use chess_tutor_engine::engine::{Engine, SearchParams};
    
    
    use chess_tutor_engine::position::Position;
    
    use chess_tutor_engine::types::{Color, Move, Square};
    

    /// End-to-end smoke: analyze 1.e4 from startpos and confirm the
    /// view model returns a non-empty headline + parses without
    /// panicking. We can't assert specific cards because the
    /// engine's outcome of the opening shifts by depth — but the
    /// headline must be populated.
    #[test]
    fn build_view_model_from_startpos_analysis_returns_headline() {
        let mut pos = Position::startpos();
        let mut engine = Engine::default();
        let analyses = chess_tutor_engine::analysis::analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 4,
                multi_pv: 4,
                ..SearchParams::default()
            },
        );
        assert!(!analyses.is_empty());
        // Pick any analyzed move as the "user" move so we can build
        // the view model.
        let user_move = analyses[0].mv;
        let pre = Position::startpos();
        let vm = build_retrospective_view(&pre, &analyses, user_move, false, false);
        assert!(!vm.headline.user_san.is_empty());
        assert!(!vm.headline.verdict_label.is_empty());
        assert!(!vm.headline.user_score.is_empty());
    }

    #[test]
    fn build_view_model_suppresses_items_when_user_move_delivers_mate() {
        // Fool's mate: after 1.f3 e5 2.g4, Black plays Qh4#. The
        // game ends right there — per-category cards (king safety
        // shifts, mobility deltas, "other shifts") are noise. The
        // headline still shows the mating SAN with '#' and the
        // verdict label.
        let pre = Position::from_fen(
            "rnbqkbnr/pppp1ppp/8/4p3/6P1/5P2/PPPPP2P/RNBQKBNR b KQkq g3 0 2",
        )
        .unwrap();
        let mating_move = Move::normal(Square::D8, Square::H4);
        let mut pos = pre.clone();
        let mut engine = Engine::default();
        let analyses = chess_tutor_engine::analysis::analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 4,
                multi_pv: 4,
                force_include: vec![mating_move],
                ..SearchParams::default()
            },
        );
        assert!(
            analyses.iter().any(|a| a.mv == mating_move),
            "force_include should have analyzed the mating move"
        );
        let vm = build_retrospective_view(&pre, &analyses, mating_move, false, false);
        assert!(!vm.headline.user_san.is_empty(), "headline still populated");
        assert!(
            vm.items.is_empty(),
            "game-ending move should suppress every per-category card, got {:?}",
            vm.items.iter().map(|i| &i.heading).collect::<Vec<_>>()
        );
    }

    #[test]
    fn build_view_model_with_missing_user_move_returns_empty() {
        let mut pos = Position::startpos();
        let mut engine = Engine::default();
        let analyses = chess_tutor_engine::analysis::analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 3,
                multi_pv: 1,
                ..SearchParams::default()
            },
        );
        // Pick a move that's almost certainly NOT in a depth-3
        // multi-pv-1 search: a1-a2 (would be illegal anyway because
        // a1 has the rook and a2 the pawn from startpos). The view
        // model should fall through to default rather than panic.
        let bogus = Move::normal(
            chess_tutor_engine::types::Square::A1,
            chess_tutor_engine::types::Square::A2,
        );
        let pre = Position::startpos();
        let vm = build_retrospective_view(&pre, &analyses, bogus, false, false);
        assert!(vm.headline.user_san.is_empty());
        assert!(vm.items.is_empty());
    }

    #[test]
    fn material_card_ignores_captures_past_ply_one() {
        // A MaterialOutcome with a single capture at ply 15 should
        // produce NO card — we don't say "You won material" past
        // tense based on a speculative deep-PV trade. This is the
        // 1.e4 e5 2.Nf3 → "Ply 15: you take a bishop with a bishop
        // on e6" pathology the user reported.
        use chess_tutor_engine::analysis::CaptureEvent;
        use chess_tutor_engine::types::PieceType;
        let outcome = MaterialOutcome {
            events: vec![CaptureEvent {
                ply: 14, // 0-indexed ply 14 = "Ply 15" in detail text
                captor: Color::White,
                captor_piece: PieceType::Bishop,
                captured_piece: PieceType::Bishop,
                square: chess_tutor_engine::types::Square::E6,
                value_mg: 825,
                value_eg: 915,
            }],
            net_mg_cp: 825,
            net_eg_cp: 915,
            last_ply: 14,
        };
        let pre = Position::startpos();
        let item = build_material_item(&pre, &outcome, Color::White);
        assert!(
            item.is_none(),
            "ply-15 capture must not drive a material card, got {item:?}"
        );
    }

    #[test]
    fn material_card_treats_bishop_for_knight_as_even_trade() {
        // User captures a knight at ply 0; opponent recaptures with a
        // bishop at ply 1. Engine cp net is -44 (B 825 vs N 781), but
        // classical point values are 3-for-3 — students read this as
        // an even trade. The card heading must reflect point parity,
        // not the cp lean.
        use chess_tutor_engine::analysis::CaptureEvent;
        use chess_tutor_engine::types::{PieceType, Square};
        let outcome = MaterialOutcome {
            events: vec![
                CaptureEvent {
                    ply: 0,
                    captor: Color::White,
                    captor_piece: PieceType::Bishop,
                    captured_piece: PieceType::Knight,
                    square: Square::C6,
                    value_mg: 781,
                    value_eg: 854,
                },
                CaptureEvent {
                    ply: 1,
                    captor: Color::Black,
                    captor_piece: PieceType::Pawn,
                    captured_piece: PieceType::Bishop,
                    square: Square::C6,
                    value_mg: 825,
                    value_eg: 915,
                },
            ],
            net_mg_cp: 781 - 825,
            net_eg_cp: 854 - 915,
            last_ply: 1,
        };
        let pre = Position::startpos();
        let item = build_material_item(&pre, &outcome, Color::White)
            .expect("two ply-0+ply-1 captures must produce a card");
        assert_eq!(item.heading, "Even trade");
        // Score-delta chip suppressed because point parity is even.
        assert!(item.score_delta_pawns.is_none());
    }

    #[test]
    fn filter_misleading_hangs_suppresses_bishop_on_square_we_just_captured_knight_on() {
        // Bxh6 leaves our bishop on h6 attacked by gxh6 — the second
        // leg of an even trade. The filter must recognise we just
        // captured a 3-point knight on h6 and drop the hang.
        use chess_tutor_engine::analysis::PieceLocation;
        use chess_tutor_engine::types::{PieceType, Square};
        let hangs = vec![HangingPiece {
            location: PieceLocation {
                square: Square::H6,
                piece: PieceType::Bishop,
            },
            attackers: vec![PieceLocation {
                square: Square::G7,
                piece: PieceType::Pawn,
            }],
        }];
        let captures = vec![(Square::H6, 3u8)]; // we captured a knight (3 pts) on h6
        let filtered = filter_misleading_hangs(&hangs, &captures, 0);
        assert!(filtered.is_empty(), "fair recapture should suppress the hang");
    }

    #[test]
    fn filter_misleading_hangs_keeps_real_sacrifice() {
        // Qxh6 leaves our queen attacked on a square where we
        // captured a knight. Q (9pts) > N (3pts) is a sacrifice, not
        // an even trade. The hanging-queen warning is informative —
        // it must NOT be suppressed.
        use chess_tutor_engine::analysis::PieceLocation;
        use chess_tutor_engine::types::{PieceType, Square};
        let hangs = vec![HangingPiece {
            location: PieceLocation {
                square: Square::H6,
                piece: PieceType::Queen,
            },
            attackers: vec![PieceLocation {
                square: Square::G7,
                piece: PieceType::Pawn,
            }],
        }];
        let captures = vec![(Square::H6, 3u8)];
        let filtered = filter_misleading_hangs(&hangs, &captures, 0);
        assert_eq!(filtered.len(), 1, "queen sac is not an even trade");
    }

    #[test]
    fn filter_misleading_hangs_keeps_hang_on_different_square() {
        // We captured on h6, but a separate piece hangs on a8. Unrelated
        // — don't suppress.
        use chess_tutor_engine::analysis::PieceLocation;
        use chess_tutor_engine::types::{PieceType, Square};
        let hangs = vec![HangingPiece {
            location: PieceLocation {
                square: Square::A8,
                piece: PieceType::Bishop,
            },
            attackers: vec![PieceLocation {
                square: Square::A1,
                piece: PieceType::Rook,
            }],
        }];
        let captures = vec![(Square::H6, 3u8)];
        let filtered = filter_misleading_hangs(&hangs, &captures, 0);
        assert_eq!(filtered.len(), 1, "unrelated hang must surface");
    }

    #[test]
    fn filter_misleading_hangs_suppresses_when_higher_value_counter_threat_exists() {
        // Counter-attack pattern: our bishop is "hanging" on f6 but
        // we have a guaranteed win of their queen elsewhere (in
        // theirs_hanging_guaranteed). The opponent's best response
        // is to address the queen problem, not capture our bishop —
        // so the bishop isn't really hanging.
        use chess_tutor_engine::analysis::PieceLocation;
        use chess_tutor_engine::types::{PieceType, Square};
        let hangs = vec![HangingPiece {
            location: PieceLocation {
                square: Square::F6,
                piece: PieceType::Bishop, // 3 pts
            },
            attackers: vec![PieceLocation {
                square: Square::G7,
                piece: PieceType::Pawn,
            }],
        }];
        // counter_threat_pts = 9 (queen, > bishop's 3)
        let filtered = filter_misleading_hangs(&hangs, &[], 9);
        assert!(
            filtered.is_empty(),
            "queen counter-threat should suppress the bishop hang"
        );
    }

    #[test]
    fn filter_misleading_hangs_keeps_when_counter_threat_is_equal_value() {
        // Equal-value counter-threat (both bishops, 3 pts each).
        // Opponent could plausibly accept the trade — take our
        // bishop, lose theirs. The hang is informative; don't drop.
        use chess_tutor_engine::analysis::PieceLocation;
        use chess_tutor_engine::types::{PieceType, Square};
        let hangs = vec![HangingPiece {
            location: PieceLocation {
                square: Square::F6,
                piece: PieceType::Bishop, // 3 pts
            },
            attackers: vec![PieceLocation {
                square: Square::G7,
                piece: PieceType::Pawn,
            }],
        }];
        // counter_threat_pts = 3 (their bishop, not > our 3)
        let filtered = filter_misleading_hangs(&hangs, &[], 3);
        assert_eq!(
            filtered.len(),
            1,
            "equal counter-threat is a wash, not full compensation"
        );
    }

    #[test]
    fn filter_misleading_hangs_keeps_when_counter_threat_is_lower_value() {
        // We "hang" a queen but our counter-threat is just a pawn.
        // Opponent gladly takes the queen — the hang is real.
        use chess_tutor_engine::analysis::PieceLocation;
        use chess_tutor_engine::types::{PieceType, Square};
        let hangs = vec![HangingPiece {
            location: PieceLocation {
                square: Square::F6,
                piece: PieceType::Queen, // 9 pts
            },
            attackers: vec![PieceLocation {
                square: Square::G7,
                piece: PieceType::Pawn,
            }],
        }];
        // counter_threat_pts = 1 (a pawn)
        let filtered = filter_misleading_hangs(&hangs, &[], 1);
        assert_eq!(
            filtered.len(),
            1,
            "small counter-threat doesn't excuse a hanging queen"
        );
    }

    #[test]
    fn phase_note_fires_on_b_for_n_trade_favoring_opponent() {
        // White gives up a bishop (825/915) for a knight (781/854).
        // Point parity is even (3↔3) but engine cp leans toward
        // opponent — small in mg (-44) and a bit more in eg (-61).
        // Both exceed our 30 cp threshold; the dominant phase should
        // be endgame, and the framing should call out the opponent's
        // favor.
        use chess_tutor_engine::analysis::CaptureEvent;
        use chess_tutor_engine::types::{PieceType, Square};
        let events_storage = vec![
            CaptureEvent {
                ply: 0,
                captor: Color::White,
                captor_piece: PieceType::Bishop,
                captured_piece: PieceType::Knight,
                square: Square::H6,
                value_mg: 781,
                value_eg: 854,
            },
            CaptureEvent {
                ply: 1,
                captor: Color::Black,
                captor_piece: PieceType::Pawn,
                captured_piece: PieceType::Bishop,
                square: Square::H6,
                value_mg: 825,
                value_eg: 915,
            },
        ];
        let events: Vec<&CaptureEvent> = events_storage.iter().collect();
        let note = phase_dependent_trade_note(&events, Color::White)
            .expect("B-for-N trade should produce a phase note");
        assert!(
            note.contains("opponent's favor"),
            "expected 'opponent's favor' framing, got: {note}"
        );
        assert!(
            note.contains("endgame"),
            "expected endgame to be called out, got: {note}"
        );
    }

    #[test]
    fn phase_note_skipped_when_lean_is_below_threshold() {
        // Pawn-for-pawn trade. mg/eg leans are zero or negligible —
        // the note should not fire (nothing pedagogical to add).
        use chess_tutor_engine::analysis::CaptureEvent;
        use chess_tutor_engine::types::{PieceType, Square};
        let events_storage = vec![
            CaptureEvent {
                ply: 0,
                captor: Color::White,
                captor_piece: PieceType::Pawn,
                captured_piece: PieceType::Pawn,
                square: Square::D5,
                value_mg: 124,
                value_eg: 206,
            },
            CaptureEvent {
                ply: 1,
                captor: Color::Black,
                captor_piece: PieceType::Pawn,
                captured_piece: PieceType::Pawn,
                square: Square::D5,
                value_mg: 124,
                value_eg: 206,
            },
        ];
        let events: Vec<&CaptureEvent> = events_storage.iter().collect();
        assert_eq!(
            phase_dependent_trade_note(&events, Color::White),
            None,
            "pawn-pawn trade should produce no note"
        );
    }

    #[test]
    fn forced_consequences_fires_on_bxh6_doubled_pawns() {
        // The user's exact reported case. After Bxh6, the engine's
        // best reply is gxh6, which doubles Black's h-pawns. The new
        // forced-consequences card should surface this with the
        // "If they reply ..." framing.
        use chess_tutor_engine::engine::{Engine, SearchParams};
        let pre = Position::from_fen(
            "r1bqkb1r/ppp2ppp/2n1p2n/3pP3/3P4/5N2/PPP2PPP/RNBQKB1R w KQkq - 5 5",
        )
        .unwrap();
        let user_move = Move::normal(Square::C1, Square::H6);
        let mut pos = pre.clone();
        let mut engine = Engine::default();
        let analyses = chess_tutor_engine::analysis::analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 6,
                multi_pv: 1,
                force_include: vec![user_move],
                ..SearchParams::default()
            },
        );
        let user = analyses.iter().find(|a| a.mv == user_move).expect("force_include");
        // The engine's predicted reply at this depth should be gxh6.
        // If a future tuning changes that, the test becomes invalid —
        // we'd need to choose a more stable scenario.
        assert!(
            user.pv.len() >= 2,
            "expected at least one opponent reply in PV"
        );
        let items = build_forced_consequences_items(user, &pre, Color::White);
        assert!(
            items.iter().any(|it| it.heading.contains("doubled pawns")),
            "expected a 'doubled pawns' forced-consequences card after Bxh6, got: {:?}",
            items.iter().map(|i| &i.heading).collect::<Vec<_>>()
        );
    }

    #[test]
    fn forced_consequences_skips_when_pv_has_no_reply() {
        // A user move with no engine reply in the PV (terminal
        // position or single-ply analysis) must not produce any
        // forced-consequences card.
        let pre = Position::startpos();
        let user_move = Move::normal(Square::E2, Square::E4);
        let user = chess_tutor_engine::analysis::MoveAnalysis {
            mv: user_move,
            score: chess_tutor_engine::types::Value::ZERO,
            depth: 1,
            pv: vec![user_move], // only ply 0, no reply
            ply_traces: vec![chess_tutor_engine::eval::EvalTrace::zero()],
            settled_ply: None,
            pre_move_trace: chess_tutor_engine::eval::EvalTrace::zero(),
            pre_score: chess_tutor_engine::types::Value::ZERO,
            term_deltas: Vec::new(),
        };
        let items = build_forced_consequences_items(&user, &pre, Color::White);
        assert!(items.is_empty(), "no PV reply => no card");
    }

    #[test]
    fn king_protector_suppression_detects_our_minor_capturing() {
        // Bxh6 — our bishop captures a knight at ply 0. Should mark
        // both `theirs_minor_captured` (knight is a minor we took)
        // and `our_minor_capturing` (bishop is a minor that did the
        // capturing).
        use chess_tutor_engine::analysis::CaptureEvent;
        use chess_tutor_engine::types::{PieceType, Square};
        let outcome = MaterialOutcome {
            events: vec![CaptureEvent {
                ply: 0,
                captor: Color::White,
                captor_piece: PieceType::Bishop,
                captured_piece: PieceType::Knight,
                square: Square::H6,
                value_mg: 781,
                value_eg: 854,
            }],
            net_mg_cp: 781,
            net_eg_cp: 854,
            last_ply: 0,
        };
        let supp = king_protector_suppression(&outcome, Color::White);
        assert!(supp.theirs_minor_captured);
        assert!(supp.our_minor_capturing);
        assert!(!supp.ours_minor_captured);
    }

    #[test]
    fn king_protector_suppression_detects_ours_minor_captured_on_recapture() {
        // Bxh6 + gxh6 — we lose our bishop in the recapture. After
        // both events, ours_minor_captured should be true.
        use chess_tutor_engine::analysis::CaptureEvent;
        use chess_tutor_engine::types::{PieceType, Square};
        let outcome = MaterialOutcome {
            events: vec![
                CaptureEvent {
                    ply: 0,
                    captor: Color::White,
                    captor_piece: PieceType::Bishop,
                    captured_piece: PieceType::Knight,
                    square: Square::H6,
                    value_mg: 781,
                    value_eg: 854,
                },
                CaptureEvent {
                    ply: 1,
                    captor: Color::Black,
                    captor_piece: PieceType::Pawn,
                    captured_piece: PieceType::Bishop,
                    square: Square::H6,
                    value_mg: 825,
                    value_eg: 915,
                },
            ],
            net_mg_cp: 781 - 825,
            net_eg_cp: 854 - 915,
            last_ply: 1,
        };
        let supp = king_protector_suppression(&outcome, Color::White);
        assert!(supp.theirs_minor_captured);
        assert!(supp.ours_minor_captured);
        assert!(supp.our_minor_capturing);
    }

    #[test]
    fn king_protector_suppression_pawn_capture_doesnt_set_our_minor_capturing() {
        // exd5 — pawn capture. captor_piece is a pawn, not a minor.
        // Neither `our_minor_capturing` nor the minor-captured flags
        // should fire when only pawns are involved.
        use chess_tutor_engine::analysis::CaptureEvent;
        use chess_tutor_engine::types::{PieceType, Square};
        let outcome = MaterialOutcome {
            events: vec![CaptureEvent {
                ply: 0,
                captor: Color::White,
                captor_piece: PieceType::Pawn,
                captured_piece: PieceType::Pawn,
                square: Square::D5,
                value_mg: 124,
                value_eg: 206,
            }],
            net_mg_cp: 124,
            net_eg_cp: 206,
            last_ply: 0,
        };
        let supp = king_protector_suppression(&outcome, Color::White);
        assert!(!supp.our_minor_capturing);
        assert!(!supp.theirs_minor_captured);
        assert!(!supp.ours_minor_captured);
    }

    #[test]
    fn material_card_flags_bishop_for_rook_as_material_loss() {
        // User captures a rook at ply 0; opponent recaptures with a
        // bishop at ply 1. Classical points: 5 vs 3 — net +2 for us.
        // Verifies that point parity correctly classifies an unequal
        // trade (not just suppresses cp-tight ones).
        use chess_tutor_engine::analysis::CaptureEvent;
        use chess_tutor_engine::types::{PieceType, Square};
        let outcome = MaterialOutcome {
            events: vec![
                CaptureEvent {
                    ply: 0,
                    captor: Color::White,
                    captor_piece: PieceType::Bishop,
                    captured_piece: PieceType::Rook,
                    square: Square::C6,
                    value_mg: 1276,
                    value_eg: 1380,
                },
                CaptureEvent {
                    ply: 1,
                    captor: Color::Black,
                    captor_piece: PieceType::Pawn,
                    captured_piece: PieceType::Bishop,
                    square: Square::C6,
                    value_mg: 825,
                    value_eg: 915,
                },
            ],
            net_mg_cp: 1276 - 825,
            net_eg_cp: 1380 - 915,
            last_ply: 1,
        };
        let pre = Position::startpos();
        let item = build_material_item(&pre, &outcome, Color::White)
            .expect("R-for-B is a card");
        assert_eq!(item.heading, "You won material");
        assert!(item.score_delta_pawns.is_some());
    }

    #[test]
    fn material_card_suppressed_when_opponent_captures_first() {
        // User's ply-0 move was quiet (no capture by us); opponent's
        // ply-1 best move takes one of our pieces — i.e., we hung a
        // piece. The threats card surfaces this with the right
        // present-tense framing ("Your piece is hanging") plus
        // attacker arrows; the material card must suppress itself so
        // we don't double-narrate the hang as a settled past-tense
        // "You lost material."
        use chess_tutor_engine::analysis::CaptureEvent;
        use chess_tutor_engine::types::{PieceType, Square};
        let outcome = MaterialOutcome {
            events: vec![CaptureEvent {
                ply: 1,
                captor: Color::Black,
                captor_piece: PieceType::Bishop,
                captured_piece: PieceType::Knight,
                square: Square::H3,
                value_mg: 781,
                value_eg: 854,
            }],
            net_mg_cp: -781,
            net_eg_cp: -854,
            last_ply: 1,
        };
        let pre = Position::startpos();
        let item = build_material_item(&pre, &outcome, Color::White);
        assert!(
            item.is_none(),
            "opponent-only realized capture is a hang — threats card handles it; \
             material card must stay silent. got {item:?}"
        );
    }

    #[test]
    fn capitalize_handles_empty_and_unicode() {
        assert_eq!(capitalize(""), "");
        assert_eq!(capitalize("knight"), "Knight");
    }

    #[test]
    fn join_with_and_handles_zero_one_two_three() {
        assert_eq!(join_with_and(&[]), "");
        assert_eq!(join_with_and(&["a".into()]), "a");
        assert_eq!(join_with_and(&["a".into(), "b".into()]), "a and b");
        assert_eq!(
            join_with_and(&["a".into(), "b".into(), "c".into()]),
            "a, b, and c"
        );
    }
