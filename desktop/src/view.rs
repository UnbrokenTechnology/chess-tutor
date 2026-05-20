//! View descriptors — semantic data each `draw::*` function paints.
//!
//! Step 2 of the platform-portable UI refactor. View descriptors
//! decouple renderers from session internals: each struct here is a
//! flat data bundle the renderer iterates without poking back into
//! [`crate::session::App`]. Flags are *semantic* (`last_move: bool`,
//! `check_tint: bool`) — every renderer picks its own palette.
//!
//! These move into `core/ui` in Step 3 alongside `session.rs` and
//! `worker.rs`.

use chess_tutor_engine::types::{Move, Piece, Square};

use crate::session::NewGameForm;

/// Top-bar panel: New Game / Takeback / Flip / Hint / Live buttons,
/// depth tuner, and a status slot that renders as either a spinner
/// ("engine thinking…") or a game-outcome banner.
pub(crate) struct TopBarView {
    pub(crate) can_takeback: bool,
    pub(crate) hint_open: bool,
    /// `(can-open-hint) || hint_open` — the user can always close a
    /// hint that's already up, even if the conditions for opening one
    /// no longer hold (engine started thinking, etc.).
    pub(crate) hint_button_enabled: bool,
    pub(crate) viewing_live: bool,
    pub(crate) depth: u32,
    pub(crate) engine_thinking: bool,
    pub(crate) game_outcome: Option<&'static str>,
}

/// Eval bar (left rail): one rectangle split into a white-advantage
/// band and a black band, with a numeric label below.
pub(crate) struct EvalBarView {
    /// Fraction of the bar that's the white-advantage band. `0.0` =
    /// pure black, `1.0` = pure white, `0.5` = balanced or no data.
    pub(crate) white_ratio: f32,
    /// Display label: `+0.30`, `-M3`, `—`, etc.
    pub(crate) label: String,
}

/// Board (central panel) in *display order* — rows top-to-bottom on
/// screen, cells left-to-right within each row. The renderer doesn't
/// need to know about flip state; each cell carries the logical
/// [`Square`] so clicks map back to game state.
pub(crate) struct BoardView {
    pub(crate) rows: [[BoardCell; 8]; 8],
    pub(crate) pending_promotion: Option<PromotionPickerView>,
}

#[derive(Clone, Copy)]
pub(crate) struct BoardCell {
    pub(crate) square: Square,
    pub(crate) is_light: bool,
    pub(crate) piece: Option<Piece>,
    pub(crate) last_move: bool,
    pub(crate) selected: bool,
    pub(crate) check_tint: bool,
    pub(crate) move_dot: Option<MoveDotKind>,
}

#[derive(Clone, Copy)]
pub(crate) enum MoveDotKind {
    /// Non-capture move target.
    Move,
    /// Capture target.
    Capture,
}

/// Promotion picker overlay: four piece options stacked from the
/// promotion target square inward along the file. Pre-oriented —
/// each entry carries its display coordinates so the renderer can
/// paint without re-doing the flip math.
pub(crate) struct PromotionPickerView {
    pub(crate) entries: [PromotionEntry; 4],
}

#[derive(Clone, Copy)]
pub(crate) struct PromotionEntry {
    pub(crate) display_col: u8,
    pub(crate) display_row: u8,
    pub(crate) piece: Piece,
    pub(crate) move_: Move,
}

/// Right side panel: move list on top, then either retrospective or
/// hint body depending on whether the hint panel is open.
pub(crate) struct SidePanelView {
    pub(crate) moves: MoveListView,
    pub(crate) body: SidePanelBody,
    /// When the user is following live play, the move-list scroll
    /// should auto-stick to the bottom. When browsing back, freeze
    /// at wherever they scrolled.
    pub(crate) stick_to_bottom: bool,
}

pub(crate) enum SidePanelBody {
    Retrospective(RetrospectivePanelView),
    Hint(HintPanelView),
}

pub(crate) struct MoveListView {
    pub(crate) rows: Vec<MoveListRow>,
}

pub(crate) struct MoveListRow {
    /// 1-based pair index for the leading "N." label.
    pub(crate) move_pair_idx: usize,
    pub(crate) white: MoveListCell,
    pub(crate) black: Option<MoveListCell>,
}

pub(crate) struct MoveListCell {
    pub(crate) history_index: usize,
    pub(crate) san: String,
    pub(crate) selected: bool,
}

pub(crate) struct RetrospectivePanelView {
    pub(crate) game_outcome: Option<&'static str>,
    pub(crate) body: RetrospectiveBody,
}

pub(crate) enum RetrospectiveBody {
    NoMoves,
    Entry {
        /// `Some(san)` when browsing back from live — renderer shows
        /// a "viewing move: {san}" header.
        viewing_back_san: Option<String>,
        kind: RetrospectiveKind,
    },
}

pub(crate) enum RetrospectiveKind {
    UserMoveAnalyzing,
    UserMoveText(String),
    UserMoveEmpty,
    EngineMove {
        san: String,
        eval_pawns: f32,
        depth: u32,
        elapsed_ms: u128,
    },
    EngineInfoMissing,
}

pub(crate) struct HintPanelView {
    pub(crate) state: HintPanelState,
}

pub(crate) enum HintPanelState {
    Loading,
    NoResult,
    NoMoves,
    Ready(Vec<HintEntryView>),
}

pub(crate) struct HintEntryView {
    pub(crate) san: String,
    pub(crate) score_str: String,
    pub(crate) depth: u32,
    pub(crate) pv_san: Vec<String>,
    /// When `Some(i)` and `i < pv_san.len()`, the renderer appends a
    /// "[settles ply i]" marker after the PV.
    pub(crate) settle_marker: Option<usize>,
}

/// New Game dialog descriptor.
///
/// The form is mutably borrowed from the session because egui's
/// immediate-mode widgets want `&mut` on each field. The locked-in
/// "payload-on-confirm" design (per HANDOFF-ux) applies fully when
/// the form moves to renderer-owned state in Step 3; for the egui
/// shell in Step 2, mutating the session-owned form in place is the
/// simplest pattern that survives across frames.
pub(crate) struct NewGameDialogView<'a> {
    pub(crate) form: &'a mut NewGameForm,
    pub(crate) first_launch: bool,
}
