//! Per-piece-type positional evaluation and mobility accumulation.
//!
//! For each minor or major piece we compute the squares it attacks
//! (with bishop / rook x-rays through queens and doubled rooks), clip
//! attack sets for pinned pieces, update the shared [`Evaluator`] attack
//! tables, count king-ring pressure, accumulate mobility, and apply the
//! piece-type-specific bonuses:
//!
//! - **Knights** / **Bishops**: outpost bonus (on a reachable outpost
//!   square supported by our pawns and out of reach of enemy pawns),
//!   knight-only reachable-outpost bonus, minor-behind-pawn bonus, and a
//!   penalty proportional to distance from our king.
//! - **Bishops**: additional pawns-on-same-colour-squares penalty scaled
//!   by blocked centre-file pawns, plus a long-diagonal bonus when the
//!   bishop sees both centre squares through pawns. Chess960's corner
//!   bishop trap is deliberately not ported.
//! - **Rooks**: rook-on-queen-file, rook-on-(semi-)open-file, and a
//!   trapped-by-own-king penalty.
//! - **Queens**: WeakQueen penalty when the queen has any slider x-ray
//!   threat against it.
//!
//! Numerical parameters carry over from Stockfish 11 verbatim — they are
//! factual data, not expression.

use super::Evaluator;
use crate::attacks::{attacks_bb, square_distance};
use crate::bitboard::{file_bb, Bitboard, CENTER, CENTER_FILES, RANK_3, RANK_4, RANK_5, RANK_6};
use crate::types::{CastlingRights, Color, Direction, File, PieceType, Score, Square};

// =========================================================================
// Weight tables
// =========================================================================

/// Mobility bonuses by attacked-square count, one row per piece type.
/// Unused entries beyond each piece's maximum mobility are never indexed
/// in practice but kept zero for safety.
const MOBILITY_KNIGHT: [Score; 9] = [
    Score::new(-62, -81),
    Score::new(-53, -56),
    Score::new(-12, -30),
    Score::new(-4, -14),
    Score::new(3, 8),
    Score::new(13, 15),
    Score::new(22, 23),
    Score::new(28, 27),
    Score::new(33, 33),
];
const MOBILITY_BISHOP: [Score; 14] = [
    Score::new(-48, -59),
    Score::new(-20, -23),
    Score::new(16, -3),
    Score::new(26, 13),
    Score::new(38, 24),
    Score::new(51, 42),
    Score::new(55, 54),
    Score::new(63, 57),
    Score::new(63, 65),
    Score::new(68, 73),
    Score::new(81, 78),
    Score::new(81, 86),
    Score::new(91, 88),
    Score::new(98, 97),
];
const MOBILITY_ROOK: [Score; 15] = [
    Score::new(-58, -76),
    Score::new(-27, -18),
    Score::new(-15, 28),
    Score::new(-10, 55),
    Score::new(-5, 69),
    Score::new(-2, 82),
    Score::new(9, 112),
    Score::new(16, 118),
    Score::new(30, 132),
    Score::new(29, 142),
    Score::new(32, 155),
    Score::new(38, 165),
    Score::new(46, 166),
    Score::new(48, 169),
    Score::new(58, 171),
];
const MOBILITY_QUEEN: [Score; 28] = [
    Score::new(-39, -36),
    Score::new(-21, -15),
    Score::new(3, 8),
    Score::new(3, 18),
    Score::new(14, 34),
    Score::new(22, 54),
    Score::new(28, 61),
    Score::new(41, 73),
    Score::new(43, 79),
    Score::new(48, 92),
    Score::new(56, 94),
    Score::new(60, 104),
    Score::new(60, 113),
    Score::new(66, 120),
    Score::new(67, 123),
    Score::new(70, 126),
    Score::new(71, 133),
    Score::new(73, 136),
    Score::new(79, 140),
    Score::new(88, 143),
    Score::new(88, 148),
    Score::new(99, 166),
    Score::new(102, 170),
    Score::new(102, 175),
    Score::new(106, 184),
    Score::new(109, 191),
    Score::new(113, 206),
    Score::new(116, 212),
];

/// King-attack weight per piece type. Indexed by `PieceType::index()`
/// (1..=6); slot 0 unused.
const KING_ATTACK_WEIGHT: [i32; 7] = [0, 0, 81, 52, 44, 10, 0];

/// Rook on a semi-open / fully-open file.
const ROOK_ON_FILE: [Score; 2] = [Score::new(21, 4), Score::new(47, 25)];

const BISHOP_PAWNS: Score = Score::new(3, 7);
const KING_PROTECTOR: Score = Score::new(7, 8);
const LONG_DIAGONAL_BISHOP: Score = Score::new(45, 0);
const MINOR_BEHIND_PAWN: Score = Score::new(18, 3);
const OUTPOST: Score = Score::new(30, 21);
const REACHABLE_OUTPOST: Score = Score::new(32, 10);
const ROOK_ON_QUEEN_FILE: Score = Score::new(7, 6);
const TRAPPED_ROOK: Score = Score::new(52, 10);
const WEAK_QUEEN: Score = Score::new(49, 15);

// =========================================================================
// Per-sub-term breakdown
// =========================================================================

/// Per-colour breakdown of the per-piece positional score into its named
/// sub-terms. Each field maps to a chess concept a student can read about
/// — knight outposts, rooks on open files, trapped rooks, etc. Values are
/// cumulative across all of this colour's relevant pieces; bonuses are
/// positive, penalties are negative.
///
/// The sum of all fields equals the aggregate per-piece positional score
/// this colour contributes — see [`total`](PiecesBreakdown::total).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PiecesBreakdown {
    /// Bonus for a knight or bishop standing on a supported square in
    /// enemy territory that cannot be challenged by an enemy pawn.
    /// Knights score twice as much on the same square as bishops.
    pub outposts: Score,
    /// Bonus for a knight that can jump to an outpost square currently
    /// unoccupied by our own piece.
    pub reachable_outposts: Score,
    /// Bonus for a knight or bishop with any pawn (either colour)
    /// directly in front of it — the pawn shields the minor from a
    /// frontal attack.
    pub minor_behind_pawn: Score,
    /// Per-step penalty proportional to the chebyshev distance between
    /// each minor piece and our own king — minors that stray stop
    /// defending the king.
    pub king_protector: Score,
    /// Penalty for pawns standing on the same colour of squares as a
    /// bishop, scaled by how many of our centre-file pawns are blocked.
    pub bishop_pawns: Score,
    /// Bonus for a bishop that sees both central squares through pawn
    /// obstructions (pawns-only occupancy).
    pub long_diagonal_bishop: Score,
    /// Bonus for a rook sharing a file with any queen (either colour).
    pub rook_on_queen_file: Score,
    /// Bonus for a rook on a fully open file (no pawns of either colour
    /// on the file).
    pub rook_on_open_file: Score,
    /// Bonus for a rook on a semi-open file (no pawn of our own colour
    /// on the file, but the enemy has one).
    pub rook_on_semiopen_file: Score,
    /// Penalty for a rook with very little scope whose natural
    /// development is blocked by our own king on the same side — heavier
    /// when we can no longer castle to free it.
    pub trapped_rook: Score,
    /// Penalty for a queen whose file, rank, or diagonal has an enemy
    /// slider one piece-removal away from attacking it.
    pub weak_queen: Score,
}

impl PiecesBreakdown {
    /// An all-zero breakdown.
    pub const fn zero() -> PiecesBreakdown {
        PiecesBreakdown {
            outposts: Score::ZERO,
            reachable_outposts: Score::ZERO,
            minor_behind_pawn: Score::ZERO,
            king_protector: Score::ZERO,
            bishop_pawns: Score::ZERO,
            long_diagonal_bishop: Score::ZERO,
            rook_on_queen_file: Score::ZERO,
            rook_on_open_file: Score::ZERO,
            rook_on_semiopen_file: Score::ZERO,
            trapped_rook: Score::ZERO,
            weak_queen: Score::ZERO,
        }
    }

    /// Sum of every sub-term. Equal to the aggregate per-piece positional
    /// score this colour contributes (excluding mobility, which is
    /// accumulated separately on the evaluator).
    pub fn total(&self) -> Score {
        self.outposts
            + self.reachable_outposts
            + self.minor_behind_pawn
            + self.king_protector
            + self.bishop_pawns
            + self.long_diagonal_bishop
            + self.rook_on_queen_file
            + self.rook_on_open_file
            + self.rook_on_semiopen_file
            + self.trapped_rook
            + self.weak_queen
    }
}

// =========================================================================
// Public entry point
// =========================================================================

/// Evaluate all our knights, bishops, rooks, and queens, in that order,
/// and return the per-sub-term breakdown for `us`. Side effects: updates
/// the attacker tables, the doubly-attacked set, king-attacker tallies,
/// and the per-colour mobility running total on `e`.
pub(crate) fn evaluate(e: &mut Evaluator<'_>, us: Color) -> PiecesBreakdown {
    let mut breakdown = PiecesBreakdown::zero();
    for &pt in &[
        PieceType::Knight,
        PieceType::Bishop,
        PieceType::Rook,
        PieceType::Queen,
    ] {
        evaluate_piece_type(e, us, pt, &mut breakdown);
    }
    breakdown
}

fn evaluate_piece_type(
    e: &mut Evaluator<'_>,
    us: Color,
    pt: PieceType,
    breakdown: &mut PiecesBreakdown,
) {
    let them = !us;
    let us_idx = us.index();
    let them_idx = them.index();
    let pos = e.pos;

    let king_sq = pos.king_square(us);
    let their_king_ring = e.king_ring[them_idx];
    let pinned = pos.blockers_for_king(us);

    // Reset this piece-type's attack bitboard for us; we're about to
    // rebuild it from the piece iteration below.
    e.attacked_by[us_idx][pt.index()] = Bitboard::EMPTY;

    let pieces = pos.pieces_of(us, pt);
    if pieces.is_empty() {
        return;
    }

    let down = Direction(-Direction::pawn_push(us).0);
    let outpost_ranks = match us {
        Color::White => RANK_4 | RANK_5 | RANK_6,
        Color::Black => RANK_5 | RANK_4 | RANK_3,
    };
    let outpost_squares = outpost_ranks
        & e.attacked_by[us_idx][PieceType::Pawn.index()]
        & !e.pawns.pawn_attacks_span[them_idx];

    for s in pieces {
        // Attack set, with x-rays for sliders so long-range piece
        // contributions "see through" queens (and for rooks, through
        // doubled friendly rooks).
        let occupancy = match pt {
            PieceType::Bishop => pos.occupied() ^ pos.pieces(PieceType::Queen),
            PieceType::Rook => {
                pos.occupied() ^ pos.pieces(PieceType::Queen) ^ pos.pieces_of(us, PieceType::Rook)
            }
            _ => pos.occupied(),
        };
        let mut attacks = attacks_bb(pt, s, occupancy);

        // Pinned pieces can only legally move along the pin line, so
        // their "effective" attack set shrinks to the line through the
        // king and the piece.
        if pinned.contains(s) {
            attacks &= crate::attacks::line_bb(king_sq, s);
        }

        // Bookkeeping: update the shared attacker tables.
        e.attacked_by_2[us_idx] |= e.attacked_by_all[us_idx] & attacks;
        e.attacked_by[us_idx][pt.index()] |= attacks;
        e.attacked_by_all[us_idx] |= attacks;

        // King-ring pressure — anything hitting the enemy king's
        // neighbourhood pays down the king-danger score aggregated in
        // the king-safety term.
        if (attacks & their_king_ring).any() {
            e.king_attackers_count[us_idx] += 1;
            e.king_attackers_weight[us_idx] += KING_ATTACK_WEIGHT[pt.index()];
            e.king_attacks_count[us_idx] +=
                (attacks & e.attacked_by[them_idx][PieceType::King.index()]).popcount() as i32;
        }

        // Mobility: number of mobility-area squares this piece attacks.
        // Accumulated per-piece-type on the granular [`MobilityBreakdown`];
        // call `.total()` on the breakdown for the aggregate the main
        // evaluator used pre-split.
        let mobility_squares = attacks & e.mobility_area[us_idx];
        let mob = mobility_squares.popcount() as usize;
        let mob_score = mobility_bonus(pt, mob);
        e.mobility[us_idx].add_for(pt, mob_score);
        // Opt-in per-piece bookkeeping (None on the hot search path,
        // Some on analysis snapshots — see [`Evaluator::per_piece_mobility`]).
        if let Some(vec) = e.per_piece_mobility.as_mut() {
            vec.push((s, us, pt, mob_score, mobility_squares));
        }

        match pt {
            PieceType::Knight => {
                accumulate_minor_piece_bonuses(
                    breakdown,
                    e,
                    us,
                    pt,
                    s,
                    attacks,
                    outpost_squares,
                    down,
                    king_sq,
                );
            }
            PieceType::Bishop => {
                accumulate_minor_piece_bonuses(
                    breakdown,
                    e,
                    us,
                    pt,
                    s,
                    attacks,
                    outpost_squares,
                    down,
                    king_sq,
                );
                accumulate_bishop_specific_bonuses(breakdown, e, us, s);
            }
            PieceType::Rook => {
                accumulate_rook_bonuses(breakdown, pos, us, s, mob as i32, king_sq);
            }
            PieceType::Queen => {
                accumulate_queen_bonuses(breakdown, pos, us, s);
            }
            _ => {}
        }
    }
}

// =========================================================================
// Per-term helpers
// =========================================================================

fn mobility_bonus(pt: PieceType, mob: usize) -> Score {
    match pt {
        PieceType::Knight => MOBILITY_KNIGHT[mob.min(MOBILITY_KNIGHT.len() - 1)],
        PieceType::Bishop => MOBILITY_BISHOP[mob.min(MOBILITY_BISHOP.len() - 1)],
        PieceType::Rook => MOBILITY_ROOK[mob.min(MOBILITY_ROOK.len() - 1)],
        PieceType::Queen => MOBILITY_QUEEN[mob.min(MOBILITY_QUEEN.len() - 1)],
        _ => Score::ZERO,
    }
}

/// Shared knight/bishop bonuses: outpost, reachable outpost (knight only),
/// minor-behind-pawn, and king-protector distance penalty. Each weight
/// lands on its own field of `breakdown`.
#[allow(clippy::too_many_arguments)]
fn accumulate_minor_piece_bonuses(
    breakdown: &mut PiecesBreakdown,
    e: &Evaluator<'_>,
    us: Color,
    pt: PieceType,
    s: Square,
    attacks: Bitboard,
    outpost_squares: Bitboard,
    down: Direction,
    king_sq: Square,
) {
    let pos = e.pos;

    // Outpost: on an outpost square. Knights benefit twice as much as
    // bishops on the same square.
    if outpost_squares.contains(s) {
        let multiplier = if pt == PieceType::Knight { 2 } else { 1 };
        breakdown.outposts += OUTPOST * multiplier;
    } else if pt == PieceType::Knight {
        // Reachable outpost: knight can jump to an outpost square not
        // currently occupied by our own piece.
        let reachable = outpost_squares & attacks & !pos.pieces_by_color(us);
        if reachable.any() {
            breakdown.reachable_outposts += REACHABLE_OUTPOST;
        }
    }

    // Minor-behind-pawn: our minor has any pawn (either colour) directly
    // in front of it. `down` shifts the pawns bitboard backward from our
    // POV, so a pawn at s + up appears on s after the shift.
    if pos.pieces(PieceType::Pawn).shift(down).contains(s) {
        breakdown.minor_behind_pawn += MINOR_BEHIND_PAWN;
    }

    // King-protector: minor that strays from our king pays a small
    // per-step penalty.
    breakdown.king_protector -= KING_PROTECTOR * square_distance(s, king_sq) as i32;
}

fn accumulate_bishop_specific_bonuses(
    breakdown: &mut PiecesBreakdown,
    e: &Evaluator<'_>,
    us: Color,
    s: Square,
) {
    let pos = e.pos;

    // Pawns-on-same-colour-squares penalty, scaled by our centre-file
    // pawns that are already blocked (a bishop fighting its own blocked
    // pawns on the centre is especially bad).
    let blocked_centre = pos.pieces_of(us, PieceType::Pawn)
        & pos.occupied().shift(Direction(-Direction::pawn_push(us).0))
        & CENTER_FILES;
    let same_colour_pawns = pos.pawns_on_same_color_squares(us, s) as i32;
    breakdown.bishop_pawns -=
        BISHOP_PAWNS * same_colour_pawns * (1 + blocked_centre.popcount() as i32);

    // Long-diagonal bishop: seeing both centre squares through pawns.
    // The "pawns-only" occupancy lets minor pieces ignored, so the
    // bishop x-rays through them.
    let through_pawns = attacks_bb(PieceType::Bishop, s, pos.pieces(PieceType::Pawn));
    if (through_pawns & CENTER).more_than_one() {
        breakdown.long_diagonal_bishop += LONG_DIAGONAL_BISHOP;
    }

    // Chess960 cornered-bishop penalty is deliberately skipped; we only
    // play standard chess.
}

fn accumulate_rook_bonuses(
    breakdown: &mut PiecesBreakdown,
    pos: &crate::position::Position,
    us: Color,
    s: Square,
    mob: i32,
    king_sq: Square,
) {
    let them = !us;

    // Rook on the same file as a queen (either colour).
    if (file_bb(s.file()) & pos.pieces(PieceType::Queen)).any() {
        breakdown.rook_on_queen_file += ROOK_ON_QUEEN_FILE;
    }

    // Rook on a (semi-)open file. Fully open = neither side has a pawn
    // on the file; semi-open = our side has none but they do.
    if pos.is_on_semiopen_file(us, s) {
        if pos.is_on_semiopen_file(them, s) {
            // Both sides clear of the file — fully open.
            breakdown.rook_on_open_file += ROOK_ON_FILE[1];
        } else {
            // Our side clear, they still have a pawn — semi-open.
            breakdown.rook_on_semiopen_file += ROOK_ON_FILE[0];
        }
    } else if mob <= 3 {
        // Trapped-by-king: when the rook has very little scope, check
        // whether our king is on the same side as the rook (blocking
        // its natural development). The multiplier is 2 when we can't
        // castle any more (that rook has nowhere to go), 1 otherwise.
        let king_file = king_sq.file();
        let rook_file = s.file();
        let king_is_queenside = king_file < File::E;
        let rook_is_left_of_king = rook_file < king_file;
        if king_is_queenside == rook_is_left_of_king {
            let can_castle = pos
                .castling_rights()
                .intersects(CastlingRights::for_color(us));
            let multiplier = if can_castle { 1 } else { 2 };
            breakdown.trapped_rook -= TRAPPED_ROOK * multiplier;
        }
    }
}

fn accumulate_queen_bonuses(
    breakdown: &mut PiecesBreakdown,
    pos: &crate::position::Position,
    us: Color,
    s: Square,
) {
    let them = !us;

    // WeakQueen: any enemy rook or bishop is one removal away from
    // attacking our queen. Because slider_blockers strictly reports
    // pieces between an aligned sniper and `s`, any non-empty blockers
    // set means the queen has an x-ray threat hovering over it.
    let enemy_rb = pos.pieces_of(them, PieceType::Rook) | pos.pieces_of(them, PieceType::Bishop);
    let (blockers, _) = pos.slider_blockers(enemy_rb, s);
    if blockers.any() {
        breakdown.weak_queen -= WEAK_QUEEN;
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::Position;

    /// Run the full piece-evaluation pipeline for `us` and return the
    /// per-sub-term breakdown. Mirrors the bootstrap pattern described in
    /// the eval-module tests: build the scratchpad, initialize both
    /// colours' attack tables, then evaluate.
    fn piece_breakdown(fen: &str, us: Color) -> PiecesBreakdown {
        let pos = Position::from_fen(fen).unwrap();
        let mut e = Evaluator::new(&pos);
        e.initialize(Color::White);
        e.initialize(Color::Black);
        // Evaluate both colours so any cross-colour state (attack tables)
        // is fully populated — matches how the main evaluator runs.
        let _other = evaluate(&mut e, !us);
        evaluate(&mut e, us)
    }

    // ---- Breakdown totals stay consistent with the aggregate -----------

    #[test]
    fn breakdown_total_sums_every_sub_term() {
        // The per-colour total() must equal the sum of every public
        // sub-term field. A future refactor that adds a field but forgets
        // to update total() would silently drift; this test catches that.
        let fen = "r1bqkb1r/ppp2ppp/2np1n2/4p3/2B1P3/2N2N2/PPPP1PPP/R1BQK2R w KQkq - 0 5";
        let b = piece_breakdown(fen, Color::White);
        let manual = b.outposts
            + b.reachable_outposts
            + b.minor_behind_pawn
            + b.king_protector
            + b.bishop_pawns
            + b.long_diagonal_bishop
            + b.rook_on_queen_file
            + b.rook_on_open_file
            + b.rook_on_semiopen_file
            + b.trapped_rook
            + b.weak_queen;
        assert_eq!(b.total(), manual);
    }

    // ---- rook_on_open_file vs rook_on_semiopen_file attribution --------

    #[test]
    fn rook_on_fully_open_file_lands_on_open_field_only() {
        // White rook on e1, no pawns on the e-file for either colour.
        // Pawns off-file so the rook is on a fully open file.
        // Matches Stockfish's ROOK_ON_FILE[1] = Score(47, 25).
        let fen = "4k3/1pp3pp/8/8/8/8/PPP3PP/4R2K w - - 0 1";
        let b = piece_breakdown(fen, Color::White);
        assert_eq!(b.rook_on_open_file, ROOK_ON_FILE[1]);
        assert_eq!(b.rook_on_semiopen_file, Score::ZERO);
    }

    #[test]
    fn rook_on_semiopen_file_lands_on_semiopen_field_only() {
        // White rook on e1, white has no pawn on e-file but black does
        // (e5). ROOK_ON_FILE[0] fires; the fully-open field stays zero.
        let fen = "4k3/1pp3pp/8/4p3/8/8/PPP3PP/4R2K w - - 0 1";
        let b = piece_breakdown(fen, Color::White);
        assert_eq!(b.rook_on_semiopen_file, ROOK_ON_FILE[0]);
        assert_eq!(b.rook_on_open_file, Score::ZERO);
    }

    // ---- outposts vs reachable_outposts attribution --------------------

    #[test]
    fn knight_on_outpost_lands_on_outposts_field() {
        // White knight on d5, supported by white pawn on c4. Black has
        // no pawns on c or e files ahead (ranks 6+), so d5 is an outpost.
        // Knight multiplier is ×2, so the field holds OUTPOST * 2.
        let fen = "4k3/8/8/3N4/2P5/8/8/4K3 w - - 0 1";
        let b = piece_breakdown(fen, Color::White);
        assert_eq!(b.outposts, OUTPOST * 2);
        assert_eq!(b.reachable_outposts, Score::ZERO);
    }

    #[test]
    fn knight_reaching_outpost_lands_on_reachable_outposts_field() {
        // Knight on f3 can jump to d4 / e5 — both supported by a pawn on
        // c3 / d4? We want exactly one reachable outpost and no direct
        // outpost. Knight on f3, pawn on c4, knight sees e5 but e5 isn't
        // an outpost without c4 supporting... Simpler: knight on c3 with
        // pawn on c4 supports d5 as an outpost target; the knight can
        // jump to d5 or b5. d5 is attacked by c4, b5 is attacked by c4
        // too — so they're both outposts. The knight is not on an
        // outpost itself.
        let fen = "4k3/8/8/8/2P5/2N5/8/4K3 w - - 0 1";
        let b = piece_breakdown(fen, Color::White);
        assert_eq!(b.outposts, Score::ZERO);
        assert_eq!(b.reachable_outposts, REACHABLE_OUTPOST);
    }

    // ---- weak_queen attribution ----------------------------------------

    #[test]
    fn weak_queen_fires_under_xray_slider_threat() {
        // White queen on d1, black rook on d8 — aligned with one piece
        // (white pawn on d5) between them. The rook x-rays the queen:
        // remove the pawn and the rook pins/attacks the queen. Expect
        // -WEAK_QUEEN in the breakdown.
        let fen = "3rk3/8/8/3P4/8/8/8/3QK3 w - - 0 1";
        let b = piece_breakdown(fen, Color::White);
        assert_eq!(b.weak_queen, Score::ZERO - WEAK_QUEEN);
    }

    #[test]
    fn weak_queen_absent_without_xray_threat() {
        // White queen on d1 with no aligned enemy rook/bishop.
        let fen = "4k3/8/8/8/8/8/8/3QK3 w - - 0 1";
        let b = piece_breakdown(fen, Color::White);
        assert_eq!(b.weak_queen, Score::ZERO);
    }

    // ---- long_diagonal_bishop attribution ------------------------------

    #[test]
    fn long_diagonal_bishop_lands_on_its_own_field() {
        // Bishop on b2 sees the long a1-h8 diagonal through pawns — its
        // pawns-only x-ray hits both d4 and e5 (the CENTER squares). No
        // friendly pawn is standing on the diagonal to block the test.
        let fen = "4k3/8/8/8/8/8/1B6/4K3 w - - 0 1";
        let b = piece_breakdown(fen, Color::White);
        assert_eq!(b.long_diagonal_bishop, LONG_DIAGONAL_BISHOP);
    }

    // ---- Per-colour symmetry mirrors -----------------------------------

    #[test]
    fn mirrored_positions_produce_mirrored_breakdowns() {
        // Colour-flipped mirror positions should produce equal breakdowns
        // for the relevant side. The total() on both sides must agree.
        let white_fen = "4k3/8/8/3N4/2P5/8/8/4K3 w - - 0 1";
        let black_fen = "4k3/8/8/2p5/3n4/8/8/4K3 w - - 0 1";
        let w = piece_breakdown(white_fen, Color::White);
        let b = piece_breakdown(black_fen, Color::Black);
        assert_eq!(w.outposts, b.outposts);
        assert_eq!(w.minor_behind_pawn, b.minor_behind_pawn);
        assert_eq!(w.total(), b.total());
    }
}
