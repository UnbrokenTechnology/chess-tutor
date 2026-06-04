//! Opening-picker tree — a read-only, renderer-neutral view over
//! [`crate::openings::entries`] for UI surfaces that let a user choose
//! which openings the bot may play (commits to
//! [`crate::opponent::BookSelection::Allowed`]).
//!
//! # Three levels, from two fields
//!
//! The opening → defense → variation hierarchy is *derived*, not stored
//! as such in the TSV:
//!
//! - **Level 1 — Opening** = White's first move ([`OpeningEntry::line`]'s
//!   first ply), mapped to a friendly label (`1.e4` → "King's Pawn
//!   (1.e4)"). This is what makes a *defense* a sub-category of the
//!   opening that leads to it: a Sicilian only exists under `1.e4`.
//! - **Level 2 — Defense / system** = the name prefix before `:`
//!   (`"Sicilian Defense"`, `"Ruy Lopez"`).
//! - **Level 3 — Variation** = the first comma-segment after `:`
//!   (`"Sicilian Defense: Najdorf, English Attack"` → `"Najdorf"`). All
//!   deeper sub-variations roll up into their lead segment, so a single
//!   "Najdorf" leaf aggregates every Najdorf line — toggling it off drops
//!   them all.
//!
//! Each node caches the [`OpeningId`]s beneath it so a UI can compute a
//! tri-state (all / some / none selected) by set membership without
//! re-walking strings each frame.
//!
//! # Systems are orthogonal ([`system_tags`])
//!
//! Lichess names a position with one combined label, so a White *system*
//! like the London smears across many Level-2 branches keyed by Black's
//! reply (`"London System"`, `"Indian Defense: London System"`, …). A
//! system is therefore **not** a single tree node; it's a cross-cutting
//! substring select resolved through [`crate::openings::find_ids_matching`].
//! [`system_tags`] is the curated set of popular such systems.
//!
//! Built once on first use and cached; the data is static for the life of
//! the process.

use std::sync::OnceLock;

use crate::openings::{self, OpeningId};
use crate::position::Position;
use crate::san;

/// The whole picker tree: Level-1 opening groups in canonical display
/// order (King's Pawn, Queen's Pawn, English, …, Irregular last).
#[derive(Debug, Clone)]
pub struct OpeningTree {
    pub openings: Vec<OpeningGroup>,
}

/// Level 1 — one White first-move family (e.g. "King's Pawn (1.e4)").
#[derive(Debug, Clone)]
pub struct OpeningGroup {
    pub label: String,
    pub families: Vec<FamilyGroup>,
    /// Every id beneath this group (union of its families' ids).
    pub ids: Vec<OpeningId>,
}

/// Level 2 — one defense / system name (e.g. "Sicilian Defense").
#[derive(Debug, Clone)]
pub struct FamilyGroup {
    pub name: String,
    pub variations: Vec<VariationLeaf>,
    /// Every id beneath this family (union of its variation leaves).
    pub ids: Vec<OpeningId>,
}

/// Level 3 — one variation (e.g. "Najdorf"), aggregating all its
/// sub-variations' rows.
#[derive(Debug, Clone)]
pub struct VariationLeaf {
    pub label: String,
    pub ids: Vec<OpeningId>,
}

/// A cross-cutting "system" quick-select: `label` is shown on the chip,
/// `pattern` is the substring fed to
/// [`crate::openings::find_ids_matching`] to resolve its ids.
#[derive(Debug, Clone, Copy)]
pub struct SystemTag {
    pub label: &'static str,
    pub pattern: &'static str,
}

/// Curated cross-cutting White systems that don't live in one tree
/// branch. Patterns are the *precise* system phrase (e.g. "London
/// System", which excludes "London Defense") — each verified non-empty
/// against the bundled TSVs by [`mod tests`]. Level-1 openings (Réti,
/// Zukertort) are deliberately *not* chips — they're already tree nodes.
const SYSTEM_TAGS: &[SystemTag] = &[
    SystemTag { label: "London System", pattern: "London System" },
    SystemTag { label: "Colle System", pattern: "Colle System" },
    SystemTag { label: "King's Indian Attack", pattern: "King's Indian Attack" },
    SystemTag { label: "Catalan", pattern: "Catalan" },
    SystemTag { label: "Stonewall", pattern: "Stonewall" },
    SystemTag { label: "Torre Attack", pattern: "Torre Attack" },
    SystemTag { label: "Trompowsky", pattern: "Trompowsky" },
];

/// The curated cross-cutting system chips. Resolve a chip's ids with
/// [`crate::openings::find_ids_matching`]`(tag.pattern)`.
pub fn system_tags() -> &'static [SystemTag] {
    SYSTEM_TAGS
}

static TREE: OnceLock<OpeningTree> = OnceLock::new();

/// The opening picker tree, built once from [`crate::openings::entries`].
pub fn tree() -> &'static OpeningTree {
    TREE.get_or_init(build_tree)
}

fn build_tree() -> OpeningTree {
    // Ordered builders: linear find-or-push keeps TSV (ECO, A→E) order
    // within each level while grouping. One-time cost at first use.
    let startpos = Position::startpos();
    let mut groups: Vec<OpeningGroup> = Vec::new();

    for entry in openings::entries() {
        let first_san = entry
            .line
            .first()
            .map(|&mv| san::format(&startpos, mv));
        let group_label = level1_label(first_san.as_deref());
        let (family_name, variation_label) = split_name(&entry.name);

        let group = find_or_push_group(&mut groups, group_label);
        let family = find_or_push_family(&mut group.families, family_name);
        let leaf = find_or_push_leaf(&mut family.variations, variation_label);
        leaf.ids.push(entry.id);
    }

    // Aggregate ids bottom-up, then sort every level by line count
    // (descending) — a good proxy for how commonly a line is played, so
    // the mainstream openings/defenses (Sicilian 380, Caro-Kann 106) sort
    // above rarities (Zukertort, KIA). "Irregular / Other" is pinned last
    // regardless of count.
    for group in &mut groups {
        for family in &mut group.families {
            family.ids = family
                .variations
                .iter()
                .flat_map(|v| v.ids.iter().copied())
                .collect();
            family.variations.sort_by_key(|v| std::cmp::Reverse(v.ids.len()));
        }
        group.ids = group
            .families
            .iter()
            .flat_map(|f| f.ids.iter().copied())
            .collect();
        group.families.sort_by_key(|f| std::cmp::Reverse(f.ids.len()));
    }
    groups.sort_by_key(|g| (g.label == "Irregular / Other", std::cmp::Reverse(g.ids.len())));

    OpeningTree { openings: groups }
}

/// Map a White first move (SAN) to its friendly Level-1 opening label.
/// `None` (an empty line) and any unrecognised first move fall into
/// "Irregular / Other".
fn level1_label(first_san: Option<&str>) -> String {
    let label = match first_san {
        Some("e4") => "King's Pawn (1.e4)",
        Some("d4") => "Queen's Pawn (1.d4)",
        Some("c4") => "English (1.c4)",
        Some("Nf3") => "Réti (1.Nf3)",
        Some("f4") => "Bird's (1.f4)",
        Some("b3") => "Nimzo-Larsen (1.b3)",
        Some("b4") => "Polish (1.b4)",
        Some("Nc3") => "Dunst (1.Nc3)",
        Some("g3") => "King's Fianchetto (1.g3)",
        _ => "Irregular / Other",
    };
    label.to_string()
}

/// Split a TSV opening name into (family, variation). Family is the part
/// before the first `:`; variation is the first comma-segment after it.
/// A bare name (no `:`) has family = whole name, variation = "Main line".
fn split_name(name: &str) -> (String, String) {
    match name.split_once(':') {
        Some((family, rest)) => {
            let variation = rest
                .split(',')
                .next()
                .unwrap_or(rest)
                .trim()
                .to_string();
            let variation = if variation.is_empty() {
                "Main line".to_string()
            } else {
                variation
            };
            (family.trim().to_string(), variation)
        }
        None => (name.trim().to_string(), "Main line".to_string()),
    }
}

fn find_or_push_group(groups: &mut Vec<OpeningGroup>, label: String) -> &mut OpeningGroup {
    if let Some(i) = groups.iter().position(|g| g.label == label) {
        &mut groups[i]
    } else {
        groups.push(OpeningGroup { label, families: Vec::new(), ids: Vec::new() });
        groups.last_mut().expect("just pushed")
    }
}

fn find_or_push_family(families: &mut Vec<FamilyGroup>, name: String) -> &mut FamilyGroup {
    if let Some(i) = families.iter().position(|f| f.name == name) {
        &mut families[i]
    } else {
        families.push(FamilyGroup { name, variations: Vec::new(), ids: Vec::new() });
        families.last_mut().expect("just pushed")
    }
}

fn find_or_push_leaf(leaves: &mut Vec<VariationLeaf>, label: String) -> &mut VariationLeaf {
    if let Some(i) = leaves.iter().position(|v| v.label == label) {
        &mut leaves[i]
    } else {
        leaves.push(VariationLeaf { label, ids: Vec::new() });
        leaves.last_mut().expect("just pushed")
    }
}

#[cfg(test)]
#[path = "opening_tree_tests.rs"]
mod tests;
