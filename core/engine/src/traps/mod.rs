//! Trap library — hand-curated tactical refutations encoded as
//! decision trees, with chess-exact **invariant** predicates for the
//! fire-or-not decision.
//!
//! A trap is fundamentally a branching structure: at several points
//! the defender has multiple choices, and each choice has its own
//! continuation and its own material consequences. [`PunisherMove`]
//! nodes carry one scripted move plus a list of known defender
//! replies ([`DefenderOption`]), each of which carries its own
//! follow-up `PunisherMove`. Terminal nodes drop their `terminal_gain
//! _cp` and tracking stops there.
//!
//! # Fire-or-not decision (four layers)
//!
//! When a move gets played, we walk through four gates in order —
//! the trap only fires if all four pass:
//!
//! 1. **Trigger pattern matches** — the just-played move has the
//!    right mover / piece / from / to (see [`TriggerPattern`]).
//! 2. **Invariants hold** — every [`Invariant`] in the trap's list is
//!    a chess-exact predicate that must be true in the current
//!    position for the trap's logic to apply. When one fails, its
//!    `label` surfaces as the diagnostic ("this *would* be Damiano,
//!    but Nc6 defends e5"). Invariants double as teaching content —
//!    the list literally enumerates *why* the trap works.
//! 3. **SEE backstop** — if invariants pass but the library's author
//!    missed an edge case, a belt-and-braces check ensures the
//!    defender doesn't have an unscripted move that beats the
//!    scripted main defense by > 50 cp on static exchange
//!    evaluation.
//! 4. **Main-line verify** — every move in the main-defense chain
//!    parses and is legal from the post-trigger position.
//!
//! # `is_main_defense`
//!
//! Each [`DefenderOption`] carries this bool:
//!
//! - `true` — best move available *given the already-lost position*.
//!   Playing it is not a new blunder; the trigger was the mistake.
//!   Multiple siblings can both be `true` when two defences are
//!   equally bad.
//! - `false` — additional blunder beyond the trigger.
//!
//! Any defender move not in the tree is an "escape" — normal move
//! evaluation applies, no trap-level verdict.

use crate::bitboard::Bitboard;
use crate::types::{Color, Piece, PieceType, Square};

// =========================================================================
// Data: the static trap schema
// =========================================================================

/// A single entry in the library. One per known trap, living as a
/// `static` so the whole library is a slice of references.
#[derive(Debug, Clone, Copy)]
pub struct TrapEntry {
    /// Human-readable trap name, shown in UIs.
    pub name: &'static str,
    /// One-paragraph explanation for the teaching layer.
    pub description: &'static str,
    /// Which side gets to execute the refutation after the trigger.
    pub punisher: Color,
    /// The defender move that springs this trap.
    pub trigger: TriggerPattern,
    /// Chess-exact predicates that must all hold in the post-trigger
    /// position for the trap to be valid. Ordered by the author;
    /// the validator short-circuits on the first failure and can
    /// surface its `label` as the reason the trap didn't fire.
    pub invariants: &'static [Invariant],
    /// Root of the refutation tree — the punisher's first move.
    pub root: &'static PunisherMove,
}

/// A move made by the punisher at some point in the refutation.
#[derive(Debug, Clone, Copy)]
pub struct PunisherMove {
    /// Canonical SAN of the move, e.g. `"Nxe5"` or `"Qh5+"`.
    pub san: &'static str,
    /// Known defender responses, in author-chosen order. The first
    /// with `is_main_defense == true` is the scripted main line.
    pub defender_options: &'static [DefenderOption],
    /// Material gain if this node is terminal (rare at punisher
    /// nodes — usually the defender has a reply).
    pub terminal_gain_cp: Option<i32>,
}

/// A candidate defender response to the parent [`PunisherMove`].
#[derive(Debug, Clone, Copy)]
pub struct DefenderOption {
    pub san: &'static str,
    /// `true` = best available in the already-lost position;
    /// `false` = additional blunder beyond the trigger.
    pub is_main_defense: bool,
    /// Optional commentary for UI rendering ("loses a rook", "only
    /// move to avoid mate-in-2", …).
    pub label: Option<&'static str>,
    /// What the punisher plays in reply. `None` = terminal (library
    /// stops tracking; engine / normal classifier take over).
    pub punisher_follow_up: Option<&'static PunisherMove>,
    /// Terminal material gain when this option is the leaf of the
    /// scripted line.
    pub terminal_gain_cp: Option<i32>,
}

/// Which move the opponent just made to land in a trap.
#[derive(Debug, Clone, Copy)]
pub struct TriggerPattern {
    pub mover: Color,
    pub piece_type: PieceType,
    pub to: Square,
    /// Origin square, or `None` to accept any. Usually redundant in
    /// early-game traps (a pawn on f6 can only have come from f7)
    /// but kept for precision.
    pub from: Option<Square>,
}

// =========================================================================
// Data: invariants
// =========================================================================

/// A named chess-exact predicate on a position. Evaluated by
/// [`check_invariant`].
#[derive(Debug, Clone, Copy)]
pub struct Invariant {
    pub kind: InvariantKind,
    /// Student-facing explanation of *why* this invariant matters
    /// for the trap to work, e.g. `"e5 has exactly one defender
    /// (the f6 pawn)"`. Surfaced both as a "here's why this works"
    /// bullet and as a diagnostic when the invariant fails.
    pub label: &'static str,
}

/// The catalogue of predicates used to validate that a trap
/// genuinely applies to the current position.
///
/// Each variant is (a) chess-exact — no heuristic approximation —
/// and (b) cheap to evaluate (a handful of bitboard ops). See
/// [`check_invariant`] for the semantics.
#[derive(Debug, Clone, Copy)]
pub enum InvariantKind {
    // ---- O(1) board inspection -----------------------------------
    /// A specific piece sits on a specific square.
    PieceOn { square: Square, piece: Piece },

    /// The square is empty.
    SquareEmpty { square: Square },

    /// Every square in `mask` is empty.
    AllEmpty { mask: Bitboard },

    /// The square holds a piece belonging to `color` (any piece
    /// type). Useful for traps that need "any friendly piece" to
    /// block a king-escape route.
    AnyPieceOfColor { color: Color, square: Square },

    /// `color` has exactly `count` pieces of `piece_type` on the
    /// board.
    PieceCount {
        color: Color,
        piece_type: PieceType,
        count: u32,
    },

    /// `color` has no pieces of `piece_type` on any square in the
    /// mask. Useful for "no defender of this region" style checks
    /// without enumerating every square.
    NoPieceInMask {
        color: Color,
        piece_type: PieceType,
        mask: Bitboard,
    },

    // ---- attackers_to-based --------------------------------------
    /// `color` has exactly `count` attackers of `square` under the
    /// current occupancy.
    AttackerCountByColor {
        color: Color,
        square: Square,
        count: u32,
    },

    /// `color` does not attack `square` under the current occupancy.
    NotAttackedBy { color: Color, square: Square },

    /// Every attacker of `square` by `color` lies inside `allowed`
    /// — i.e. no attacker outside that mask exists. Use to express
    /// "the only defender of e5 is on f6".
    AttackersSubsetOf {
        color: Color,
        square: Square,
        allowed: Bitboard,
    },

    /// The set of `color`'s attackers of `square` exactly equals
    /// `mask`. Stricter than `AttackersSubsetOf`.
    AttackersEqual {
        color: Color,
        square: Square,
        mask: Bitboard,
    },

    // ---- slider ray checks ---------------------------------------
    /// A hypothetical slider on `from` would attack `to` through
    /// the current occupancy. Equivalent to: `from` and `to` lie on
    /// a shared rank / file / diagonal, AND all squares strictly
    /// between them are empty. Use for "queen on h5 would see e5"
    /// / "queen on h5 would check the e8 king".
    RayClear { from: Square, to: Square },
}

// =========================================================================
// Data: runtime hit / threat payloads
// =========================================================================

/// A freshly-matched trap, ready to surface in the UI. Carries the
/// scripted main line as SAN plus the main-line material gain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrapHit {
    pub name: String,
    pub description: String,
    /// SAN sequence from the punisher's first move through to the
    /// first terminal, following the main-defense branch at each
    /// defender node.
    pub main_line_san: Vec<String>,
    pub main_line_gain_cp: i32,
    pub punisher: Color,
}

/// Pre-move warning: a candidate move the side-to-move could play
/// that would hand a known trap to the opponent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrapThreatened {
    /// UCI form of the candidate move, e.g. `"f7f6"`.
    pub candidate_uci: String,
    /// Same move in canonical SAN for user display.
    pub candidate_san: String,
    /// The trap that would become available to the opponent after
    /// the candidate is played.
    pub hit: TrapHit,
}

// =========================================================================
// Pending-trap state machine — cursor that walks the refutation tree
// move-by-move after a trap fires, so UIs can narrate what's coming
// ("you played fxe5 — here's what happens next") instead of just
// announcing the initial hit and going silent.
// =========================================================================

/// A trap that's currently firing. The cursor into the refutation
/// tree plus the [`TrapHit`] snapshot computed at trigger time (kept
/// stable so UIs can reference the same scripted main line throughout
/// the trap's lifetime).
#[derive(Debug, Clone)]
pub struct PendingTrap {
    pub entry: &'static TrapEntry,
    pub hit: TrapHit,
    pub expectation: TrapExpectation,
}

impl PendingTrap {
    /// Create a fresh pending state from a scan hit. The initial
    /// expectation is the punisher playing the root refutation move.
    pub fn new(entry: &'static TrapEntry, hit: TrapHit) -> PendingTrap {
        PendingTrap {
            entry,
            hit,
            expectation: TrapExpectation::PunisherNext(entry.root),
        }
    }
}

/// What the next played move ought to be for the trap to continue.
#[derive(Debug, Clone, Copy)]
pub enum TrapExpectation {
    /// The punisher is to move. They should play `node.san`.
    PunisherNext(&'static PunisherMove),
    /// The defender is to move. They should pick one of
    /// `parent.defender_options`; `parent` is the last punisher
    /// move played.
    DefenderNext(&'static PunisherMove),
}

/// Emitted by [`advance_pending`] after each played move. The
/// variants narrate what happened relative to the scripted tree:
/// punisher executed / missed, defender stayed in-tree / escaped,
/// tree reached a terminal node.
#[derive(Debug, Clone)]
pub enum TrapEvent {
    /// The punisher played the scripted refutation move. Trap
    /// continues; defender is expected next.
    PunisherExecuted {
        trap: &'static TrapEntry,
        move_san: &'static str,
    },
    /// The punisher was expected to refute but played off-script.
    /// Trap is dead.
    PunisherMissed {
        trap: &'static TrapEntry,
        expected_san: &'static str,
    },
    /// The defender played one of the scripted options. If `option
    /// .punisher_follow_up` is `Some`, the trap continues; if it's
    /// `None`, the tree ends here (either a main-defense terminal
    /// that the author didn't chase further, or a walks-deeper
    /// branch cut at a catastrophic position).
    DefenderInTree {
        trap: &'static TrapEntry,
        option: &'static DefenderOption,
    },
    /// The defender played a move not in the scripted options.
    /// Trap is dead.
    DefenderEscaped { trap: &'static TrapEntry },
    /// The punisher played a terminal refutation move — the tree
    /// has played out to completion.
    TreeComplete {
        trap: &'static TrapEntry,
        gain_cp: Option<i32>,
    },
}

impl TrapEvent {
    /// True when this event ends the pending trap — the caller
    /// should drop their `Option<PendingTrap>` after handling it.
    pub fn is_terminal(&self) -> bool {
        match self {
            Self::PunisherExecuted { .. } => false,
            Self::PunisherMissed { .. } => true,
            Self::DefenderInTree { option, .. } => option.punisher_follow_up.is_none(),
            Self::DefenderEscaped { .. } => true,
            Self::TreeComplete { .. } => true,
        }
    }

    /// The trap this event belongs to.
    pub fn trap(&self) -> &'static TrapEntry {
        match self {
            Self::PunisherExecuted { trap, .. }
            | Self::PunisherMissed { trap, .. }
            | Self::DefenderInTree { trap, .. }
            | Self::DefenderEscaped { trap }
            | Self::TreeComplete { trap, .. } => trap,
        }
    }
}

// =========================================================================
// Library. Each trap lives in its own submodule; adding a trap is
// data-only: one new submodule file plus one entry in LIBRARY.
// =========================================================================

pub mod damiano;

pub static LIBRARY: &[TrapEntry] = &[damiano::DAMIANO];

mod logic;
pub use logic::{advance_pending, check_invariant, scan_after_move, scan_threats};

