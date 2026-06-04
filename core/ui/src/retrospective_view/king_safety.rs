//! King-safety card builder.
//!
//! The prose (heading + pre→post detail, with the "you" / "they"
//! reframe and the flank-aware / direction-aware wording) is produced
//! by the shared teaching translator ([`chess_tutor_teaching`]) from a
//! [`Claim::KingSafety`]; the shared salience (per-side direction with
//! exposure-over-safer precedence, the attacker-count and threshold-
//! gated shelter clauses, the endgame shelter suppression) lives in
//! [`king_safety_claims`]. This builder owns only the *structured* card
//! surface the translator deliberately doesn't carry — the sentiment,
//! the terse stat summary, and the per-square board annotations.
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::{king_ring_and_attackers, KingSafetyOutcome};
use chess_tutor_engine::bitboard::Bitboard;
use chess_tutor_engine::pawns::king_shield_pawns;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{PieceType, Square};

use chess_tutor_teaching::claim::{
    king_safety_claims, Claim, CountShift, KingSide, PressureShift, SafetyDirection, ShelterShift,
};
use chess_tutor_teaching::phrasing::{phrase, Locale, Perspective, PhrasingContext, Verbosity};

use crate::view::{
    AnnotationKind, BoardAnnotation, RetrospectiveCategory, RetrospectiveItem, Sentiment,
};

// ---------------------------------------------------------------------
// King safety
// ---------------------------------------------------------------------

/// Build every king-safety card for one analysed move. `perspective`
/// selects "you" vs "they" and drives the student-POV sentiment colour.
///
/// `pre` / `post` are the positions immediately before and after the
/// user's move. An **exposure** card paints the exposed king's ring plus
/// an arrow from each enemy attacker to the ring square it bears on, so
/// the student sees *where* the pressure comes from. A **safer** card
/// does the mirror image: it diffs the attacker geometry across the move
/// and paints green arrows from every attacker the move *neutralized*
/// (blocked, captured, or escaped by moving the king) to the ring square
/// it used to hit — "look what you stopped."
pub(super) fn build_king_safety_items(
    outcome: &KingSafetyOutcome,
    perspective: Perspective,
    pre: &Position,
    post: &Position,
) -> Vec<RetrospectiveItem> {
    let ctx = PhrasingContext {
        perspective,
        locale: Locale::En,
        verbosity: Verbosity::Normal,
        reveal_moves: false,
    };
    king_safety_claims(outcome)
        .into_iter()
        .map(|claim| {
            let mut item = king_safety_item(&claim, &ctx);
            match &claim {
                // Exposure: the current attackers closing in on the ring.
                Claim::KingSafety {
                    direction: SafetyDirection::MoreExposed,
                    king_sq,
                    ..
                } => append_ring_and_attackers(&mut item.annotations, *king_sq, post),
                // Safer: the attackers this move took off the board / off
                // the king's lines.
                Claim::KingSafety {
                    direction: SafetyDirection::Safer,
                    king_sq,
                    shield,
                    ..
                } => {
                    append_removed_attackers(&mut item.annotations, *king_sq, pre, post);
                    // Edge case: a "safer" card driven purely by a stronger
                    // pawn shield (no attacker removed) would otherwise carry
                    // no board annotation, leaving the student guessing what
                    // changed. Highlight the shield pawns in blue to ground
                    // the wording.
                    if item.annotations.is_empty() && shield.is_some() {
                        append_shield_pawns(&mut item.annotations, *king_sq, post);
                    }
                }
                _ => {}
            }
            item
        })
        .collect()
}

/// Append the exposed king's ring (as `KingRing` square highlights) plus
/// an `Attacker` arrow from each enemy piece bearing on it to the king.
/// The king square itself already carries a highlight from
/// [`king_safety_item`], so it's skipped in the ring loop.
fn append_ring_and_attackers(anns: &mut Vec<BoardAnnotation>, king_sq: Square, post: &Position) {
    let Some(king) = post.piece_on(king_sq) else {
        return;
    };
    if king.kind() != PieceType::King {
        return;
    }
    let (ring, attackers) = king_ring_and_attackers(post, king.color());
    for sq in ring {
        if sq != king_sq {
            anns.push(BoardAnnotation::SquareHighlight {
                square: sq,
                kind: AnnotationKind::KingRing,
            });
        }
    }
    // Arrow to the ring square the attacker actually bears on, not the
    // king square — a slider rarely attacks the king itself (a bishop on
    // the long diagonal hits g3/h2 of a g1 king's ring, never g1), so an
    // arrow to the king would draw a line the piece can't make.
    for (from, target) in attackers {
        anns.push(BoardAnnotation::Arrow {
            from,
            to: target,
            kind: AnnotationKind::Attacker,
        });
    }
}

/// Paint what a "safer" move did to the king's attackers, per attack
/// *ray* (a direction an attacker bears on the ring along). For each ray
/// present before the move, compare the ring squares it hit before vs
/// after:
///
/// - **Fully neutralized** (the ray is gone — attacker captured, line
///   fully blocked, or king moved away): a green `NeutralizedAttacker`
///   arrow to the square it used to reach, plus green `FreedSquare`
///   highlights on every ring square it gave up.
/// - **Partially mitigated** (the ray survives but reaches fewer squares
///   — a blocker shortened it): a yellow `MitigatedAttacker` arrow to the
///   new farthest square it still attacks, plus green `FreedSquare`
///   highlights on the squares it no longer reaches.
/// - **Unchanged / extended**: nothing — the ray wasn't mitigated.
///
/// The freed-square diff is what distinguishes a genuine mitigation from
/// a ray whose farthest target merely *slid out* when a blocker cleared
/// (a queen reaching f1 instead of f3 — more pressure, not less): that
/// ray's pre-squares are a subset of its post-squares, so nothing is
/// freed and nothing is drawn. `king_sq` is the king's *post*-move square
/// — only its colour is used, to ask `king_ring_and_attackers` about the
/// same king in both positions; the king's own square is never flagged.
fn append_removed_attackers(
    anns: &mut Vec<BoardAnnotation>,
    king_sq: Square,
    pre: &Position,
    post: &Position,
) {
    let Some(king) = post.piece_on(king_sq) else {
        return;
    };
    if king.kind() != PieceType::King {
        return;
    }
    let color = king.color();
    let (pre_ring, pre_arrows) = king_ring_and_attackers(pre, color);
    let (post_ring, post_arrows) = king_ring_and_attackers(post, color);

    for (from, pre_target) in pre_arrows {
        let dir = ray_dir(from, pre_target);
        let post_target = post_arrows
            .iter()
            .find(|(pf, pt)| *pf == from && ray_dir(*pf, *pt) == dir)
            .map(|(_, pt)| *pt);

        let pre_squares = ray_ring_squares(from, pre_target, pre_ring);
        let post_squares = match post_target {
            Some(t) => ray_ring_squares(from, t, post_ring),
            None => Vec::new(),
        };
        let freed: Vec<Square> = pre_squares
            .into_iter()
            .filter(|s| *s != king_sq && !post_squares.contains(s))
            .collect();
        if freed.is_empty() {
            continue;
        }

        // Green arrow if the whole ray died, yellow if it only shrank.
        let (arrow_to, kind) = match post_target {
            None => (pre_target, AnnotationKind::NeutralizedAttacker),
            Some(t) => (t, AnnotationKind::MitigatedAttacker),
        };
        anns.push(BoardAnnotation::Arrow {
            from,
            to: arrow_to,
            kind,
        });
        for sq in freed {
            anns.push(BoardAnnotation::SquareHighlight {
                square: sq,
                kind: AnnotationKind::FreedSquare,
            });
        }
    }
}

/// The ring squares an attacker hits along the ray toward `target` — the
/// squares sharing the ray's direction from `from` and no farther than
/// `target`, intersected with the ring. A slider's ray is contiguous up
/// to its farthest attacked square, so this reconstructs the per-ray
/// attacked-ring set from the `(from, farthest-target)` arrow without
/// re-deriving attacks. Excludes `from` itself.
fn ray_ring_squares(from: Square, target: Square, ring: Bitboard) -> Vec<Square> {
    let dir = ray_dir(from, target);
    let max_dist = chebyshev(from, target);
    ring.into_iter()
        .filter(|&sq| sq != from && ray_dir(from, sq) == dir && chebyshev(from, sq) <= max_dist)
        .collect()
}

/// Chebyshev (king-step) distance between two squares — monotonic along a
/// ray, so it orders "how far out" each ring square sits.
fn chebyshev(a: Square, b: Square) -> i32 {
    let df = ((a.raw() & 7) as i32 - (b.raw() & 7) as i32).abs();
    let dr = ((a.raw() >> 3) as i32 - (b.raw() >> 3) as i32).abs();
    df.max(dr)
}

/// Append a blue `ShieldPawn` highlight on each friendly pawn covering the
/// king — the shelter the move strengthened. Used only as the fallback
/// visual for a shield-only "safer" card (see the caller).
fn append_shield_pawns(anns: &mut Vec<BoardAnnotation>, king_sq: Square, post: &Position) {
    let Some(king) = post.piece_on(king_sq) else {
        return;
    };
    if king.kind() != PieceType::King {
        return;
    }
    for sq in king_shield_pawns(post, king.color()) {
        anns.push(BoardAnnotation::SquareHighlight {
            square: sq,
            kind: AnnotationKind::ShieldPawn,
        });
    }
}

/// The gcd-reduced step vector from `a` to `b` — the ray direction,
/// matching the engine's `ray_arrows` grouping so two targets on the same
/// ray (e.g. a slider's farthest square before vs after a blocker clears)
/// share a key. `a != b` for any attacker→ring pair, so the gcd is ≥ 1.
fn ray_dir(a: Square, b: Square) -> (i32, i32) {
    let df = (b.raw() & 7) as i32 - (a.raw() & 7) as i32;
    let dr = (b.raw() >> 3) as i32 - (a.raw() >> 3) as i32;
    let g = gcd(df.unsigned_abs(), dr.unsigned_abs()) as i32;
    (df / g, dr / g)
}

/// Greatest common divisor (Euclid); `gcd(0, n) == n` so a pure
/// rank/file delta reduces correctly.
fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// Turn one [`Claim::KingSafety`] into a card — prose from the
/// translator, structured surface (sentiment, stat summary,
/// annotations) computed here from the claim's payload.
fn king_safety_item(claim: &Claim, ctx: &PhrasingContext) -> RetrospectiveItem {
    let phrasing = phrase(claim, ctx);
    let Claim::KingSafety {
        side,
        direction,
        attackers,
        shield,
        pressure,
        king_sq,
    } = claim
    else {
        unreachable!("king_safety_claims always returns Claim::KingSafety");
    };

    // The shifted king is the user's when the moving side is the user
    // (Player + Mover); the player's POV is fixed here.
    let king_is_user =
        (*side == KingSide::Mover) == (ctx.perspective == Perspective::Player);

    // Sentiment is a function of "good for the user?" — exposing the
    // user's own king is bad; exposing the opponent's is good.
    let sentiment = match (direction, king_is_user) {
        (SafetyDirection::MoreExposed, true) => Sentiment::Negative,
        (SafetyDirection::MoreExposed, false) => Sentiment::Positive,
        (SafetyDirection::Safer, true) => Sentiment::Positive,
        (SafetyDirection::Safer, false) => Sentiment::Negative,
    };

    // Only an exposure card marks the king square itself, as the centre
    // of the danger zone its ring highlights surround. A "safer" card no
    // longer paints a blanket green king highlight — the neutralized /
    // mitigated arrows and freed-square highlights now show precisely
    // what changed, so a green king on top of them is just noise.
    let annotations = match direction {
        SafetyDirection::MoreExposed => vec![BoardAnnotation::SquareHighlight {
            square: *king_sq,
            kind: AnnotationKind::KingRing,
        }],
        SafetyDirection::Safer => Vec::new(),
    };

    RetrospectiveItem {
        category: RetrospectiveCategory::KingSafety,
        heading: phrasing.summary,
        summary: stat_summary(*direction, attackers.as_ref(), shield.as_ref(), pressure.as_ref()),
        detail: phrasing.detail.unwrap_or_default(),
        score_delta_pawns: None,
        sentiment,
        annotations,
    }
}

/// The terse, perspective-neutral stat line shown under the heading —
/// the structured summary the translator's prose deliberately omits.
fn stat_summary(
    direction: SafetyDirection,
    attackers: Option<&CountShift>,
    shield: Option<&ShelterShift>,
    pressure: Option<&PressureShift>,
) -> String {
    let mut parts = Vec::new();
    if let Some(c) = attackers {
        match direction {
            SafetyDirection::MoreExposed => {
                parts.push(format!("{} attackers (up from {})", c.post, c.pre))
            }
            SafetyDirection::Safer => {
                parts.push(format!("attackers down to {} (from {})", c.post, c.pre))
            }
        }
    }
    if let Some(s) = shield {
        parts.push(format!("shield {:+.2}", (s.post_mg - s.pre_mg) as f32 / 100.0));
    }
    // Pressure-only fallback: show the adjacent-attack count when it
    // moved (the count-flat / danger-driven case leaves it empty — the
    // heading and the board arrows carry that story instead).
    if let Some(p) = pressure {
        if p.pre != p.post {
            match direction {
                SafetyDirection::MoreExposed => {
                    parts.push(format!("{} attacks next to the king (up from {})", p.post, p.pre))
                }
                SafetyDirection::Safer => {
                    parts.push(format!("attacks next to the king down to {} (from {})", p.post, p.pre))
                }
            }
        }
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::analysis::KingSafetySnapshot;
    use chess_tutor_engine::types::Square;

    /// Build a [`KingSafetyOutcome`] from per-side `(attackers,
    /// shield_mg)` pre/post tuples; king squares default to central
    /// (e1 / e8) so the flank wording falls back to "king ring".
    fn ks(
        ours: ((i32, i32), (i32, i32)),
        theirs: ((i32, i32), (i32, i32)),
    ) -> KingSafetyOutcome {
        ks_kings((Square::E1, Square::E1), (Square::E8, Square::E8), ours, theirs)
    }

    fn ks_kings(
        ours_kings: (Square, Square),
        theirs_kings: (Square, Square),
        ours: ((i32, i32), (i32, i32)),
        theirs: ((i32, i32), (i32, i32)),
    ) -> KingSafetyOutcome {
        let snap = |king_sq: Square, (atk, shield): (i32, i32)| KingSafetySnapshot {
            king_sq,
            attackers_count: atk,
            attacks_count: 0,
            pawn_shield_mg: shield,
            pawn_shield_eg: 0,
            pawn_storm_mg: 0,
            pawn_storm_eg: 0,
            king_pawn_distance_eg: 0,
            king_danger_mg: 0,
        };
        KingSafetyOutcome {
            ours_pre: snap(ours_kings.0, ours.0),
            ours_post: snap(ours_kings.1, ours.1),
            theirs_pre: snap(theirs_kings.0, theirs.0),
            theirs_post: snap(theirs_kings.1, theirs.1),
            phase: 128,
        }
    }

    #[test]
    fn no_shift_yields_no_card() {
        let items = build_king_safety_items(&ks(((1, 80), (1, 80)), ((0, 80), (0, 80))), Perspective::Player, &Position::startpos(), &Position::startpos());
        assert!(items.is_empty());
    }

    #[test]
    fn our_king_exposed_is_negative_with_translator_heading() {
        let items = build_king_safety_items(&ks(((1, 80), (3, 80)), ((0, 80), (0, 80))), Perspective::Player, &Position::startpos(), &Position::startpos());
        let card = items.first().expect("an exposure card");
        assert_eq!(card.heading, "Your king is more exposed: 3 attackers on the king ring (up from 1).");
        assert_eq!(card.summary, "3 attackers (up from 1)");
        assert_eq!(card.sentiment, Sentiment::Negative);
        assert!(matches!(
            card.annotations[0],
            BoardAnnotation::SquareHighlight { kind: AnnotationKind::KingRing, .. }
        ));
    }

    #[test]
    fn exposing_their_king_is_positive_opportunity() {
        let items = build_king_safety_items(&ks(((0, 80), (0, 80)), ((0, 80), (2, 80))), Perspective::Player, &Position::startpos(), &Position::startpos());
        let card = items.first().expect("a their-exposure card");
        assert!(card.heading.starts_with("You expose the opponent's king"), "{}", card.heading);
        assert_eq!(card.sentiment, Sentiment::Positive);
    }

    #[test]
    fn our_king_safer_is_positive_without_king_highlight() {
        let items = build_king_safety_items(&ks(((3, 80), (1, 80)), ((0, 80), (0, 80))), Perspective::Player, &Position::startpos(), &Position::startpos());
        let card = items.first().expect("a safer card");
        assert!(card.heading.starts_with("Your king is safer"), "{}", card.heading);
        assert_eq!(card.summary, "attackers down to 1 (from 3)");
        assert_eq!(card.sentiment, Sentiment::Positive);
        // A safer card no longer paints a blanket green king highlight —
        // the freed-square / neutralized-arrow annotations carry the story.
        // (On startpos there are no attackers to diff, so it's empty here.)
        assert!(
            !card
                .annotations
                .iter()
                .any(|a| matches!(a, BoardAnnotation::SquareHighlight { kind: AnnotationKind::GoodPiece, .. })),
            "safer card should not highlight the king green, got {:?}",
            card.annotations
        );
    }

    #[test]
    fn shelter_clause_suppressed_in_endgame() {
        let mut outcome = ks(((1, 80), (1, 20)), ((0, 80), (0, 80)));
        outcome.phase = 16;
        assert!(build_king_safety_items(&outcome, Perspective::Player, &Position::startpos(), &Position::startpos()).is_empty());
    }

    /// A "safer" card draws green NeutralizedAttacker arrows from each
    /// attacker the move removed (here a captured g3 rook that had borne
    /// on the white king's ring) to the ring square it used to hit.
    #[test]
    fn safer_card_draws_neutralized_attacker_arrows() {
        // Pre: black rook on g3 bears on the white g1 king's ring.
        // Post: the rook is gone (captured) — the pressure is neutralized.
        let pre = Position::from_fen("4k3/8/8/8/8/6r1/5PPP/6K1 w - - 0 1").unwrap();
        let post = Position::from_fen("4k3/8/8/8/8/8/5PPP/6K1 w - - 0 1").unwrap();
        let outcome = ks_kings(
            (Square::G1, Square::G1),
            (Square::E8, Square::E8),
            ((3, 80), (1, 80)), // our king's attacker count falls → Safer
            ((0, 80), (0, 80)),
        );
        let items = build_king_safety_items(&outcome, Perspective::Player, &pre, &post);
        let card = items.first().expect("a safer card");
        let from_g3 = card.annotations.iter().any(|a| {
            matches!(
                a,
                BoardAnnotation::Arrow {
                    from: Square::G3,
                    kind: AnnotationKind::NeutralizedAttacker,
                    ..
                }
            )
        });
        assert!(
            from_g3,
            "expected green neutralized-attacker arrows from g3, got {:?}",
            card.annotations
        );
    }

    /// Regression: a queen that still attacks the ring — merely reaching a
    /// farther square now that a blocker cleared (f3 → f1 down the f-file)
    /// — must NOT get a neutralized arrow. The ray survives even though its
    /// farthest target slid, so keying on direction (not target) is silent.
    #[test]
    fn safer_card_no_arrow_when_ray_only_slides_farther() {
        // Pre: white knight on f3 blocks the black queen's f-file at f3
        // (queen reaches the f3 ring square). Post: knight gone — the queen
        // now reaches f1, the same ray just longer. Still attacking.
        let pre = Position::from_fen("4k3/8/5q2/8/8/5N2/8/6K1 w - - 0 1").unwrap();
        let post = Position::from_fen("4k3/8/5q2/8/8/8/8/6K1 b - - 0 1").unwrap();
        let outcome = ks_kings(
            (Square::G1, Square::G1),
            (Square::E8, Square::E8),
            ((3, 80), (1, 80)),
            ((0, 80), (0, 80)),
        );
        let items = build_king_safety_items(&outcome, Perspective::Player, &pre, &post);
        let card = items.first().expect("a safer card");
        // The ray gained squares (f1/f2), gave up none, so no arrow of
        // either kind and no freed-square highlight should appear for it.
        let any_safer_mark = card.annotations.iter().any(|a| {
            matches!(
                a,
                BoardAnnotation::Arrow {
                    from: Square::F6,
                    kind: AnnotationKind::NeutralizedAttacker | AnnotationKind::MitigatedAttacker,
                    ..
                } | BoardAnnotation::SquareHighlight {
                    kind: AnnotationKind::FreedSquare,
                    ..
                }
            )
        });
        assert!(
            !any_safer_mark,
            "queen still attacks the ring (ray slid f3→f1), so nothing is freed, got {:?}",
            card.annotations
        );
    }

    /// Partial mitigation: a black rook's g-file ray hit g3/g2/g1 of the
    /// f1 king's ring; interposing a pawn on g2 shortens it to g3/g2. The
    /// ray survives, so it gets a yellow `MitigatedAttacker` arrow to its
    /// new farthest square (g2) plus a green `FreedSquare` on the square it
    /// gave up (g1).
    #[test]
    fn safer_card_partial_mitigation_yellow_arrow_and_freed_square() {
        let pre = Position::from_fen("k5r1/8/8/8/8/8/8/5K2 w - - 0 1").unwrap();
        let post = Position::from_fen("k5r1/8/8/8/8/8/6P1/5K2 b - - 0 1").unwrap();
        let outcome = ks_kings(
            (Square::F1, Square::F1),
            (Square::A8, Square::A8),
            ((3, 80), (1, 80)),
            ((0, 80), (0, 80)),
        );
        let items = build_king_safety_items(&outcome, Perspective::Player, &pre, &post);
        let card = items.first().expect("a safer card");
        let mitigated = card.annotations.iter().any(|a| {
            matches!(
                a,
                BoardAnnotation::Arrow {
                    from: Square::G8,
                    kind: AnnotationKind::MitigatedAttacker,
                    ..
                }
            )
        });
        let freed_g1 = card.annotations.iter().any(|a| {
            matches!(
                a,
                BoardAnnotation::SquareHighlight {
                    square: Square::G1,
                    kind: AnnotationKind::FreedSquare,
                }
            )
        });
        assert!(
            mitigated,
            "expected yellow mitigated arrow from g8, got {:?}",
            card.annotations
        );
        assert!(
            freed_g1,
            "expected freed-square highlight on g1, got {:?}",
            card.annotations
        );
    }

    /// Shield-only "safer" card (no attacker removed): falls back to blue
    /// `ShieldPawn` highlights on the pawns covering the king, so the card
    /// isn't left without any board annotation.
    #[test]
    fn safer_card_shield_only_highlights_shield_pawns_blue() {
        // No king-ring attackers in either position, so the attacker diff
        // adds nothing; the shield improved (30 → 80 cp), so the fallback
        // fires. White king g1 behind f2/g2/h2.
        let board = Position::from_fen("4k3/8/8/8/8/8/5PPP/6K1 w - - 0 1").unwrap();
        let outcome = ks_kings(
            (Square::G1, Square::G1),
            (Square::E8, Square::E8),
            ((0, 30), (0, 80)), // attackers flat, shield up → Safer (shield only)
            ((0, 80), (0, 80)),
        );
        let items = build_king_safety_items(&outcome, Perspective::Player, &board, &board);
        let card = items.first().expect("a safer card");
        let shield_pawns: Vec<Square> = card
            .annotations
            .iter()
            .filter_map(|a| match a {
                BoardAnnotation::SquareHighlight {
                    square,
                    kind: AnnotationKind::ShieldPawn,
                } => Some(*square),
                _ => None,
            })
            .collect();
        assert_eq!(
            shield_pawns,
            vec![Square::F2, Square::G2, Square::H2],
            "shield-only safer card should highlight the cover pawns, got {:?}",
            card.annotations
        );
    }

    /// Attacker count flat on the opponent's king but attacks on its
    /// adjacent (escape) squares jump — the "knight closes in" case. The
    /// number-free "more pressure" card fires even though the bare count
    /// says nothing changed, with the adjacent-attack count in the stat.
    #[test]
    fn pressure_only_card_when_attacker_count_flat() {
        let snap = |attackers_count, attacks_count| KingSafetySnapshot {
            king_sq: Square::E8,
            attackers_count,
            attacks_count,
            pawn_shield_mg: 80,
            pawn_shield_eg: 0,
            pawn_storm_mg: 0,
            pawn_storm_eg: 0,
            king_pawn_distance_eg: 0,
            king_danger_mg: 0,
        };
        let our = KingSafetySnapshot { king_sq: Square::E1, ..snap(0, 0) };
        let outcome = KingSafetyOutcome {
            ours_pre: our,
            ours_post: our,
            theirs_pre: snap(2, 1),
            theirs_post: snap(2, 3),
            phase: 128,
        };
        let items = build_king_safety_items(&outcome, Perspective::Player, &Position::startpos(), &Position::startpos());
        let card = items.first().expect("a pressure card");
        assert_eq!(card.heading, "You pile more pressure on the opponent's king.");
        assert_eq!(card.summary, "3 attacks next to the king (up from 1)");
        assert_eq!(card.sentiment, Sentiment::Positive);
        assert!(matches!(
            card.annotations[0],
            BoardAnnotation::SquareHighlight { kind: AnnotationKind::KingRing, .. }
        ));
    }

    #[test]
    fn flank_label_after_castling() {
        let outcome = ks_kings(
            (Square::E1, Square::G1),
            (Square::E8, Square::E8),
            ((0, 80), (2, 80)),
            ((0, 80), (0, 80)),
        );
        let items = build_king_safety_items(&outcome, Perspective::Player, &Position::startpos(), &Position::startpos());
        let card = items.first().expect("an exposure card");
        assert!(card.heading.contains("kingside"), "{}", card.heading);
    }
}
