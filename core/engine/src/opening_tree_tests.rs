use super::*;
use std::collections::HashSet;

// ---- coverage: every entry lands in exactly one leaf -----------------

#[test]
fn every_entry_appears_in_exactly_one_leaf() {
    let t = tree();
    let mut all: Vec<OpeningId> = Vec::new();
    for g in &t.openings {
        for f in &g.families {
            for v in &f.variations {
                all.extend(v.ids.iter().copied());
            }
        }
    }
    let unique: HashSet<OpeningId> = all.iter().copied().collect();
    assert_eq!(
        all.len(),
        openings::entries().len(),
        "leaf id total must equal entry count (no row dropped or duplicated)",
    );
    assert_eq!(
        unique.len(),
        all.len(),
        "no id may appear in two leaves",
    );
}

#[test]
fn node_ids_are_unions_of_children() {
    let t = tree();
    for g in &t.openings {
        let fam_union: usize = g.families.iter().map(|f| f.ids.len()).sum();
        assert_eq!(g.ids.len(), fam_union, "group ids = sum of family ids");
        for f in &g.families {
            let var_union: usize = f.variations.iter().map(|v| v.ids.len()).sum();
            assert_eq!(f.ids.len(), var_union, "family ids = sum of leaf ids");
        }
    }
}

// ---- Level-1 ordering and content -----------------------------------

#[test]
fn kings_pawn_is_first_and_irregular_last() {
    let t = tree();
    assert_eq!(
        t.openings.first().map(|g| g.label.as_str()),
        Some("King's Pawn (1.e4)"),
        "1.e4 dominates and should sort first",
    );
    assert_eq!(
        t.openings.last().map(|g| g.label.as_str()),
        Some("Irregular / Other"),
        "the catch-all bucket sorts last",
    );
}

#[test]
fn sicilian_nests_under_kings_pawn_with_a_najdorf_leaf() {
    let t = tree();
    let kp = t
        .openings
        .iter()
        .find(|g| g.label == "King's Pawn (1.e4)")
        .expect("King's Pawn group");
    let sicilian = kp
        .families
        .iter()
        .find(|f| f.name == "Sicilian Defense")
        .expect("Sicilian Defense under King's Pawn");
    let najdorf = sicilian
        .variations
        .iter()
        .find(|v| v.label.contains("Najdorf"))
        .expect("a Najdorf leaf");
    assert!(
        najdorf.ids.len() >= 2,
        "the Najdorf leaf should aggregate several sub-variation rows, got {}",
        najdorf.ids.len(),
    );
}

// ---- name splitting --------------------------------------------------

#[test]
fn split_name_handles_family_variation_and_bare() {
    assert_eq!(
        split_name("Sicilian Defense: Najdorf Variation, English Attack"),
        ("Sicilian Defense".to_string(), "Najdorf Variation".to_string()),
    );
    assert_eq!(
        split_name("London System"),
        ("London System".to_string(), "Main line".to_string()),
    );
    assert_eq!(
        split_name("Ruy Lopez: Berlin Defense"),
        ("Ruy Lopez".to_string(), "Berlin Defense".to_string()),
    );
}

#[test]
fn level1_label_maps_known_and_unknown_first_moves() {
    assert_eq!(level1_label(Some("e4")), "King's Pawn (1.e4)");
    assert_eq!(level1_label(Some("c4")), "English (1.c4)");
    assert_eq!(level1_label(Some("h4")), "Irregular / Other");
    assert_eq!(level1_label(None), "Irregular / Other");
}

// ---- system chips ----------------------------------------------------

#[test]
fn every_system_tag_resolves_to_a_nonempty_set() {
    for tag in system_tags() {
        let ids = openings::find_ids_matching(tag.pattern);
        assert!(
            !ids.is_empty(),
            "system chip {:?} (pattern {:?}) matched nothing — dead chip",
            tag.label,
            tag.pattern,
        );
    }
}

#[test]
fn london_system_chip_excludes_london_defense() {
    // The precision guarantee: "London System" must not grab
    // "Grob Opening: London Defense" or "… London Defensive System".
    for id in openings::find_ids_matching("London System") {
        let name = openings::entry(id).expect("entry").name.clone();
        assert!(
            !name.contains("London Defense"),
            "London System chip wrongly matched a London Defense row: {name}",
        );
    }
}
