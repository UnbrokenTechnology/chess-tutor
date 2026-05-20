//! View descriptors — semantic data each renderer paints.
//!
//! View descriptors decouple renderers from session internals: each
//! struct here is a flat data bundle the renderer iterates without
//! poking back into [`crate::session::Session`]. Flags are *semantic*
//! (`last_move: bool`, `check_tint: bool`) — every renderer picks its
//! own palette.

use chess_tutor_engine::types::{Move, Piece, Square};

use crate::session::NewGameForm;

/// Top-bar panel: New Game / Takeback / Flip / Hint / Live buttons,
/// depth tuner, and a status slot that renders as either a spinner
/// ("engine thinking…") or a game-outcome banner.
pub struct TopBarView {
    pub can_takeback: bool,
    pub hint_open: bool,
    /// `(can-open-hint) || hint_open` — the user can always close a
    /// hint that's already up, even if the conditions for opening one
    /// no longer hold (engine started thinking, etc.).
    pub hint_button_enabled: bool,
    pub viewing_live: bool,
    pub depth: u32,
    pub engine_thinking: bool,
    pub game_outcome: Option<&'static str>,
}

/// Eval bar (left rail): one rectangle split into a white-advantage
/// band and a black band, with a numeric label below.
pub struct EvalBarView {
    /// Fraction of the bar that's the white-advantage band. `0.0` =
    /// pure black, `1.0` = pure white, `0.5` = balanced or no data.
    pub white_ratio: f32,
    /// Display label: `+0.30`, `-M3`, `—`, etc.
    pub label: String,
}

/// Board (central panel) in *display order* — rows top-to-bottom on
/// screen, cells left-to-right within each row. The renderer doesn't
/// need to know about flip state; each cell carries the logical
/// [`Square`] so clicks map back to game state.
pub struct BoardView {
    pub rows: [[BoardCell; 8]; 8],
    pub pending_promotion: Option<PromotionPickerView>,
}

#[derive(Clone, Copy)]
pub struct BoardCell {
    pub square: Square,
    pub is_light: bool,
    pub piece: Option<Piece>,
    pub last_move: bool,
    pub selected: bool,
    pub check_tint: bool,
    pub move_dot: Option<MoveDotKind>,
}

#[derive(Clone, Copy)]
pub enum MoveDotKind {
    /// Non-capture move target.
    Move,
    /// Capture target.
    Capture,
}

/// Promotion picker overlay: four piece options stacked from the
/// promotion target square inward along the file. Pre-oriented —
/// each entry carries its display coordinates so the renderer can
/// paint without re-doing the flip math.
pub struct PromotionPickerView {
    pub entries: [PromotionEntry; 4],
}

#[derive(Clone, Copy)]
pub struct PromotionEntry {
    pub display_col: u8,
    pub display_row: u8,
    pub piece: Piece,
    pub move_: Move,
}

/// Right side panel: move list on top, then either retrospective or
/// hint body depending on whether the hint panel is open.
pub struct SidePanelView {
    pub moves: MoveListView,
    pub body: SidePanelBody,
    /// When the user is following live play, the move-list scroll
    /// should auto-stick to the bottom. When browsing back, freeze
    /// at wherever they scrolled.
    pub stick_to_bottom: bool,
}

pub enum SidePanelBody {
    Retrospective(RetrospectivePanelView),
    Hint(HintPanelView),
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
}

pub struct RetrospectivePanelView {
    pub game_outcome: Option<&'static str>,
    pub body: RetrospectiveBody,
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

pub struct HintPanelView {
    pub state: HintPanelState,
}

pub enum HintPanelState {
    Loading,
    NoResult,
    NoMoves,
    Ready(Vec<HintEntryView>),
}

pub struct HintEntryView {
    pub san: String,
    pub score_str: String,
    pub depth: u32,
    pub pv_san: Vec<String>,
    /// When `Some(i)` and `i < pv_san.len()`, the renderer appends a
    /// "[settles ply i]" marker after the PV.
    pub settle_marker: Option<usize>,
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
