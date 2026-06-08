use super::*;

fn base_dials() -> BotDials {
    BotDials {
        depth: 2,
        qsearch: None,
        perception: 0.5,
        avg_move_rank: 2.0,
        endgame_skill: None,
        mask_safety: false,
        mask_positional: false,
    }
}

// ---- Elo -> dials: ladder reproduction + interpolation -----------------

#[test]
fn config_reproduces_ladder_rungs() {
    let c = config_for_elo(1000.0);
    assert_eq!(c.depth, 1);
    assert_eq!(c.qsearch, Some(2));
    assert_eq!(c.endgame_skill, Some(2));
    assert!((c.perception - 0.80).abs() < 1e-4, "perc {}", c.perception);
    assert!((c.avg_move_rank - 2.7).abs() < 1e-4, "rank {}", c.avg_move_rank);

    let bottom = config_for_elo(500.0);
    assert_eq!((bottom.depth, bottom.qsearch, bottom.endgame_skill), (1, Some(1), Some(1)));
    assert!((bottom.perception - 0.20).abs() < 1e-4);

    let top = config_for_elo(2000.0);
    assert_eq!((top.depth, top.qsearch, top.endgame_skill), (4, None, None));
    assert!((top.perception - 1.0).abs() < 1e-4);
    assert!((top.avg_move_rank - 1.0).abs() < 1e-4);
}

#[test]
fn config_interpolates_rank_within_a_band() {
    // 1450 sits between t1400 (r1.9) and t1500 (r1.7), same d2q2 band.
    let c = config_for_elo(1450.0);
    assert_eq!(c.depth, 2);
    assert_eq!(c.qsearch, Some(2));
    assert!((c.avg_move_rank - 1.8).abs() < 1e-4, "rank {}", c.avg_move_rank);
}

#[test]
fn config_clamps_outside_the_ladder() {
    assert_eq!(config_for_elo(100.0), config_for_elo(ELO_MIN));
    assert_eq!(config_for_elo(9999.0), config_for_elo(ELO_MAX));
}

// ---- the ladder-anchoring guarantee ------------------------------------

#[test]
fn default_config_displays_its_target_exactly() {
    // elo_for_dials(config_for_elo(t), t) must equal t (the anchor cancels).
    let mut t = 500.0;
    while t <= 2500.0 {
        let d = config_for_elo(t);
        let shown = elo_for_dials(&d, t);
        assert!((shown - t).abs() < 1e-6, "target {t} displayed {shown}");
        t += 50.0;
    }
}

#[test]
fn tweaking_a_dial_moves_the_displayed_elo() {
    // From a 1400 default, lowering perception must drop the shown Elo;
    // raising rank must drop it; both correctly signed and non-zero.
    let def = config_for_elo(1400.0);
    let base_elo = elo_for_dials(&def, 1400.0);

    let mut weaker_p = def;
    weaker_p.perception = 0.5;
    assert!(elo_for_dials(&weaker_p, 1400.0) < base_elo - 1.0);

    let mut weaker_r = def;
    weaker_r.avg_move_rank = def.avg_move_rank + 1.5;
    assert!(elo_for_dials(&weaker_r, 1400.0) < base_elo - 1.0);
}

// ---- monotonicity of the forward model in every dial -------------------

fn assert_monotone(label: &str, vals: &[f64], increasing: bool) {
    for w in vals.windows(2) {
        let ok = if increasing { w[1] >= w[0] - 1e-6 } else { w[1] <= w[0] + 1e-6 };
        assert!(ok, "{label} not monotone: {w:?}");
    }
}

#[test]
fn model_is_monotone_per_dial() {
    // perception ↑ ⇒ Elo ↑
    let p: Vec<f64> = (0..=20)
        .map(|i| {
            let mut d = base_dials();
            d.perception = i as f32 / 20.0;
            model_elo(&d)
        })
        .collect();
    assert_monotone("perception", &p, true);

    // avg_move_rank ↑ ⇒ Elo ↓
    let r: Vec<f64> = (10..=80)
        .map(|i| {
            let mut d = base_dials();
            d.avg_move_rank = i as f32 / 10.0;
            model_elo(&d)
        })
        .collect();
    assert_monotone("rank", &r, false);

    // depth ↑ ⇒ Elo ↑
    let dp: Vec<f64> = (1..=7)
        .map(|i| {
            let mut d = base_dials();
            d.depth = i;
            model_elo(&d)
        })
        .collect();
    assert_monotone("depth", &dp, true);

    // qsearch: q1 < q2 < full
    let q: Vec<f64> = [Some(1), Some(2), None]
        .iter()
        .map(|&q| {
            let mut d = base_dials();
            d.qsearch = q;
            model_elo(&d)
        })
        .collect();
    assert_monotone("qsearch", &q, true);

    // endgame: 0 < 1 < 2 < Full
    let eg: Vec<f64> = [Some(0), Some(1), Some(2), None]
        .iter()
        .map(|&e| {
            let mut d = base_dials();
            d.endgame_skill = e;
            model_elo(&d)
        })
        .collect();
    assert_monotone("eg", &eg, true);
}

#[test]
fn perception_monotone_even_at_high_rank() {
    // The cross-term was clamped precisely so this holds: at rank 8 (very
    // weak), more perception must still not LOWER Elo.
    let vals: Vec<f64> = (0..=20)
        .map(|i| {
            let mut d = base_dials();
            d.avg_move_rank = 8.0;
            d.perception = i as f32 / 20.0;
            model_elo(&d)
        })
        .collect();
    assert_monotone("perception@r8", &vals, true);
}

#[test]
fn safety_mask_is_a_handicap_fading_with_depth() {
    let mut d1 = base_dials();
    d1.depth = 1;
    let mut d1m = d1;
    d1m.mask_safety = true;
    let shallow = model_elo(&d1) - model_elo(&d1m); // penalty at d1
    assert!(shallow > 15.0, "safety should cost real Elo at d1, got {shallow}");

    let mut d6 = base_dials();
    d6.depth = 6;
    let mut d6m = d6;
    d6m.mask_safety = true;
    let deep = model_elo(&d6) - model_elo(&d6m);
    assert!(deep < shallow, "safety penalty should fade with depth ({deep} vs {shallow})");
}

// ---- inverse rank solve ------------------------------------------------

#[test]
fn solve_rank_round_trips() {
    // Off-ladder config (full vision, full eg): find the rank that lands a
    // target, then confirm the forward model agrees.
    let mut d = BotDials {
        depth: 2,
        qsearch: None,
        perception: 1.0,
        avg_move_rank: 1.0,
        endgame_skill: None,
        mask_safety: false,
        mask_positional: false,
    };
    for &target in &[1100.0, 1300.0, 1500.0] {
        let r = solve_rank(target, &d);
        d.avg_move_rank = r;
        let got = elo_for_dials(&d, target);
        assert!((got - target).abs() < 25.0, "target {target}: rank {r} -> {got}");
    }
}
