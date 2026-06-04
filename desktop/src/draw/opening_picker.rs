//! The "Openings" section of the New Game dialog: choose which openings
//! the bot may play. Renders the engine's read-only
//! [`chess_tutor_engine::opening_tree`] as a master-detail tri-state tree
//! (opening → defense → variation), plus cross-cutting "system" chips and
//! a search box, all mutating a [`OpeningSelection`] in place.
//!
//! No engine state is touched here — the selection is committed onto the
//! bot's [`chess_tutor_engine::opponent::BookSelection`] only when the
//! user clicks Play (see [`crate::draw::dialog`]).

use eframe::egui;
use egui_phosphor::regular as phos;

use chess_tutor_engine::opening_tree::{self};
use chess_tutor_engine::openings::{self, OpeningId};
use chess_tutor_ui::session::{OpeningMode, OpeningSelection};

use super::icon;
use super::theme;

/// Transient view state (focused opening, search text) kept in egui's
/// temp memory so it survives across frames without polluting the form.
#[derive(Clone, Default)]
struct PickerState {
    focused: usize,
    search: String,
}

/// Draw the whole Openings section into `ui`, mutating `sel`.
pub(crate) fn draw(ui: &mut egui::Ui, sel: &mut OpeningSelection) {
    // ---- mode ----
    ui.horizontal(|ui| {
        ui.radio_value(&mut sel.mode, OpeningMode::Any, "Any opening");
        ui.radio_value(&mut sel.mode, OpeningMode::Only, "Only these…");
        ui.radio_value(&mut sel.mode, OpeningMode::None, "None");
    });

    match sel.mode {
        OpeningMode::Any => {
            muted(ui, "The bot may play any theoretical opening.");
            return;
        }
        OpeningMode::None => {
            muted(ui, "No opening book — the bot plays from move 1 by search.");
            return;
        }
        OpeningMode::Only => {}
    }

    let id = ui.make_persistent_id("opening_picker_state");
    let mut state: PickerState = ui.data(|d| d.get_temp(id).unwrap_or_default());

    // ---- summary ----
    muted(ui, &format!("→ {} lines allowed", sel.allowed.len()));

    // ---- system chips (cross-cutting) ----
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new("Systems:").color(theme::TEXT_MUTED));
        for tag in opening_tree::system_tags() {
            let ids = openings::find_ids_matching(tag.pattern);
            let active = !ids.is_empty() && ids.iter().all(|id| sel.allowed.contains(id));
            if ui.selectable_label(active, tag.label).clicked() {
                set_ids(sel, &ids, !active);
            }
        }
    });

    // ---- search ----
    ui.horizontal(|ui| {
        ui.label(icon::icon(phos::MAGNIFYING_GLASS).color(theme::TEXT_MUTED));
        ui.add(
            egui::TextEdit::singleline(&mut state.search)
                .hint_text("search any opening, defense, or system…")
                .desired_width(f32::INFINITY),
        );
        if !state.search.is_empty() && ui.small_button("✕").clicked() {
            state.search.clear();
        }
    });
    ui.add_space(4.0);

    let query = state.search.trim().to_ascii_lowercase();
    if query.is_empty() {
        draw_master_detail(ui, sel, &mut state);
    } else {
        draw_search_results(ui, sel, &query);
    }

    ui.data_mut(|d| d.insert_temp(id, state));
}

/// Left = Level-1 openings (click to focus); right = the focused
/// opening's families, each expanding to variation leaves.
fn draw_master_detail(ui: &mut egui::Ui, sel: &mut OpeningSelection, state: &mut PickerState) {
    let tree = opening_tree::tree();
    if tree.openings.is_empty() {
        return;
    }
    state.focused = state.focused.min(tree.openings.len() - 1);

    ui.horizontal_top(|ui| {
        // -- left: opening list --
        egui::ScrollArea::vertical()
            .id_salt("op_l1")
            .max_height(260.0)
            .show(ui, |ui| {
                ui.set_min_width(160.0);
                ui.set_max_width(160.0);
                for (i, g) in tree.openings.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if tri_box(ui, tri_of(sel, &g.ids)).clicked() {
                            toggle(sel, &g.ids);
                        }
                        let label = format!("{} ({})", g.label, g.ids.len());
                        if ui.selectable_label(state.focused == i, label).clicked() {
                            state.focused = i;
                        }
                    });
                }
            });

        ui.separator();

        // -- right: focused opening's families --
        let group = &tree.openings[state.focused];
        egui::ScrollArea::vertical()
            .id_salt("op_r")
            .max_height(260.0)
            .show(ui, |ui| {
                ui.set_min_width(280.0);
                for (fi, fam) in group.families.iter().enumerate() {
                    let header_id = ui.make_persistent_id(("op_fam", state.focused, fi));
                    egui::collapsing_header::CollapsingState::load_with_default_open(
                        ui.ctx(),
                        header_id,
                        false,
                    )
                    .show_header(ui, |ui| {
                        if tri_box(ui, tri_of(sel, &fam.ids)).clicked() {
                            toggle(sel, &fam.ids);
                        }
                        ui.label(format!("{} ({})", fam.name, fam.ids.len()));
                    })
                    .body(|ui| {
                        for leaf in &fam.variations {
                            ui.horizontal(|ui| {
                                ui.add_space(8.0);
                                if tri_box(ui, tri_of(sel, &leaf.ids)).clicked() {
                                    toggle(sel, &leaf.ids);
                                }
                                ui.label(format!("{} ({})", leaf.label, leaf.ids.len()));
                            });
                        }
                    });
                }
            });
    });
}

/// Flat list of variation leaves whose opening / family / variation text
/// matches the search, with a breadcrumb so the user knows where each
/// lives. Capped so a broad query can't render thousands of rows.
fn draw_search_results(ui: &mut egui::Ui, sel: &mut OpeningSelection, query: &str) {
    const CAP: usize = 200;
    let tree = opening_tree::tree();
    let mut shown = 0usize;
    let mut truncated = false;

    egui::ScrollArea::vertical()
        .id_salt("op_search")
        .max_height(280.0)
        .show(ui, |ui| {
            for g in &tree.openings {
                for fam in &g.families {
                    for leaf in &fam.variations {
                        let hay = format!("{} {} {}", g.label, fam.name, leaf.label)
                            .to_ascii_lowercase();
                        if !hay.contains(query) {
                            continue;
                        }
                        if shown >= CAP {
                            truncated = true;
                            break;
                        }
                        shown += 1;
                        ui.horizontal(|ui| {
                            if tri_box(ui, tri_of(sel, &leaf.ids)).clicked() {
                                toggle(sel, &leaf.ids);
                            }
                            ui.label(leaf.label.clone());
                            ui.label(
                                egui::RichText::new(format!("— {} · {}", fam.name, g.label))
                                    .small()
                                    .color(theme::TEXT_MUTED),
                            );
                        });
                    }
                }
            }
            if shown == 0 {
                muted(ui, "No opening matches that search.");
            } else if truncated {
                muted(ui, &format!("Showing first {CAP} matches — refine the search."));
            }
        });
}

// ---- tri-state helpers ------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Tri {
    None,
    Some,
    All,
}

fn tri_of(sel: &OpeningSelection, ids: &[OpeningId]) -> Tri {
    if ids.is_empty() {
        return Tri::None;
    }
    let n = ids.iter().filter(|id| sel.allowed.contains(*id)).count();
    if n == 0 {
        Tri::None
    } else if n == ids.len() {
        Tri::All
    } else {
        Tri::Some
    }
}

/// Toggle a node: anything not fully selected becomes fully selected;
/// fully selected becomes cleared.
fn toggle(sel: &mut OpeningSelection, ids: &[OpeningId]) {
    let on = !matches!(tri_of(sel, ids), Tri::All);
    set_ids(sel, ids, on);
}

fn set_ids(sel: &mut OpeningSelection, ids: &[OpeningId], on: bool) {
    if on {
        sel.allowed.extend(ids.iter().copied());
    } else {
        for id in ids {
            sel.allowed.remove(id);
        }
    }
}

/// A frameless tri-state checkbox button (checked / partial / empty).
fn tri_box(ui: &mut egui::Ui, state: Tri) -> egui::Response {
    let glyph = match state {
        Tri::All => phos::CHECK_SQUARE,
        Tri::Some => phos::MINUS_SQUARE,
        Tri::None => phos::SQUARE,
    };
    let color = match state {
        Tri::All => theme::GOOD,
        Tri::Some => theme::WARN,
        Tri::None => theme::TEXT_MUTED,
    };
    ui.add(egui::Button::new(icon::icon(glyph).size(16.0).color(color)).frame(false))
}

fn muted(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).small().color(theme::TEXT_MUTED));
}
