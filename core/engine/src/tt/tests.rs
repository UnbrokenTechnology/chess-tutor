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
