//! Build the [`crate::view`] descriptors the renderers consume: top bar,
//! eval bar, board, side panel, move list, retrospective, the hint
//! pop-over, the new-game dialog, and the on-demand game review.

use super::*;

use super::queries::entry_eval_white_pov;
use chess_tutor_engine::types::{Color, Move, Square, Value};

use crate::learning_mode::build_intervention_panel;
use crate::view::{
    ActionBarView, BoardView, EvalBarView, EvalSample, HintPopoverView, MoveListCell, MoveListRow,
    MoveListView, NewGameDialogView, PromotionPickerView, RetrospectiveBody, RetrospectiveKind,
    RetrospectivePanelView, ReviewModeView, ReviewTallyRow, ReviewVerdictTier, SidePanelBody,
    SidePanelView, TopBarView,
};

/// Map an engine [`MoveVerdict`] to the summary tally tier. `Best` and
/// `BestAvailable` both fold into `Best` (both are "as good as it
/// gets"); the rest map one-to-one.
fn verdict_tier(v: chess_tutor_engine::analysis::MoveVerdict) -> ReviewVerdictTier {
    use chess_tutor_engine::analysis::MoveVerdict;
    match v {
        MoveVerdict::Best | MoveVerdict::BestAvailable => ReviewVerdictTier::Best,
        MoveVerdict::Good => ReviewVerdictTier::Good,
        MoveVerdict::Inaccuracy => ReviewVerdictTier::Inaccuracy,
        MoveVerdict::Mistake => ReviewVerdictTier::Mistake,
        MoveVerdict::Miss => ReviewVerdictTier::Miss,
        MoveVerdict::Blunder => ReviewVerdictTier::Blunder,
    }
}

/// Saturation extent for the eval-over-time graph, in pawns. Mate /
/// huge-swing samples clamp here so one extreme value doesn't flatten
/// the rest of the curve. Same spirit as the eval bar's cp saturation.
const EVAL_GRAPH_SATURATION_PAWNS: f32 = 10.0;

/// Saturation point for the eval bar's score→ratio mapping. Used by
/// [`Session::build_eval_bar_view`]; lives at module scope so the only
/// constant referenced by view-building stays adjacent to the
/// session.
const EVAL_BAR_SATURATION_CP: f32 = 1000.0;

/// Map the eval-bar's white-POV score to a `(fill_ratio, label)`.
///
/// The score is the analysis of the move that *reached* the viewed
/// position, so it is rooted **one ply before** that position. For a
/// continuous cp score that one-ply offset is invisible. For a **mate**
/// score it is not: `MATE − |v|` is the plies-to-mate from the analysis
/// root, so the distance from the position the bar actually labels is
/// one ply less (the move already on the board). We therefore drop that
/// ply and render the result in **moves** (`M{n}`), matching the
/// retrospective headline's `#n` — not the raw plies the old formula
/// showed. When the dropped ply *was* the mating move, the viewed
/// position is checkmate itself: show a bare `#` and let the game-over
/// text carry the result rather than printing a misleading "M0".
///
/// (This corrects only the display off-by-one + plies-vs-moves; the
/// separate issue that independent retrospective searches can disagree
/// on the true mate distance — the MultiPV-around-mate pathology — is
/// tracked separately and not addressed here.)
fn eval_bar_fill_and_label(score: Option<Value>) -> (f32, String) {
    match score {
        Some(v) if v.abs() >= Value::MATE_IN_MAX_PLY => {
            let plies_here = (Value::MATE.0 - v.0.abs()) - 1;
            let sign = if v.0 > 0 { "" } else { "-" };
            let ratio = if v.0 > 0 { 1.0 } else { 0.0 };
            let label = if plies_here <= 0 {
                format!("{sign}#")
            } else {
                format!("{sign}M{}", (plies_here + 1) / 2)
            };
            (ratio, label)
        }
        Some(v) => {
            let ratio = (v.0 as f32 / EVAL_BAR_SATURATION_CP).clamp(-1.0, 1.0);
            let pawns = v.0 as f32 / Value::PAWN_EG.0 as f32;
            // chess.com-style rounding keeps the in-bar number narrow:
            // whole pawns once we're +/-10 or more (the position is
            // decided), one decimal below that — so "+25", "+5.4", "+0.6",
            // never a five-char "+25.14" overflowing the thin bar. The cut
            // is 9.95, not 10.0, so a value that would *round up* to "10.0"
            // (5 chars) renders as "+10" instead.
            let label = if pawns.abs() >= 9.95 {
                format!("{pawns:+.0}")
            } else {
                format!("{pawns:+.1}")
            };
            (0.5 + 0.5 * ratio, label)
        }
        None => (0.5, String::from("—")),
    }
}

/// Whether a board annotation's square(s) intersect `keys` — used by the
/// review-mode auto-render to decide if a card's spatial story bears on
/// the engine's best move or the move played.
fn annotation_touches(ann: &crate::view::BoardAnnotation, keys: &[Square]) -> bool {
    use crate::view::BoardAnnotation as A;
    match ann {
        A::Arrow { from, to, .. } => keys.contains(from) || keys.contains(to),
        A::SquareHighlight { square, .. } => keys.contains(square),
    }
}

impl Session {
    /// Material-aware [`MoveVerdict`] for *any* analysed move at history
    /// index `idx` (either colour), or `None` when its retrospective
    /// hasn't arrived. Opponent moves are analysed retrospectively too,
    /// so this grades them with the same classifier the student's moves
    /// use — the summary table reads it for the Black/White split.
    pub(crate) fn move_verdict_for(
        &self,
        idx: usize,
    ) -> Option<chess_tutor_engine::analysis::MoveVerdict> {
        use chess_tutor_engine::analysis::compute_material_outcome;

        let entry = self.history.get(idx)?;
        let retro = entry.retrospective.as_ref()?;
        let best = retro.analyses.first()?;
        let user = retro.analyses.iter().find(|a| a.mv == retro.user_move)?;
        let pre = self.pre_move_position(idx);
        let root_stm = pre.side_to_move();
        let user_net = compute_material_outcome(user, &pre, root_stm).net_mg_cp;
        let best_net = compute_material_outcome(best, &pre, root_stm).net_mg_cp;
        Some(user.classify_with_material(best.score, user_net, best_net))
    }

    /// Per-verdict tally split by side, in display order
    /// ([`ReviewVerdictTier::ALL`]). Counts every analysed move on each
    /// colour — the summary renders it as a White/Black table.
    fn review_tallies(&self) -> Vec<ReviewTallyRow> {
        let mut white = [0usize; ReviewVerdictTier::ALL.len()];
        let mut black = [0usize; ReviewVerdictTier::ALL.len()];
        for idx in 0..self.history.len() {
            if let Some(v) = self.move_verdict_for(idx) {
                let tier = verdict_tier(v);
                if let Some(slot) = ReviewVerdictTier::ALL.iter().position(|t| *t == tier) {
                    match self.history[idx].moved_by {
                        Color::White => white[slot] += 1,
                        Color::Black => black[slot] += 1,
                    }
                }
            }
        }
        ReviewVerdictTier::ALL
            .iter()
            .enumerate()
            .map(|(slot, tier)| ReviewTallyRow {
                tier: *tier,
                label: tier.label(),
                white: white[slot],
                black: black[slot],
            })
            .collect()
    }

    /// White-POV eval samples for the eval-over-time graph, one per
    /// analysed ply, clamped to the graph's saturation extent.
    fn review_eval_series(&self) -> Vec<EvalSample> {
        self.history
            .iter()
            .enumerate()
            .filter_map(|(idx, entry)| {
                let v = entry_eval_white_pov(entry)?;
                let pawns = (v.0 as f32 / Value::PAWN_EG.0 as f32)
                    .clamp(-EVAL_GRAPH_SATURATION_PAWNS, EVAL_GRAPH_SATURATION_PAWNS);
                Some(EvalSample {
                    history_index: idx,
                    pawns,
                })
            })
            .collect()
    }

    pub fn build_game_review(&self) -> Option<crate::view::GameReviewView> {
        use crate::view::GameReviewView;

        let user_move_count = self
            .history
            .iter()
            .filter(|e| self.is_user_move(e))
            .count();
        if user_move_count == 0 {
            return None;
        }
        Some(GameReviewView {
            game_outcome: self.game_outcome(),
            user_move_count,
            tallies: self.review_tallies(),
            user_is_white: self.user_color() == Color::White,
            eval_series: self.review_eval_series(),
        })
    }

    /// Whether either game-review surface (summary or review mode) is
    /// currently showing.
    pub fn is_game_review_open(&self) -> bool {
        self.review_phase.is_open()
    }

    // ---- Event dispatch ------------------------------------------------

    pub fn build_top_bar_view(&self) -> TopBarView {
        TopBarView {
            viewing_live: self.is_viewing_live(),
            engine_thinking: self.engine_thinking,
            game_outcome: self.game_outcome(),
        }
    }

    pub fn build_action_bar_view(&self) -> ActionBarView {
        let hint_can_open = self.is_viewing_live()
            && !self.engine_thinking
            && self.is_users_turn()
            && self.game_outcome().is_none();
        let review_button_enabled = self
            .history
            .iter()
            .any(|e| self.is_user_move(e) && e.retrospective.is_some());
        ActionBarView {
            can_takeback: !self.history.is_empty(),
            hint_open: self.hint_open,
            hint_button_enabled: hint_can_open || self.hint_open,
            game_over: self.game_outcome().is_some(),
            review_open: self.review_phase.is_open(),
            review_button_enabled,
        }
    }

    pub fn build_eval_bar_view(&self) -> EvalBarView {
        let (white_ratio, label) = eval_bar_fill_and_label(self.viewed_eval_white_pov());
        EvalBarView { white_ratio, label }
    }

    pub fn build_board_view(&self) -> BoardView {
        let viewed_pos = self.viewed_position().clone();
        let viewed_mv = self.viewed_entry().map(|e| e.mv);
        let live = self.is_viewing_live();
        let pending_promotion = self.pending_promotion.as_ref().map(|p| {
            PromotionPickerView::compose(
                p.to,
                p.candidates,
                self.position.side_to_move(),
                self.flipped,
            )
        });
        // When browsing back, suppress mouse-state overlays: the
        // selected piece and its legal-move dots belong to the *live*
        // position, not the historical one we're displaying. The
        // BoardCell.selected / move_dot fields stay None.
        let (selected, legals): (Option<Square>, &[Move]) = if live {
            (self.selected, &self.legal_from_selected)
        } else {
            (None, &[])
        };
        let annotations = self.collect_board_annotations();
        BoardView::compose(
            &viewed_pos,
            self.flipped,
            viewed_mv,
            selected,
            legals,
            pending_promotion,
            annotations,
        )
    }

    /// Gather any annotations to draw on the board. Sources:
    /// - Active board overlays (always-on, computed against the
    ///   currently-viewed position).
    /// - The currently-viewed user-move entry's retrospective: best-
    ///   move arrow always shown; the selected card's annotations
    ///   layer on top.
    /// - Future: trap-refutation arrows, pin renderer per HANDOFF-ux.
    pub(crate) fn collect_board_annotations(&self) -> Vec<crate::view::BoardAnnotation> {
        let mut out = Vec::new();

        // Overlays first, so retrospective annotations paint on top.
        if !self.active_overlays.is_empty() {
            crate::overlays_view::push_overlay_annotations(
                &mut out,
                &chess_tutor_engine::analysis::compute_overlays(self.viewed_position()),
                self.user_color(),
                &self.active_overlays,
            );
        }

        let Some((entry_idx, entry)) = self.panel_entry_with_index() else {
            return out;
        };
        let Some(result) = &entry.retrospective else {
            return out;
        };
        // Same card path for user and engine moves — only the perspective
        // differs (the board annotations themselves are perspective-neutral
        // geometry; perspective only flips which side a card frames).
        let perspective = if self.is_user_move(entry) {
            chess_tutor_teaching::phrasing::Perspective::Player
        } else {
            chess_tutor_teaching::phrasing::Perspective::Opponent
        };
        let pre = self.pre_move_position(entry_idx);
        let prior_move = self.prior_move_for(entry_idx);
        let vm = crate::retrospective_view::build_retrospective_view(
            &pre,
            &result.analyses,
            result.user_move,
            self.show_all_signals,
            self.reveal_best_moves_effective(),
            prior_move,
            perspective,
        );
        if let Some(ann) = vm.headline.best_move_annotation {
            out.push(ann);
        }
        if let Some((selected_entry, item_idx)) = self.selected_retrospective {
            if selected_entry == entry_idx {
                if let Some(item) = vm.items.get(item_idx) {
                    out.extend(item.annotations.iter().copied());
                }
            }
        }
        // Review-mode auto-render: paint, without a click, every card whose
        // spatial story bears on the engine's best move or the move played
        // — "only the important stuff" (the hanging piece *if* the best move
        // captures it, the fork you played, …). A positional card on
        // squares unrelated to either move stays click-only, so the board
        // doesn't drown in arrows. Post-game an answer key is appropriate;
        // during live play we never auto-reveal (decision #9), so this is
        // gated to review.
        if self.review_phase == ReviewPhase::Reviewing {
            let mut keys: Vec<Square> = Vec::new();
            if let Some(best) = result.analyses.first() {
                keys.push(best.mv.from());
                keys.push(best.mv.to());
            }
            keys.push(result.user_move.from());
            keys.push(result.user_move.to());
            for item in &vm.items {
                if item.annotations.iter().any(|a| annotation_touches(a, &keys)) {
                    out.extend(item.annotations.iter().copied());
                }
            }
        }
        out
    }

    /// Snapshot of the currently-active overlay set. Renderers consume
    /// this to draw the overlay checkboxes with the right initial
    /// state.
    pub fn active_overlays(&self) -> &std::collections::HashSet<crate::view::OverlayKind> {
        &self.active_overlays
    }

    pub fn build_side_panel_view(&self) -> SidePanelView {
        // The side-panel body is *only ever* backward-looking now
        // (PLAN §"coaching/hint model"): coaching pops over instead of
        // sharing this slot. Body priority, top to bottom:
        //   Intervention
        //     > Retrospective (the default; also the body in review mode,
        //       which adds the nav-bar chrome via `review_mode`).
        // The game-review summary is no longer a body — it floats over
        // this surface as a popover (`build_review_summary_view`).
        let mut review_mode: Option<ReviewModeView> = None;
        let body = if let Some(pending) = self.pending_intervention.as_ref() {
            SidePanelBody::Intervention(build_intervention_panel(pending))
        } else {
            if self.review_phase == ReviewPhase::Reviewing {
                review_mode = Some(self.build_review_mode_view());
            }
            SidePanelBody::Retrospective(self.build_retrospective_view())
        };
        SidePanelView {
            moves: self.build_move_list_view(),
            body,
            active_overlays: self.active_overlays.clone(),
            learning: self.learning,
            stick_to_bottom: self.is_viewing_live() && review_mode.is_none(),
            review_mode,
        }
    }

    /// Nav-bar chrome for step-through review mode. The feedback zone
    /// itself reuses the retrospective body; this carries only which nav
    /// buttons are live + autoplay state.
    fn build_review_mode_view(&self) -> ReviewModeView {
        let total = self.history.len();
        let cur = self.viewing_index.unwrap_or(total.saturating_sub(1));
        ReviewModeView {
            can_step_back: cur > 0,
            can_step_forward: total > 0 && cur + 1 < total,
            autoplay: self.review_autoplay,
        }
    }

    /// The game-review summary popover (White/Black verdict table + eval
    /// curve), or `None` when it's dismissed or there is nothing to
    /// review. Floats over the step-through panel rather than gating entry
    /// to it.
    pub fn build_review_summary_view(&self) -> Option<crate::view::GameReviewView> {
        if !self.review_summary_open {
            return None;
        }
        self.build_game_review()
    }

    /// Build the on-demand Hint pop-over from the live position, or
    /// `None` when it's closed. The pop-over is the *only* coaching
    /// surface now (PLAN §"coaching/hint model"); it floats over the
    /// backward-looking side panel so the two coexist. The "what to
    /// notice" content is the same `build_coaching_view` snapshot the
    /// old persistent panel used — names patterns/squares, never the
    /// move — sub-millisecond and rebuilt every frame the pop-over is
    /// open.
    ///
    /// Gated to the live position on the user's turn: a pop-over while
    /// browsing back or while it's the engine's move would describe a
    /// position the student can't act on. `hint_open` may still be set
    /// in those moments (e.g. auto-coach fired and the engine started
    /// thinking); returning `None` keeps the pop-over from showing
    /// stale advice without forcing the caller to also clear the flag.
    pub fn build_hint_popover_view(&self) -> Option<HintPopoverView> {
        if !self.hint_open
            || !self.is_viewing_live()
            || !self.is_users_turn()
            || self.game_outcome().is_some()
        {
            return None;
        }
        let tactic_hint = self.coaching_tactic_hint();
        let view_model = crate::coaching_view::build_coaching_view(
            &self.position,
            self.user_color(),
            tactic_hint.as_ref(),
            self.coaching_prior_move(),
        );
        Some(HintPopoverView { view_model })
    }

    pub(crate) fn build_move_list_view(&self) -> MoveListView {
        let viewing = self.viewing_index;
        let history_len = self.history.len();
        // Inline rating glyphs are a post-game answer key — only populate
        // them while stepping through review (decision #9). Computing a
        // material-aware verdict per row is cheap (it reads cached
        // analyses) but pointless during live play, where ratings would
        // spoil. Both sides are rated: recognising the opponent's blunders
        // is as instructive as recognising your own.
        let rate = self.review_phase == ReviewPhase::Reviewing;
        let rating_at = |idx: usize| -> Option<ReviewVerdictTier> {
            rate.then(|| self.move_verdict_for(idx).map(verdict_tier))
                .flatten()
        };
        let rows = (0..history_len.div_ceil(2))
            .map(|pair| {
                let i_white = pair * 2;
                let i_black = i_white + 1;
                let white = MoveListCell {
                    history_index: i_white,
                    san: self.history[i_white].san.clone(),
                    selected: viewing == Some(i_white),
                    rating: rating_at(i_white),
                };
                let black = self.history.get(i_black).map(|e| MoveListCell {
                    history_index: i_black,
                    san: e.san.clone(),
                    selected: viewing == Some(i_black),
                    rating: rating_at(i_black),
                });
                MoveListRow {
                    move_pair_idx: pair + 1,
                    white,
                    black,
                }
            })
            .collect();
        MoveListView { rows }
    }

    /// Whether the retrospective should reveal the engine's preferred
    /// move (SAN chip + board arrow). The student's `reveal_best_moves`
    /// preference enables it for live after-move feedback; **review mode**
    /// also enables it unconditionally — post-game it's an answer key, not
    /// a mid-game spoiler (the chess.com-familiar idiom), so showing the
    /// alternative helps the student see what they missed faster.
    pub(crate) fn reveal_best_moves_effective(&self) -> bool {
        self.learning.reveal_best_moves || self.review_phase == ReviewPhase::Reviewing
    }

    pub(crate) fn build_retrospective_view(&self) -> RetrospectivePanelView {
        let game_outcome = self.game_outcome();
        let Some((entry_index, entry)) = self.panel_entry_with_index() else {
            return RetrospectivePanelView {
                game_outcome,
                body: RetrospectiveBody::NoMoves,
                show_all_signals: self.show_all_signals,
                expanded: self.retro_expanded,
            };
        };
        let viewing_back_san = (!self.is_viewing_live()).then(|| entry.san.clone());
        // Both user and engine moves are now analysed retrospectively and
        // rendered through the *same* card path — only the perspective
        // differs (`Player` for the user's own moves, `Opponent` for the
        // engine's). A move whose retrospective worker job hasn't returned
        // yet shows the analysing spinner regardless of who moved.
        let perspective = if self.is_user_move(entry) {
            chess_tutor_teaching::phrasing::Perspective::Player
        } else {
            chess_tutor_teaching::phrasing::Perspective::Opponent
        };
        let kind = match &entry.retrospective {
            Some(result) => {
                let pre = self.pre_move_position(entry_index);
                let prior_move = self.prior_move_for(entry_index);
                let view_model = crate::retrospective_view::build_retrospective_view(
                    &pre,
                    &result.analyses,
                    result.user_move,
                    self.show_all_signals,
                    self.reveal_best_moves_effective(),
                    prior_move,
                    perspective,
                );
                let selected_item = match self.selected_retrospective {
                    Some((h, i)) if h == entry_index => Some(i),
                    _ => None,
                };
                RetrospectiveKind::MoveReady {
                    perspective,
                    view_model: Box::new(view_model),
                    selected_item,
                }
            }
            None => RetrospectiveKind::Analyzing,
        };
        RetrospectivePanelView {
            game_outcome,
            body: RetrospectiveBody::Entry {
                viewing_back_san,
                kind,
            },
            show_all_signals: self.show_all_signals,
            expanded: self.retro_expanded,
        }
    }

    pub fn build_new_game_dialog_view(&mut self) -> Option<NewGameDialogView<'_>> {
        let first_launch = self.first_launch;
        let form = self.new_game_form.as_mut()?;
        Some(NewGameDialogView { form, first_launch })
    }

    /// Build the mid-game ⚙ settings descriptor, or `None` when the
    /// gear surface is closed. Snapshots the live option values so the
    /// renderer paints each control with its current state; the
    /// renderer's per-option intents edit the live session directly.
    pub fn build_settings_view(&self) -> Option<crate::view::SettingsView> {
        if !self.settings_open {
            return None;
        }
        Some(crate::view::SettingsView {
            retrospective_depth: self.retrospective_depth,
            show_eval_bar: self.show_eval_bar,
            learning: self.learning,
            active_overlays: self.active_overlays.clone(),
        })
    }

    /// Whether the eval bar should be rendered. Renderers reserve the
    /// left gutter only when this is `true`.
    pub fn eval_bar_visible(&self) -> bool {
        self.show_eval_bar
    }
}

#[cfg(test)]
#[path = "view_builders_tests.rs"]
mod tests;
