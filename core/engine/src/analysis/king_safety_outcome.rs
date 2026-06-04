//! [`KingSafetyOutcome`] — raw scalar snapshots of king-safety
//! signals at the pre-move position and the position immediately
//! after the user's move, for both kings. The CLI diffs snapshots
//! to narrate lines like *"Your king is more exposed: 2 attackers
//! on the kingside (up from 0)."*

use super::{post_user_move, MoveAnalysis};
use crate::bitboard::Bitboard;
use crate::position::Position;
use crate::types::{Color, PieceType, Square};

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
/// - `king_danger_mg` — the holistic king-danger pressure: the
///   magnitude of the quadratic king-safety penalty Stockfish derives
///   from attacker count × weight, weak ring squares, safe checks,
///   pinned defenders, attacks on adjacent squares, and flank pressure.
///   Positive = the king is under more pressure. This is the aggregate
///   that moves even when the *distinct attacker count* is flat (e.g. a
///   piece that blocks one attacker while a closer one takes its place),
///   so it's the signal that backs "your king is under more pressure"
///   when the bare count says nothing changed.
///
/// Units: counts are raw; shelter components and `king_danger_mg` are in
/// engine-cp.
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
    pub king_danger_mg: i32,
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
    pub fn ours_king_danger_delta(&self) -> i32 {
        self.ours_post.king_danger_mg - self.ours_pre.king_danger_mg
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
    pub fn theirs_king_danger_delta(&self) -> i32 {
        self.theirs_post.king_danger_mg - self.theirs_pre.king_danger_mg
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
    // The full king-safety aggregate needs the attack/mobility tables the
    // priming above populated; `danger` is the negated quadratic penalty,
    // so flip the sign for a "higher = more pressure" magnitude.
    let king = crate::eval::king::evaluate(&e, our_color);
    KingSafetySnapshot {
        king_sq: pos.king_square(our_color),
        attackers_count: e.king_attackers_count[enemy.index()],
        attacks_count: e.king_attacks_count[enemy.index()],
        pawn_shield_mg: shelter.pawn_shield.mg().0,
        pawn_shield_eg: shelter.pawn_shield.eg().0,
        pawn_storm_mg: shelter.pawn_storm.mg().0,
        pawn_storm_eg: shelter.pawn_storm.eg().0,
        king_pawn_distance_eg: shelter.king_pawn_distance.eg().0,
        king_danger_mg: -king.danger.mg().0,
    }
}

/// The `king_color` king's ring (the danger-zone bitboard the
/// king-safety term scores against) plus, for every enemy piece bearing
/// on it, **one `(from, target)` arrow per direction it attacks the ring
/// in**. Primes the opt-in tracker so the attacker set matches exactly
/// what `king_attackers_count` counted. The retrospective paints the
/// ring + these arrows, so an exposure card shows the pressure *closing
/// in* on the king.
///
/// Each arrow runs to the ring square *farthest* from the attacker in
/// that direction, so a slider's single arrow visually runs through the
/// nearer ring squares it also attacks — a rook on the d-file against an
/// e-file king draws one line covering d6/d7/d8 (the whole escape file
/// cut off). Squares are grouped by their gcd-reduced step vector from
/// the attacker: a slider's collinear ring squares collapse to one
/// arrow, while a knight's jumps (never collinear) and a close queen's
/// separate rays (file, diagonal, rank) each get their own. A slider
/// never attacks the king square itself, so an arrow to the king would
/// misrepresent the geometry (a bishop bearing on g3/h2 of a g1 king
/// ring, never on g1).
pub fn king_ring_and_attackers(
    pos: &Position,
    king_color: Color,
) -> (Bitboard, Vec<(Square, Square)>) {
    let mut e = crate::eval::Evaluator::new(pos);
    e.per_piece_king_attacker = Some(Vec::new());
    e.initialize(Color::White);
    e.initialize(Color::Black);
    crate::eval::pieces::evaluate(&mut e, Color::White);
    crate::eval::pieces::evaluate(&mut e, Color::Black);
    let ring = e.king_ring[king_color.index()];
    let enemy = !king_color;

    // Sliders / knights, from the per-piece tracker primed above.
    let mut attackers: Vec<(Square, Square)> = e
        .per_piece_king_attacker
        .take()
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, threatened, _)| *threatened == king_color)
        .flat_map(|(from, _, attacked_ring)| {
            ray_arrows(from, attacked_ring)
                .into_iter()
                .map(move |target| (from, target))
        })
        .collect();

    // Pawns aren't iterated by `pieces::evaluate`, so the per-piece
    // tracker never records them — but a pawn's diagonal attack on a ring
    // square is real pressure (it cuts off an escape square and is folded
    // into `king_attackers_count`), so it deserves an arrow too. A pawn
    // attacks at most two squares, in different directions, so `ray_arrows`
    // emits one arrow per ring square it hits.
    for pawn in pos.pieces_of(enemy, PieceType::Pawn) {
        let attacked_ring = crate::attacks::pawn_attacks_from(enemy, pawn) & ring;
        for target in ray_arrows(pawn, attacked_ring) {
            attackers.push((pawn, target));
        }
    }

    (ring, attackers)
}

/// Reduce the ring squares an attacker bears on to one target per
/// direction — the farthest square in each — for the king-safety arrows.
/// Direction is the gcd-reduced step vector from `from`, so collinear
/// (slider-ray) squares share a direction and a knight's targets don't.
/// `attacked_ring` never contains `from`, so each delta is non-zero and
/// the gcd is ≥ 1. Encounter order is the ascending-square iteration of
/// the bitboard, keeping the output deterministic.
fn ray_arrows(from: Square, attacked_ring: Bitboard) -> Vec<Square> {
    let ff = (from.raw() & 7) as i32;
    let fr = (from.raw() >> 3) as i32;
    // (direction, farthest-square-so-far) per encountered direction.
    let mut groups: Vec<((i32, i32), Square)> = Vec::new();
    for to in attacked_ring {
        let df = (to.raw() & 7) as i32 - ff;
        let dr = (to.raw() >> 3) as i32 - fr;
        let g = gcd(df.unsigned_abs(), dr.unsigned_abs()) as i32;
        let dir = (df / g, dr / g);
        let dist = crate::bitboard::king_distance(from, to);
        match groups.iter_mut().find(|(d, _)| *d == dir) {
            Some(entry) if crate::bitboard::king_distance(from, entry.1) < dist => entry.1 = to,
            Some(_) => {}
            None => groups.push((dir, to)),
        }
    }
    groups.into_iter().map(|(_, sq)| sq).collect()
}

/// Greatest common divisor (Euclid). `gcd(0, n) == n`, so a pure
/// rank/file delta reduces correctly (e.g. `(0, 3) → (0, 1)`).
fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
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
    fn king_ring_and_attackers_names_the_attacker_square() {
        // Black rook on g3 bears on White's g1 king ring along three
        // directions: down the g-file onto g2 (an escape square), and
        // along rank 3 onto f3 and h3 (the ring's two-rank extension).
        let pos = Position::from_fen("4k3/8/8/8/8/6r1/5PPP/6K1 w - - 0 1").unwrap();
        let (ring, attackers) = king_ring_and_attackers(&pos, Color::White);
        assert!(ring.any(), "white king ring should be non-empty");
        let mut targets: Vec<Square> = attackers
            .iter()
            .filter(|(from, _)| *from == Square::G3)
            .map(|(_, to)| *to)
            .collect();
        assert!(
            !targets.is_empty(),
            "expected the g3 rook among white-king attackers, got {attackers:?}"
        );
        targets.sort_by_key(|s| s.raw());
        // The rook reaches g2 (blocked there by the pawn, never g1), f3,
        // and h3 — one arrow per ray direction, each to the farthest
        // attacked ring square (here a single square per ray). Sorted by
        // square index: g2 (14), f3 (21), h3 (23).
        assert_eq!(targets, vec![Square::G2, Square::F3, Square::H3]);
    }

    #[test]
    fn pawn_attacking_the_ring_gets_an_arrow() {
        // White pawn on d6 attacks c7 and e7; e7 is an escape square of
        // the e8 king (c7 is off the ring). Pawns aren't iterated by the
        // piece evaluator, so this checks the dedicated pawn pass.
        let pos = Position::from_fen("4k3/8/3P4/8/8/8/8/4K3 w - - 0 1").unwrap();
        let (_, attackers) = king_ring_and_attackers(&pos, Color::Black);
        let pawn_targets: Vec<Square> = attackers
            .iter()
            .filter(|(from, _)| *from == Square::D6)
            .map(|(_, to)| *to)
            .collect();
        assert_eq!(
            pawn_targets,
            vec![Square::E7],
            "d6 pawn should draw one arrow to e7 (the ring square it attacks), got {attackers:?}"
        );
    }

    #[test]
    fn ray_arrows_close_queen_draws_one_per_ray() {
        // White queen on d6, Black king on e8. The queen bears on the
        // ring in three directions: up the d-file (escape file → d8),
        // along rank 6 (→ f6), and up the d6-f8 diagonal (→ f8). Each is
        // its own arrow; the queen never lines up on the king square.
        let pos = Position::from_fen("4k3/8/3Q4/8/8/8/8/K7 w - - 0 1").unwrap();
        let (_, attackers) = king_ring_and_attackers(&pos, Color::Black);
        let mut targets: Vec<Square> = attackers
            .iter()
            .filter(|(from, _)| *from == Square::D6)
            .map(|(_, to)| *to)
            .collect();
        targets.sort_by_key(|s| s.raw());
        // f6 (45), d8 (59), f8 (61).
        assert_eq!(targets, vec![Square::F6, Square::D8, Square::F8]);
    }

    #[test]
    fn ray_arrows_farthest_per_direction_for_open_file_rook() {
        // White rook on d3, Black king on e8: the d-file is the king's
        // escape file. The rook attacks d6/d7/d8 of the ring — all one
        // direction, so a single arrow runs to the farthest (d8), through
        // d6/d7.
        let pos = Position::from_fen("4k3/8/8/8/8/3R4/8/4K3 w - - 0 1").unwrap();
        let (_, attackers) = king_ring_and_attackers(&pos, Color::Black);
        let d_file_targets: Vec<Square> = attackers
            .iter()
            .filter(|(from, _)| *from == Square::D3)
            .map(|(_, to)| *to)
            .collect();
        assert_eq!(
            d_file_targets,
            vec![Square::D8],
            "open-file rook should draw one arrow to the farthest ring square (d8), got {attackers:?}"
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
                king_danger_mg: 100,
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
                king_danger_mg: 250,
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
                king_danger_mg: 0,
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
                king_danger_mg: 120,
            },
            phase: 128,
        };
        assert_eq!(outcome.ours_attackers_delta(), 2);
        assert_eq!(outcome.ours_attacks_delta(), 3);
        assert_eq!(outcome.ours_king_danger_delta(), 150);
        assert_eq!(outcome.theirs_king_danger_delta(), 120);
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
