use super::*;

/// Equal material / equal eval → a coin flip.
#[test]
fn zero_is_a_coin_flip() {
    assert!(win_chances(Value::ZERO).abs() < 1e-9);
}

/// Mate scores saturate regardless of distance.
#[test]
fn mate_saturates_to_plus_one() {
    assert_eq!(win_chances(Value::mate_in(1)), 1.0);
    assert_eq!(win_chances(Value::mate_in(40)), 1.0);
    assert_eq!(win_chances(Value::MATE), 1.0);
}

#[test]
fn mated_saturates_to_minus_one() {
    assert_eq!(win_chances(Value::mated_in(1)), -1.0);
    assert_eq!(win_chances(Value::mated_in(40)), -1.0);
}

/// The function is odd: `win_chances(-s) == -win_chances(s)`.
#[test]
fn symmetric_about_zero() {
    for cp in [50, 213, 600, 2538] {
        let pos = win_chances(Value(cp));
        let neg = win_chances(Value(-cp));
        assert!((pos + neg).abs() < 1e-9, "asymmetry at {cp}: {pos} vs {neg}");
    }
}

/// Monotonic: more advantage → higher win chance.
#[test]
fn monotonic_increasing() {
    let mut prev = win_chances(Value(-3000));
    for cp in [-1000, -213, 0, 213, 1000, 3000] {
        let wc = win_chances(Value(cp));
        assert!(wc > prev, "not increasing at {cp}");
        prev = wc;
    }
}

/// Sanity against the conventional-cp scale: a one-pawn advantage on our
/// internal scale (PAWN_EG = 213 ≈ 100 conventional cp) lands near lila's
/// published win-chance for +100 cp (≈ 0.18). If we forgot to normalize
/// and fed raw 213 to the sigmoid, we'd get ≈ 0.37 instead — this pins
/// the normalization down.
#[test]
fn one_pawn_matches_conventional_scale() {
    let wc = win_chances(Value::PAWN_EG);
    let expected = 2.0 / (1.0 + (-0.00368208_f64 * 100.0).exp()) - 1.0;
    assert!((wc - expected).abs() < 1e-9, "got {wc}, expected {expected}");
    assert!(wc > 0.15 && wc < 0.22, "one pawn should be ~0.18, got {wc}");
}

/// A large material edge approaches but never reaches ±1 (only mate does).
#[test]
fn large_advantage_approaches_one() {
    // A queen up (≈ +1260 conventional cp) is decisive but not certain.
    let queen = win_chances(Value::QUEEN_EG);
    assert!(queen > 0.95 && queen < 1.0, "a queen up should be decisive, got {queen}");
    // A huge non-mate edge gets very close to 1 without saturating.
    let huge = win_chances(Value(5000));
    assert!(huge > 0.99 && huge < 1.0, "a huge edge should near 1, got {huge}");
}
