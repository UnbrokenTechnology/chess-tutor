//! Read-only accessors over session state: live / viewed position,
//! history, opponent, depth, learning preferences, and pending
//! trap / intervention.

use super::*;

use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::opponent::OpponentProfile;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::traps::{self, PendingTrap, TrapThreatened};
use chess_tutor_engine::types::{Color, Value};

use crate::learning_mode::{
    LearningPreferences, PendingIntervention,
};

/// Pick a post-move evaluation (white POV) off a single
/// [`HistoryEntry`]. Engine moves carry the score directly on
/// `engine_info`; user moves carry it on the retrospective's analysis
/// of the move they actually played. The eval bar walks history
/// backward through this so it updates on every move, not only engine
/// replies.
pub(crate) fn entry_eval_white_pov(e: &HistoryEntry) -> Option<Value> {
    if let Some(info) = &e.engine_info {
        return Some(info.score_white_pov);
    }
    let retro = e.retrospective.as_ref()?;
    let analysis = retro.analyses.iter().find(|a| a.mv == retro.user_move)?;
    Some(if e.moved_by == Color::White {
        analysis.score
    } else {
        -analysis.score
    })
}

impl Session {
    pub fn game_outcome(&self) -> Option<&'static str> {
        let mut scratch = self.position.clone();
        if legal_moves_vec(&mut scratch).is_empty() {
            return Some(if self.position.in_check() {
                match self.position.side_to_move() {
                    Color::White => "Checkmate — Black wins.",
                    Color::Black => "Checkmate — White wins.",
                }
            } else {
                "Stalemate — draw."
            });
        }
        if self.position.halfmove_clock() >= 100 {
            return Some("Draw — 50-move rule.");
        }
        if threefold_reached(&self.position_keys) {
            return Some("Draw — threefold repetition.");
        }
        if self.position.has_insufficient_material() {
            return Some("Draw — insufficient material.");
        }
        None
    }

    // ---- View helpers ---------------------------------------------------

    pub(crate) fn viewed_entry(&self) -> Option<&HistoryEntry> {
        match self.viewing_index {
            Some(i) => self.history.get(i),
            None => self.history.last(),
        }
    }

    pub(crate) fn viewed_position(&self) -> &Position {
        match self.viewing_index {
            Some(i) => self
                .history
                .get(i)
                .map(|e| &e.position_after)
                .unwrap_or(&self.position),
            None => &self.position,
        }
    }

    pub(crate) fn is_viewing_live(&self) -> bool {
        self.viewing_index.is_none()
    }

    /// The most-recent post-move evaluation (white POV) at or before
    /// the currently viewed history index — used by the eval bar.
    ///
    /// Both engine moves (`engine_info`) and user moves (the
    /// retrospective worker's analysis of the user's chosen move) are
    /// valid sources. Scanning backward picks up the first either-or
    /// hit, so the bar updates on every move that has reached the
    /// analysis stage — not only engine moves.
    ///
    /// When the user is browsing back to a position whose retrospective
    /// hasn't arrived yet, we fall further back to the most recent
    /// pre-existing evaluation. That's an approximation, but it gives
    /// a sensible "trend" view while the worker catches up.
    pub(crate) fn viewed_eval_white_pov(&self) -> Option<Value> {
        let upper = self.viewing_index.map_or(self.history.len(), |i| i + 1);
        self.history[..upper].iter().rev().find_map(entry_eval_white_pov)
    }

    /// Picks the (index, entry) to show in the retrospective panel:
    ///   - Viewing back: the viewed entry.
    ///   - Live: the most recent user-move entry (so the engine's
    ///     reply doesn't bury the analysis of the user's own move).
    pub(crate) fn panel_entry_with_index(&self) -> Option<(usize, &HistoryEntry)> {
        if let Some(i) = self.viewing_index {
            return self.history.get(i).map(|e| (i, e));
        }
        if let Some(found) = self
            .history
            .iter()
            .enumerate()
            .rev()
            .find(|(_, e)| self.is_user_move(e))
        {
            return Some(found);
        }
        self.history
            .last()
            .map(|e| (self.history.len() - 1, e))
    }

    /// Pre-move position for history entry `i` — needed by anything
    /// that wants to format an analysis whose root was the position
    /// the user faced before their move.
    pub(crate) fn pre_move_position(&self, i: usize) -> Position {
        if i == 0 {
            self.start_position.clone()
        } else {
            self.history[i - 1].position_after.clone()
        }
    }

    /// The opponent's move that produced the position for history
    /// entry `i` (i.e. `history[i-1]`), paired with the piece it
    /// captured. Feeds the tactic detector's hanging-capture recapture
    /// guard so a trade isn't mis-labelled as a free piece. `None` at
    /// the start of the game (i = 0) where no prior move exists.
    pub(crate) fn prior_move_for(
        &self,
        i: usize,
    ) -> Option<chess_tutor_engine::analysis::PriorMove> {
        if i == 0 {
            return None;
        }
        let prior_entry = &self.history[i - 1];
        let pos_before_prior = if i == 1 {
            self.start_position.clone()
        } else {
            self.history[i - 2].position_after.clone()
        };
        Some(chess_tutor_engine::analysis::PriorMove::new(
            &pos_before_prior,
            prior_entry.mv,
        ))
    }

    /// Pre-move tactic hint for the live coaching panel.
    ///
    /// Two paths, tried in order:
    ///
    /// 1. **PV-reuse** — when a previous user-move retrospective exists
    ///    and the bot followed the analytical engine's predicted reply
    ///    (`history[u+1].mv == pv[1]`), `pv[2..]` is the engine's best
    ///    continuation from the live position. No new search.
    /// 2. **Static fork-shape scan** — when PV-reuse can't fire (move
    ///    1 of a game, bot deviated, retrospective still arriving),
    ///    enumerate the user's legal moves and run the same static
    ///    detectors on each one. No search either — the detectors look
    ///    at the post-move position's attacker bitboards. Catches
    ///    1-ply tactics (forks, hanging captures, pins/skewers,
    ///    discovered checks, mate-in-1). Multi-ply combinations stay
    ///    missed; a worker-based fallback search would be the next step
    ///    if real play surfaces them.
    pub(crate) fn coaching_tactic_hint(
        &self,
    ) -> Option<chess_tutor_engine::analysis::TacticHit> {
        self.coaching_tactic_hint_pv_reuse()
            .or_else(|| self.coaching_tactic_hint_static_scan())
    }

    /// PV-reuse path: see [`Self::coaching_tactic_hint`] for the
    /// design. Returns `None` on any freshness-gate failure, falling
    /// through to the static-scan fallback.
    fn coaching_tactic_hint_pv_reuse(
        &self,
    ) -> Option<chess_tutor_engine::analysis::TacticHit> {
        use chess_tutor_engine::analysis::{find_tactic_in_line, PriorMove};
        let (u, user_entry) = self
            .history
            .iter()
            .enumerate()
            .rev()
            .find(|(_, e)| self.is_user_move(e))?;
        let bot_reply = self.history.get(u + 1)?;
        let retro = user_entry.retrospective.as_ref()?;
        let user_analysis = retro
            .analyses
            .iter()
            .find(|a| a.mv == retro.user_move)?;
        if user_analysis.pv.len() < 3 {
            return None;
        }
        if user_analysis.pv[1] != bot_reply.mv {
            return None;
        }
        let live_pos = &bot_reply.position_after;
        let prior = PriorMove::new(&user_entry.position_after, bot_reply.mv);
        find_tactic_in_line(
            live_pos,
            &user_analysis.pv[2..],
            self.user_color(),
            Some(prior),
        )
    }

    /// Static scan path: enumerate the user's legal moves and pick the
    /// best 1-ply tactic via [`find_best_tactic_in_position`]. No PV
    /// needed; the detectors are static predicates over `(pos, move)`.
    /// Prior move (for the hanging-capture recapture guard) is the
    /// opponent's last move into the live position, or `None` at game
    /// start.
    fn coaching_tactic_hint_static_scan(
        &self,
    ) -> Option<chess_tutor_engine::analysis::TacticHit> {
        use chess_tutor_engine::analysis::{find_best_tactic_in_position, PriorMove};
        let prior = if let Some(last_entry) = self.history.last() {
            let last_idx = self.history.len() - 1;
            let pre = self.pre_move_position(last_idx);
            Some(PriorMove::new(&pre, last_entry.mv))
        } else {
            None
        };
        find_best_tactic_in_position(&self.position, self.user_color(), prior)
    }

    pub(crate) fn is_user_move(&self, entry: &HistoryEntry) -> bool {
        match self.engine_plays {
            EngineMode::None => true,
            EngineMode::Side(c) => entry.moved_by != c,
            EngineMode::Both => false,
        }
    }

    /// "User's" colour for POV-flipped overlays. When the engine plays
    /// one side, the user is the other; otherwise we fall back to the
    /// side-to-move at the currently-viewed position (the natural POV
    /// for two-human / self-play modes).
    pub(crate) fn user_color(&self) -> Color {
        match self.engine_plays {
            EngineMode::Side(eng) => !eng,
            EngineMode::None | EngineMode::Both => self.viewed_position().side_to_move(),
        }
    }

    // ---- Public accessors (CLI / headless callers) ---------------------

    /// Current live position.
    pub fn position(&self) -> &Position {
        &self.position
    }

    /// Move history, in play order. Engine moves have
    /// [`HistoryEntry::engine_info`] populated; user moves have
    /// [`HistoryEntry::retrospective_text`] (when auto-retrospective
    /// is on).
    pub fn history(&self) -> &[HistoryEntry] {
        &self.history
    }

    /// Opponent profile (book selection, noise, eval mask, seed) for
    /// the current game.
    pub fn opponent(&self) -> &OpponentProfile {
        &self.opponent
    }

    /// Mutable opponent access. Most fields take effect on the next
    /// engine move (noise / eval-mask are read per search). Mutating
    /// `book` mid-game does *not* affect the active book cursor —
    /// that's frozen at game start; the field change applies to
    /// the next game.
    pub fn opponent_mut(&mut self) -> &mut OpponentProfile {
        &mut self.opponent
    }

    /// Engine mode for the current game.
    pub fn engine_plays(&self) -> EngineMode {
        self.engine_plays
    }

    /// Live trap cursor. `Some` between a trap firing and its
    /// refutation tree reaching a terminal node. Renderers use this
    /// to decide whether to suppress pre-move threat warnings (CLI)
    /// or to surface a "trap active" badge (future GUI).
    pub fn pending_trap(&self) -> Option<&PendingTrap> {
        self.pending_trap.as_ref()
    }

    /// Pre-move trap threats for the side currently to move: legal
    /// moves that would walk into a known refutation. Computed fresh
    /// each call (the underlying library scan is cheap). Renderers
    /// typically suppress this when [`Self::pending_trap`] is already
    /// `Some` — a trap mid-refutation is doing its own narration.
    pub fn trap_threats(&self) -> Vec<TrapThreatened> {
        traps::scan_threats(&self.position)
    }

    /// Bot-play depth.
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// Current learning preferences (assistance level, mistake
    /// handling, blunder safety, reveal-best-moves).
    pub fn learning_preferences(&self) -> &LearningPreferences {
        &self.learning
    }

    /// Replace the full learning preferences bundle. Renderers can
    /// either dispatch [`Event::ApplyLearningPreset`] for the named
    /// presets or call this for per-axis edits.
    pub fn set_learning_preferences(&mut self, prefs: LearningPreferences) {
        self.learning = prefs;
    }

    /// `Some` while an intervention prompt is showing. CLI callers
    /// can inspect this to know the bot reply is being held.
    pub fn pending_intervention(&self) -> Option<&PendingIntervention> {
        self.pending_intervention.as_ref()
    }
}
