//! The opening picker: choose which openings the bot may play. Lives in
//! its **own** resizable window (launched by a button in the New Game
//! dialog) rather than inline — the New Game modal auto-sizes to its
//! content, so width-hungry tree/column content placed inside it balloons
//! the modal off-screen. A dedicated bounded-width window gives the
//! three-column layout (opening → defense → system) room to lay out and
//! scroll properly.
//!
//! Renders the engine's read-only [`chess_tutor_engine::opening_tree`] and
//! mutates an [`OpeningSelection`] in place. No engine state is touched;
//! the selection commits onto the bot's
//! [`chess_tutor_engine::opponent::BookSelection`] only when the user
//! clicks Play (see [`crate::draw::dialog`]).

use eframe::egui;
use egui_phosphor::regular as phos;

use chess_tutor_engine::openings::{self, OpeningId};
use chess_tutor_engine::{book, opening_tree};
use chess_tutor_ui::session::OpeningSelection;

use super::icon;
use super::theme;

/// egui memory id for the "is the picker window open" flag and the
/// transient view state.
fn open_id() -> egui::Id {
    egui::Id::new("opening_picker_open")
}
fn state_id() -> egui::Id {
    egui::Id::new("opening_picker_state")
}

/// Transient view state (focused opening, search text).
#[derive(Clone, Default)]
struct PickerState {
    focused: usize,
    search: String,
}

/// One-line summary of the current selection, for the launch button.
pub(crate) fn summary(sel: &OpeningSelection) -> String {
    if sel.allowed.is_empty() {
        "no book (from move 1)".to_string()
    } else if sel.allowed.len() == book::all_ids().len() {
        "any opening".to_string()
    } else {
        format!("{} lines", sel.allowed.len())
    }
}

/// Open the picker window (called from the launch button).
pub(crate) fn open(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(open_id(), true));
}

/// Force the picker window closed — called when the New Game dialog is
/// dismissed so it doesn't auto-reappear next time the dialog opens.
pub(crate) fn close(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(open_id(), false));
}

/// Draw the picker window if it's open. Call once per frame at top level
/// (outside the New Game modal), after the dialog's main window.
pub(crate) fn draw_window(ctx: &egui::Context, sel: &mut OpeningSelection) {
    let is_open = ctx.data(|d| d.get_temp::<bool>(open_id()).unwrap_or(false));
    if !is_open {
        return;
    }
    let mut still_open = true;
    egui::Window::new("Choose openings")
        .open(&mut still_open)
        .collapsible(false)
        // Fixed size + centered + anchored: a consistent, locked window
        // that physically can't grow when content (e.g. search results)
        // gets wide.
        .fixed_size(egui::vec2(680.0, 470.0))
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| draw_contents(ui, sel));
    ctx.data_mut(|d| d.insert_temp(open_id(), still_open));
}

fn draw_contents(ui: &mut egui::Ui, sel: &mut OpeningSelection) {
    // ---- bulk ops + summary ----
    // An empty selection IS "no book" (bot plays from move 1) — there's
    // no separate toggle; Clear carries that meaning.
    ui.horizontal(|ui| {
        if ui.button("Select all").clicked() {
            sel.allowed = book::all_ids().into_iter().collect();
        }
        if ui.button("Clear").clicked() {
            sel.allowed.clear();
        }
        if sel.allowed.is_empty() {
            muted(ui, "→ no book — the bot plays from move 1");
        } else {
            muted(ui, &format!("→ {} lines allowed", sel.allowed.len()));
        }
    });

    let mut state: PickerState = ui.data(|d| d.get_temp(state_id()).unwrap_or_default());

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
    ui.separator();

    let query = state.search.trim().to_ascii_lowercase();
    if query.is_empty() {
        draw_three_columns(ui, sel, &mut state);
    } else {
        draw_search_results(ui, sel, &query);
    }

    ui.data_mut(|d| d.insert_temp(state_id(), state));
}

/// Three side-by-side columns: openings (Level 1) | the focused opening's
/// defenses + variations (Level 2/3) | cross-cutting systems. Each column
/// is given an explicit bounded width so its vertical scroll lays out
/// correctly regardless of window size.
fn draw_three_columns(ui: &mut egui::Ui, sel: &mut OpeningSelection, state: &mut PickerState) {
    let tree = opening_tree::tree();
    if tree.openings.is_empty() {
        return;
    }
    state.focused = state.focused.min(tree.openings.len() - 1);

    // Fixed column widths sized to their content — NOT fill-to-available,
    // which would stretch one column into a wall of white space and drag
    // the whole window wide.
    const OPEN_W: f32 = 190.0;
    const DEF_W: f32 = 265.0;
    const SYS_W: f32 = 175.0;
    const HEIGHT: f32 = 360.0;

    ui.horizontal_top(|ui| {
        // -- column 1: openings --
        column(ui, OPEN_W, HEIGHT, "op_open", |ui| {
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

        // -- column 2: defenses + variations of the focused opening --
        let group = &tree.openings[state.focused];
        column(ui, DEF_W, HEIGHT, "op_def", |ui| {
            ui.label(egui::RichText::new(&group.label).strong());
            ui.add_space(2.0);
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
        ui.separator();

        // -- column 3: cross-cutting systems --
        column(ui, SYS_W, HEIGHT, "op_sys", |ui| {
            ui.label(egui::RichText::new("Systems").strong());
            muted(ui, "Setups that cut across defenses.");
            ui.add_space(2.0);
            for tag in opening_tree::system_tags() {
                let ids = openings::find_ids_matching(tag.pattern);
                let active = !ids.is_empty() && ids.iter().all(|id| sel.allowed.contains(id));
                let label = format!("{} ({})", tag.label, ids.len());
                if ui.selectable_label(active, label).clicked() {
                    set_ids(sel, &ids, !active);
                }
            }
        });
    });
}

/// A fixed-width column hosting a vertical scroll area.
fn column(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    salt: &str,
    contents: impl FnOnce(&mut egui::Ui),
) {
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_min_width(width);
            ui.set_max_width(width);
            egui::ScrollArea::vertical()
                .id_salt(salt)
                .auto_shrink([false, false])
                .show(ui, contents);
        },
    );
}

/// Flat list of variation leaves matching the search, with a breadcrumb.
fn draw_search_results(ui: &mut egui::Ui, sel: &mut OpeningSelection, query: &str) {
    const CAP: usize = 300;
    let tree = opening_tree::tree();
    let mut shown = 0usize;
    let mut truncated = false;

    egui::ScrollArea::vertical()
        .id_salt("op_search")
        .auto_shrink([false, false])
        .max_height(360.0)
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
