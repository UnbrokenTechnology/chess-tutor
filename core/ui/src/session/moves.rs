//! User input and move application: click handling, square selection,
//! user / engine move application, takeback / undo, and the hint toggle.

use super::*;

use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::san;
use chess_tutor_engine::traps::{self, PendingTrap};
use chess_tutor_engine::types::{Move, MoveKind, Square};

use crate::learning_mode::MistakeHandling;
use crate::worker::WorkerJob;

impl Session {
    pub(crate) fn close_hint(&mut self) {
        self.hint_open = false;
        self.hint_thinking = false;
        self.hint_result = None;
    }

    pub(crate) fn toggle_hint(&mut self) {
        if self.hint_open {
            self.close_hint();
            return;
        }
        // Open and queue an Analyze job for the current live position.
        self.hint_open = true;
        self.hint_thinking = true;
        self.hint_result = None;
        let _ = self.worker_tx.send(WorkerJob::Analyze {
            pos: Box::new(self.position.clone()),
            // Analytical paths use ANALYTICAL_DEPTH, independent of
            // self.depth (the bot's play depth). See the constant
            // for rationale.
            depth: ANALYTICAL_DEPTH,
            multi_pv: HINT_MULTI_PV,
            game_history: game_history_for_search(&self.position_keys),
            for_key: self.position.key(),
        });
    }

    pub(crate) fn handle_click(&mut self, sq: Square) {
        // Don't let board clicks fall through when the New Game modal
        // is up — egui Windows don't block clicks below them by
        // default, so without this guard the user could move pieces
        // through the dialog (and at first launch `engine_plays` is
        // None, so `is_users_turn` would say yes).
        if self.new_game_form.is_some() {
            return;
        }
        // Clicks on the board while viewing back snap to live first;
        // the click itself doesn't otherwise act this frame.
        if self.viewing_index.is_some() {
            self.viewing_index = None;
            return;
        }
        if self.engine_thinking || !self.is_users_turn() {
            return;
        }
        if Some(sq) == self.selected {
            self.deselect();
            return;
        }
        if self.selected.is_some() && self.try_move_to(sq) {
            self.maybe_queue_engine_search();
            return;
        }
        self.select(sq);
    }

    pub(crate) fn is_users_turn(&self) -> bool {
        !self.engine_plays.is_engine_turn(self.position.side_to_move())
    }

    pub(crate) fn select(&mut self, sq: Square) {
        match self.position.piece_on(sq) {
            Some(piece) if piece.color() == self.position.side_to_move() => {
                self.selected = Some(sq);
                let mut scratch = self.position.clone();
                self.legal_from_selected = legal_moves_vec(&mut scratch)
                    .into_iter()
                    .filter(|m| m.from() == sq)
                    .collect();
            }
            _ => self.deselect(),
        }
    }

    pub(crate) fn try_move_to(&mut self, target: Square) -> bool {
        let candidates: Vec<Move> = self
            .legal_from_selected
            .iter()
            .copied()
            .filter(|m| m.to() == target)
            .collect();
        if candidates.is_empty() {
            return false;
        }

        // Promotion: legal-move generation produces one move per piece
        // type (Q / R / B / N). Open the picker instead of silently
        // queening — `apply_promotion_choice` will run once the user
        // clicks one of the four pieces.
        if candidates.iter().all(|m| m.kind() == MoveKind::Promotion) {
            if let Some(pending) = build_pending_promotion(&candidates) {
                self.pending_promotion = Some(PendingPromotion {
                    to: target,
                    candidates: pending,
                });
                return true;
            }
        }

        let mv = candidates[0];
        self.apply_user_move(mv);
        true
    }

    /// Apply a user move and queue the engine's reply if it's now the
    /// engine's turn. The convenience entry point for CLI / headless
    /// callers that parse a [`Move`] directly (SAN / UCI input). The
    /// desktop's click path goes through [`Self::apply_user_move`] +
    /// [`Self::maybe_queue_engine_search`] separately because it
    /// re-resolves through [`Event::SelectSquare`].
    pub fn play_user_move(&mut self, mv: Move) {
        self.apply_user_move(mv);
        self.maybe_queue_engine_search();
    }

    /// Finalise a move chosen via the regular click path *or* the
    /// promotion picker. Snapshots pre-move state for the retrospective
    /// job (when [`Self::auto_retrospective`] is set), applies the
    /// move, and clears the hint panel.
    ///
    /// Sets [`Self::awaiting_intervention_decision`] when the user's
    /// preferences want the classifier to run on this move — that
    /// flag causes `maybe_queue_engine_search` to hold the bot
    /// reply until the classifier returns (or the user resolves the
    /// resulting prompt). Without auto-retrospective there's no
    /// classifier to wait for, so the flag stays false.
    pub fn apply_user_move(&mut self, mv: Move) {
        if self.auto_retrospective {
            let pre_move_pos = self.position.clone();
            let pre_move_history = game_history_for_search(&self.position_keys);
            self.apply_move(mv);
            let target_index = self.history.len() - 1;
            if self.intervention_mode_active() {
                self.awaiting_intervention_decision = true;
            }
            let _ = self.worker_tx.send(WorkerJob::Retrospective {
                pre_move_pos: Box::new(pre_move_pos),
                user_move: mv,
                // Independent of self.depth (the bot's play depth) so a
                // weakened bot still gives strong teaching feedback.
                depth: self.retrospective_depth,
                game_history: pre_move_history,
                gen: self.gen,
                target_index,
            });
        } else {
            self.apply_move(mv);
        }
        self.close_hint();
    }

    /// `true` when the user's learning preferences want the engine
    /// classifier to inspect each user move (and pause the game if it
    /// flags one). Both gates — blunder safety and mistake handling —
    /// route through the classifier; we only skip when *neither* is
    /// active.
    pub(crate) fn intervention_mode_active(&self) -> bool {
        !matches!(
            self.learning.mistake_handling,
            MistakeHandling::SilentRetrospective
        ) || matches!(
            self.learning.blunder_safety,
            crate::learning_mode::BlunderSafety::OfferTakeback
        )
    }

    pub(crate) fn apply_move(&mut self, mv: Move) {
        let san_str = san::format(&self.position, mv);
        let moved_by = self.position.side_to_move();

        // ---- Trap bookkeeping, pre-move pass ----
        // Snapshot for undo restore.
        let pending_trap_before = self.pending_trap.clone();
        // Advance the cursor (if any). The pre-move position is what
        // `advance_pending` wants — the cursor was scripted against
        // moves played FROM the position before each ply.
        let mut trap_events = Vec::new();
        if let Some(pending) = self.pending_trap.as_mut() {
            let event = traps::advance_pending(pending, &self.position, mv);
            let terminal = event.is_terminal();
            trap_events.push(event);
            if terminal {
                self.pending_trap = None;
            }
        }
        // Capture pre-move data the post-move scan needs (piece kind
        // can only be read while the source square still has the
        // piece).
        let scan_inputs = self
            .position
            .piece_on(mv.from())
            .map(|piece| (moved_by, piece.kind(), mv.from(), mv.to()));

        let state = self.position.do_move(mv);
        self.position_keys.push(self.position.key());

        // ---- Trap bookkeeping, post-move pass ----
        let mut trap_hit = None;
        if self.pending_trap.is_none() {
            if let Some((mover, piece_kind, from, to)) = scan_inputs {
                if let Some((entry, hit)) =
                    traps::scan_after_move(&self.position, mover, piece_kind, from, to)
                        .into_iter()
                        .next()
                {
                    trap_hit = Some(hit.clone());
                    self.pending_trap = Some(PendingTrap::new(entry, hit));
                }
            }
        }

        self.history.push(HistoryEntry {
            mv,
            state,
            san: san_str,
            moved_by,
            position_after: self.position.clone(),
            retrospective: None,
            engine_info: None,
            noise_pick: None,
            pending_trap_before,
            trap_events,
            trap_hit,
        });
        // No book-cursor advance: BookCursor is stateless and
        // re-derives from history at each peek. Takeback is similarly
        // free of book bookkeeping.
        self.deselect();
    }

    pub(crate) fn takeback(&mut self) {
        if self.engine_thinking {
            self.gen = self.gen.wrapping_add(1);
            self.engine_thinking = false;
        } else {
            // Bump anyway: pending retrospective jobs (which don't
            // toggle engine_thinking) need to be invalidated.
            self.gen = self.gen.wrapping_add(1);
        }
        // Any active or pending intervention referred to the move
        // we're about to undo — drop it so the panel snaps back to
        // the normal retrospective surface.
        self.pending_intervention = None;
        self.awaiting_intervention_decision = false;
        self.game_review_open = false;
        self.viewing_index = None;
        self.close_hint();
        self.undo_one();
        // In user-vs-engine mode, takeback returns to the user's
        // prior turn — undo a second ply if we just landed on the
        // engine's turn. Self-play (Both) and user-plays-both (None)
        // are both happy with a single ply rewind.
        if let EngineMode::Side(eng_color) = self.engine_plays {
            if self.position.side_to_move() == eng_color && !self.history.is_empty() {
                self.undo_one();
            }
        }
        // Re-arm the "out of book" announcement: the user may now
        // be back in book territory (the cursor will re-derive that
        // on its next peek), and either way, if they deviate again
        // they should see the line print again.
        self.book_out_announced = false;
        self.maybe_queue_engine_search();
    }

    pub(crate) fn undo_one(&mut self) {
        if let Some(entry) = self.history.pop() {
            self.position.undo_move(entry.mv, entry.state);
            self.position_keys.pop();
            // Roll the trap cursor back to its pre-move snapshot so
            // the refutation tree is walked in lockstep with the
            // position.
            self.pending_trap = entry.pending_trap_before;
            // No book-cursor restore needed — the stateless cursor
            // re-derives from history on the next peek.
            self.deselect();
        }
    }

    pub(crate) fn deselect(&mut self) {
        self.selected = None;
        self.legal_from_selected.clear();
        self.pending_promotion = None;
    }
}
