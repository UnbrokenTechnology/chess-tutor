//! Sibling tests for [`super`] (`glossary.rs`). The directional gloss
//! is the one piece of agent-facing context the engine's own `label()`
//! / `pretty_label()` don't carry, so the tests guard against it
//! silently going missing.

use super::*;
use chess_tutor_engine::analysis::TermId;

#[test]
fn every_term_has_a_non_empty_description() {
    // Exhaustiveness: the match in `description` covers every variant
    // and every gloss is at least a sentence long.
    for &id in &TermId::ALL {
        let d = description(id);
        assert!(
            d.len() > 30,
            "description for {:?} is too short: {:?}",
            id,
            d,
        );
    }
}

#[test]
fn descriptions_carry_a_directional_marker() {
    // Every gloss has to tell the agent whose side a positive number
    // favours. We enforce that by requiring "WE" or "THEY" or
    // "THEIR" / "THEIRS" or "OUR" or "OURS" to appear in every gloss.
    // (Initiative is the documented exception — it has no per-side
    // direction; positive = mover gains tempo.)
    for &id in &TermId::ALL {
        let d = description(id);
        let has_marker = d.contains("WE")
            || d.contains("THEY")
            || d.contains("THEIR")
            || d.contains("OUR")
            || matches!(id, TermId::Initiative);
        assert!(
            has_marker,
            "{:?}: gloss missing directional marker: {:?}",
            id, d,
        );
    }
}

#[test]
fn render_glossary_table_includes_every_label() {
    let table = render_glossary_table();
    for &id in &TermId::ALL {
        assert!(
            table.contains(id.label()),
            "{:?} ({}) missing from glossary table",
            id,
            id.label(),
        );
    }
}
