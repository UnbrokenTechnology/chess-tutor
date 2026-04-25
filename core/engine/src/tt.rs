//! Transposition table: a shared cache of search results keyed by the
//! Zobrist hash. Search visits the same position many times through
//! different move orders; caching the result under the Zobrist key
//! turns those re-visits into O(1) lookups and lets the search prune
//! whole subtrees based on the cached bound.
//!
//! The layout mirrors Stockfish 11: a power-of-two-ish number of
//! **clusters**, each cluster holding a small constant number of
//! **entries** so a full cluster fits inside a single cache line. The
//! high 16 bits of the Zobrist key identify an entry inside its
//! cluster; the low 32 bits pick which cluster.
//!
//! **Concurrency.** The reference accepts racy non-atomic writes and
//! detects torn reads via the 16-bit key check. We do the same in Rust,
//! but spell it out explicitly with relaxed atomic loads and stores —
//! this way `probe` and `save` take `&self`, a single TT can be shared
//! across future search threads without API churn, and the compiler
//! never believes entries are immutable. Two entry halves (the 64-bit
//! payload and the 16-bit gen/depth word) are updated independently;
//! the save is atomic *per half*, racy at the entry level, but the key
//! check in probe catches the cases where that matters.
//!
//! **Layout.** Each entry is 16 bytes: an 8-byte payload atomic, a
//! 2-byte gen/depth atomic, and natural alignment padding. Three
//! entries per cluster plus 16 bytes of tail padding make a cluster
//! exactly 64 bytes, one modern cache line. Stockfish uses 10-byte
//! entries with 6-entry cache lines; we trade a little density for a
//! clean all-atomic implementation.

use std::sync::atomic::{AtomicU16, AtomicU64, AtomicU8, Ordering};

use crate::types::{Bound, Depth, Move, Value};

// =========================================================================
// Tunables
// =========================================================================

const ENTRIES_PER_CLUSTER: usize = 3;

/// Total bytes per cluster. 3 entries × 16 B = 48 B; 16 B tail pad pushes
/// the cluster up to a 64-byte cache line so clusters never straddle.
const CLUSTER_BYTES: usize = 64;

/// How many megabytes of TT to allocate by default. Search code can
/// override via [`TranspositionTable::new`] or [`TranspositionTable::resize`].
pub const DEFAULT_TT_MB: usize = 16;

// =========================================================================
// Entry
// =========================================================================

/// One transposition-table entry. Stored as two atomic halves so
/// simultaneous probes and saves observe each half consistently, even
/// if the entry as a whole tears under contention. All reads and
/// writes use `Ordering::Relaxed`; we rely on the 16-bit key check to
/// detect stale data.
#[repr(C, align(8))]
struct TTEntry {
    /// Bits (low→high):
    /// - `[0..16)`:  `key16`   — top 16 bits of the Zobrist key.
    /// - `[16..32)`: `move16`  — the stored move (`Move::raw()`).
    /// - `[32..48)`: `value16` — search value, i16.
    /// - `[48..64)`: `eval16`  — static eval, i16.
    payload: AtomicU64,

    /// Bits (low→high):
    /// - `[0..2)`: bound (encoding of [`Bound`]).
    /// - bit 2:    PV flag.
    /// - `[3..8)`: generation (top 5 bits).
    /// - `[8..16)`: `depth8` — depth offset by `Depth::OFFSET` so the
    ///   smallest legal depth maps to 0 in the byte.
    gen_depth: AtomicU16,
}

impl TTEntry {
    const fn zero() -> TTEntry {
        TTEntry {
            payload: AtomicU64::new(0),
            gen_depth: AtomicU16::new(0),
        }
    }

    /// Snapshot the entry into a plain-old-data [`TTData`]. Both halves
    /// are loaded with Relaxed ordering; they may be from different
    /// saves, which is fine — the caller should validate the key match
    /// before acting on any other field.
    fn load(&self) -> TTData {
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

    #[allow(clippy::too_many_arguments)]
    fn store(
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
struct Cluster {
    entries: [TTEntry; ENTRIES_PER_CLUSTER],
    _padding: [u8; CLUSTER_BYTES - ENTRIES_PER_CLUSTER * std::mem::size_of::<TTEntry>()],
}

impl Cluster {
    const fn zero() -> Cluster {
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

// =========================================================================
// Snapshot types
// =========================================================================

/// The immutable snapshot of an entry returned by [`TranspositionTable::probe`].
///
/// The `key16` field is the identity check: on a hit, it equals the top
/// 16 bits of the probed Zobrist key. The remaining fields are only
/// meaningful when `key16` matches — i.e., when [`ProbeResult::hit`] is
/// true.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TTData {
    /// Top 16 bits of the Zobrist key. Zero for empty slots.
    pub key16: u16,
    pub mv: Move,
    pub value: Value,
    pub eval: Value,
    pub depth: Depth,
    pub bound: Bound,
    pub is_pv: bool,
    /// The 5-bit generation this entry was written in, shifted up by 3.
    /// Aged entries are candidates for replacement.
    pub generation: u8,
}

impl TTData {
    const fn empty() -> TTData {
        TTData {
            key16: 0,
            mv: Move::NONE,
            value: Value(0),
            eval: Value(0),
            depth: Depth::NONE,
            bound: Bound::None,
            is_pv: false,
            generation: 0,
        }
    }
}

// =========================================================================
// Probe result
// =========================================================================

/// The output of a TT lookup. On a hit, [`data`] carries the stored
/// record. On a miss, `hit` is false and [`data`] is empty; the slot
/// selected for a future save is tracked internally so [`save`] can
/// write back to the exact same place without re-walking the cluster.
pub struct ProbeResult<'a> {
    pub hit: bool,
    pub data: TTData,
    slot: &'a TTEntry,
    generation: u8,
}

impl<'a> ProbeResult<'a> {
    /// Write a new record into the slot this probe pre-selected.
    ///
    /// Preservation rule: if the caller has no better move to record
    /// (`mv == Move::NONE`) and the slot already holds a move for the
    /// same position, we keep the old move. The reference's
    /// replacement rule is otherwise applied.
    #[allow(clippy::too_many_arguments)]
    pub fn save(
        &self,
        key: u64,
        value: Value,
        is_pv: bool,
        bound: Bound,
        depth: Depth,
        mv: Move,
        eval: Value,
    ) {
        let new_key16 = (key >> 48) as u16;
        let existing = self.slot.load();

        // Choose the move to store. Matches `TTEntry::save` in the
        // reference: preserve the existing move when the caller has
        // none and the key already matches.
        let stored_move = if mv != Move::NONE || existing.key16 != new_key16 {
            mv
        } else {
            existing.mv
        };

        // Apply the replacement policy: we only clobber the slot if
        // the new entry comes with a different position, brings in
        // new information (deeper search), or produces an exact bound.
        let overwrite = existing.key16 != new_key16
            || (depth.0 - Depth::OFFSET) > (existing.depth.0 - Depth::OFFSET) - 4
            || bound == Bound::Exact;

        if overwrite {
            self.slot.store(
                new_key16,
                stored_move,
                value,
                eval,
                bound,
                is_pv,
                depth,
                self.generation,
            );
        }
    }
}

// =========================================================================
// Transposition table
// =========================================================================

/// A shared transposition table, sized at construction time.
///
/// All public methods take `&self`, so a single TT instance can be
/// shared across threads — each thread does its own Relaxed atomic
/// reads and writes against the underlying cluster array. The table
/// itself is heap-allocated via a `Box<[Cluster]>`.
pub struct TranspositionTable {
    clusters: Box<[Cluster]>,
    /// Incremented at the start of each search. Used by the
    /// replacement strategy to prefer older entries for eviction.
    /// Increment is `+8` so the low three bits remain available for
    /// the PV flag and bound.
    generation: AtomicU8,
}

impl TranspositionTable {
    /// Allocate a table sized to fit roughly `mb` megabytes, rounded
    /// down to a whole number of clusters.
    pub fn new(mb: usize) -> TranspositionTable {
        let mb = mb.max(1);
        let total_bytes = mb * 1024 * 1024;
        let cluster_count = (total_bytes / CLUSTER_BYTES).max(1);

        let mut clusters: Vec<Cluster> = Vec::with_capacity(cluster_count);
        for _ in 0..cluster_count {
            clusters.push(Cluster::zero());
        }

        TranspositionTable {
            clusters: clusters.into_boxed_slice(),
            generation: AtomicU8::new(0),
        }
    }

    /// Wipe every cluster. Takes `&self` so callers holding a shared
    /// reference can still clear in between searches.
    pub fn clear(&self) {
        for cluster in self.clusters.iter() {
            for entry in cluster.entries.iter() {
                entry.payload.store(0, Ordering::Relaxed);
                entry.gen_depth.store(0, Ordering::Relaxed);
            }
        }
        self.generation.store(0, Ordering::Relaxed);
    }

    /// Signal the start of a new search. Bumps the generation counter
    /// by 8 so every entry saved from now on carries a newer age tag.
    pub fn new_search(&self) {
        self.generation.fetch_add(8, Ordering::Relaxed);
    }

    /// Probe the table for `key`.
    ///
    /// On a hit, the returned [`ProbeResult::data`] carries the stored
    /// record and the slot is refreshed to the current generation. On a
    /// miss, `data` is empty and the pre-selected replacement slot is
    /// tracked internally; call [`ProbeResult::save`] to write to it.
    pub fn probe(&self, key: u64) -> ProbeResult<'_> {
        let cluster = self.cluster_for(key);
        let key16 = (key >> 48) as u16;
        let generation = self.generation.load(Ordering::Relaxed);

        // Fast path: exact-key hit or empty slot. An empty slot is
        // technically a miss but it's the cheapest replacement target
        // so we stop looking.
        for entry in cluster.entries.iter() {
            let data = entry.load();
            if data.key16 == 0 || data.key16 == key16 {
                // Refresh the age tag so a concurrently-read entry
                // isn't kicked out on the next replacement search.
                let gd = entry.gen_depth.load(Ordering::Relaxed);
                let refreshed = (gd & 0xFF07) | ((generation as u16) & 0xF8);
                entry.gen_depth.store(refreshed, Ordering::Relaxed);

                return ProbeResult {
                    hit: data.key16 != 0,
                    data,
                    slot: entry,
                    generation,
                };
            }
        }

        // Miss: find the least-valuable slot. Replace-value =
        // depth8 − 8 × aged-generation. Older entries age up
        // faster; deeper entries resist eviction.
        let mut replace = &cluster.entries[0];
        let mut replace_data = replace.load();
        for entry in cluster.entries[1..].iter() {
            let entry_data = entry.load();
            if replace_score(&replace_data, generation) > replace_score(&entry_data, generation) {
                replace = entry;
                replace_data = entry_data;
            }
        }

        ProbeResult {
            hit: false,
            data: TTData::empty(),
            slot: replace,
            generation,
        }
    }

    /// Approximate permille fill — the fraction of the first 1000 slots
    /// that hold an entry from the current generation, per UCI
    /// convention. Useful for search diagnostics.
    pub fn hashfull(&self) -> i32 {
        let current_gen = self.generation.load(Ordering::Relaxed) & 0xF8;
        let cluster_count = (1000 / ENTRIES_PER_CLUSTER).min(self.clusters.len());
        let mut count = 0i32;
        for cluster in &self.clusters[..cluster_count] {
            for entry in &cluster.entries {
                let gd = entry.gen_depth.load(Ordering::Relaxed);
                let gen_bound = (gd & 0xFF) as u8;
                if (gen_bound & 0xF8) == current_gen && (gen_bound & 0x3) != 0 {
                    count += 1;
                }
            }
        }
        count * 1000 / (ENTRIES_PER_CLUSTER as i32 * cluster_count as i32)
    }

    /// Number of clusters allocated. Exposed for diagnostics / tests.
    pub fn cluster_count(&self) -> usize {
        self.clusters.len()
    }

    fn cluster_for(&self, key: u64) -> &Cluster {
        // Mul-shift-reduce: `(u32(key) * cluster_count) >> 32`. Maps
        // the low 32 bits of the key into `[0, cluster_count)` with
        // (reasonably) uniform distribution; avoids requiring
        // cluster_count to be a power of two.
        let index = (((key as u32) as u64) * (self.clusters.len() as u64)) >> 32;
        &self.clusters[index as usize]
    }
}

impl Clone for TranspositionTable {
    /// Deep-clone the table: each cluster (including all atomic
    /// entries) is snapshotted into a fresh allocation. The clone
    /// shares no memory with the source. Used by the CLI's
    /// analytical commands so that running `search` / `analyze`
    /// inherits the play loop's accumulated TT state without
    /// mutating it.
    fn clone(&self) -> Self {
        let mut new_clusters: Vec<Cluster> = Vec::with_capacity(self.clusters.len());
        for cluster in self.clusters.iter() {
            new_clusters.push(cluster.clone());
        }
        TranspositionTable {
            clusters: new_clusters.into_boxed_slice(),
            generation: AtomicU8::new(self.generation.load(Ordering::Relaxed)),
        }
    }
}

/// "How valuable is this entry, for purposes of keeping it around?"
/// Higher means more valuable → less likely to be chosen for eviction.
fn replace_score(data: &TTData, current_gen: u8) -> i32 {
    // Age = (current_gen − entry_gen) mod 256, with the low 3 bits
    // ignored because those hold the bound/PV flags. The +263 matches
    // the reference's trick to keep the computation monotonic across
    // generation rollovers.
    let raw_age = (263u32
        .wrapping_add(current_gen as u32)
        .wrapping_sub(data.generation as u32))
        & 0xF8;
    let depth8 = data.depth.0 - Depth::OFFSET;
    depth8 - raw_age as i32
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Move, Square};

    fn sample_move() -> Move {
        Move::normal(Square::E2, Square::E4)
    }

    #[test]
    fn cluster_is_one_cache_line() {
        assert_eq!(std::mem::size_of::<Cluster>(), CLUSTER_BYTES);
    }

    #[test]
    fn entry_is_sixteen_bytes() {
        assert_eq!(std::mem::size_of::<TTEntry>(), 16);
    }

    #[test]
    fn empty_table_misses() {
        let tt = TranspositionTable::new(1);
        let probe = tt.probe(0xDEAD_BEEF_CAFE_F00D);
        assert!(!probe.hit);
        assert_eq!(probe.data.key16, 0);
    }

    #[test]
    fn save_then_probe_returns_hit_with_stored_data() {
        let tt = TranspositionTable::new(1);
        tt.new_search(); // bump generation so stored entries aren't gen 0
        let key: u64 = 0x1234_5678_9ABC_DEF0;

        let probe = tt.probe(key);
        assert!(!probe.hit);
        probe.save(
            key,
            Value(42),
            true,
            Bound::Exact,
            Depth(10),
            sample_move(),
            Value(-7),
        );

        let probe2 = tt.probe(key);
        assert!(probe2.hit, "saved entry should be found on re-probe");
        assert_eq!(probe2.data.value, Value(42));
        assert_eq!(probe2.data.eval, Value(-7));
        assert_eq!(probe2.data.bound, Bound::Exact);
        assert_eq!(probe2.data.depth, Depth(10));
        assert_eq!(probe2.data.mv, sample_move());
        assert!(probe2.data.is_pv);
    }

    #[test]
    fn probe_with_wrong_key_does_not_report_hit() {
        let tt = TranspositionTable::new(1);
        tt.new_search();
        // Same low 32 bits (same cluster) but different top 16
        // (different identity). Only the identity should matter for
        // hit detection.
        let key = 0x1111_2222_3333_4444u64;
        let wrong = 0x2222_2222_3333_4444u64;

        tt.probe(key).save(
            key,
            Value(100),
            false,
            Bound::Exact,
            Depth(5),
            sample_move(),
            Value(0),
        );

        let p = tt.probe(wrong);
        assert!(!p.hit);
    }

    #[test]
    fn replacement_preserves_move_when_none_saved() {
        let tt = TranspositionTable::new(1);
        tt.new_search();
        let key = 0xAAAA_BBBB_CCCC_DDDDu64;

        // First save with a real move.
        tt.probe(key).save(
            key,
            Value(50),
            false,
            Bound::Lower,
            Depth(4),
            sample_move(),
            Value(10),
        );

        // Overwrite with Move::NONE — the stored move should survive,
        // per the reference's preservation rule.
        tt.probe(key).save(
            key,
            Value(75),
            false,
            Bound::Exact,
            Depth(6),
            Move::NONE,
            Value(12),
        );

        let p = tt.probe(key);
        assert!(p.hit);
        assert_eq!(p.data.mv, sample_move());
        assert_eq!(p.data.value, Value(75));
        assert_eq!(p.data.depth, Depth(6));
    }

    #[test]
    fn exact_bound_overwrites_shallower_existing_entry() {
        let tt = TranspositionTable::new(1);
        tt.new_search();
        let key = 0xFEED_FACE_CAFE_BABEu64;

        tt.probe(key).save(
            key,
            Value(1),
            false,
            Bound::Lower,
            Depth(10),
            sample_move(),
            Value(0),
        );

        // Shallower but Exact — should overwrite per the replacement rule.
        tt.probe(key).save(
            key,
            Value(2),
            false,
            Bound::Exact,
            Depth(2),
            sample_move(),
            Value(0),
        );

        let p = tt.probe(key);
        assert_eq!(p.data.value, Value(2));
        assert_eq!(p.data.bound, Bound::Exact);
        assert_eq!(p.data.depth, Depth(2));
    }

    #[test]
    fn clear_wipes_all_entries() {
        let tt = TranspositionTable::new(1);
        tt.new_search();
        // Real Zobrist keys use the full 64 bits; a key with only
        // low-end entropy wouldn't hit the 16-bit identity check.
        let key = 0xBADC_AFED_EADB_EEFDu64 | (1u64 << 48);
        tt.probe(key).save(
            key,
            Value(99),
            false,
            Bound::Exact,
            Depth(5),
            sample_move(),
            Value(0),
        );
        assert!(tt.probe(key).hit);

        tt.clear();
        assert!(!tt.probe(key).hit);
    }

    #[test]
    fn new_search_bumps_generation() {
        let tt = TranspositionTable::new(1);
        let g0 = tt.generation.load(Ordering::Relaxed);
        tt.new_search();
        let g1 = tt.generation.load(Ordering::Relaxed);
        assert_eq!(g1, g0.wrapping_add(8));
    }

    #[test]
    fn hashfull_reports_non_zero_after_saves() {
        // Populate enough entries that the first 1000 slots are
        // plausibly touched.
        let tt = TranspositionTable::new(1);
        tt.new_search();
        for i in 0..500u64 {
            // Stagger keys across the whole 64-bit range so different
            // clusters are hit.
            let key = i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            tt.probe(key).save(
                key,
                Value(i as i32),
                false,
                Bound::Exact,
                Depth(3),
                sample_move(),
                Value(0),
            );
        }
        let hf = tt.hashfull();
        assert!(
            hf > 0,
            "hashfull should report some occupancy after saves, got {hf}"
        );
    }

    #[test]
    fn different_keys_in_same_cluster_coexist_up_to_cluster_size() {
        // Construct keys that map to the same cluster (same low 32 bits)
        // but differ in the high 16 identification bits. All three
        // should coexist.
        let tt = TranspositionTable::new(1);
        tt.new_search();
        let low = 0xCAFE_BABEu32 as u64;
        let keys = [
            low | (0x0001u64 << 48),
            low | (0x0002u64 << 48),
            low | (0x0003u64 << 48),
        ];
        for (i, &k) in keys.iter().enumerate() {
            tt.probe(k).save(
                k,
                Value(i as i32),
                false,
                Bound::Exact,
                Depth(1),
                sample_move(),
                Value(0),
            );
        }
        for (i, &k) in keys.iter().enumerate() {
            let p = tt.probe(k);
            assert!(p.hit, "key {i} should still be present");
            assert_eq!(p.data.value, Value(i as i32));
        }
    }
}
