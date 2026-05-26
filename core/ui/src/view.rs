//! View descriptors — semantic data each renderer paints.
//!
//! View descriptors decouple renderers from session internals: each
//! struct here is a flat data bundle the renderer iterates without
//! poking back into [`crate::session::Session`]. Flags are *semantic*
//! (`last_move: bool`, `check_tint: bool`) — every renderer picks its
//! own palette.

use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Color, File, Move, Piece, PieceType, Rank, Square};

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
    /// `true` while the game-review surface is showing in the side
    /// panel. The "Review" button is a toggle.
    pub review_open: bool,
    /// `true` when there's at least one user move whose retrospective
    /// has arrived (so the review would have something to show).
    pub review_button_enabled: bool,
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
///
/// `annotations` is an overlay layer (arrows + square highlights)
/// drawn on top of the regular cell grid. Empty when no retrospective
/// item is selected; non-empty when the user has clicked a card and
/// the board is illustrating that item's spatial story.
pub struct BoardView {
    pub rows: [[BoardCell; 8]; 8],
    pub pending_promotion: Option<PromotionPickerView>,
    pub annotations: Vec<BoardAnnotation>,
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
    ///
    /// `annotations` carries the overlay layer (arrows + square
    /// highlights) — typically the spatial story of a retrospective
    /// card the user is hovering / has selected. Pass an empty `Vec`
    /// when nothing is selected.
    pub fn compose(
        pos: &Position,
        flipped: bool,
        last_move: Option<Move>,
        selected: Option<Square>,
        legal_from_selected: &[Move],
        pending_promotion: Option<PromotionPickerView>,
        annotations: Vec<BoardAnnotation>,
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
            annotations,
        }
    }
}

/// Spatial annotation drawn on top of the regular cell grid. Each
/// renderer maps `kind` to its own visual language (egui paints
/// arrows; the CLI can ignore the layer entirely or print a text
/// summary under the board).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoardAnnotation {
    /// An arrow from `from` to `to`. Used for moves (best-move,
    /// engine-preferred line, capture sequence) and threats
    /// (attacker → target).
    Arrow {
        from: Square,
        to: Square,
        kind: AnnotationKind,
    },
    /// Tint or border the square. Used for hanging pieces, king
    /// rings, outposts, weak pawns, etc.
    SquareHighlight {
        square: Square,
        kind: AnnotationKind,
    },
}

/// Semantic role of a [`BoardAnnotation`] — drives renderer color +
/// style choices. The renderer maps each kind to its own palette; no
/// specific colors are dictated here. Kinds are deliberately
/// fine-grained so renderers can theme them consistently (e.g.,
/// every "Attacker" arrow renders the same regardless of which item
/// produced it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnnotationKind {
    /// Engine's preferred move (the one the user missed). Subtle —
    /// a teaching nudge, not an alarm.
    BestMove,
    /// A capture in the expected line. Used for material narration's
    /// capture sequence arrows.
    Capture,
    /// A piece (typically the user's) is under attack or hanging.
    /// Square highlight kind. Red-ish.
    Threat,
    /// An attacker piece pointing at its target. Arrow kind.
    Attacker,
    /// A defender piece pointing at the piece it covers. Arrow kind.
    Defender,
    /// The king's ring of nearby squares (king-safety narration).
    KingRing,
    /// A piece the narrator considers active / well-placed. Green-ish
    /// square highlight.
    GoodPiece,
    /// A piece the narrator flags as weak / misplaced. Orange-ish
    /// square highlight.
    BadPiece,
    /// A square the moving piece newly attacks (mobility gained).
    NewMobility,
    /// A square the moving piece used to attack but no longer does
    /// (mobility lost).
    LostMobility,
    /// "Front" space — a safe square in your central camp that
    /// contributes to the space score. Subtle teal tint.
    SpaceFront,
    /// "Reinforced" space — a safe square on or behind one of your
    /// pawns that no enemy piece attacks, doubly rewarded by the
    /// space term. Stronger teal / blue tint than [`SpaceFront`].
    SpaceReinforced,
    /// "Front" space for the opponent. Same role as [`SpaceFront`]
    /// but rendered in a distinct hue so a "both space overlays on"
    /// view distinguishes the two sides.
    OpponentSpaceFront,
    /// "Reinforced" space for the opponent. Same role as
    /// [`SpaceReinforced`].
    OpponentSpaceReinforced,
    /// A square excluded from the mobility area (own king/queen,
    /// pinned piece, blocked or low-rank own pawn, or enemy-pawn-
    /// attacked). Painted by the mobility-area overlay so the
    /// student sees what the engine's per-piece-type mobility term
    /// stops counting.
    MobilityExcluded,
    /// A pinned piece — `Position::blockers_for_king(us)` membership.
    /// Painted by the pin overlay.
    Pin,
    /// One net attacker advantage for our side at this square.
    HeatOurs1,
    /// Two or more net attacker advantage for our side at this square.
    HeatOurs2,
    /// One net attacker advantage for the opponent at this square.
    HeatTheirs1,
    /// Two or more net attacker advantage for the opponent.
    HeatTheirs2,
    /// Generic teaching highlight (yellow). Used when no more
    /// specific kind applies.
    Highlight,
}

/// Per-side sentiment of a retrospective item — used by renderers to
/// pick an accent color (green / red / amber / grey). "User" here is
/// the side that just moved (`pre_move_pos.side_to_move()`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sentiment {
    /// Helps the user.
    Positive,
    /// Hurts the user.
    Negative,
    /// Both sides affected, no clear net direction.
    Mixed,
    /// Informational, no sign.
    #[default]
    Neutral,
}

/// Structured per-move retrospective. Replaces the prior
/// "format-to-text-blob" path: each card the renderer paints comes
/// from one [`RetrospectiveItem`], and clicking a card surfaces its
/// `annotations` on the board.
///
/// The narration crate's [`chess_tutor_narration::format_retrospective`]
/// is still the canonical *text* renderer; this view model is the
/// canonical *structured* surface for visual renderers. They share
/// the underlying engine outcome computations (see
/// [`chess_tutor_engine::analysis`]) but format independently —
/// converging is a deferred refactor.
#[derive(Clone, Debug, Default)]
pub struct RetrospectiveViewModel {
    pub headline: RetrospectiveHeadline,
    pub items: Vec<RetrospectiveItem>,
}

/// First card of every retrospective: the verdict, the post-move
/// score, the engine's preferred alternative (if any), and an
/// optional surprise / sharp note.
///
/// `best_move_annotation` carries the from→to arrow for the engine-
/// preferred move when the user wasn't best — renderers can paint
/// it as the "always-on" arrow for this retrospective even when no
/// individual card is selected.
#[derive(Clone, Debug, Default)]
pub struct RetrospectiveHeadline {
    pub user_san: String,
    /// Annotation suffix: "!!", "!", "?!", "?", "??" — empty when no
    /// SAN annotation applies (`Best` without the sharp flag, `Good`).
    pub san_annotation: &'static str,
    /// Human-readable verdict label ("Best", "Inaccuracy", etc.).
    pub verdict_label: &'static str,
    pub verdict_sentiment: Sentiment,
    /// Formatted post-move score from root STM's POV, e.g. "+0.30" or
    /// "M5".
    pub user_score: String,
    /// Engine's preferred move in SAN — set when distinct from the
    /// user's move.
    pub best_san: Option<String>,
    pub best_score: Option<String>,
    /// `user_score − best_score`, formatted, e.g. "-0.40".
    pub gap: Option<String>,
    /// "Position was already lost" (BestAvailable), "Well spotted —
    /// this looks risky at first glance, but the longer line pays
    /// off." (sharp Best), or `None`.
    pub note: Option<String>,
    /// Arrow for the engine-preferred move, when present.
    pub best_move_annotation: Option<BoardAnnotation>,
}

/// One card in the retrospective. The renderer paints these in
/// order, framed with a sentiment-coloured strip, a heading, a
/// summary, and (when selected) an expanded detail block. Clicking
/// a card emits a `SelectRetrospectiveItem(i)` event.
#[derive(Clone, Debug)]
pub struct RetrospectiveItem {
    pub category: RetrospectiveCategory,
    /// Short title — appears as the card heading. e.g. "Bishop
    /// activity", "Hanging piece", "King exposed".
    pub heading: String,
    /// One-line subtitle. e.g. "improved (+0.20 → +0.80)".
    pub summary: String,
    /// Optional multi-line explanation rendered when the card is
    /// selected or expanded.
    pub detail: String,
    /// Score delta in pawns, signed from user's POV. Renderers may
    /// show "+0.30" / "-1.20" as a small chip on the card.
    pub score_delta_pawns: Option<f32>,
    pub sentiment: Sentiment,
    /// Board annotations to paint when this card is selected. Empty
    /// for cards that don't have a spatial story (e.g. raw secondary
    /// term shifts).
    pub annotations: Vec<BoardAnnotation>,
}

/// Category of a retrospective item — drives icon / glyph + colour
/// theming in renderers that want consistent styling across cards
/// of the same family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetrospectiveCategory {
    Material,
    Threats,
    KingSafety,
    PawnStructure,
    Mobility,
    PassedPawns,
    PiecePlacement,
    Initiative,
    BlockedCenter,
    Castling,
    Space,
    /// Secondary / fallback eval-term shifts (the old "Helped" /
    /// "Hurt" lines).
    Secondary,
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
}

pub enum SidePanelBody {
    /// An in-game intervention is pending — the engine reply is held
    /// until the user dismisses, takes back, or reveals the concept.
    /// Takes priority over the retrospective panel so the prompt is
    /// the first thing the user sees.
    Intervention(InterventionPanelView),
    /// Live coaching surface, shown when AssistanceLevel::Coached is
    /// active AND it's the user's turn AND no higher-priority body
    /// is active. Lists features-to-notice from the current
    /// position; never names a move.
    Coaching(CoachingPanelView),
    Retrospective(RetrospectivePanelView),
    Hint(HintPanelView),
    /// Post-game (or on-demand) review surface — a ranked list of
    /// significant moments the user should study. Click any moment
    /// to jump the rest of the UI to that move.
    GameReview(GameReviewView),
}

/// Wrapper that lets the renderer paint a header / empty-state /
/// disabled-state alongside the items themselves. When `items` is
/// empty, the renderer should show an encouraging neutral message
/// rather than a blank panel.
pub struct CoachingPanelView {
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
}

/// Post-game review surface: a list of significant moments derived
/// from running [`chess_tutor_engine::analysis::classify_user_move`]
/// over the user's moves. Renderers paint a clickable list; clicking
/// a row emits [`crate::event::Event::JumpToReviewMoment`] which
/// snaps the rest of the UI to that history index.
pub struct GameReviewView {
    /// Optional one-line outcome label (e.g. "Checkmate — White wins.")
    /// when the game is over.
    pub game_outcome: Option<&'static str>,
    /// Total user moves in the game (so the renderer can show "3 of
    /// 28 moves flagged" context).
    pub user_move_count: usize,
    /// Ranked list of significant moments. Empty when no user moves
    /// crossed an intervention gate.
    pub moments: Vec<GameReviewMoment>,
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
    /// Per-square attacker imbalance. Squares with a net advantage
    /// for you tint green; squares with a net advantage for the
    /// opponent tint red; even-but-contested squares stay clear.
    /// Intensity steps with magnitude (one tier for |net| = 1, a
    /// stronger tier for |net| ≥ 2).
    AttackHeatmap,
}

impl OverlayKind {
    pub const ALL: [OverlayKind; 6] = [
        OverlayKind::MySpace,
        OverlayKind::OpponentSpace,
        OverlayKind::MyMobilityArea,
        OverlayKind::KingRings,
        OverlayKind::Pins,
        OverlayKind::AttackHeatmap,
    ];

    pub fn label(self) -> &'static str {
        match self {
            OverlayKind::MySpace => "My space",
            OverlayKind::OpponentSpace => "Opponent's space",
            OverlayKind::MyMobilityArea => "Mobility area (excluded)",
            OverlayKind::KingRings => "King rings",
            OverlayKind::Pins => "Pins",
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
    /// User move; retrospective worker job still in flight.
    UserMoveAnalyzing,
    /// User move; retrospective is ready. The session builds a
    /// structured [`RetrospectiveViewModel`] per frame from the raw
    /// analyses; renderers paint cards from it and emit
    /// [`crate::event::Event::SelectRetrospectiveItem`] on click.
    ///
    /// `selected_item` is the index into `view_model.items` of the
    /// currently-selected card (if any). Renderers use it to choose
    /// which card to highlight and which annotations to surface on
    /// the board.
    ///
    /// Boxed because the view model is the largest variant and
    /// would otherwise inflate every other arm's size.
    UserMoveReady {
        view_model: Box<RetrospectiveViewModel>,
        selected_item: Option<usize>,
    },
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
