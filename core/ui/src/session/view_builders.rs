//! Build the [`crate::view`] descriptors the renderers consume: top bar,
//! eval bar, board, side panel, move list, retrospective, coaching, hint,
//! the new-game dialog, and the on-demand game review.

use super::*;

use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::types::{Color, Move, Square, Value};

use crate::learning_mode::{
    build_intervention_panel, gating_config_for,
};
use crate::view::{
    ActionBarView, BoardView, CoachingPanelView, EvalBarView, HintEntryView, HintPanelState,
    HintPanelView, MoveListCell, MoveListRow, MoveListView, NewGameDialogView, PromotionPickerView,
    RetrospectiveBody, RetrospectiveKind, RetrospectivePanelView, SidePanelBody, SidePanelView,
    TopBarView,
};

/// Build the short headline shown for a moment in the game review
/// list. Mirrors the in-game prompt phrasing without ever naming the
/// engine's preferred move.
pub(crate) fn review_headline_for(
    assessment: &chess_tutor_engine::analysis::MoveAssessment,
) -> String {
    if let Some(b) = assessment.blunder {
        let pawns = (b.material_loss_cp as f32) / 100.0;
        return match b.lost_piece_square {
            Some(sq) => format!(
                "Material at risk: piece on {} ({:.1} pawns)",
                sq.to_algebraic(),
                pawns
            ),
            None => format!("Material at risk: {:.1} pawns", pawns),
        };
    }
    if let Some(a) = assessment.allowed.as_ref() {
        // ALLOWED-not-MISSED: the row leads with what the move allowed,
        // not a missed point. Pattern name + swing, no squares.
        return format!(
            "You allowed {} ({:.1} pawns swing)",
            crate::learning_mode::allowed_pattern_phrase_pub(a.walked_into.pattern),
            (a.conceded_cp as f32) / 100.0,
        );
    }
    if let Some(t) = assessment.teaching {
        let (area_a, _) = crate::learning_mode::term_prompt_copy(t.dominant.term);
        return match t.secondary {
            None => format!(
                "Missed point: {} ({:.1} pawns concentrated)",
                area_a,
                (t.dominant.severity_cp as f32) / 100.0
            ),
            Some(secondary) => {
                let (area_b, _) = crate::learning_mode::term_prompt_copy(secondary.term);
                let combined = ((t.dominant.severity_cp + secondary.severity_cp) as f32) / 100.0;
                format!(
                    "Missed points: {} and {} ({:.1} pawns split)",
                    area_a, area_b, combined
                )
            }
        };
    }
    "Significant moment".to_string()
}

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
            (0.5 + 0.5 * ratio, format!("{:+.2}", pawns))
        }
        None => (0.5, String::from("—")),
    }
}

/// Format a score for display in the hint panel. Root-stm POV is
/// the natural reading there ("if you play this, you'll be at
/// +0.30").
pub(crate) fn format_score_root_pov(score: Value, _root_stm: Color) -> String {
    if score.abs() >= Value::MATE_IN_MAX_PLY {
        if score.0 > 0 {
            format!("M{}", (Value::MATE.0 - score.0).max(1))
        } else {
            format!("-M{}", (Value::MATE.0 + score.0).max(1))
        }
    } else {
        let pawns = score.0 as f32 / Value::PAWN_EG.0 as f32;
        format!("{:+.2}", pawns)
    }
}

/// Walk a PV applying moves to a clone of `root` and producing a
/// SAN per ply. Stops on any ply that doesn't apply cleanly
/// (shouldn't happen with a real PV from the engine).
pub(crate) fn pv_to_san(root: &Position, pv: &[Move]) -> Vec<String> {
    let mut out = Vec::with_capacity(pv.len());
    let mut pos = root.clone();
    for mv in pv {
        out.push(san::format(&pos, *mv));
        pos.do_move(*mv);
    }
    out
}

impl Session {
    /// Walk every user move's cached retrospective analysis through
    /// the engine classifier and return the ranked list of moments
    /// worth reviewing. Returns `None` when the game has no user
    /// moves whose retrospective has arrived yet.
    ///
    /// Reuses [`crate::learning_mode::gating_config_for`] with the
    /// user's current `mistake_handling` preference so the same gate
    /// drives both the in-game prompt and the post-game review —
    /// switching to "AllMistakes" before opening review surfaces
    /// every non-best move, switching back tightens the list.
    pub fn build_game_review(&self) -> Option<crate::view::GameReviewView> {
        use crate::view::{GameReviewMoment, GameReviewView, ReviewMomentKind};

        let mut moments: Vec<GameReviewMoment> = Vec::new();
        let mut user_move_count: usize = 0;
        let config = gating_config_for(self.learning.mistake_handling);

        for (idx, entry) in self.history.iter().enumerate() {
            if !self.is_user_move(entry) {
                continue;
            }
            user_move_count += 1;
            let Some(retro) = entry.retrospective.as_ref() else {
                // Analysis hasn't arrived yet — skip silently. Most
                // common case is the very-latest move while the worker
                // is still computing.
                continue;
            };
            let pre = self.pre_move_position(idx);
            let prior_move = self.prior_move_for(idx);
            let assessment = chess_tutor_engine::analysis::classify_user_move(
                &pre,
                &retro.analyses,
                retro.user_move,
                &config,
                prior_move,
            );
            // ALLOWED collapses into the same review row as a teaching
            // moment (both are "your move had a teachable cost"); the
            // headline below distinguishes them.
            let teaching_like = assessment.teaching.is_some() || assessment.allowed.is_some();
            let kind = match (assessment.blunder.is_some(), teaching_like) {
                (true, true) => ReviewMomentKind::BlunderWithLesson,
                (true, false) => ReviewMomentKind::Blunder,
                (false, true) => ReviewMomentKind::TeachingMoment,
                (false, false) => continue,
            };
            let headline = review_headline_for(&assessment);
            let move_pair_number = idx / 2 + 1;
            let side_to_move_label = if entry.moved_by == Color::White {
                "White"
            } else {
                "Black"
            };
            moments.push(GameReviewMoment {
                history_index: idx,
                move_pair_number,
                side_to_move_label,
                san: entry.san.clone(),
                kind,
                headline,
            });
        }

        if user_move_count == 0 {
            return None;
        }
        Some(GameReviewView {
            game_outcome: self.game_outcome(),
            user_move_count,
            moments,
        })
    }

    /// Whether the game-review surface is currently being shown.
    pub fn is_game_review_open(&self) -> bool {
        self.game_review_open
    }

    // ---- Event dispatch ------------------------------------------------

    pub fn build_top_bar_view(&self) -> TopBarView {
        let review_button_enabled = self.history.iter().any(|e| {
            self.is_user_move(e) && e.retrospective.is_some()
        });
        TopBarView {
            viewing_live: self.is_viewing_live(),
            depth: self.depth,
            engine_thinking: self.engine_thinking,
            game_outcome: self.game_outcome(),
            review_open: self.game_review_open,
            review_button_enabled,
        }
    }

    pub fn build_action_bar_view(&self) -> ActionBarView {
        let hint_can_open = self.is_viewing_live()
            && !self.engine_thinking
            && self.is_users_turn()
            && self.game_outcome().is_none();
        ActionBarView {
            can_takeback: !self.history.is_empty(),
            hint_open: self.hint_open,
            hint_button_enabled: hint_can_open || self.hint_open,
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
            self.learning.reveal_best_moves,
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
        out
    }

    /// Snapshot of the currently-active overlay set. Renderers consume
    /// this to draw the overlay checkboxes with the right initial
    /// state.
    pub fn active_overlays(&self) -> &std::collections::HashSet<crate::view::OverlayKind> {
        &self.active_overlays
    }

    pub fn build_side_panel_view(&self) -> SidePanelView {
        // Body priority, top to bottom:
        //   Intervention > GameReview (when explicitly opened)
        //     > Hint (when explicitly opened)
        //     > Coaching (live, when assistance = Coached, user's turn,
        //                 viewing live, game in progress)
        //     > Retrospective (the default)
        let body = if let Some(pending) = self.pending_intervention.as_ref() {
            SidePanelBody::Intervention(build_intervention_panel(pending))
        } else if self.game_review_open {
            // build_game_review returns None only when there are no
            // user moves at all — in that case fall back to the
            // regular retrospective so the panel isn't blank.
            match self.build_game_review() {
                Some(review) => SidePanelBody::GameReview(review),
                None => SidePanelBody::Retrospective(self.build_retrospective_view()),
            }
        } else if self.hint_open {
            SidePanelBody::Hint(self.build_hint_panel_view())
        } else if self.coaching_should_show() {
            SidePanelBody::Coaching(self.build_coaching_panel_view())
        } else {
            SidePanelBody::Retrospective(self.build_retrospective_view())
        };
        SidePanelView {
            moves: self.build_move_list_view(),
            body,
            active_overlays: self.active_overlays.clone(),
            learning: self.learning,
            stick_to_bottom: self.is_viewing_live(),
        }
    }

    pub(crate) fn build_move_list_view(&self) -> MoveListView {
        let viewing = self.viewing_index;
        let history_len = self.history.len();
        let rows = (0..history_len.div_ceil(2))
            .map(|pair| {
                let i_white = pair * 2;
                let i_black = i_white + 1;
                let white = MoveListCell {
                    history_index: i_white,
                    san: self.history[i_white].san.clone(),
                    selected: viewing == Some(i_white),
                };
                let black = self.history.get(i_black).map(|e| MoveListCell {
                    history_index: i_black,
                    san: e.san.clone(),
                    selected: viewing == Some(i_black),
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
                    self.learning.reveal_best_moves,
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

    /// Conditions for the live coaching panel to appear:
    /// - User explicitly turned it on via `AssistanceLevel::Coached`.
    /// - It's the user's turn (coaching applies to the live position
    ///   the student is about to move from).
    /// - The user is viewing live, not browsing back.
    /// - The game isn't over.
    /// - There's no other higher-priority body active (the caller
    ///   already drained those — this function only sees the lower-
    ///   priority case).
    pub(crate) fn coaching_should_show(&self) -> bool {
        matches!(
            self.learning.assistance,
            crate::learning_mode::AssistanceLevel::Coached
        ) && self.is_viewing_live()
            && self.is_users_turn()
            && self.game_outcome().is_none()
    }

    pub(crate) fn build_coaching_panel_view(&self) -> CoachingPanelView {
        let tactic_hint = self.coaching_tactic_hint();
        let view_model = crate::coaching_view::build_coaching_view(
            &self.position,
            self.user_color(),
            tactic_hint.as_ref(),
            self.coaching_prior_move(),
        );
        CoachingPanelView { view_model }
    }

    pub(crate) fn build_hint_panel_view(&self) -> HintPanelView {
        if self.hint_thinking && self.hint_result.is_none() {
            return HintPanelView {
                state: HintPanelState::Loading,
            };
        }
        let Some(result) = &self.hint_result else {
            return HintPanelView {
                state: HintPanelState::NoResult,
            };
        };
        if result.analyses.is_empty() {
            return HintPanelView {
                state: HintPanelState::NoMoves,
            };
        }
        let root_stm = result.pos.side_to_move();
        let entries: Vec<HintEntryView> = result
            .analyses
            .iter()
            .map(|ma| {
                let san = san::format(&result.pos, ma.mv);
                let score_str = format_score_root_pov(ma.score, root_stm);
                let pv_san = pv_to_san(&result.pos, &ma.pv);
                let settle_marker = ma.settled_ply.filter(|&i| i < pv_san.len());
                HintEntryView {
                    san,
                    score_str,
                    depth: ma.depth,
                    pv_san,
                    settle_marker,
                }
            })
            .collect();
        HintPanelView {
            state: HintPanelState::Ready(entries),
        }
    }

    pub fn build_new_game_dialog_view(&mut self) -> Option<NewGameDialogView<'_>> {
        let first_launch = self.first_launch;
        let form = self.new_game_form.as_mut()?;
        Some(NewGameDialogView { form, first_launch })
    }
}

#[cfg(test)]
#[path = "view_builders_tests.rs"]
mod tests;
