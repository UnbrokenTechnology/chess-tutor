//! Endgame bitbases — precomputed win/draw tables for specific
//! material signatures too subtle for classical evaluation.
//!
//! Currently holds one table: **KPK** (king + pawn vs king). For every
//! legal configuration of white king, white pawn (restricted to files
//! A-D; the other half mirrors), black king, and side-to-move, the
//! bitbase records whether the position is a *win* for the strong side
//! with perfect play. Positions absent from the table are drawn.
//!
//! The table is built by retrograde analysis at first use: seed every
//! entry with its immediate classification (invalid position, immediate
//! win by safe promotion, immediate draw by stalemate or pawn capture)
//! and then iterate, propagating WIN and DRAW backwards one ply at a
//! time until no UNKNOWN entries change. Takes ~15 sweeps and a handful
//! of ms on a modern CPU. Wrapped in [`LazyLock`] so the cost is paid
//! once per process and only if the engine ever touches a KPK position.

use std::sync::LazyLock;

use crate::attacks::{square_distance, KING_ATTACKS, PAWN_ATTACKS};
use crate::bitboard::square_bb;
use crate::types::{Color, File, Rank, Square};

// =========================================================================
// Index format
// =========================================================================
//
// There are 2 × 24 × 64 × 64 = 196_608 positions the bitbase tracks:
//
// - side to move (2)
// - pawn square on files A-D, ranks 2-7 (4 × 6 = 24)
// - strong king on any of 64 squares
// - weak king on any of 64 squares
//
// Each entry is 1 bit. Positions on files E-H are mirrored to A-D
// before probing, so the caller must normalise before calling [`probe`].

const BITBASE_ENTRIES: usize = 2 * 24 * 64 * 64;
const BITBASE_WORDS: usize = BITBASE_ENTRIES / 32;

fn kpk_index(stm: Color, strong_ksq: Square, weak_ksq: Square, pawn_sq: Square) -> usize {
    let stm_bit = match stm {
        Color::White => 0,
        Color::Black => 1,
    };
    let file = pawn_sq.file() as usize;
    // Encode rank as `R7 - rank` (where R7's internal index is 6). Maps
    // rank 7 → 0 and rank 2 → 5. This collapses the 6 legal pawn ranks
    // into the low 3 bits of the encoding, keeping the total index
    // inside 196_608 = 2 × 24 × 64 × 64.
    let rank_enc = (Rank::R7 as usize) - (pawn_sq.rank() as usize);
    strong_ksq.index() | (weak_ksq.index() << 6) | (stm_bit << 12) | (file << 13) | (rank_enc << 15)
}

// =========================================================================
// Probe
// =========================================================================

/// `true` iff the strong side (with king `strong_ksq` and pawn
/// `pawn_sq`) wins a K+P vs K endgame against a weak king on `weak_ksq`
/// with `stm` to move. Caller is responsible for normalising the
/// position so the strong side is treated as white and the pawn is on
/// files A-D — see [`normalize`].
pub fn kpk_probe(strong_ksq: Square, pawn_sq: Square, weak_ksq: Square, stm: Color) -> bool {
    debug_assert!(pawn_sq.file() as usize <= File::D as usize);
    let idx = kpk_index(stm, strong_ksq, weak_ksq, pawn_sq);
    (BITBASE[idx / 32] & (1u32 << (idx & 31))) != 0
}

/// Map a square into the coordinate frame the bitbase expects:
/// - strong side is white (black is vertically flipped);
/// - the pawn sits on files A-D (the board is horizontally mirrored
///   through the central file when the pawn starts on E-H).
///
/// Caller passes the strong side's pawn square via `pawn_sq` so this
/// helper knows whether a file-mirror is needed; pass the same value
/// for all three squares (king squares + pawn itself) so every piece
/// lands in the same mirrored frame.
pub fn normalize(strong_side: Color, pawn_sq: Square, sq: Square) -> Square {
    let mirrored = if (pawn_sq.file() as usize) >= File::E as usize {
        // xor with 7 mirrors the file: a → h, b → g, etc.
        Square::from_index(sq.index() as u8 ^ 7)
    } else {
        sq
    };
    match strong_side {
        Color::White => mirrored,
        Color::Black => mirrored.flip_vertical(),
    }
}

// =========================================================================
// Table build
// =========================================================================

static BITBASE: LazyLock<Vec<u32>> = LazyLock::new(build_bitbase);

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Classification {
    /// Illegal position (kings adjacent, piece overlap, stm-white but
    /// white pawn attacks the black king). Never contributes to result.
    Invalid,
    /// Not yet decided; needs more retrograde sweeps.
    Unknown,
    /// Drawn with perfect play.
    Draw,
    /// Winning for the strong side with perfect play.
    Win,
}

impl Classification {
    /// Bit flag used to OR results together during retrograde classification.
    /// Reserved values match the reference (INVALID=0, UNKNOWN=1, DRAW=2, WIN=4).
    fn flag(self) -> u8 {
        match self {
            Classification::Invalid => 0,
            Classification::Unknown => 1,
            Classification::Draw => 2,
            Classification::Win => 4,
        }
    }
}

#[derive(Copy, Clone, Debug)]
struct Entry {
    stm: Color,
    strong_ksq: Square,
    weak_ksq: Square,
    pawn_sq: Square,
    result: Classification,
}

impl Entry {
    /// Seed one entry with its immediate classification: illegal if
    /// the three pieces conflict or the strong-to-move pawn already
    /// attacks the weak king; immediate WIN if strong-to-move and the
    /// pawn can promote safely; immediate DRAW if weak-to-move and the
    /// weak king is either stalemated or can capture an undefended
    /// pawn; otherwise UNKNOWN.
    fn from_index(idx: usize) -> Entry {
        let strong_ksq = Square::from_index((idx & 0x3F) as u8);
        let weak_ksq = Square::from_index(((idx >> 6) & 0x3F) as u8);
        let stm = if ((idx >> 12) & 1) == 0 {
            Color::White
        } else {
            Color::Black
        };
        let pawn_file =
            File::from_index(((idx >> 13) & 0x3) as u8).expect("file index in range 0..4");
        // Inverse of the encoding in `kpk_index`: decoded rank is
        // `R7 - rank_enc`, so encodings 0..5 map to ranks 7..2.
        let rank_enc = (idx >> 15) & 0x7;
        let pawn_rank = Rank::from_index(((Rank::R7 as usize) - rank_enc) as u8)
            .expect("rank index in range 2..=7");
        let pawn_sq = Square::new(pawn_file, pawn_rank);

        let result = initial_classification(stm, strong_ksq, weak_ksq, pawn_sq);
        Entry {
            stm,
            strong_ksq,
            weak_ksq,
            pawn_sq,
            result,
        }
    }

    /// Retrograde-classify this entry by inspecting the results of
    /// every position reachable in one ply. Returns the new
    /// classification (which may still be UNKNOWN if some children
    /// are themselves UNKNOWN and no shortcut fires).
    fn reclassify(&self, db: &[Entry]) -> Classification {
        // Strong-to-move wants a WIN child; weak-to-move wants a DRAW
        // child. "Good" and "bad" follow from the side-to-move.
        let (good, bad, next_stm) = match self.stm {
            Color::White => (Classification::Win, Classification::Draw, Color::Black),
            Color::Black => (Classification::Draw, Classification::Win, Color::White),
        };

        let mut acc = Classification::Invalid.flag();

        // King moves for the side to move.
        let our_king = match self.stm {
            Color::White => self.strong_ksq,
            Color::Black => self.weak_ksq,
        };
        let king_moves = KING_ATTACKS[our_king.index()];
        for dest in king_moves {
            let (new_strong, new_weak) = match self.stm {
                Color::White => (dest, self.weak_ksq),
                Color::Black => (self.strong_ksq, dest),
            };
            let child_idx = kpk_index(next_stm, new_strong, new_weak, self.pawn_sq);
            acc |= db[child_idx].result.flag();
        }

        // Pawn moves are only available to the strong side (always white
        // after normalisation).
        if self.stm == Color::White {
            if self.pawn_sq.rank() != Rank::R7 {
                // Single push; the landing square being occupied by a
                // king produces an INVALID child which contributes 0.
                let landing = Square::from_index(self.pawn_sq.index() as u8 + 8);
                let child_idx = kpk_index(Color::Black, self.strong_ksq, self.weak_ksq, landing);
                acc |= db[child_idx].result.flag();
            }
            if self.pawn_sq.rank() == Rank::R2 {
                let jump = Square::from_index(self.pawn_sq.index() as u8 + 8);
                let landing = Square::from_index(self.pawn_sq.index() as u8 + 16);
                // The jump-over square must be empty of both kings; if
                // blocked the double push is illegal.
                if jump != self.strong_ksq && jump != self.weak_ksq {
                    let child_idx =
                        kpk_index(Color::Black, self.strong_ksq, self.weak_ksq, landing);
                    acc |= db[child_idx].result.flag();
                }
            }
        }

        if acc & good.flag() != 0 {
            good
        } else if acc & Classification::Unknown.flag() != 0 {
            Classification::Unknown
        } else {
            bad
        }
    }
}

fn initial_classification(
    stm: Color,
    strong_ksq: Square,
    weak_ksq: Square,
    pawn_sq: Square,
) -> Classification {
    // Illegal: kings touching, or pieces overlapping, or the strong
    // side is to move but their pawn is already attacking the weak
    // king (only reachable from an already-impossible predecessor).
    if square_distance(strong_ksq, weak_ksq) <= 1
        || strong_ksq == pawn_sq
        || weak_ksq == pawn_sq
        || (stm == Color::White
            && (PAWN_ATTACKS[Color::White.index()][pawn_sq.index()] & square_bb(weak_ksq)).any())
    {
        return Classification::Invalid;
    }

    // Immediate promotion win for the strong side: white-to-move, pawn
    // on rank 7, and the promotion square is covered by our king or
    // beyond the weak king's reach.
    if stm == Color::White && pawn_sq.rank() == Rank::R7 {
        let promo_sq = Square::from_index(pawn_sq.index() as u8 + 8);
        let strong_not_blocking = strong_ksq != promo_sq;
        let strong_covers_promo = (KING_ATTACKS[strong_ksq.index()] & square_bb(promo_sq)).any();
        let weak_too_far = square_distance(weak_ksq, promo_sq) > 1;
        if strong_not_blocking && (weak_too_far || strong_covers_promo) {
            return Classification::Win;
        }
    }

    // Immediate draw when weak side is to move: stalemate (no legal
    // king move) or king can capture an undefended pawn.
    if stm == Color::Black {
        // Attacked by strong side: strong king's squares + pawn's
        // attack squares (always from white's perspective after
        // normalisation).
        let attacked_by_strong =
            KING_ATTACKS[strong_ksq.index()] | PAWN_ATTACKS[Color::White.index()][pawn_sq.index()];
        let weak_king_moves = KING_ATTACKS[weak_ksq.index()] & !attacked_by_strong;
        if weak_king_moves.is_empty() {
            return Classification::Draw;
        }
        let pawn_undefended = !(KING_ATTACKS[strong_ksq.index()] & square_bb(pawn_sq)).any();
        let weak_king_takes_pawn = (KING_ATTACKS[weak_ksq.index()] & square_bb(pawn_sq)).any();
        if pawn_undefended && weak_king_takes_pawn {
            return Classification::Draw;
        }
    }

    Classification::Unknown
}

fn build_bitbase() -> Vec<u32> {
    // Seed every entry with its immediate classification.
    let mut db: Vec<Entry> = (0..BITBASE_ENTRIES).map(Entry::from_index).collect();

    // Iterate to fixed point. Each sweep promotes UNKNOWN entries that
    // can now be decided. Roughly 15 sweeps in practice.
    let mut changed = true;
    while changed {
        changed = false;
        for i in 0..BITBASE_ENTRIES {
            if db[i].result != Classification::Unknown {
                continue;
            }
            let new_r = db[i].reclassify(&db);
            if new_r != Classification::Unknown {
                db[i].result = new_r;
                changed = true;
            }
        }
    }

    // Pack WIN bits into the output table.
    let mut bits = vec![0u32; BITBASE_WORDS];
    for (i, entry) in db.iter().enumerate() {
        if entry.result == Classification::Win {
            bits[i / 32] |= 1u32 << (i & 31);
        }
    }
    bits
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trivial_win_white_to_move_pawn_on_7th() {
        // White K on a6, pawn on a7, black K on c8. White to move —
        // Kb7 / Kb6 push the black king and a8=Q follows. Clear win.
        let strong_ksq = Square::A6;
        let pawn_sq = Square::A7;
        let weak_ksq = Square::C8;
        assert!(kpk_probe(strong_ksq, pawn_sq, weak_ksq, Color::White));
    }

    #[test]
    fn rook_pawn_can_draw_weak_king_in_front() {
        // Rook pawn with the weak king in the corner drawing zone:
        // white K a6, pawn a5, black K a8, black to move → draw
        // (classical "wrong rook pawn" draw where the black king
        // oscillates between a8 / b8 and stalemate saves the day).
        let strong_ksq = Square::A6;
        let pawn_sq = Square::A5;
        let weak_ksq = Square::A8;
        assert!(!kpk_probe(strong_ksq, pawn_sq, weak_ksq, Color::Black));
    }

    #[test]
    fn strong_king_with_opposition_wins() {
        // Pawn on file D keeps us within the bitbase's native A-D
        // pawn range so no normalisation is needed. White K d6, pawn
        // d5, black K d8 — white has the opposition and the pawn
        // promotes.
        let strong_ksq = Square::D6;
        let pawn_sq = Square::D5;
        let weak_ksq = Square::D8;
        assert!(kpk_probe(strong_ksq, pawn_sq, weak_ksq, Color::White));
    }

    #[test]
    fn normalize_round_trip_draws_hfile_rook_pawn() {
        // H-file rook pawn with the weak king in front. The caller is
        // expected to normalise, which mirrors the H-file down to A and
        // lets the bitbase's single-half table cover both sides. Black
        // K h6, pawn h4, white K h3 — classical draw.
        let strong_ksq = normalize(Color::White, Square::H4, Square::H3);
        let pawn_sq = normalize(Color::White, Square::H4, Square::H4);
        let weak_ksq = normalize(Color::White, Square::H4, Square::H6);
        assert!(!kpk_probe(strong_ksq, pawn_sq, weak_ksq, Color::Black));
    }

    #[test]
    fn normalize_mirrors_kingside_pawn_to_queenside() {
        // White pawn on h2 should mirror onto the a-file. We supply the
        // pawn's own square as the reference for mirroring.
        assert_eq!(normalize(Color::White, Square::H2, Square::H2), Square::A2);
        assert_eq!(normalize(Color::White, Square::H2, Square::E1), Square::D1);
    }

    #[test]
    fn normalize_flips_for_black_strong_side() {
        // If black is the strong side, the whole board gets flipped
        // vertically so that black's pawn looks like it's marching up.
        // Black pawn on a7 mirrors to a2 (file A stays — no mirror —
        // then vertical flip takes a7 → a2).
        assert_eq!(normalize(Color::Black, Square::A7, Square::A7), Square::A2);
    }
}
