//! Slider-piece attack lookups (rook / bishop / queen).
//!
//! For a slider on square `s`, the squares it attacks depend on which squares
//! *between* `s` and the board edge are occupied. Naive computation casts
//! rays in each direction, `O(N)` per query. Both schemes below replace that
//! with an `O(1)` table lookup using a precomputed attack table; they differ
//! only in how the (occupancy → table-index) hash is computed.
//!
//! ## The PEXT path (when `target_feature = "bmi2"`)
//!
//! BMI2's `PEXT` instruction extracts bits from a source according to a
//! mask, packing them into the low bits of the result. For each square we
//! compute the **relevant occupancy mask** (the set of squares whose
//! occupancy actually changes the attack set — own square and ray-terminal
//! edge squares are excluded). Then:
//!
//! 1. Build the per-square attack table by enumerating every subset of the
//!    mask and storing the ray-cast result at index `_pext_u64(subset, mask)`.
//! 2. At query time: `attacks = table[offset + _pext_u64(occupancy, mask)]`.
//!
//! No multiplications, no magic numbers, no PRNG search — just a single
//! PEXT instruction (about 3 cycles on Intel; native on AMD Zen 3+).
//!
//! ## The magic-bitboard path (fallback when BMI2 isn't available)
//!
//! Used on ARM (M-series Macs, mobile devices) and any x86 chip without
//! BMI2. The classical trick: find a 64-bit integer `M` such that for every
//! subset `O` of the mask, `((O * M) >> shift)` produces a unique index
//! into a per-square attack table. `M` is discovered by trial with a
//! sparse-bit PRNG (Stockfish's idiom — sparse numbers hash more cleanly
//! than dense ones).
//!
//! Magic search runs lazily on first use with a fixed PRNG seed (so the
//! computed magics are deterministic across runs); it takes a few tens of
//! milliseconds. Subsequent lookups are still a handful of cycles — just
//! slightly more than PEXT.
//!
//! Both paths build the same `Slider` struct and expose the same public
//! API; the call site is unaware of which is in use.

use std::sync::LazyLock;

use crate::bitboard::Bitboard;
use crate::types::Square;

// =========================================================================
// Per-square magic descriptor
// =========================================================================

#[derive(Clone, Copy)]
struct Magic {
    mask: Bitboard,
    /// Magic multiplier — used only by the non-BMI2 build path. Under
    /// BMI2 the index is `_pext_u64(occupancy, mask)` directly, with
    /// no multiplication, so this stays zero.
    #[cfg_attr(target_feature = "bmi2", allow(dead_code))]
    magic: u64,
    /// Right-shift used to fold the magic-multiply result into the
    /// attack-table index. Like `magic`, unused under BMI2.
    #[cfg_attr(target_feature = "bmi2", allow(dead_code))]
    shift: u32,
    offset: u32,
}

impl Magic {
    /// Hash an occupancy into the per-square attack-table slot.
    ///
    /// Under BMI2, this is a single `PEXT` instruction; `self.magic` and
    /// `self.shift` are unused in that build. Without BMI2 we fall back
    /// to the classical magic-multiply scheme.
    #[cfg(target_feature = "bmi2")]
    #[inline(always)]
    fn index(&self, occupancy: Bitboard) -> usize {
        // SAFETY: the function is `#[cfg(target_feature = "bmi2")]`, so
        // the compiler only emits this body when the BMI2 ISA extension
        // is available — the runtime contract of `_pext_u64`.
        unsafe { core::arch::x86_64::_pext_u64(occupancy.raw(), self.mask.raw()) as usize }
    }

    #[cfg(not(target_feature = "bmi2"))]
    #[inline(always)]
    fn index(&self, occupancy: Bitboard) -> usize {
        let relevant = occupancy.raw() & self.mask.raw();
        (relevant.wrapping_mul(self.magic) >> self.shift) as usize
    }

    const EMPTY: Magic = Magic {
        mask: Bitboard::EMPTY,
        magic: 0,
        shift: 0,
        offset: 0,
    };
}

// =========================================================================
// Slider: the combined per-square magic table + shared attack storage
// =========================================================================

struct Slider {
    per_square: [Magic; 64],
    attacks: Box<[Bitboard]>,
}

impl Slider {
    #[inline(always)]
    fn attacks_from(&self, square: Square, occupancy: Bitboard) -> Bitboard {
        let m = &self.per_square[square.index()];
        self.attacks[m.offset as usize + m.index(occupancy)]
    }
}

// =========================================================================
// Direction sets for each slider
// =========================================================================

const ROOK_DIRS: [(i8, i8); 4] = [(0, 1), (1, 0), (0, -1), (-1, 0)];
const BISHOP_DIRS: [(i8, i8); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];

/// Seeds for the two magic searches. Any seed that converges quickly
/// works; these were picked by running the search and keeping ones with
/// short times. Only consumed by the magic-search build path; under
/// `target_feature = "bmi2"` they're passed through as ignored `_seed`.
#[cfg_attr(target_feature = "bmi2", allow(dead_code))]
const ROOK_SEED: u64 = 0x9E37_79B9_7F4A_7C15;
#[cfg_attr(target_feature = "bmi2", allow(dead_code))]
const BISHOP_SEED: u64 = 0xBF58_476D_1CE4_E5B9;

// =========================================================================
// Ground-truth sliding attack (reference implementation)
// =========================================================================

/// Cast rays from `square` in each of the four directions. Each ray is
/// extended until it walks off the board or until it hits (and includes) an
/// occupied square. Every square touched is part of the attack set.
///
/// This is the O(N) reference implementation used to populate the magic
/// table. Once the table is built, production code uses the O(1) lookup.
fn ray_attacks(dirs: &[(i8, i8); 4], square: Square, occupancy: Bitboard) -> Bitboard {
    let file = (square.raw() & 7) as i8;
    let rank = (square.raw() >> 3) as i8;
    let occ = occupancy.raw();
    let mut bb: u64 = 0;
    for &(df, dr) in dirs {
        let mut nf = file + df;
        let mut nr = rank + dr;
        while (0..8).contains(&nf) && (0..8).contains(&nr) {
            let bit = 1u64 << (nr * 8 + nf);
            bb |= bit;
            if occ & bit != 0 {
                break;
            }
            nf += df;
            nr += dr;
        }
    }
    Bitboard(bb)
}

/// The set of squares whose occupancy actually affects the attack set from
/// `square`. Excludes `square` itself and each ray's terminal edge square.
fn relevant_mask(dirs: &[(i8, i8); 4], square: Square) -> Bitboard {
    let file = (square.raw() & 7) as i8;
    let rank = (square.raw() >> 3) as i8;
    let mut bb: u64 = 0;
    for &(df, dr) in dirs {
        let mut nf = file + df;
        let mut nr = rank + dr;
        while (0..8).contains(&nf) && (0..8).contains(&nr) {
            // Skip this square if the next step along the ray would walk off
            // the board — that makes this the ray's edge square, whose
            // occupancy doesn't change the attack set.
            let next_nf = nf + df;
            let next_nr = nr + dr;
            if !((0..8).contains(&next_nf) && (0..8).contains(&next_nr)) {
                break;
            }
            bb |= 1u64 << (nr * 8 + nf);
            nf = next_nf;
            nr = next_nr;
        }
    }
    Bitboard(bb)
}

// =========================================================================
// Subset enumeration (Carry-Rippler)
// =========================================================================

/// Invoke `f` once for each of the `2^popcount(mask)` subsets of `mask`,
/// including the empty subset. Uses the well-known bit-manipulation idiom:
/// stepping a subset forward by `(subset - mask) & mask` cycles through
/// every subset of `mask` and returns to zero exactly once.
fn for_each_subset(mask: Bitboard, mut f: impl FnMut(Bitboard)) {
    let m = mask.raw();
    let mut subset: u64 = 0;
    loop {
        f(Bitboard(subset));
        subset = subset.wrapping_sub(m) & m;
        if subset == 0 {
            break;
        }
    }
}

// =========================================================================
// PRNG for magic search (non-BMI2 build path only)
// =========================================================================

#[cfg(not(target_feature = "bmi2"))]
struct XorShift64 {
    state: u64,
}

#[cfg(not(target_feature = "bmi2"))]
impl XorShift64 {
    fn new(seed: u64) -> Self {
        // Zero is a fixed point for xorshift; never let the state sit there.
        let state = if seed == 0 { 0x1 } else { seed };
        Self { state }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// Sparse random: bits are set with roughly 1/8 probability after three
    /// rounds of AND. Sparse magics are much more likely to hash cleanly than
    /// dense ones because multiplicative mixing concentrates the output bits.
    fn sparse_u64(&mut self) -> u64 {
        self.next_u64() & self.next_u64() & self.next_u64()
    }
}

// =========================================================================
// Magic search (non-BMI2 build path only)
// =========================================================================

/// For a single square, pre-compute every (occupancy, attacks) pair, then
/// search for a magic number that hashes each occupancy to a unique slot
/// (with "constructive collisions" allowed — two occupancies may share a
/// slot if and only if they produce the same attack set).
///
/// Returns the magic and the populated attack-table entries.
#[cfg(not(target_feature = "bmi2"))]
fn find_magic_for_square(
    dirs: &[(i8, i8); 4],
    square: Square,
    rng: &mut XorShift64,
) -> (Magic, Vec<Bitboard>) {
    let mask = relevant_mask(dirs, square);
    let bits = mask.popcount();
    let size = 1usize << bits;
    let shift = 64 - bits;

    // Cache ground-truth attacks for each subset. The subset order isn't
    // important as long as the lookup agrees: we'll re-enumerate the same
    // order when populating the table below.
    let mut reference: Vec<(Bitboard, Bitboard)> = Vec::with_capacity(size);
    for_each_subset(mask, |occ| {
        reference.push((occ, ray_attacks(dirs, square, occ)));
    });

    // Scratch storage reused across magic attempts. `epoch` avoids the cost
    // of zeroing `table` after every failed attempt: a slot is considered
    // populated only when its epoch matches the current attempt.
    let mut table: Vec<Bitboard> = vec![Bitboard::EMPTY; size];
    let mut epoch: Vec<u32> = vec![0u32; size];
    let mut attempt: u32 = 0;

    loop {
        let candidate = rng.sparse_u64();

        // Quick rejection: if `(magic * mask)` doesn't spread enough bits
        // into the high byte, the high-bit mixing is too weak and the magic
        // won't hash cleanly. Stockfish uses 6 as the threshold; keep it.
        if (candidate.wrapping_mul(mask.raw()) >> 56).count_ones() < 6 {
            continue;
        }

        attempt = attempt.wrapping_add(1);
        let mut ok = true;

        for &(occ, attacks) in &reference {
            let idx = (occ.raw().wrapping_mul(candidate) >> shift) as usize;
            if epoch[idx] < attempt {
                // First time this slot has been touched this attempt: claim it.
                epoch[idx] = attempt;
                table[idx] = attacks;
            } else if table[idx] != attacks {
                // A previous occupancy this attempt produced a different
                // attack set at the same index: destructive collision.
                ok = false;
                break;
            }
            // Otherwise: constructive collision, same attack set, fine.
        }

        if ok {
            return (
                Magic {
                    mask,
                    magic: candidate,
                    shift,
                    offset: 0, // Filled in by the caller once concatenation knows the offset.
                },
                table,
            );
        }
    }
}

/// Build a fully-populated `Slider` for one slider type.
///
/// Two implementations, cfg-switched on `target_feature = "bmi2"`:
///
/// - With BMI2: skip the magic search entirely. Each subset of each
///   square's mask gets its own slot indexed by `PEXT(subset, mask)`,
///   filled directly from the ray-cast reference. Build is also faster
///   because there's no PRNG trial loop.
/// - Without BMI2: run the classical magic-number PRNG search for each
///   square (see [`find_magic_for_square`]) and concatenate the
///   per-square tables.
///
/// Both paths produce the same `Slider` struct and the same query
/// semantics; the unused-in-BMI2 fields (`magic`, `shift`) stay zeroed.
#[cfg(target_feature = "bmi2")]
fn build_slider(dirs: &[(i8, i8); 4], _seed: u64) -> Slider {
    let mut per_square = [Magic::EMPTY; 64];
    let mut table: Vec<Bitboard> = Vec::new();

    for i in 0u8..64 {
        let square = Square::from_index(i);
        let mask = relevant_mask(dirs, square);
        let size = 1usize << mask.popcount();
        let offset = table.len() as u32;
        // Pre-extend so the per-subset indexed writes below land in
        // already-allocated slots.
        table.resize(table.len() + size, Bitboard::EMPTY);
        let raw_mask = mask.raw();
        for_each_subset(mask, |occ| {
            // SAFETY: `cfg(target_feature = "bmi2")` gates the function.
            let idx = unsafe { core::arch::x86_64::_pext_u64(occ.raw(), raw_mask) } as usize;
            table[offset as usize + idx] = ray_attacks(dirs, square, occ);
        });
        per_square[i as usize] = Magic {
            mask,
            magic: 0,
            shift: 0,
            offset,
        };
    }

    Slider {
        per_square,
        attacks: table.into_boxed_slice(),
    }
}

#[cfg(not(target_feature = "bmi2"))]
fn build_slider(dirs: &[(i8, i8); 4], seed: u64) -> Slider {
    let mut rng = XorShift64::new(seed);
    let mut per_square = [Magic::EMPTY; 64];
    let mut table: Vec<Bitboard> = Vec::new();

    for i in 0u8..64 {
        let square = Square::from_index(i);
        let (mut magic, entries) = find_magic_for_square(dirs, square, &mut rng);
        magic.offset = table.len() as u32;
        table.extend(entries);
        per_square[i as usize] = magic;
    }

    Slider {
        per_square,
        attacks: table.into_boxed_slice(),
    }
}

// =========================================================================
// Statics (lazy-initialised on first use)
// =========================================================================

static ROOK: LazyLock<Slider> = LazyLock::new(|| build_slider(&ROOK_DIRS, ROOK_SEED));
static BISHOP: LazyLock<Slider> = LazyLock::new(|| build_slider(&BISHOP_DIRS, BISHOP_SEED));

// =========================================================================
// Public API
// =========================================================================

/// The squares a rook on `square` attacks given the current `occupancy`.
/// Includes the capturing square at the end of each ray; excludes the
/// square the rook is on.
#[inline]
pub fn rook_attacks(square: Square, occupancy: Bitboard) -> Bitboard {
    ROOK.attacks_from(square, occupancy)
}

/// The squares a bishop on `square` attacks given the current `occupancy`.
#[inline]
pub fn bishop_attacks(square: Square, occupancy: Bitboard) -> Bitboard {
    BISHOP.attacks_from(square, occupancy)
}

/// The squares a queen on `square` attacks given the current `occupancy`.
/// Simply the union of the rook and bishop attacks from the same square.
#[inline]
pub fn queen_attacks(square: Square, occupancy: Bitboard) -> Bitboard {
    rook_attacks(square, occupancy) | bishop_attacks(square, occupancy)
}

/// Warm the magic tables. Calling this at startup avoids the first-use
/// latency from otherwise triggering the search on a hot path. Not required
/// for correctness.
pub fn warm_up() {
    LazyLock::force(&ROOK);
    LazyLock::force(&BISHOP);
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitboard::square_bb;

    // ---- Relevant mask shape ----------------------------------------

    #[test]
    fn rook_mask_excludes_self_and_edges() {
        // A rook on d4 sees 10 relevant squares: d5, d6, d7 (N), d3, d2 (S),
        // e4, f4, g4 (E), c4, b4 (W). Edge squares d1, d8, a4, h4 and d4
        // itself are all excluded.
        let mask = relevant_mask(&ROOK_DIRS, Square::D4);
        assert_eq!(mask.popcount(), 10);
        assert!(!mask.contains(Square::D4));
        assert!(!mask.contains(Square::D1));
        assert!(!mask.contains(Square::D8));
        for sq in &["d5", "d6", "d7", "d3", "d2", "e4", "f4", "g4", "c4", "b4"] {
            assert!(
                mask.contains(Square::from_algebraic(sq).unwrap()),
                "rook mask on d4 should include {}",
                sq
            );
        }
    }

    #[test]
    fn rook_mask_on_corner_has_twelve_bits() {
        // A rook on a1 sees b1..g1 (6) and a2..a7 (6).
        assert_eq!(relevant_mask(&ROOK_DIRS, Square::A1).popcount(), 12);
        assert_eq!(relevant_mask(&ROOK_DIRS, Square::H1).popcount(), 12);
        assert_eq!(relevant_mask(&ROOK_DIRS, Square::A8).popcount(), 12);
        assert_eq!(relevant_mask(&ROOK_DIRS, Square::H8).popcount(), 12);
    }

    #[test]
    fn bishop_mask_sizes() {
        // Corner bishop: 6 relevant inner-diagonal squares (b2..g7 from a1).
        assert_eq!(relevant_mask(&BISHOP_DIRS, Square::A1).popcount(), 6);
        // Centre bishop: 9 relevant squares.
        assert_eq!(relevant_mask(&BISHOP_DIRS, Square::D4).popcount(), 9);
    }

    // ---- Ground-truth ray casting ------------------------------------

    #[test]
    fn rook_from_a1_empty_board_covers_rank_1_and_a_file() {
        let attacks = ray_attacks(&ROOK_DIRS, Square::A1, Bitboard::EMPTY);
        assert_eq!(attacks.popcount(), 14);
        // Entire rank 1 except a1.
        for f in 1..8u8 {
            assert!(attacks.contains(Square::from_index(f)));
        }
        // Entire a-file except a1.
        for r in 1..8u8 {
            assert!(attacks.contains(Square::from_index(r * 8)));
        }
    }

    #[test]
    fn rook_stops_at_first_occupied_square_on_ray() {
        // Rook on a1, blocker on d1. Should attack b1, c1, d1 (capture) — not
        // e1..h1, not anything behind d1 on the rank.
        let occ = square_bb(Square::D1);
        let attacks = ray_attacks(&ROOK_DIRS, Square::A1, occ);
        assert!(attacks.contains(Square::B1));
        assert!(attacks.contains(Square::C1));
        assert!(attacks.contains(Square::D1));
        assert!(!attacks.contains(Square::E1));
        assert!(!attacks.contains(Square::F1));
    }

    #[test]
    fn bishop_from_a1_empty_board_covers_long_diagonal() {
        let attacks = ray_attacks(&BISHOP_DIRS, Square::A1, Bitboard::EMPTY);
        assert_eq!(attacks.popcount(), 7);
        for sq in &["b2", "c3", "d4", "e5", "f6", "g7", "h8"] {
            assert!(attacks.contains(Square::from_algebraic(sq).unwrap()));
        }
    }

    // ---- Subset enumeration -----------------------------------------

    #[test]
    fn subset_enumeration_visits_each_subset_exactly_once() {
        // Pick a small but non-trivial mask and verify the Carry-Rippler
        // trick visits each of the 2^popcount subsets exactly once.
        let mask = Bitboard(0b1010_0101);
        let mut seen: Vec<u64> = Vec::new();
        for_each_subset(mask, |s| seen.push(s.raw()));
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 1usize << mask.popcount());
        // Every element must be a subset of the mask.
        for s in seen {
            assert_eq!(s & !mask.raw(), 0);
        }
    }

    // ---- Magic lookups vs ray casting --------------------------------

    #[test]
    fn rook_magic_matches_ray_casting_on_empty_board() {
        for i in 0u8..64 {
            let sq = Square::from_index(i);
            assert_eq!(
                rook_attacks(sq, Bitboard::EMPTY),
                ray_attacks(&ROOK_DIRS, sq, Bitboard::EMPTY),
            );
        }
    }

    #[test]
    fn bishop_magic_matches_ray_casting_on_empty_board() {
        for i in 0u8..64 {
            let sq = Square::from_index(i);
            assert_eq!(
                bishop_attacks(sq, Bitboard::EMPTY),
                ray_attacks(&BISHOP_DIRS, sq, Bitboard::EMPTY),
            );
        }
    }

    #[test]
    fn magic_matches_ray_casting_across_every_subset_on_a_few_squares() {
        // Full verification on four representative squares: corner, edge,
        // centre, near-edge. For each, enumerate every subset of the mask
        // and confirm the magic table agrees with the naive ray cast.
        let squares = [Square::A1, Square::E4, Square::D1, Square::H4];
        for &sq in &squares {
            let rm = relevant_mask(&ROOK_DIRS, sq);
            for_each_subset(rm, |occ| {
                assert_eq!(
                    rook_attacks(sq, occ),
                    ray_attacks(&ROOK_DIRS, sq, occ),
                    "rook disagreement on {} with occupancy 0x{:016x}",
                    sq.to_algebraic(),
                    occ.raw()
                );
            });

            let bm = relevant_mask(&BISHOP_DIRS, sq);
            for_each_subset(bm, |occ| {
                assert_eq!(
                    bishop_attacks(sq, occ),
                    ray_attacks(&BISHOP_DIRS, sq, occ),
                    "bishop disagreement on {} with occupancy 0x{:016x}",
                    sq.to_algebraic(),
                    occ.raw()
                );
            });
        }
    }

    #[test]
    fn magic_ignores_bits_outside_the_mask() {
        // Occupancy bits that fall outside the relevant mask should not
        // change the computed attacks. Pick a square, compute attacks with
        // a clean occupancy, then OR in some outside bits and confirm the
        // lookup is unchanged.
        let sq = Square::D4;
        let base_occ = square_bb(Square::D6) | square_bb(Square::F4);
        let noisy_occ = base_occ
            | square_bb(Square::D8) // on the d-file edge, outside mask
            | square_bb(Square::A4) // on the 4th rank edge, outside mask
            | square_bb(Square::D4); // the rook's own square, outside mask
        assert_eq!(rook_attacks(sq, base_occ), rook_attacks(sq, noisy_occ));
    }

    #[test]
    fn queen_attacks_are_union_of_rook_and_bishop() {
        let sq = Square::E4;
        let occ = square_bb(Square::E6) | square_bb(Square::B4) | square_bb(Square::H7);
        assert_eq!(
            queen_attacks(sq, occ),
            rook_attacks(sq, occ) | bishop_attacks(sq, occ),
        );
    }

    #[test]
    fn rook_attacks_include_capturing_square() {
        // Rook on a1 with a blocker on a4: the rook attacks a2, a3, and a4
        // (the capture square), but not a5..a8.
        let occ = square_bb(Square::from_algebraic("a4").unwrap());
        let attacks = rook_attacks(Square::A1, occ);
        for sq in &["a2", "a3", "a4"] {
            assert!(attacks.contains(Square::from_algebraic(sq).unwrap()));
        }
        for sq in &["a5", "a6", "a7", "a8"] {
            assert!(!attacks.contains(Square::from_algebraic(sq).unwrap()));
        }
    }

    #[test]
    fn warm_up_is_idempotent() {
        // Calling warm_up before any attack query should not change behaviour.
        warm_up();
        warm_up();
        let r = rook_attacks(Square::E4, Bitboard::EMPTY);
        assert!(r.any());
    }
}
