//! TT storage layer: 16-byte `TTEntry` and the cache-line `Cluster`,
//! split out of the tt module.

use super::*;

// =========================================================================
// Entry
// =========================================================================

/// One transposition-table entry. Stored as two atomic halves so
/// simultaneous probes and saves observe each half consistently, even
/// if the entry as a whole tears under contention. All reads and
/// writes use `Ordering::Relaxed`; we rely on the 16-bit key check to
/// detect stale data.
#[repr(C, align(8))]
pub(super) struct TTEntry {
    /// Bits (low→high):
    /// - `[0..16)`:  `key16`   — top 16 bits of the Zobrist key.
    /// - `[16..32)`: `move16`  — the stored move (`Move::raw()`).
    /// - `[32..48)`: `value16` — search value, i16.
    /// - `[48..64)`: `eval16`  — static eval, i16.
    pub(super) payload: AtomicU64,

    /// Bits (low→high):
    /// - `[0..2)`: bound (encoding of [`Bound`]).
    /// - bit 2:    PV flag.
    /// - `[3..8)`: generation (top 5 bits).
    /// - `[8..16)`: `depth8` — depth offset by `Depth::OFFSET` so the
    ///   smallest legal depth maps to 0 in the byte.
    pub(super) gen_depth: AtomicU16,
}

impl TTEntry {
    pub(super) const fn zero() -> TTEntry {
        TTEntry {
            payload: AtomicU64::new(0),
            gen_depth: AtomicU16::new(0),
        }
    }

    /// Snapshot the entry into a plain-old-data [`TTData`]. Both halves
    /// are loaded with Relaxed ordering; they may be from different
    /// saves, which is fine — the caller should validate the key match
    /// before acting on any other field.
    ///
    /// `#[inline(always)]` because every TT probe calls this in a tight
    /// loop over cluster entries (millions per search). Without it,
    /// profiling showed `core::sync::atomic::atomic_load` as a top
    /// hot function — the wrapper wasn't getting inlined across the
    /// function-call boundary, leaving the per-probe cost dominated
    /// by call/return rather than the underlying `mov` instruction
    /// the Relaxed load lowers to. With inlining, the returned
    /// `TTData` struct is also rarely materialized on the stack —
    /// most probes only read 1-2 of its fields, and the rest get
    /// dead-code-eliminated at the call site.
    #[inline(always)]
    pub(super) fn load(&self) -> TTData {
        let p = self.payload.load(Ordering::Relaxed);
        let gd = self.gen_depth.load(Ordering::Relaxed);

        let key16 = (p & 0xFFFF) as u16;
        let move16 = ((p >> 16) & 0xFFFF) as u16;
        let value16 = ((p >> 32) & 0xFFFF) as u16 as i16;
        let eval16 = ((p >> 48) & 0xFFFF) as u16 as i16;

        let gen_bound = (gd & 0xFF) as u8;
        let depth8 = ((gd >> 8) & 0xFF) as u8;

        TTData {
            key16,
            mv: Move::from_raw(move16),
            value: Value(value16 as i32),
            eval: Value(eval16 as i32),
            depth: Depth(depth8 as i32 + Depth::OFFSET),
            bound: Bound::from_u8(gen_bound),
            is_pv: (gen_bound & 0x4) != 0,
            generation: gen_bound & 0xF8,
        }
    }

    /// `#[inline(always)]` for the same reason as [`load`](Self::load):
    /// every `save` after a search node pays this, and the call
    /// boundary cost dominates the underlying Relaxed atomic stores.
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn store(
        &self,
        key16: u16,
        mv: Move,
        value: Value,
        eval: Value,
        bound: Bound,
        is_pv: bool,
        depth: Depth,
        generation: u8,
    ) {
        let payload = (key16 as u64)
            | ((mv.raw() as u64) << 16)
            | (((value.0 as i16) as u16 as u64) << 32)
            | (((eval.0 as i16) as u16 as u64) << 48);

        let depth8 = (depth.0 - Depth::OFFSET) as u8;
        let gen_bound = (generation & 0xF8) | ((is_pv as u8) << 2) | bound.as_u8();
        let gen_depth = (gen_bound as u16) | ((depth8 as u16) << 8);

        self.payload.store(payload, Ordering::Relaxed);
        self.gen_depth.store(gen_depth, Ordering::Relaxed);
    }
}

impl Clone for TTEntry {
    fn clone(&self) -> Self {
        // Snapshot via Relaxed loads. Single-threaded callers see the
        // current value; concurrent writers may produce torn halves,
        // which the key16 check at probe time treats as a miss — same
        // contract the live entries already follow.
        TTEntry {
            payload: AtomicU64::new(self.payload.load(Ordering::Relaxed)),
            gen_depth: AtomicU16::new(self.gen_depth.load(Ordering::Relaxed)),
        }
    }
}

// =========================================================================
// Cluster
// =========================================================================

#[repr(C, align(64))]
pub(super) struct Cluster {
    pub(super) entries: [TTEntry; ENTRIES_PER_CLUSTER],
    _padding: [u8; CLUSTER_BYTES - ENTRIES_PER_CLUSTER * std::mem::size_of::<TTEntry>()],
}

impl Cluster {
    pub(super) const fn zero() -> Cluster {
        Cluster {
            entries: [TTEntry::zero(), TTEntry::zero(), TTEntry::zero()],
            _padding: [0; CLUSTER_BYTES - ENTRIES_PER_CLUSTER * std::mem::size_of::<TTEntry>()],
        }
    }
}

impl Clone for Cluster {
    fn clone(&self) -> Self {
        Cluster {
            entries: [
                self.entries[0].clone(),
                self.entries[1].clone(),
                self.entries[2].clone(),
            ],
            _padding: self._padding,
        }
    }
}
