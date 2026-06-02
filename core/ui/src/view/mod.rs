//! View descriptors — semantic data each renderer paints.
//!
//! View descriptors decouple renderers from session internals: each
//! struct here is a flat data bundle the renderer iterates without
//! poking back into [`crate::session::Session`]. Flags are *semantic*
//! (`last_move: bool`, `check_tint: bool`) — every renderer picks its
//! own palette.

use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Color, File, Move, Piece, PieceType, Rank, Square};

mod panels;
pub use panels::*;



/// Slim title bar: app title, a ⚙ settings button, a flip-board
/// button, and a status slot that renders as either a spinner
/// ("engine thinking…") or a game-outcome banner. The primary play
/// actions (Takeback / Hint / New Game) live in [`ActionBarView`] at
/// the bottom of the right column now, not here.
///
/// Review / Live stay here for now — they're relocated to the
/// post-game review surface in a later redesign step; keeping them on
/// the title bar avoids losing the functionality in the interim.
///
/// `depth` is parked here as a minimal control: its true home is the
/// Options/⚙ surface (a later step), but with no settings screen yet
/// it stays minimally accessible on the title bar so depth tuning
/// isn't lost.
pub struct TopBarView {
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

/// The big, obvious bottom-of-the-right-column action bar
/// (chess.com idiom): Takeback / Hint / New Game. These are the
/// primary play controls, sized large for legibility.
pub struct ActionBarView {
    pub can_takeback: bool,
    /// Whether the Hint surface is currently open (the button doubles
    /// as a toggle / "Hide Hint").
    pub hint_open: bool,
    /// `(can-open-hint) || hint_open` — the user can always close a
    /// hint that's already up, even if the conditions for opening one
    /// no longer hold (engine started thinking, etc.).
    pub hint_button_enabled: bool,
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
    /// A "dead" escape square in a trapped piece's cage — a square the
    /// piece could legally move to, but every option in the cage is
    /// unsafe. Painted by the trapped-piece overlay around the doomed
    /// piece itself (which uses [`Self::BadPiece`]).
    TrappedEscape,
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
    /// The move that *springs* a future tactic — drawn as an arrow from
    /// the piece's current square to where it lands. Used by the
    /// walked-into pin card: the pinning piece isn't on the pin line yet,
    /// so this arrow shows the opponent reply that puts it there (e.g.
    /// `…Bf4`), making the pin-line arrow from the (still-empty) pinning
    /// square read. Distinct hue from [`Self::BestMove`] so a "you walked
    /// into …" card never paints the opponent's move in the engine's
    /// own best-move colour.
    TriggerMove,
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
/// The teaching crate's [`chess_tutor_teaching::format_retrospective`]
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
    /// Human-readable verdict label. The engine-truth ladder
    /// ("Best", "Inaccuracy", …) remapped to chess.com's presentation
    /// tiers by the teaching translator — "Great" / "Brilliant" for an
    /// only-good-move (sacrifice). Owned because the tier is dynamic.
    pub verdict_label: String,
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
    /// A named tactic — `Fork`, `Pin`, `Skewer`, `Checkmate`, etc. —
    /// played by the user or missed by the user. Drives the
    /// retrospective "you played a fork" / "you missed a tactic" cards.
    /// Walked-into tactics surface as a separate forced-consequences
    /// card and don't use this category.
    Tactic,
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
