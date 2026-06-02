//! Side-panel, hint pop-over, review, intervention, move-list, and
//! new-game-dialog view descriptors. Split out of `view.rs`; the
//! board/annotation/retrospective-card types live in the parent module.

use super::{BoardAnnotation, RetrospectiveCategory, RetrospectiveViewModel, Sentiment};
use crate::session::NewGameForm;
use chess_tutor_teaching::phrasing::Perspective;

/// Right side panel: the backward-looking feedback zone (retrospective /
/// review / intervention) plus the compact move list. Coaching is *not*
/// here any more — it pops over via [`HintPopoverView`] so the
/// backward-looking feedback and the forward-looking "what to notice"
/// can coexist instead of fighting over one slot (PLAN §"coaching/hint
/// model").
pub struct SidePanelView {
    pub moves: MoveListView,
    pub body: SidePanelBody,
    /// Currently-active board overlays. Renderers iterate
    /// [`OverlayKind::ALL`] and check membership to draw each
    /// checkbox with the right initial state.
    pub active_overlays: std::collections::HashSet<OverlayKind>,
    /// Snapshot of the user's learning preferences. Renderers paint
    /// a small picker (preset + the reveal-best-moves toggle) so
    /// users can change modes mid-game without going through the
    /// New Game dialog.
    pub learning: crate::learning_mode::LearningPreferences,
    /// When the user is following live play, the move-list scroll
    /// should auto-stick to the bottom. When browsing back, freeze
    /// at wherever they scrolled.
    pub stick_to_bottom: bool,
    /// `Some` while the step-through **review mode** is active (PLAN
    /// step 6). The body is the selected move's retrospective; this
    /// carries the nav-bar chrome the renderer paints alongside it. The
    /// move list rows also gain inline rating icons in this mode.
    pub review_mode: Option<ReviewModeView>,
}

pub enum SidePanelBody {
    /// An in-game intervention is pending — the engine reply is held
    /// until the user dismisses, takes back, or reveals the concept.
    /// Takes priority over the retrospective panel so the prompt is
    /// the first thing the user sees.
    Intervention(InterventionPanelView),
    Retrospective(RetrospectivePanelView),
    /// Post-game (or on-demand) review surface — a ranked list of
    /// significant moments the user should study. Click any moment
    /// to jump the rest of the UI to that move.
    GameReview(GameReviewView),
}

/// The on-demand **Hint pop-over** — a dismissible "what to notice"
/// panel opened by the Hint button (PLAN §"coaching/hint model").
/// Lists features-to-notice in the current position, naming patterns
/// and squares but **never the move** (the opposite of chess.com's
/// answer-flashing Hint). Built from
/// [`crate::coaching_view::build_coaching_view`]; rendered as a
/// floating pop-over so the side panel's backward-looking feedback
/// zone stays visible underneath. `None` when the pop-over is closed.
///
/// A renderer-neutral descriptor: the renderer chooses the floating-
/// panel chrome and the dismiss affordance, and emits
/// [`crate::event::Event::ToggleHint`] to close. When `view_model.items`
/// is empty, the renderer shows an encouraging neutral message rather
/// than a blank pop-over.
pub struct HintPopoverView {
    pub view_model: CoachingViewModel,
}

/// Structured features-to-notice for the current position. Same
/// shape as a retrospective view model but without a headline and
/// without score deltas — coaching describes the current snapshot,
/// not a change.
#[derive(Clone, Debug, Default)]
pub struct CoachingViewModel {
    pub items: Vec<CoachingItem>,
}

/// One card in the coaching panel. Mirrors the retrospective card
/// shape (heading + summary + detail + annotations) but explicitly
/// has no `score_delta_pawns` — coaching cards describe state, not
/// change.
#[derive(Clone, Debug)]
pub struct CoachingItem {
    pub category: RetrospectiveCategory,
    pub heading: String,
    pub summary: String,
    pub detail: String,
    pub sentiment: Sentiment,
    pub annotations: Vec<BoardAnnotation>,
    /// `true` when this is a positional / quiet-position note that has
    /// been demoted because the position is tactically live (the
    /// tactical-mode gate fired). The renderer collapses demoted items
    /// under a muted "Quiet-position notes — not the priority right
    /// now" section, rendered *after* the tactical cards. Always
    /// `false` when the gate is not live (positional cards lead, as
    /// before). See `PLAN-teaching-gui.md` §2.
    pub demoted: bool,
}

/// Post-game review **summary** surface (build-order step 6): the
/// verdict tallies, an eval-over-time graph, the ranked significant
/// moments, and the big **Start Review** call-to-action. Renderers
/// paint the tallies + graph, then a Start-Review button emitting
/// [`crate::event::Event::StartReview`]; clicking a moment row emits
/// [`crate::event::Event::JumpToReviewMoment`] (which enters review
/// mode at that history index).
///
/// No estimated-ELO field — deliberately skipped (PLAN step 6).
pub struct GameReviewView {
    /// Optional one-line outcome label (e.g. "Checkmate — White wins.")
    /// when the game is over.
    pub game_outcome: Option<&'static str>,
    /// Total user moves in the game (so the renderer can show "3 of
    /// 28 moves flagged" context).
    pub user_move_count: usize,
    /// Per-verdict tally over the user's moves, in chess.com display
    /// order (Best → Blunder). Built from the same material-aware
    /// classifier that drives the in-game verdict.
    pub tallies: Vec<ReviewTally>,
    /// Per-ply white-POV eval samples for the eval-over-time graph, in
    /// play order. One entry per analysed history ply; plies whose
    /// analysis hasn't arrived are omitted (the renderer draws a
    /// best-effort line through what's present).
    pub eval_series: Vec<EvalSample>,
    /// Ranked list of significant moments. Empty when no user moves
    /// crossed an intervention gate.
    pub moments: Vec<GameReviewMoment>,
}

/// One verdict bucket in the game-review summary (e.g. "Best ×12").
pub struct ReviewTally {
    /// Display tier — drives icon + colour.
    pub tier: ReviewVerdictTier,
    /// Human-readable label ("Best", "Good", "Inaccuracy", …).
    pub label: &'static str,
    /// How many of the user's moves landed in this tier.
    pub count: usize,
}

/// Verdict tiers surfaced in the summary tally, mapped from the
/// engine's [`chess_tutor_engine::analysis::MoveVerdict`]. `BestAvailable`
/// folds into `Best` for the tally (both are "as good as it gets").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewVerdictTier {
    Best,
    Good,
    Inaccuracy,
    Mistake,
    Miss,
    Blunder,
}

impl ReviewVerdictTier {
    /// Display order + the set the renderer iterates to lay out the
    /// tally rows (so a zero-count tier still gets a consistent slot if
    /// the renderer chooses to show it).
    pub const ALL: [ReviewVerdictTier; 6] = [
        ReviewVerdictTier::Best,
        ReviewVerdictTier::Good,
        ReviewVerdictTier::Inaccuracy,
        ReviewVerdictTier::Mistake,
        ReviewVerdictTier::Miss,
        ReviewVerdictTier::Blunder,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ReviewVerdictTier::Best => "Best",
            ReviewVerdictTier::Good => "Good",
            ReviewVerdictTier::Inaccuracy => "Inaccuracy",
            ReviewVerdictTier::Mistake => "Mistake",
            ReviewVerdictTier::Miss => "Miss",
            ReviewVerdictTier::Blunder => "Blunder",
        }
    }
}

/// One sample on the eval-over-time graph: a play-order ply index and
/// its post-move evaluation in white-POV pawns (chess.com convention —
/// up = white better). Mate scores are clamped to the saturation
/// extent so the line stays on-graph.
pub struct EvalSample {
    /// History index of the move that produced this evaluation.
    pub history_index: usize,
    /// White-POV evaluation in pawns, clamped to roughly ±10 so a
    /// single mate/blunder doesn't flatten the rest of the curve.
    pub pawns: f32,
}

/// One significant moment in the game review. Renderers paint these
/// in order and emit [`crate::event::Event::JumpToReviewMoment`] when
/// the user clicks one.
pub struct GameReviewMoment {
    /// Index into `Session::history()`.
    pub history_index: usize,
    /// Move pair number (1-indexed, like the move list).
    pub move_pair_number: usize,
    /// Which side made the move.
    pub side_to_move_label: &'static str,
    /// SAN of the user's move.
    pub san: String,
    /// What kind of moment this is — drives icon + colour theming.
    pub kind: ReviewMomentKind,
    /// Short label describing the moment ("Blunder — lost knight",
    /// "Missed positional point: king safety").
    pub headline: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewMomentKind {
    /// Realized material loss — `BlunderInfo` fired.
    Blunder,
    /// Teaching moment — `TeachingInfo` fired. Drives a less alarming
    /// colour than [`Self::Blunder`].
    TeachingMoment,
    /// Both gates fired on the same move.
    BlunderWithLesson,
}

/// Mid-game prompt rendered while a [`crate::learning_mode::PendingIntervention`]
/// is active. Renderers paint the headline + summary; expand the
/// concept reveal when `concept` is `Some`; and emit one of the
/// intervention events on the user's response.
pub struct InterventionPanelView {
    pub kind: InterventionPanelKind,
    /// Short one-line prompt to show prominently. Never names the
    /// engine's preferred move.
    pub headline: String,
    /// Secondary descriptive line below the headline. Empty when the
    /// headline carries all the needed context.
    pub summary: String,
    /// The "what you missed" prose, populated after the user clicks
    /// the reveal button. Renderers render only when `Some`.
    pub concept: Option<String>,
    /// Buttons the renderer should offer. Order suggests display
    /// order; emit semantics are in [`crate::event::Event`].
    pub actions: Vec<InterventionAction>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterventionPanelKind {
    /// Material loss imminent — the user's piece is at risk.
    BlunderSafety,
    /// A teachable concept the user's move worsened. The dominant
    /// family drives the headline phrasing.
    TeachingMoment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterventionAction {
    /// "Take it back" / "Try a different move" — undoes the move and
    /// returns to the pre-move state. Emits `Event::TakeBackDuringIntervention`.
    TakeBack,
    /// "Show me what I missed" — reveals the concept reveal text in
    /// place. Emits `Event::RevealMissedConcept`.
    RevealConcept,
    /// "Continue" — dismisses the prompt; the original move stands
    /// and the bot's reply is queued. Emits `Event::ContinueDespitePrompt`.
    Continue,
}

pub struct MoveListView {
    pub rows: Vec<MoveListRow>,
}

pub struct MoveListRow {
    /// 1-based pair index for the leading "N." label.
    pub move_pair_idx: usize,
    pub white: MoveListCell,
    pub black: Option<MoveListCell>,
}

pub struct MoveListCell {
    pub history_index: usize,
    pub san: String,
    pub selected: bool,
    /// Inline move-rating icon (review mode only). `None` during live
    /// play and for moves that weren't the user's / weren't analysed;
    /// `Some(tier)` paints a small chess.com-style rating glyph next to
    /// the SAN in the move list. Only populated while the review-mode
    /// surface is active (PLAN step 6 — ratings are a post-game answer
    /// key, not a live spoiler).
    pub rating: Option<ReviewVerdictTier>,
}

/// Step-through review navigation targets (review-mode only). Emitted
/// inside [`crate::event::Event::ReviewNav`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewNav {
    /// One ply earlier (clamped at the first move).
    Back,
    /// One ply later (clamped at the last move).
    Forward,
    /// Jump to the first move.
    Restart,
    /// Jump to the last move.
    End,
}

/// Review-mode chrome (build-order step 6): the big step-through nav
/// bar shown while the user is walking the game move-by-move. The
/// feedback zone itself reuses the retrospective surface (the selected
/// move's deep breakdown); this view model only carries the nav state.
///
/// Renderers paint four big nav buttons (◀ ▶ ⏮ ⏭ — or restart/end +
/// back/forward) plus an autoplay toggle, emitting
/// [`crate::event::Event::ReviewNav`] / [`crate::event::Event::ToggleReviewAutoplay`].
/// A "Back to summary" affordance emits [`crate::event::Event::OpenGameReview`].
pub struct ReviewModeView {
    /// 1-based position in the game ("move 7 of 34") for a progress
    /// readout. `0` when there are no moves (shouldn't happen in review).
    pub current_ply: usize,
    /// Total plies in the game.
    pub total_plies: usize,
    /// Whether a step-back is possible (not already at the first move).
    pub can_step_back: bool,
    /// Whether a step-forward is possible (not already at the last move).
    pub can_step_forward: bool,
    /// Whether autoplay is currently running. The renderer ticks
    /// `ReviewNav(Forward)` on a timer while this is `true`.
    pub autoplay: bool,
}

pub struct RetrospectivePanelView {
    pub game_outcome: Option<&'static str>,
    pub body: RetrospectiveBody,
    /// Current state of the "show all signals" preference. Renderers
    /// surface this as a checkbox on the panel; toggling emits
    /// [`crate::event::Event::ToggleShowAllSignals`]. When `true`,
    /// retrospectives include every per-piece-type mobility shift and
    /// every residual term in "Other shifts".
    pub show_all_signals: bool,
    /// Feedback-zone expansion state (decision #1). When `false`, the
    /// renderer paints only the one-line verdict headline plus a "why
    /// this move?" affordance; when `true`, the full per-term eval
    /// breakdown with deltas expands in place below the headline.
    /// Toggling the affordance emits
    /// [`crate::event::Event::ToggleRetrospectiveDetail`]. The
    /// "show all signals" checkbox only makes sense while expanded.
    pub expanded: bool,
    /// Engine best-line PV in SAN — **review-mode only** (decision #9:
    /// the engine's preferred continuation is an answer key, forbidden
    /// during live play). `None` during live play and for the analysing
    /// state; `Some` while stepping through review and the selected
    /// move's analysis has arrived. The first element is the engine's
    /// best move from the pre-move position; the renderer shows it as
    /// the move-vs-move comparison line ("you played X; the engine's
    /// line was …").
    pub review_pv: Option<ReviewPvLine>,
}

/// The engine's best continuation for a reviewed move, in SAN, plus
/// the user's move for the move-vs-move comparison header. Review-mode
/// only (decision #9).
#[derive(Clone, Debug)]
pub struct ReviewPvLine {
    /// SAN of the move the user actually played.
    pub user_san: String,
    /// SAN of the engine's preferred move (`pv[0]`). Equal to `user_san`
    /// when the user found the best move (the renderer can say "you
    /// found the best move" instead of comparing).
    pub best_san: String,
    /// The engine's full best line in SAN, starting from the pre-move
    /// position (so `best_line[0] == best_san`). Capped to a handful of
    /// plies by the builder so the line stays readable.
    pub best_line: Vec<String>,
}

/// Persistent board overlays the user can toggle from the side panel.
/// Each overlay paints a set of [`BoardAnnotation`]s on the live
/// (or historically-viewed) position so the student can see what
/// the engine considers, independently of any retrospective card.
///
/// Renderers iterate [`OverlayKind::ALL`] to draw their checkboxes;
/// toggling emits [`crate::event::Event::ToggleOverlay`]. The
/// currently active set lives on [`crate::session::Session`] and
/// flows into the next [`BoardView`] via
/// [`crate::session::Session::build_board_view`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OverlayKind {
    /// Your space — the safe + reinforced squares in your central
    /// camp. Painted teal / blue.
    MySpace,
    /// The opponent's space — same definition flipped. Painted in a
    /// distinct hue so both overlays can be on at once.
    OpponentSpace,
    /// Squares excluded from the mobility area for your side —
    /// engine-relevant "dead" squares (own king/queen, pinned, low-
    /// rank pawns, enemy-pawn-attacked).
    MyMobilityArea,
    /// Both kings' ring squares (a 3×3 box around each king, clamped
    /// to the b2..g7 interior so corner kings still get 8 neighbours).
    KingRings,
    /// Each pinned piece's square — `Position::blockers_for_king(us)`
    /// for both sides.
    Pins,
    /// Every trapped piece on the board (either side) plus the "cage"
    /// of dead escape squares closing in on each one. A trapped piece
    /// is attacked, has no safe square, and no favourable trade out;
    /// the engine null-flips the turn so an enemy piece you're about
    /// to win shows up on your own move (the flagship case).
    TrappedPieces,
    /// Per-square attacker imbalance. Squares with a net advantage
    /// for you tint green; squares with a net advantage for the
    /// opponent tint red; even-but-contested squares stay clear.
    /// Intensity steps with magnitude (one tier for |net| = 1, a
    /// stronger tier for |net| ≥ 2).
    AttackHeatmap,
}

impl OverlayKind {
    pub const ALL: [OverlayKind; 7] = [
        OverlayKind::MySpace,
        OverlayKind::OpponentSpace,
        OverlayKind::MyMobilityArea,
        OverlayKind::KingRings,
        OverlayKind::Pins,
        OverlayKind::TrappedPieces,
        OverlayKind::AttackHeatmap,
    ];

    pub fn label(self) -> &'static str {
        match self {
            OverlayKind::MySpace => "My space",
            OverlayKind::OpponentSpace => "Opponent's space",
            OverlayKind::MyMobilityArea => "Mobility area (excluded)",
            OverlayKind::KingRings => "King rings",
            OverlayKind::Pins => "Pins",
            OverlayKind::TrappedPieces => "Trapped pieces",
            OverlayKind::AttackHeatmap => "Attack heatmap",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            OverlayKind::MySpace => {
                "Safe central squares (c–f × ranks 2–4 from your POV) you control. \
                 Darker = on/behind a friendly pawn and unattacked (doubly rewarded)."
            }
            OverlayKind::OpponentSpace => {
                "Opponent's safe central squares (c–f × ranks 5–7 from their POV). \
                 Darker = reinforced subset."
            }
            OverlayKind::MyMobilityArea => {
                "Squares excluded from your mobility area — own king/queen square, \
                 pinned-piece squares, blocked or rank-2/3 own pawns, and squares \
                 attacked by enemy pawns. Pieces don't get mobility credit for \
                 attacking these."
            }
            OverlayKind::KingRings => {
                "The 3×3 box around each king (clamped to the board interior). The \
                 king-safety term tallies enemy pieces attacking this ring."
            }
            OverlayKind::Pins => {
                "Pieces pinned to their own king — pieces whose movement would \
                 expose the king to a slider's attack."
            }
            OverlayKind::TrappedPieces => {
                "Pieces with no safe square — attacked, every legal move loses \
                 material. The piece itself is tinted; the surrounding \"cage\" \
                 paints the dead escape squares it can't run to. Both sides shown."
            }
            OverlayKind::AttackHeatmap => {
                "Per-square attacker imbalance. Green = you have more attackers; \
                 red = the opponent does. Stronger intensity = bigger imbalance."
            }
        }
    }
}

pub enum RetrospectiveBody {
    NoMoves,
    Entry {
        /// `Some(san)` when browsing back from live — renderer shows
        /// a "viewing move: {san}" header.
        viewing_back_san: Option<String>,
        kind: RetrospectiveKind,
    },
}

pub enum RetrospectiveKind {
    /// A move whose retrospective worker job is still in flight —
    /// regardless of who made it. The renderer shows a spinner. (The
    /// engine's reply and the user's own move both land here while their
    /// analysis is computing; the `perspective`-correct cards arrive once
    /// the worker returns and the kind becomes [`Self::MoveReady`].)
    Analyzing,
    /// A move whose retrospective is ready — the user's *or* the engine's.
    /// The session builds a structured [`RetrospectiveViewModel`] per
    /// frame from the raw analyses; renderers paint cards from it and emit
    /// [`crate::event::Event::SelectRetrospectiveItem`] on click.
    ///
    /// `perspective` is `Player` when the user made the move
    /// (`moved_by == user_color`) and `Opponent` when the engine did; it
    /// is baked into the view model's prose already, but is also surfaced
    /// here so renderers can theme the "you / they" framing if desired.
    /// The cards render identically regardless of mover (decision: one
    /// translation layer, one renderer).
    ///
    /// `selected_item` is the index into `view_model.items` of the
    /// currently-selected card (if any). Renderers use it to choose
    /// which card to highlight and which annotations to surface on
    /// the board.
    ///
    /// Boxed because the view model is the largest variant and
    /// would otherwise inflate every other arm's size.
    MoveReady {
        perspective: Perspective,
        view_model: Box<RetrospectiveViewModel>,
        selected_item: Option<usize>,
    },
}

/// Mid-game settings (⚙ gear) descriptor — the live-edit counterpart
/// of the pre-game Start/Options screen (decision #2). Carries a
/// snapshot of the current option values so the renderer paints each
/// control with its true state; the renderer emits the matching
/// per-option intent (`SetEvalBarVisible`, `SetSupport`, `SetAutoCoach`,
/// `SetRetrospectiveDepth`, `ChangeDepth`, `SetRevealBestMoves`,
/// `ToggleOverlay`) which edit the live session directly — no new game
/// needed. `None` when the gear is closed (`build_settings_view`).
///
/// Opponent strength (bot noise / eval-mask) is intentionally *not*
/// here: changing the opponent mid-game requires a new game, so it
/// lives only on the Start screen (PLAN open question — "⚙ mid-game
/// settings scope").
pub struct SettingsView {
    /// Bot play (search) depth.
    pub depth: u32,
    /// Move-feedback (retrospective) search depth.
    pub retrospective_depth: u32,
    /// Whether the eval bar is currently shown.
    pub show_eval_bar: bool,
    /// Snapshot of the live learning preferences (Support / auto-coach
    /// / reveal-best-moves derived via the accessors).
    pub learning: crate::learning_mode::LearningPreferences,
    /// Currently-active board overlays — renderers iterate
    /// [`OverlayKind::ALL`] and check membership for each checkbox.
    pub active_overlays: std::collections::HashSet<OverlayKind>,
}

/// New Game dialog descriptor.
///
/// The form is mutably borrowed from the session because immediate-
/// mode UI frameworks (egui in particular) want `&mut` on each field.
/// The locked-in "payload-on-confirm" design from HANDOFF-ux applies
/// in full to frameworks that can't borrow session state across
/// frames — at that point we'll add the `UpdateNewGameDraft` route
/// and move form state to the renderer. For now `&mut NewGameForm`
/// is the lightest pattern that works.
pub struct NewGameDialogView<'a> {
    pub form: &'a mut NewGameForm,
    pub first_launch: bool,
}
