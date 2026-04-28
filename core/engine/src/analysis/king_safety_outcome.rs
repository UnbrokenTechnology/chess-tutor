//! [`KingSafetyOutcome`] — raw scalar snapshots of king-safety
//! signals at the pre-move position and the position immediately
//! after the user's move, for both kings. The CLI diffs snapshots
//! to narrate lines like *"Your king is more exposed: 2 attackers
//! on the kingside (up from 0)."*

use super::{post_user_move, MoveAnalysis};
use crate::position::Position;
use crate::types::{Color, Square};

/// Raw king-safety signals captured at a single position. The
/// scalars come straight from the Stockfish-11 king-safety
/// machinery; `king_sq` is stored alongside so renderers can
/// categorize the king's location (kingside / queenside / center)
/// without needing the `Position` back.
///
/// - `king_sq` — our king's square at this snapshot.
/// - `attackers_count` — number of distinct enemy pieces attacking
///   any square in our king ring. Sourced from
///   [`crate::eval::Evaluator::king_attackers_count`].
/// - `attacks_count` — total enemy attacks landing on squares
///   immediately adjacent to our king.
/// - `pawn_shield_mg` / `pawn_shield_eg` — friendly-pawn-shield
///   bonus (positive = better shield). The teaching surface for
///   "you castled" / "h-pawn push cracked your shield."
/// - `pawn_storm_mg` / `pawn_storm_eg` — enemy-pawn-storm penalty,
///   stored negated so positive = less storm pressure. Surfaced
///   separately so storm shifts (which Stockfish's tables score
///   non-monotonically in rank) don't get narrated as shield
///   shifts.
/// - `king_pawn_distance_eg` — endgame king-to-nearest-own-pawn
///   penalty (mg = 0). Mostly noise outside endgames.
///
/// Units: counts are raw; shelter components are in engine-cp.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct KingSafetySnapshot {
    pub king_sq: Square,
    pub attackers_count: i32,
    pub attacks_count: i32,
    pub pawn_shield_mg: i32,
    pub pawn_shield_eg: i32,
    pub pawn_storm_mg: i32,
    pub pawn_storm_eg: i32,
    pub king_pawn_distance_eg: i32,
}

/// Pre/post snapshots of the king-safety signals on both sides.
/// Callers diff `*_pre` vs `*_post` to answer "did this move
/// expose someone?".
///
/// POV convention: `ours_*` snapshots refer to the user's king
/// (`root_stm`); `theirs_*` to the opponent's king.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct KingSafetyOutcome {
    pub ours_pre: KingSafetySnapshot,
    pub ours_post: KingSafetySnapshot,
    pub theirs_pre: KingSafetySnapshot,
    pub theirs_post: KingSafetySnapshot,
    /// Game-phase blend at the post-move position. `128` = pure
    /// midgame, `0` = pure endgame. UI layers use this to suppress
    /// shelter narration in late endgames where pawn cover is no
    /// longer the primary king-safety concern.
    pub phase: i32,
}

impl KingSafetyOutcome {
    pub fn ours_attackers_delta(&self) -> i32 {
        self.ours_post.attackers_count - self.ours_pre.attackers_count
    }
    pub fn ours_attacks_delta(&self) -> i32 {
        self.ours_post.attacks_count - self.ours_pre.attacks_count
    }
    pub fn ours_pawn_shield_mg_delta(&self) -> i32 {
        self.ours_post.pawn_shield_mg - self.ours_pre.pawn_shield_mg
    }
    pub fn ours_pawn_storm_mg_delta(&self) -> i32 {
        self.ours_post.pawn_storm_mg - self.ours_pre.pawn_storm_mg
    }
    pub fn theirs_attackers_delta(&self) -> i32 {
        self.theirs_post.attackers_count - self.theirs_pre.attackers_count
    }
    pub fn theirs_attacks_delta(&self) -> i32 {
        self.theirs_post.attacks_count - self.theirs_pre.attacks_count
    }
    pub fn theirs_pawn_shield_mg_delta(&self) -> i32 {
        self.theirs_post.pawn_shield_mg - self.theirs_pre.pawn_shield_mg
    }
    pub fn theirs_pawn_storm_mg_delta(&self) -> i32 {
        self.theirs_post.pawn_storm_mg - self.theirs_pre.pawn_storm_mg
    }
}

/// Build a fresh `Evaluator` for `pos`, run the standard
/// `initialize` + `pieces::evaluate` priming for both colours, then
/// extract the king-safety signals affecting `our_color`'s king.
///
/// Priming order matches the real evaluator — `initialize(W)` +
/// `initialize(B)` populate king rings and pawn/king attack tables;
/// `pieces::evaluate(W)` + `pieces::evaluate(B)` walk
/// knights/bishops/rooks/queens and bump `king_attackers_count` /
/// `king_attacks_count` as each piece touches the enemy king ring.
fn snapshot_king_safety(pos: &Position, our_color: Color) -> KingSafetySnapshot {
    let mut e = crate::eval::Evaluator::new(pos);
    e.initialize(Color::White);
    e.initialize(Color::Black);
    crate::eval::pieces::evaluate(&mut e, Color::White);
    crate::eval::pieces::evaluate(&mut e, Color::Black);
    let enemy = !our_color;
    let shelter = crate::pawns::king_safety(pos, our_color);
    KingSafetySnapshot {
        king_sq: pos.king_square(our_color),
        attackers_count: e.king_attackers_count[enemy.index()],
        attacks_count: e.king_attacks_count[enemy.index()],
        pawn_shield_mg: shelter.pawn_shield.mg().0,
        pawn_shield_eg: shelter.pawn_shield.eg().0,
        pawn_storm_mg: shelter.pawn_storm.mg().0,
        pawn_storm_eg: shelter.pawn_storm.eg().0,
        king_pawn_distance_eg: shelter.king_pawn_distance.eg().0,
    }
}

/// Snapshot king safety at the pre-move position and at the position
/// immediately after the user's move, on both sides.
pub fn compute_king_safety_outcome(
    ma: &MoveAnalysis,
    pre_move_pos: &Position,
    root_stm: Color,
) -> KingSafetyOutcome {
    let ours_pre = snapshot_king_safety(pre_move_pos, root_stm);
    let theirs_pre = snapshot_king_safety(pre_move_pos, !root_stm);

    let scratch = post_user_move(pre_move_pos, ma);

    let ours_post = snapshot_king_safety(&scratch, root_stm);
    let theirs_post = snapshot_king_safety(&scratch, !root_stm);
    let phase = crate::material::evaluate(&scratch).game_phase.0;

    KingSafetyOutcome {
        ours_pre,
        ours_post,
        theirs_pre,
        theirs_post,
        phase,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::ma_with_pv;
    use super::*;
    use crate::types::Move;

    #[test]
    fn snapshot_king_safety_startpos_has_zero_attackers_and_is_symmetric() {
        let pos = Position::startpos();
        let w = snapshot_king_safety(&pos, Color::White);
        let b = snapshot_king_safety(&pos, Color::Black);
        assert_eq!(w.attackers_count, 0);
        assert_eq!(b.attackers_count, 0);
        assert_eq!(w.attacks_count, 0);
        assert_eq!(b.attacks_count, 0);
        assert_eq!(
            w.pawn_shield_mg, b.pawn_shield_mg,
            "startpos pawn_shield_mg should be symmetric"
        );
        assert_eq!(
            w.pawn_storm_mg, b.pawn_storm_mg,
            "startpos pawn_storm_mg should be symmetric"
        );
        assert_eq!(w.king_sq, Square::E1);
        assert_eq!(b.king_sq, Square::E8);
    }

    #[test]
    fn snapshot_king_safety_picks_up_rook_on_king_flank() {
        let fen = "4k3/8/8/8/8/6r1/5PPP/6K1 w - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let snap = snapshot_king_safety(&pos, Color::White);
        assert!(
            snap.attackers_count >= 1,
            "expected ≥1 black attacker on white's king ring, got {}",
            snap.attackers_count,
        );
        assert!(
            snap.attacks_count >= 1,
            "expected ≥1 attack landing adjacent to white king, got {}",
            snap.attacks_count,
        );
    }

    #[test]
    fn snapshot_king_safety_sheltered_king_scores_better_than_exposed() {
        let sheltered_fen = "4k3/8/8/8/8/8/5PPP/6K1 w - - 0 1";
        let exposed_fen = "4k3/8/8/8/8/5P1P/6P1/6K1 w - - 0 1";
        let s = snapshot_king_safety(&Position::from_fen(sheltered_fen).unwrap(), Color::White);
        let x = snapshot_king_safety(&Position::from_fen(exposed_fen).unwrap(), Color::White);
        // The pawn-shield component is the friendly-pawn-cover term;
        // an exposed king has gaps in the shield, so its mg is
        // strictly smaller.
        assert!(
            s.pawn_shield_mg > x.pawn_shield_mg,
            "sheltered pawn_shield_mg ({}) should beat exposed ({})",
            s.pawn_shield_mg,
            x.pawn_shield_mg,
        );
    }

    #[test]
    fn compute_king_safety_outcome_detects_new_attacker_on_their_king() {
        let pre_fen = "4k3/8/8/8/8/8/8/R6K w - - 0 1";
        let pre = Position::from_fen(pre_fen).unwrap();
        let pre_theirs = snapshot_king_safety(&pre, Color::Black);
        assert_eq!(
            pre_theirs.attackers_count, 0,
            "pre-move should have no white attackers on black's king ring",
        );

        let mv = Move::normal(Square::A1, Square::D1);
        let ma = ma_with_pv(vec![mv], Some(0));
        let outcome = compute_king_safety_outcome(&ma, &pre, Color::White);
        assert!(
            outcome.theirs_attackers_delta() >= 1,
            "Rd1 should add at least one attacker on the black king ring, got delta {} (pre={}, post={})",
            outcome.theirs_attackers_delta(),
            outcome.theirs_pre.attackers_count,
            outcome.theirs_post.attackers_count,
        );
    }

    #[test]
    fn king_safety_outcome_delta_accessors_are_post_minus_pre() {
        let outcome = KingSafetyOutcome {
            ours_pre: KingSafetySnapshot {
                king_sq: Square::E1,
                attackers_count: 1,
                attacks_count: 2,
                pawn_shield_mg: 80,
                pawn_shield_eg: 4,
                pawn_storm_mg: -20,
                pawn_storm_eg: 0,
                king_pawn_distance_eg: -8,
            },
            ours_post: KingSafetySnapshot {
                king_sq: Square::G1,
                attackers_count: 3,
                attacks_count: 5,
                pawn_shield_mg: 30,
                pawn_shield_eg: 0,
                pawn_storm_mg: -50,
                pawn_storm_eg: 0,
                king_pawn_distance_eg: -8,
            },
            theirs_pre: KingSafetySnapshot {
                king_sq: Square::E8,
                attackers_count: 0,
                attacks_count: 0,
                pawn_shield_mg: 90,
                pawn_shield_eg: 5,
                pawn_storm_mg: -10,
                pawn_storm_eg: 0,
                king_pawn_distance_eg: -8,
            },
            theirs_post: KingSafetySnapshot {
                king_sq: Square::E8,
                attackers_count: 2,
                attacks_count: 3,
                pawn_shield_mg: 50,
                pawn_shield_eg: 2,
                pawn_storm_mg: -25,
                pawn_storm_eg: 0,
                king_pawn_distance_eg: -8,
            },
            phase: 128,
        };
        assert_eq!(outcome.ours_attackers_delta(), 2);
        assert_eq!(outcome.ours_attacks_delta(), 3);
        assert_eq!(outcome.ours_pawn_shield_mg_delta(), -50);
        assert_eq!(outcome.ours_pawn_storm_mg_delta(), -30);
        assert_eq!(outcome.theirs_attackers_delta(), 2);
        assert_eq!(outcome.theirs_attacks_delta(), 3);
        assert_eq!(outcome.theirs_pawn_shield_mg_delta(), -40);
        assert_eq!(outcome.theirs_pawn_storm_mg_delta(), -15);
    }

    #[test]
    fn snapshot_king_safety_records_king_square() {
        let pos = Position::startpos();
        let w = snapshot_king_safety(&pos, Color::White);
        let b = snapshot_king_safety(&pos, Color::Black);
        assert_eq!(w.king_sq, Square::E1);
        assert_eq!(b.king_sq, Square::E8);
    }
}
