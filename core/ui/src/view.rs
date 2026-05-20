//! View descriptors — semantic data each renderer paints.
//!
//! View descriptors decouple renderers from session internals: each
//! struct here is a flat data bundle the renderer iterates without
//! poking back into [`crate::session::Session`]. Flags are *semantic*
//! (`last_move: bool`, `check_tint: bool`) — every renderer picks its
//! own palette.

use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Color, File, Move, Piece, PieceType, Rank, Square};

use crate::session::{NewGameForm, RetrospectiveResult};

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

impl BoardView {
    /// Build a [`BoardView`] from a position plus optional UI overlays.
    ///
    /// Callers without a mouse-like input (CLI, headless tests) pass
    /// `selected = None`, `legal_from_selected = &[]`, and
    /// `pending_promotion = None`. The check-tint square and the
    /// last-move flag are derived from `pos` and `last_move`. `pos`
    /// is *also* used for `is_capture` lookups on `legal_from_selected`
    /// entries — pass the live position there if you want capture
    /// rings to read correctly.
    pub fn compose(
        pos: &Position,
        flipped: bool,
        last_move: Option<Move>,
        selected: Option<Square>,
        legal_from_selected: &[Move],
        pending_promotion: Option<PromotionPickerView>,
    ) -> Self {
        let king_in_check = pos
            .in_check()
            .then(|| pos.king_square(pos.side_to_move()));

        let mut rows: [[BoardCell; 8]; 8] = std::array::from_fn(|_| {
            std::array::from_fn(|_| BoardCell {
                square: Square::new(File::A, Rank::R1),
                is_light: false,
                piece: None,
                last_move: false,
                selected: false,
                check_tint: false,
                move_dot: None,
            })
        });

        for display_row in 0..8u8 {
            for display_col in 0..8u8 {
                let (file_idx, rank_idx) = if flipped {
                    (7 - display_col, display_row)
                } else {
                    (display_col, 7 - display_row)
                };
                let is_light = (rank_idx + file_idx) % 2 != 0;
                let sq = Square::new(
                    File::from_index(file_idx).unwrap(),
                    Rank::from_index(rank_idx).unwrap(),
                );
                let last_move_hit = last_move
                    .map(|mv| mv.from() == sq || mv.to() == sq)
                    .unwrap_or(false);
                let move_dot = legal_from_selected
                    .iter()
                    .find(|m| m.to() == sq)
                    .copied()
                    .map(|m| {
                        if pos.is_capture(m) {
                            MoveDotKind::Capture
                        } else {
                            MoveDotKind::Move
                        }
                    });
                rows[display_row as usize][display_col as usize] = BoardCell {
                    square: sq,
                    is_light,
                    piece: pos.piece_on(sq),
                    last_move: last_move_hit,
                    selected: Some(sq) == selected,
                    check_tint: Some(sq) == king_in_check,
                    move_dot,
                };
            }
        }

        BoardView {
            rows,
            pending_promotion,
        }
    }
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

impl PromotionPickerView {
    /// Build a [`PromotionPickerView`] anchored at `target` with the
    /// four candidate moves stacked Q / R / B / N. `promoter_color`
    /// determines the rendered piece colour (the move's `promoted_to`
    /// only carries the piece type).
    pub fn compose(
        target: Square,
        candidates: [Move; 4],
        promoter_color: Color,
        flipped: bool,
    ) -> Self {
        let entries: [PromotionEntry; 4] = std::array::from_fn(|i| {
            let mv = candidates[i];
            let pt = mv.promoted_to();
            let sq = picker_square_at(target, i);
            let (display_col, display_row) = square_to_display_coords(sq, flipped);
            PromotionEntry {
                display_col,
                display_row,
                piece: Piece::new(promoter_color, pt),
                move_: mv,
            }
        });
        Self { entries }
    }
}

/// Display (column, row) for `sq` given board orientation. Public
/// because non-egui renderers (CLI ANSI, future mobile) need it too.
pub fn square_to_display_coords(sq: Square, flipped: bool) -> (u8, u8) {
    let file_idx = sq.file().index() as u8;
    let rank_idx = sq.rank().index() as u8;
    if flipped {
        (7 - file_idx, rank_idx)
    } else {
        (file_idx, 7 - rank_idx)
    }
}

/// The `i`-th square in the promotion picker stack: index 0 is the
/// promotion target, then walking back along the file toward the
/// centre of the board. Always returns a valid square because
/// promotions land on rank 1 or rank 8, leaving four ranks of
/// headroom in the relevant direction.
fn picker_square_at(target: Square, i: usize) -> Square {
    let target_rank = target.rank().index() as i8;
    let direction: i8 = if target_rank == 7 { -1 } else { 1 };
    let rank_idx = (target_rank + direction * i as i8) as u8;
    Square::new(target.file(), Rank::from_index(rank_idx).unwrap())
}

/// `PieceType` -> `char` helper exposed for renderers that pick their
/// own glyph set. Returns uppercase letters; callers can lowercase
/// for black pieces.
pub fn piece_type_letter(pt: PieceType) -> char {
    match pt {
        PieceType::King => 'K',
        PieceType::Queen => 'Q',
        PieceType::Rook => 'R',
        PieceType::Bishop => 'B',
        PieceType::Knight => 'N',
        PieceType::Pawn => 'P',
    }
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
    /// User move; retrospective worker job still in flight.
    UserMoveAnalyzing,
    /// User move; retrospective is ready. Renderer formats the
    /// analyses into prose via its own `chess_tutor_narration` (or
    /// future arrow / highlight rendering). Boxed because the
    /// payload (a `Position` plus a `Vec<MoveAnalysis>`) is large
    /// relative to other variants and would otherwise bloat the
    /// enum size for every panel body.
    UserMoveReady(Box<UserMoveReadyData>),
    EngineMove {
        san: String,
        eval_pawns: f32,
        depth: u32,
        elapsed_ms: u128,
    },
    EngineInfoMissing,
}

pub struct UserMoveReadyData {
    /// Position the analyses are for; what `format_retrospective`
    /// wants as its first argument.
    pub pre_move_pos: Position,
    pub result: RetrospectiveResult,
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
