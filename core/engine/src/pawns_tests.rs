use super::*;


    // ---- Starting position ------------------------------------------

    #[test]
    fn startpos_pawn_scores_are_mirrored() {
        // Starting position is perfectly symmetric in pawn structure, so
        // white's and black's pawn scores must be equal. The signed
        // aggregate score is therefore zero.
        let p = Position::startpos();
        let e = evaluate(&p);
        assert_eq!(e.scores[0], e.scores[1]);
        assert_eq!(e.score(), Score::ZERO);
    }

    #[test]
    fn startpos_has_no_passed_pawns() {
        let p = Position::startpos();
        let e = evaluate(&p);
        assert!(e.passed_pawns[0].is_empty());
        assert!(e.passed_pawns[1].is_empty());
    }

    #[test]
    fn startpos_pawn_attacks_cover_ranks_3_and_6() {
        let p = Position::startpos();
        let e = evaluate(&p);
        // White pawns on rank 2 attack every square on rank 3.
        let rank3 = crate::bitboard::RANK_3;
        assert_eq!(e.pawn_attacks[Color::White.index()], rank3);
        // Black pawns on rank 7 attack every square on rank 6.
        let rank6 = crate::bitboard::RANK_6;
        assert_eq!(e.pawn_attacks[Color::Black.index()], rank6);
    }

    // ---- Passed pawn detection --------------------------------------

    #[test]
    fn isolated_advanced_pawn_is_passed() {
        // White pawn on d7 with no black pawn in front or on adjacent
        // files: a textbook passed pawn.
        let p = Position::from_fen("4k3/3P4/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let e = evaluate(&p);
        assert!(e.passed_pawns[Color::White.index()].contains(Square::D7));
    }

    #[test]
    fn pawn_with_stopper_is_not_passed() {
        // White d4, black d5 directly blocks it. Not passed.
        let p = Position::from_fen("4k3/8/8/3p4/3P4/8/8/4K3 w - - 0 1").unwrap();
        let e = evaluate(&p);
        assert!(e.passed_pawns[Color::White.index()].is_empty());
    }

    #[test]
    fn pawn_with_unlevered_adjacent_stopper_ahead_is_not_passed() {
        // White e4 with a black pawn on f6 — the f6 pawn defends f5 and
        // would attack e5 if we push. It's a stopper the e4 pawn cannot
        // capture, and we don't outnumber it on the phalanx, so this is
        // not a passed pawn. (Contrast with e4 + black-d5, which Stockfish
        // *does* consider passed because e4xd5 clears the path.)
        let p = Position::from_fen("4k3/8/5p2/8/4P3/8/8/4K3 w - - 0 1").unwrap();
        let e = evaluate(&p);
        assert!(e.passed_pawns[Color::White.index()].is_empty());
    }

    // ---- Structural penalties ---------------------------------------

    #[test]
    fn doubled_pawns_cost_more_than_undoubled() {
        // Identical otherwise, but white has two pawns stacked on the
        // e-file (e2 and e3). The doubled pawn penalty applies to the
        // back pawn which has no support on adjacent files.
        let doubled = Position::from_fen("4k3/8/8/8/8/4P3/4P3/4K3 w - - 0 1").unwrap();
        let singled = Position::from_fen("4k3/8/8/8/8/8/4P3/4K3 w - - 0 1").unwrap();
        let d = evaluate(&doubled);
        let s = evaluate(&singled);
        assert!(
            d.scores[Color::White.index()].mg().0 < s.scores[Color::White.index()].mg().0,
            "doubled pawn should score worse than a single pawn"
        );
    }

    #[test]
    fn isolated_pawn_costs_more_than_connected_pair() {
        // Isolated: a single pawn on d4. Connected pair: c4 and d4. The
        // isolated case should score lower than the connected case.
        let isolated = Position::from_fen("4k3/8/8/8/3P4/8/8/4K3 w - - 0 1").unwrap();
        let connected = Position::from_fen("4k3/8/8/8/2PP4/8/8/4K3 w - - 0 1").unwrap();
        let i = evaluate(&isolated);
        let c = evaluate(&connected);
        assert!(
            i.scores[Color::White.index()].mg().0
                < c.scores[Color::White.index()].mg().0
                    / i32::max(connected.count(Color::White, PieceType::Pawn) as i32, 1),
            "isolated pawn should be worse than one pawn within a connected pair"
        );
    }

    // ---- Attack-span ------------------------------------------------

    #[test]
    fn attacks_span_extends_to_promotion_for_healthy_pawn() {
        // A lone white pawn on e4 with no obstructions — attack span
        // covers d5..d8, f5..f8 (plus the immediate d5/f5 pawn attacks).
        let p = Position::from_fen("4k3/8/8/8/4P3/8/8/4K3 w - - 0 1").unwrap();
        let e = evaluate(&p);
        let span = e.pawn_attacks_span[Color::White.index()];
        for sq in &[
            Square::D5,
            Square::D6,
            Square::D7,
            Square::D8,
            Square::F5,
            Square::F6,
            Square::F7,
            Square::F8,
        ] {
            assert!(span.contains(*sq), "span should contain {:?}", sq);
        }
    }

    // ---- King safety ------------------------------------------------

    #[test]
    fn intact_white_shelter_scores_better_than_exposed_king() {
        // Kinged on g1 with the f2/g2/h2 trio intact vs. the same king but
        // all three shelter pawns pushed one rank forward (weaker shelter).
        let intact = Position::from_fen("4k3/8/8/8/8/8/5PPP/6K1 w - - 0 1").unwrap();
        let pushed = Position::from_fen("4k3/8/8/8/8/5PPP/8/6K1 w - - 0 1").unwrap();
        let a = king_safety(&intact, Color::White).total();
        let b = king_safety(&pushed, Color::White).total();
        assert!(
            a.mg().0 > b.mg().0,
            "intact f2/g2/h2 shelter ({}) should beat f3/g3/h3 ({})",
            a.mg().0,
            b.mg().0,
        );
    }

    #[test]
    fn king_safety_is_equal_for_mirrored_positions() {
        // A position and its colour-flipped mirror produce the same score
        // for each side's own king.
        let white_fen = "4k3/8/8/8/8/8/5PPP/6K1 w - - 0 1";
        let black_fen = "6k1/5ppp/8/8/8/8/8/4K3 w - - 0 1";
        let w = Position::from_fen(white_fen).unwrap();
        let b = Position::from_fen(black_fen).unwrap();
        assert_eq!(
            king_safety(&w, Color::White).total().mg(),
            king_safety(&b, Color::Black).total().mg(),
            "mirrored king safety should agree"
        );
    }

    #[test]
    fn king_far_from_pawns_gets_endgame_penalty() {
        // King on a1, only pawn on h7 — maximum king-pawn distance. The
        // eg component should be strictly more negative than a variant
        // where the king sits next to the pawn.
        let far = Position::from_fen("4k3/7P/8/8/8/8/8/K7 w - - 0 1").unwrap();
        let near = Position::from_fen("4k3/7P/6K1/8/8/8/8/8 w - - 0 1").unwrap();
        let a = king_safety(&far, Color::White).total();
        let b = king_safety(&near, Color::White).total();
        assert!(
            a.eg().0 < b.eg().0,
            "distant king should score worse in the endgame half"
        );
    }

    #[test]
    fn shelter_components_sum_to_legacy_aggregate() {
        // The split must be exact: pawn_shield + pawn_storm +
        // king_pawn_distance == the score the pre-split function
        // returned (which equals SHELTER_BASE + SHELTER_STRENGTH +
        // STORM penalties + KING_TO_NEAREST_PAWN_PENALTY_EG terms).
        let positions = [
            "4k3/8/8/8/8/8/5PPP/6K1 w - - 0 1",
            "4k3/7P/8/8/8/8/8/K7 w - - 0 1",
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1",
        ];
        for fen in positions {
            let pos = Position::from_fen(fen).unwrap();
            let comps = king_safety(&pos, pos.side_to_move());
            let summed = comps.pawn_shield + comps.pawn_storm + comps.king_pawn_distance;
            assert_eq!(
                comps.total(),
                summed,
                "components must sum to total for {fen}",
            );
        }
    }

    // ---- Determinism ------------------------------------------------

    #[test]
    fn evaluate_is_pure() {
        // Calling evaluate twice on the same position must yield identical
        // results. Guards against accidental reliance on hidden state.
        let p = Position::from_fen(
            "r1bqkb1r/ppp2ppp/2np1n2/4p3/2B1P3/2N2N2/PPPP1PPP/R1BQK2R w KQkq - 0 5",
        )
        .unwrap();
        let a = evaluate(&p);
        let b = evaluate(&p);
        assert_eq!(a, b);
    }

    // ---- Spot check: symmetric pawn arrangement -----------------

    #[test]
    fn symmetric_pawns_produce_zero_signed_score() {
        // Same pawn structure for both colours (vertically mirrored) =>
        // signed pawn score is zero.
        let p = Position::from_fen("4k3/1p1p1p1p/8/8/8/8/1P1P1P1P/4K3 w - - 0 1").unwrap();
        let e = evaluate(&p);
        assert_eq!(e.score(), Score::ZERO);
        assert_eq!(e.scores[0], e.scores[1]);
        // And the passed-pawn sets are also mirrored.
        assert_eq!(e.passed_pawns[0].popcount(), e.passed_pawns[1].popcount());
    }

    // ---- PawnsBreakdown granular attribution ------------------------

    fn white_breakdown(fen: &str) -> PawnsBreakdown {
        let p = Position::from_fen(fen).unwrap();
        evaluate(&p).breakdowns[Color::White.index()]
    }

    #[test]
    fn breakdown_total_sums_every_sub_term() {
        // total() must equal the sum of every field. A future refactor
        // that adds a field but forgets to update total() would drift
        // silently — this test catches that.
        let b = white_breakdown("4k3/1p1p1p1p/8/8/8/8/1P1P1P1P/4K3 w - - 0 1");
        let manual =
            b.connected + b.isolated + b.backward + b.doubled + b.weak_unopposed + b.weak_lever;
        assert_eq!(b.total(), manual);
    }

    #[test]
    fn breakdown_total_equals_scores_field() {
        // scores[c] is a cached sum of the per-colour breakdown — the two
        // must be identical by construction.
        let p = Position::from_fen("4k3/1p1p1p1p/8/8/8/8/1P1P1P1P/4K3 w - - 0 1").unwrap();
        let e = evaluate(&p);
        for &c in &Color::both() {
            assert_eq!(
                e.scores[c.index()],
                e.breakdowns[c.index()].total(),
                "scores and breakdown.total() must agree for {:?}",
                c
            );
        }
    }

    #[test]
    fn isolated_pawn_lands_on_isolated_and_weak_unopposed_fields() {
        // Lone white pawn on d4 — isolated (no c/e neighbours) and
        // unopposed (no black pawn on d-file ahead). Connected must stay
        // at zero; backward / doubled must stay at zero.
        let b = white_breakdown("4k3/8/8/8/3P4/8/8/4K3 w - - 0 1");
        assert_eq!(b.isolated, Score::ZERO - ISOLATED);
        assert_eq!(b.weak_unopposed, Score::ZERO - WEAK_UNOPPOSED);
        assert_eq!(b.connected, Score::ZERO);
        assert_eq!(b.backward, Score::ZERO);
        assert_eq!(b.doubled, Score::ZERO);
        assert_eq!(b.weak_lever, Score::ZERO);
    }

    #[test]
    fn connected_pair_lands_on_connected_field() {
        // Phalanx c4-d4 — both pawns have a same-rank neighbour. The
        // connected field accumulates the rank-scaled bonus; isolated /
        // backward / doubled all stay at zero.
        let b = white_breakdown("4k3/8/8/8/2PP4/8/8/4K3 w - - 0 1");
        assert!(
            b.connected.mg().0 > 0,
            "phalanx should award a positive connected bonus, got {:?}",
            b.connected
        );
        assert_eq!(b.isolated, Score::ZERO);
        assert_eq!(b.backward, Score::ZERO);
        assert_eq!(b.doubled, Score::ZERO);
    }

    #[test]
    fn doubled_pawn_lands_on_doubled_field() {
        // e2 / e3 stacked — the front pawn is "doubled" in Stockfish
        // terms (it has a same-colour pawn directly behind it) and has
        // no support from adjacent files. Doubled field picks up one
        // -DOUBLED penalty; isolated fires on both pawns (no neighbours).
        let b = white_breakdown("4k3/8/8/8/8/4P3/4P3/4K3 w - - 0 1");
        assert_eq!(
            b.doubled,
            Score::ZERO - DOUBLED,
            "exactly one doubled penalty on the stacked pair"
        );
    }

    #[test]
    fn backward_pawn_lands_on_backward_field() {
        // White pawn on b2, black pawn directly in front on b3 (blocks
        // the push), white neighbour on a3 so b2 is not isolated. b2's
        // only neighbour sits on rank 3 — not strictly behind the push
        // square b3 — so "no advancing neighbour" holds and b2 meets the
        // backward predicate. The a3 pawn itself is not backward; it
        // contributes a connected-bonus via b2's support, but that lands
        // in a separate field we don't assert here.
        let b = white_breakdown("4k3/8/8/8/8/Pp6/1P6/4K3 w - - 0 1");
        assert_eq!(
            b.backward,
            Score::ZERO - BACKWARD,
            "b2 blocked by b3 with no advancing a-file neighbour should be backward"
        );
        assert_eq!(b.weak_unopposed, Score::ZERO, "b2 is opposed by b3");
        assert_eq!(b.doubled, Score::ZERO);
        assert_eq!(b.weak_lever, Score::ZERO);
    }

    // ---- Mirror symmetry of the breakdown ---------------------------

    #[test]
    fn mirrored_positions_produce_mirrored_breakdowns() {
        // Colour-flipped mirror positions produce equal per-colour
        // breakdowns for the relevant side.
        let white = Position::from_fen("4k3/8/8/8/3P4/8/8/4K3 w - - 0 1").unwrap();
        let black = Position::from_fen("4k3/8/8/3p4/8/8/8/4K3 w - - 0 1").unwrap();
        let w = evaluate(&white).breakdowns[Color::White.index()];
        let b = evaluate(&black).breakdowns[Color::Black.index()];
        assert_eq!(w.isolated, b.isolated);
        assert_eq!(w.weak_unopposed, b.weak_unopposed);
        assert_eq!(w.total(), b.total());
    }
