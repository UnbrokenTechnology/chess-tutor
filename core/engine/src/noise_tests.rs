use super::*;
use crate::engine::SearchLine;
use crate::types::{Move, Square, Value};

/// Stub line with the given score and an empty PV — `pick` only
/// reads `score`, so the rest is filler.
fn line(score_cp: i32) -> SearchLine {
    SearchLine {
        pv: Vec::<Move>::new(),
        score: Value(score_cp),
        depth: 1,
        ply_traces: Vec::new(),
        settled_ply: None,
    }
}

/// Distinct stub moves keyed by an index — used by the wild-branch
/// tests where we need to tell apart "which legal move came back".
fn stub_move(seed: u8) -> Move {
    // Any two squares will do; the picker treats Move as an opaque
    // value. Mapping `seed` to a unique from-square gives us a
    // stable identity for assertion comparisons.
    let from = Square::from_index(seed % 64);
    let to = Square::from_index(seed.wrapping_add(8) % 64);
    Move::normal(from, to)
}

#[test]
fn off_profile_always_picks_first() {
    let noise = NoiseProfile::default();
    let lines = vec![line(50), line(40), line(30), line(20)];
    for ply in 0..20 {
        assert_eq!(pick(&noise, 0xCAFE, ply, &lines, &[]), NoisePick::Line(0));
    }
}

#[test]
fn single_line_always_picks_zero() {
    let noise = NoiseProfile {
        candidate_pool: 4,
        temperature_cp: 200,
        blunder_chance: 1.0, // even a guaranteed blunder has nothing to pick
        ..Default::default()
    };
    let lines = vec![line(10)];
    // Wild is off → only one line and no qualifying alternative.
    assert_eq!(pick(&noise, 0xCAFE, 1, &lines, &[]), NoisePick::Line(0));
}

#[test]
fn empty_lines_picks_zero() {
    // Defensive — caller checks emptiness, but pick shouldn't panic.
    let noise = NoiseProfile::default();
    let lines: Vec<SearchLine> = Vec::new();
    assert_eq!(pick(&noise, 0, 0, &lines, &[]), NoisePick::Line(0));
}

#[test]
fn pool_one_skips_softmax_even_with_temperature() {
    // candidate_pool=1 is the "softmax off" signal regardless of
    // temperature. The user must opt into pool > 1 to get noise.
    let noise = NoiseProfile {
        candidate_pool: 1,
        temperature_cp: 1_000,
        ..Default::default()
    };
    let lines = vec![line(0), line(-10), line(-20)];
    for ply in 0..10 {
        assert_eq!(pick(&noise, 0xBEEF, ply, &lines, &[]), NoisePick::Line(0));
    }
}

#[test]
fn zero_temperature_with_pool_picks_first() {
    // Without temperature, softmax collapses to "always #1" even at
    // wide pool. This is the "give me variety only when scores are
    // close" knob if the user later sets temperature.
    let noise = NoiseProfile {
        candidate_pool: 4,
        temperature_cp: 0,
        ..Default::default()
    };
    let lines = vec![line(100), line(99), line(98), line(97)];
    for ply in 0..10 {
        assert_eq!(pick(&noise, 0xFEED, ply, &lines, &[]), NoisePick::Line(0));
    }
}

#[test]
fn softmax_picks_within_pool_only() {
    // High temperature + 3-deep pool: the picker must never return
    // 3 (which sits outside the pool), even though we provided 4
    // lines.
    let noise = NoiseProfile {
        candidate_pool: 3,
        temperature_cp: 500, // very flat — all three weighted similarly
        ..Default::default()
    };
    let lines = vec![line(20), line(15), line(10), line(-200)];
    for ply in 0..200 {
        let pick = pick(&noise, 0xABCD, ply, &lines, &[]);
        match pick {
            NoisePick::Line(idx) => assert!(idx < 3, "softmax leaked outside pool: {idx}"),
            other => panic!("non-softmax pick at ply {ply}: {other:?}"),
        }
    }
}

#[test]
fn softmax_actually_varies_across_plies() {
    let noise = NoiseProfile {
        candidate_pool: 3,
        temperature_cp: 50,
        ..Default::default()
    };
    let lines = vec![line(0), line(-10), line(-20)];
    let mut seen = [0usize; 3];
    for ply in 0..200 {
        match pick(&noise, 0xDEAD, ply, &lines, &[]) {
            NoisePick::Line(idx) => seen[idx] += 1,
            other => panic!("non-softmax pick at ply {ply}: {other:?}"),
        }
    }
    let distinct = seen.iter().filter(|&&c| c > 0).count();
    assert!(distinct >= 2, "softmax never varied: {seen:?}");
    assert!(seen[0] >= seen[1] && seen[0] >= seen[2], "modal pick wasn't #1: {seen:?}");
}

#[test]
fn blunder_with_no_in_band_lines_picks_closest_below() {
    // No line falls in the band [100, INF]. The fallback pool is
    // the line(s) with the largest loss strictly below the band's
    // lower edge — here that's idx 3 (loss=90). The bot picks
    // there rather than playing #1, preserving the "gradual
    // decline" property in quiet positions where no real blunder
    // is available.
    let noise = NoiseProfile {
        candidate_pool: 1,
        blunder_chance: 1.0,
        blunder_min_loss_cp: 100,
        blunder_max_loss_cp: i32::MAX,
        ..Default::default()
    };
    let lines = vec![line(0), line(-10), line(-50), line(-90)];
    for ply in 0..20 {
        assert_eq!(
            pick(&noise, 0xABCD, ply, &lines, &[]),
            NoisePick::Blunder(3),
            "fallback should pick the largest sub-band loss (idx 3, -90)",
        );
    }
}

#[test]
fn blunder_picks_only_in_band_lines_when_some_qualify() {
    // Band [100, INF] with losses 50, 99, 100, 300: in-band set
    // is {idx 3 (loss=100), idx 4 (loss=300)}. The picker must
    // never pick #1 or the sub-band lines (50, 99).
    let noise = NoiseProfile {
        candidate_pool: 1,
        blunder_chance: 1.0,
        blunder_min_loss_cp: 100,
        blunder_max_loss_cp: i32::MAX,
        guaranteed_mate_in: 0,
        ..Default::default()
    };
    let lines = vec![line(0), line(-50), line(-99), line(-100), line(-300)];
    for ply in 0..50 {
        match pick(&noise, 0x1234, ply, &lines, &[]) {
            NoisePick::Blunder(idx) => assert!(
                idx == 3 || idx == 4,
                "blunder picked outside in-band set: {idx}",
            ),
            NoisePick::Line(idx) => panic!(
                "blunder branch should fire (chance=1.0), got Line({idx})",
            ),
            NoisePick::BlunderSkipped { .. } => panic!(
                "in-band set is non-empty; should never skip",
            ),
            NoisePick::Wild(_) => panic!("wild fired without wild_chance > 0"),
        }
    }
}

#[test]
fn blunder_band_excludes_too_catastrophic() {
    // The whole point of the upper band: with max=400, an alt
    // line at loss=1000 (queen-hang territory) must never be
    // picked when a 200-cp option exists. Band = [100, 400];
    // in-band set = {idx 2 (loss=200)}; the loss=1000 line is
    // excluded.
    let noise = NoiseProfile {
        candidate_pool: 1,
        blunder_chance: 1.0,
        blunder_min_loss_cp: 100,
        blunder_max_loss_cp: 400,
        guaranteed_mate_in: 0,
        ..Default::default()
    };
    let lines = vec![line(0), line(-50), line(-200), line(-1000)];
    for ply in 0..50 {
        assert_eq!(
            pick(&noise, 0xCAFE, ply, &lines, &[]),
            NoisePick::Blunder(2),
            "should only pick the in-band move (idx 2, -200)",
        );
    }
}

#[test]
fn blunder_band_fallback_pools_closest_on_each_side() {
    // Band [50, 100] with losses 10, 30, 110, 240: in-band is
    // empty. Closest-below (largest loss < 50) is idx 2 (loss=30).
    // Closest-above (smallest loss > 100) is idx 3 (loss=110).
    // The 240-cp line is excluded because 110 is closer to the
    // band from above. Pool = {idx 2, idx 3}; pick must be one
    // of those.
    let noise = NoiseProfile {
        candidate_pool: 1,
        blunder_chance: 1.0,
        blunder_min_loss_cp: 50,
        blunder_max_loss_cp: 100,
        guaranteed_mate_in: 0,
        ..Default::default()
    };
    let lines = vec![line(0), line(-10), line(-30), line(-110), line(-240)];
    let mut seen_below = 0;
    let mut seen_above = 0;
    for ply in 0..200 {
        match pick(&noise, 0xBEEF, ply, &lines, &[]) {
            NoisePick::Blunder(2) => seen_below += 1,
            NoisePick::Blunder(3) => seen_above += 1,
            NoisePick::Blunder(idx) => panic!(
                "fallback picked outside the closest-on-each-side pool: {idx}",
            ),
            other => panic!("non-blunder pick: {other:?}"),
        }
    }
    assert!(seen_below > 0, "closest-below tier never picked");
    assert!(seen_above > 0, "closest-above tier never picked");
}

#[test]
fn blunder_skipped_when_only_alternative_is_catastrophic() {
    // The motivating case for BLUNDER_FALLBACK_TOLERANCE: in a
    // forcing position the engine's #1 may be much stronger than
    // every alternative (e.g. found a tactic, every other move
    // loses 20+ pawns). The old code happily picked the 20-pawn
    // drop because "the closest above-band line wins by default."
    // The new behaviour skips the blunder entirely so the bot
    // plays its best move and the configured rate is slightly
    // under-delivered rather than producing absurd play.
    let noise = NoiseProfile {
        candidate_pool: 1,
        blunder_chance: 1.0,
        blunder_min_loss_cp: 50,
        blunder_max_loss_cp: 100,
        guaranteed_mate_in: 0,
        ..Default::default()
    };
    // Tolerance 2.0× → above cap is 200 cp. All alts (2156, etc.)
    // exceed that → no fallback admitted → skip.
    let lines = vec![line(0), line(-2156), line(-2300), line(-3000)];
    for ply in 0..30 {
        match pick(&noise, 0xCAFE, ply, &lines, &[]) {
            NoisePick::BlunderSkipped { closest_above_loss_cp } => {
                assert_eq!(
                    closest_above_loss_cp, 2156,
                    "skipped pick should report the closest rejected loss",
                );
            }
            other => panic!(
                "with no plausible alternative and no below tier the picker must \
                 skip; got {other:?}",
            ),
        }
    }
}

#[test]
fn blunder_fallback_admits_above_within_tolerance() {
    // Above-tier loss 180 cp; cap = 2.0 × 100 = 200. Admitted.
    let noise = NoiseProfile {
        candidate_pool: 1,
        blunder_chance: 1.0,
        blunder_min_loss_cp: 50,
        blunder_max_loss_cp: 100,
        guaranteed_mate_in: 0,
        ..Default::default()
    };
    let lines = vec![line(0), line(-180), line(-500)];
    for ply in 0..30 {
        match pick(&noise, 0xCAFE, ply, &lines, &[]) {
            NoisePick::Blunder(1) => {} // expected
            other => panic!("admitted above tier should be picked: {other:?}"),
        }
    }
}

#[test]
fn blunder_fallback_below_works_even_when_above_capped() {
    // Tight band [50, 100]. Alts: 30 (below), 1500 (above, way
    // over cap). Cap rejects 1500; pool = {idx 1 (loss=30)}.
    // Subtle decline path still works.
    let noise = NoiseProfile {
        candidate_pool: 1,
        blunder_chance: 1.0,
        blunder_min_loss_cp: 50,
        blunder_max_loss_cp: 100,
        guaranteed_mate_in: 0,
        ..Default::default()
    };
    let lines = vec![line(0), line(-30), line(-1500)];
    for ply in 0..30 {
        match pick(&noise, 0xCAFE, ply, &lines, &[]) {
            NoisePick::Blunder(1) => {} // expected
            other => panic!(
                "below-tier should still be admitted even when above is capped: \
                 {other:?}",
            ),
        }
    }
}

#[test]
fn blunder_band_fallback_with_only_above_band_lines() {
    // No in-band, no below-band — every line is catastrophic
    // (e.g. forced position where any deviation loses heavily).
    // The pool collapses to the smallest above-band loss; the bot
    // takes the least-bad of the bad options.
    let noise = NoiseProfile {
        candidate_pool: 1,
        blunder_chance: 1.0,
        blunder_min_loss_cp: 100,
        blunder_max_loss_cp: 300,
        guaranteed_mate_in: 0,
        ..Default::default()
    };
    // Losses: 500, 800, 1200 — all > max=300.
    let lines = vec![line(0), line(-500), line(-800), line(-1200)];
    for ply in 0..30 {
        assert_eq!(
            pick(&noise, 0xFACE, ply, &lines, &[]),
            NoisePick::Blunder(1),
            "should pick the least-catastrophic above-band line (idx 1, -500)",
        );
    }
}

#[test]
fn blunder_band_fallback_includes_tied_losses() {
    // Two lines at the same closest-below loss should both be
    // in the fallback pool — ties are kept rather than the picker
    // arbitrarily favouring one.
    let noise = NoiseProfile {
        candidate_pool: 1,
        blunder_chance: 1.0,
        blunder_min_loss_cp: 200,
        blunder_max_loss_cp: 400,
        guaranteed_mate_in: 0,
        ..Default::default()
    };
    // Losses: 50, 100, 100 — in-band empty; closest-below = 100
    // (tied at idx 2 and idx 3). Pool = {2, 3}.
    let lines = vec![line(0), line(-50), line(-100), line(-100)];
    let mut seen = [0usize; 4];
    for ply in 0..200 {
        match pick(&noise, 0xDEAD, ply, &lines, &[]) {
            NoisePick::Blunder(idx) => {
                assert!(idx == 2 || idx == 3, "out-of-pool pick: {idx}");
                seen[idx] += 1;
            }
            other => panic!("non-blunder pick: {other:?}"),
        }
    }
    assert!(seen[2] > 0 && seen[3] > 0, "tied losses must both be reachable: {seen:?}");
}

#[test]
fn blunder_suppressed_when_mate_guarded() {
    let noise = NoiseProfile {
        candidate_pool: 1,
        blunder_chance: 1.0,
        blunder_min_loss_cp: 100,
        blunder_max_loss_cp: i32::MAX,
        guaranteed_mate_in: 3,
        ..Default::default()
    };
    let mate_in_2 = Value::MATE.0 - 3;
    let lines = vec![line(mate_in_2), line(0), line(-100)];
    for ply in 0..20 {
        assert_eq!(pick(&noise, 0xFACE, ply, &lines, &[]), NoisePick::Line(0));
    }
}

#[test]
fn blunder_allowed_for_mate_beyond_guarantee() {
    let noise = NoiseProfile {
        candidate_pool: 1,
        blunder_chance: 1.0,
        blunder_min_loss_cp: 100,
        blunder_max_loss_cp: i32::MAX,
        guaranteed_mate_in: 3,
        ..Default::default()
    };
    let mate_in_5 = Value::MATE.0 - 9;
    let lines = vec![line(mate_in_5), line(0), line(-100)];
    let mut saw_blunder = false;
    for ply in 0..20 {
        if matches!(pick(&noise, 0xFACE, ply, &lines, &[]), NoisePick::Blunder(_)) {
            saw_blunder = true;
            break;
        }
    }
    assert!(saw_blunder, "blunder branch never fired against mate-in-5");
}

#[test]
fn guaranteed_mate_zero_disables_protection() {
    let noise = NoiseProfile {
        candidate_pool: 1,
        blunder_chance: 1.0,
        blunder_min_loss_cp: 100,
        blunder_max_loss_cp: i32::MAX,
        guaranteed_mate_in: 0,
        ..Default::default()
    };
    let mate_in_1 = Value::MATE.0 - 1;
    let lines = vec![line(mate_in_1), line(0), line(-100)];
    let mut saw_blunder = false;
    for ply in 0..20 {
        if matches!(pick(&noise, 0xFACE, ply, &lines, &[]), NoisePick::Blunder(_)) {
            saw_blunder = true;
            break;
        }
    }
    assert!(saw_blunder, "guaranteed_mate_in=0 should not protect mate-in-1");
}

#[test]
fn mate_guard_does_not_protect_being_mated() {
    let noise = NoiseProfile {
        candidate_pool: 1,
        blunder_chance: 1.0,
        blunder_min_loss_cp: 100,
        blunder_max_loss_cp: i32::MAX,
        guaranteed_mate_in: 5,
        ..Default::default()
    };
    let getting_mated_in_2 = -(Value::MATE.0 - 3);
    let lines = vec![line(getting_mated_in_2), line(-200), line(-1000)];
    for ply in 0..20 {
        // No assertion on exact pick — just that the function
        // doesn't panic and returns a valid line index. Either
        // Line(idx) or Blunder(idx) is fine here.
        match pick(&noise, 0xBABE, ply, &lines, &[]) {
            NoisePick::Line(idx) | NoisePick::Blunder(idx) => {
                assert!(idx < lines.len())
            }
            NoisePick::BlunderSkipped { .. } => {
                // Valid outcome: mate-zone above tier got capped
                // out by the fallback tolerance.
            }
            NoisePick::Wild(_) => panic!("wild fired without wild_chance > 0"),
        }
    }
}

#[test]
fn pick_is_deterministic_for_same_inputs() {
    let noise = NoiseProfile {
        candidate_pool: 4,
        temperature_cp: 200,
        blunder_chance: 0.3,
        blunder_min_loss_cp: 80,
        blunder_max_loss_cp: i32::MAX,
        guaranteed_mate_in: 1,
        wild_chance: 0.1,
    };
    let lines = vec![line(0), line(-20), line(-50), line(-150), line(-400)];
    let legal = vec![stub_move(0), stub_move(1), stub_move(2), stub_move(3)];
    for ply in 0..20 {
        let a = pick(&noise, 0xABCD, ply, &lines, &legal);
        let b = pick(&noise, 0xABCD, ply, &lines, &legal);
        assert_eq!(a, b, "same inputs gave different picks at ply {ply}");
    }
}

#[test]
fn pick_varies_with_seed() {
    let noise = NoiseProfile {
        candidate_pool: 4,
        temperature_cp: 200,
        ..Default::default()
    };
    let lines = vec![line(0), line(-20), line(-40), line(-80)];
    let seq_a: Vec<_> = (0..50).map(|p| pick(&noise, 0x1111_2222, p, &lines, &[])).collect();
    let seq_b: Vec<_> = (0..50).map(|p| pick(&noise, 0xAAAA_BBBB, p, &lines, &[])).collect();
    assert_ne!(seq_a, seq_b, "seed didn't affect the pick sequence");
}

// ---- wild branch -------------------------------------------------

#[test]
fn wild_fires_only_when_chance_set() {
    // Default profile → no wild even with legal moves provided.
    let noise = NoiseProfile::default();
    let legal = vec![stub_move(0), stub_move(1)];
    for ply in 0..20 {
        assert_eq!(pick(&noise, 0x9999, ply, &[], &legal), NoisePick::Line(0));
    }
}

#[test]
fn wild_with_no_legal_moves_falls_through() {
    // Wild can't fire without a legal-move list; should fall back
    // to the engine-result branches (which also have nothing here).
    let noise = NoiseProfile {
        wild_chance: 1.0,
        ..Default::default()
    };
    assert_eq!(pick(&noise, 0x9999, 0, &[line(0)], &[]), NoisePick::Line(0));
}

#[test]
fn wild_picks_from_full_legal_list_not_just_top_k() {
    // 8 legal moves, only 3 "search lines". With wild_chance=1.0
    // every pick should be a Wild that comes from the legal list —
    // including moves the search never surfaced.
    let noise = NoiseProfile {
        wild_chance: 1.0,
        guaranteed_mate_in: 0, // disable mate-guard
        ..Default::default()
    };
    let lines = vec![line(0), line(-10), line(-20)];
    let legal: Vec<Move> = (0..8).map(stub_move).collect();
    let mut seen_indices = [false; 8];
    for ply in 0..200 {
        match pick(&noise, 0xC0DE, ply, &lines, &legal) {
            NoisePick::Wild(mv) => {
                let idx = legal.iter().position(|m| *m == mv).expect("wild move not in legal list");
                seen_indices[idx] = true;
            }
            other => panic!("wild_chance=1.0 must always pick Wild; got {other:?}"),
        }
    }
    let distinct = seen_indices.iter().filter(|&&b| b).count();
    assert!(distinct >= 4, "wild barely varied — saw only {distinct}/8 legal moves");
}

#[test]
fn wild_suppressed_when_mate_guarded() {
    // Bot has mate-in-1, guaranteed_mate_in=1 — wild must not fire
    // (would throw away the forced mate).
    let noise = NoiseProfile {
        wild_chance: 1.0,
        guaranteed_mate_in: 1,
        ..Default::default()
    };
    let mate_in_1 = Value::MATE.0 - 1;
    let lines = vec![line(mate_in_1)];
    let legal: Vec<Move> = (0..4).map(stub_move).collect();
    for ply in 0..20 {
        assert_eq!(pick(&noise, 0xFACE, ply, &lines, &legal), NoisePick::Line(0));
    }
}

#[test]
fn blunder_takes_precedence_over_wild_and_softmax() {
    // With blunder_chance=1.0 the blunder branch should always
    // win, regardless of how the other knobs are set. Pins the
    // branch ordering documented at the module level: blunder is
    // the calibrated mistake signal and gets first crack, then
    // wild, then softmax.
    let noise = NoiseProfile {
        candidate_pool: 4,
        temperature_cp: 200,
        blunder_chance: 1.0,
        wild_chance: 1.0,
        guaranteed_mate_in: 0,
        ..Default::default()
    };
    let lines = vec![line(0), line(-50), line(-200), line(-400)];
    let legal: Vec<Move> = (0..6).map(stub_move).collect();
    for ply in 0..30 {
        match pick(&noise, 0xBEEF, ply, &lines, &legal) {
            NoisePick::Blunder(idx) => assert!(
                idx >= 1,
                "blunder must never pick #1 (got Blunder({idx}))",
            ),
            other => panic!(
                "non-blunder pick at ply {ply}: {other:?} (blunder rolls first \
                 at chance=1.0 — must always win)",
            ),
        }
    }
}
