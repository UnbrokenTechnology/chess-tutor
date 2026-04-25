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

use crate::attacks::{attacks_bb, between_bb};
use crate::bitboard::Bitboard;
use crate::movegen::legal_moves_vec;
use crate::position::Position;
use crate::san;
use crate::types::{Color, Move, MoveKind, Piece, PieceType, Square, Value};

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

// =========================================================================
// Public scanning API
// =========================================================================

/// Scan every legal move for the side-to-move in `pos`. Return the
/// ones that would trigger a library trap — i.e. "if you play this,
/// your opponent gets a known refutation against you".
///
/// Intended for pre-move UI hints. Expensive for wide positions
/// (quadratic in candidate count × library size, with invariant /
/// SEE / main-line checks per candidate), but legal-move counts are
/// bounded and the library is small so this stays cheap enough to
/// run on every position change.
pub fn scan_threats(pos: &Position) -> Vec<TrapThreatened> {
    let side_to_move = pos.side_to_move();
    let mut out = Vec::new();

    for trap in LIBRARY {
        if trap.trigger.mover != side_to_move {
            continue;
        }
        let mut scratch = pos.clone();
        let legal = legal_moves_vec(&mut scratch);
        for mv in legal {
            if !trap.trigger.matches(side_to_move, pos, mv) {
                continue;
            }
            let mut after = pos.clone();
            let candidate_san = san::format(&after, mv);
            let _ = after.do_move(mv);
            if let Some(hit) = validate_and_build_hit(&after, trap) {
                out.push(TrapThreatened {
                    candidate_uci: uci_of(mv),
                    candidate_san,
                    hit,
                });
            }
        }
    }

    out
}

/// Scan the library for traps triggered by the just-played move.
/// `pos` is the position **after** the move landed (so the side-to-
/// move is the punisher's side). Returns one entry per firing trap.
pub fn scan_after_move(
    pos: &Position,
    last_move_mover: Color,
    last_move_piece_type: PieceType,
    last_move_from: Square,
    last_move_to: Square,
) -> Vec<(&'static TrapEntry, TrapHit)> {
    let mut hits = Vec::new();
    for trap in LIBRARY {
        if !trap.trigger.matches_parts(
            last_move_mover,
            last_move_piece_type,
            last_move_from,
            last_move_to,
        ) {
            continue;
        }
        if let Some(hit) = validate_and_build_hit(pos, trap) {
            hits.push((trap, hit));
        }
    }
    hits
}

/// Advance a pending trap by a played move. `pre_pos` is the
/// position **before** the move was played (needed to resolve the
/// scripted SAN into a concrete [`Move`]). The `pending` cursor is
/// mutated in place; callers check [`TrapEvent::is_terminal`] on
/// the returned event to decide whether to drop their `Option<
/// PendingTrap>`.
pub fn advance_pending(pending: &mut PendingTrap, pre_pos: &Position, played: Move) -> TrapEvent {
    match pending.expectation {
        TrapExpectation::PunisherNext(node) => {
            if !scripted_matches_played(pre_pos, node.san, played) {
                return TrapEvent::PunisherMissed {
                    trap: pending.entry,
                    expected_san: node.san,
                };
            }
            // Punisher executed the scripted move.
            if node.defender_options.is_empty() {
                // Terminal punisher node — tree has played out.
                return TrapEvent::TreeComplete {
                    trap: pending.entry,
                    gain_cp: node.terminal_gain_cp,
                };
            }
            pending.expectation = TrapExpectation::DefenderNext(node);
            TrapEvent::PunisherExecuted {
                trap: pending.entry,
                move_san: node.san,
            }
        }
        TrapExpectation::DefenderNext(parent) => {
            for option in parent.defender_options {
                if !scripted_matches_played(pre_pos, option.san, played) {
                    continue;
                }
                // Defender picked a scripted option. Advance the
                // cursor only if there's a follow-up; a `None`
                // follow-up leaves the event terminal and the caller
                // drops the pending trap.
                if let Some(next) = option.punisher_follow_up {
                    pending.expectation = TrapExpectation::PunisherNext(next);
                }
                return TrapEvent::DefenderInTree {
                    trap: pending.entry,
                    option,
                };
            }
            TrapEvent::DefenderEscaped {
                trap: pending.entry,
            }
        }
    }
}

/// True when the scripted SAN, parsed in `pos`, resolves to the same
/// [`Move`] as `played`. Used to match scripted tree nodes against
/// actual played moves.
fn scripted_matches_played(pos: &Position, scripted_san: &str, played: Move) -> bool {
    let mut scratch = pos.clone();
    match san::parse(&mut scratch, scripted_san) {
        Ok(mv) => mv == played,
        Err(_) => false,
    }
}

/// Evaluate a single invariant against a position. Made public so
/// UIs can render the full "why this works" list by calling each
/// invariant individually and showing which passed.
pub fn check_invariant(pos: &Position, kind: &InvariantKind) -> bool {
    match *kind {
        InvariantKind::PieceOn { square, piece } => pos.piece_on(square) == Some(piece),

        InvariantKind::SquareEmpty { square } => pos.piece_on(square).is_none(),

        InvariantKind::AllEmpty { mask } => (pos.occupied() & mask).is_empty(),

        InvariantKind::AnyPieceOfColor { color, square } => {
            pos.pieces_by_color(color).contains(square)
        }

        InvariantKind::PieceCount {
            color,
            piece_type,
            count,
        } => pos.count(color, piece_type) == count,

        InvariantKind::NoPieceInMask {
            color,
            piece_type,
            mask,
        } => (pos.pieces_of(color, piece_type) & mask).is_empty(),

        InvariantKind::AttackerCountByColor {
            color,
            square,
            count,
        } => {
            let attackers = pos.attackers_to(square, pos.occupied()) & pos.pieces_by_color(color);
            attackers.popcount() == count
        }

        InvariantKind::NotAttackedBy { color, square } => {
            let attackers = pos.attackers_to(square, pos.occupied()) & pos.pieces_by_color(color);
            attackers.is_empty()
        }

        InvariantKind::AttackersSubsetOf {
            color,
            square,
            allowed,
        } => {
            let attackers = pos.attackers_to(square, pos.occupied()) & pos.pieces_by_color(color);
            (attackers & !allowed).is_empty()
        }

        InvariantKind::AttackersEqual {
            color,
            square,
            mask,
        } => {
            let attackers = pos.attackers_to(square, pos.occupied()) & pos.pieces_by_color(color);
            attackers == mask
        }

        InvariantKind::RayClear { from, to } => {
            // `attacks_bb(QUEEN, from, occ)` returns the squares a
            // queen on `from` would attack through current
            // occupancy; membership of `to` means the two squares
            // are both aligned and separated only by empty squares.
            attacks_bb(PieceType::Queen, from, pos.occupied()).contains(to)
                && between_bb(from, to) & pos.occupied() == Bitboard::EMPTY
        }
    }
}

// =========================================================================
// Validator pipeline (invariants → SEE → main-line verify)
// =========================================================================

/// Run the three validation gates and, if they all pass, build the
/// [`TrapHit`] that describes the scripted refutation. `pos` must be
/// the post-trigger position — i.e. after the trigger move has been
/// played and it's the punisher's turn.
fn validate_and_build_hit(pos: &Position, trap: &TrapEntry) -> Option<TrapHit> {
    // Gate 2: invariants.
    for inv in trap.invariants {
        if !check_invariant(pos, &inv.kind) {
            return None;
        }
    }
    // Gates 3 and 4 happen together inside main-line verification:
    // at every defender branch we SEE-check the unscripted
    // alternatives and also verify the scripted move is legal.
    let (main_line_san, main_line_gain_cp) = walk_main_line(pos, trap.punisher, trap.root)?;
    Some(TrapHit {
        name: trap.name.to_string(),
        description: trap.description.to_string(),
        main_line_san,
        main_line_gain_cp,
        punisher: trap.punisher,
    })
}

/// Walk the main line from `start`, following the first
/// `is_main_defense` branch at each defender node. Returns
/// `(san sequence, material gain from `pos` to the terminal)` or
/// `None` if any scripted move fails to parse / is illegal, or the
/// SEE backstop detects a better unscripted defender move.
fn walk_main_line(
    pos: &Position,
    punisher: Color,
    start: &'static PunisherMove,
) -> Option<(Vec<String>, i32)> {
    let mut scratch = pos.clone();
    let material_before = material_delta_for(&scratch, punisher);
    let mut line = Vec::new();
    let mut node = start;

    loop {
        let mv = san::parse(&mut scratch, node.san).ok()?;
        let _ = scratch.do_move(mv);
        line.push(node.san.to_string());
        if node.defender_options.is_empty() {
            break;
        }

        // SEE backstop on the position where the defender is to move.
        if defender_has_better_unscripted_move(&scratch, node.defender_options) {
            return None;
        }

        let option = node.defender_options.iter().find(|o| o.is_main_defense)?;
        let reply = san::parse(&mut scratch, option.san).ok()?;
        let _ = scratch.do_move(reply);
        line.push(option.san.to_string());

        match option.punisher_follow_up {
            Some(next) => node = next,
            None => break,
        }
    }

    let material_after = material_delta_for(&scratch, punisher);
    Some((line, material_after - material_before))
}

/// True when the defender — in the position handed in — has a legal
/// move that isn't in the library's option list AND outscores the
/// scripted main-defense by more than 50 cp on static exchange
/// evaluation. Fires when the author missed a defender resource.
///
/// The 50 cp tolerance absorbs minor SEE wobble; at a pawn-and-a-
/// half difference we're confident a real player would pick the
/// unscripted move instead and the library's premise has broken.
fn defender_has_better_unscripted_move(pos: &Position, options: &[DefenderOption]) -> bool {
    const TOLERANCE: i32 = 50;

    let mut scratch = pos.clone();
    let legal = legal_moves_vec(&mut scratch);

    // Collect the moves that correspond to scripted options.
    let mut scripted: Vec<Move> = Vec::with_capacity(options.len());
    for opt in options {
        let mut parse_scratch = pos.clone();
        if let Ok(mv) = san::parse(&mut parse_scratch, opt.san) {
            scripted.push(mv);
        }
    }

    // SEE of the scripted main-defense — our baseline.
    let main_defense_san = match options.iter().find(|o| o.is_main_defense) {
        Some(o) => o.san,
        None => return false, // nothing to compare against
    };
    let mut md_scratch = pos.clone();
    let main_defense_mv = match san::parse(&mut md_scratch, main_defense_san) {
        Ok(mv) => mv,
        Err(_) => return false,
    };
    let main_defense_see = see_score(pos, main_defense_mv);

    for mv in &legal {
        if scripted.contains(mv) {
            continue;
        }
        let unscripted_see = see_score(pos, *mv);
        if unscripted_see > main_defense_see + TOLERANCE {
            return true;
        }
    }
    false
}

/// Rough SEE "score" for a move: 0 if the move is a quiet (non-
/// capture), otherwise the threshold bisection pinned to pawn-
/// valued increments. Built from our [`Position::see_ge`] which is
/// a boolean "≥ threshold" test.
///
/// Returns a centipawn value in `[-2000, +2000]` (queen-ish bounds).
/// Exact to the nearest pawn; that resolution is fine at the 50 cp
/// tolerance we apply upstream.
fn see_score(pos: &Position, mv: Move) -> i32 {
    if pos.piece_on(mv.to()).is_none() {
        return 0;
    }
    // Bisect on pawn-valued thresholds. We don't need precision —
    // the caller just wants "is this materially better than the
    // scripted alternative by more than half a pawn".
    let candidates = [-2000, -1000, -500, -200, -100, 0, 100, 200, 500, 1000, 2000];
    let mut best = -2000;
    for &t in &candidates {
        if pos.see_ge(mv, Value(t)) {
            best = t;
        }
    }
    best
}

/// Material delta for `color` in **conventional centipawns**
/// (pawn = 100, knight = 300, bishop = 325, rook = 500, queen = 900)
/// — the units a teaching UI and hand-written `terminal_gain_cp`
/// fixtures speak in. The engine's internal piece values (pawn EG
/// = 213, etc.) are calibrated for the classical evaluator and
/// aren't what a student means when they say "you lost a rook".
fn material_delta_for(pos: &Position, color: Color) -> i32 {
    const VALUES: [(PieceType, i32); 5] = [
        (PieceType::Pawn, 100),
        (PieceType::Knight, 300),
        (PieceType::Bishop, 325),
        (PieceType::Rook, 500),
        (PieceType::Queen, 900),
    ];
    let mut total = 0i32;
    for (pt, value) in VALUES {
        let ours = pos.count(color, pt) as i32;
        let theirs = pos.count(!color, pt) as i32;
        total += (ours - theirs) * value;
    }
    total
}

fn uci_of(mv: Move) -> String {
    let mut s = String::with_capacity(5);
    s.push_str(&mv.from().to_algebraic());
    s.push_str(&mv.to().to_algebraic());
    if mv.kind() == MoveKind::Promotion {
        s.push(match mv.promoted_to() {
            PieceType::Knight => 'n',
            PieceType::Bishop => 'b',
            PieceType::Rook => 'r',
            PieceType::Queen => 'q',
            _ => '?',
        });
    }
    s
}

// =========================================================================
// Trigger matching
// =========================================================================

impl TriggerPattern {
    /// True if `mv`, played in `pos` by `mover`, matches this trigger.
    /// `pos` is used to resolve the moving piece's type.
    fn matches(&self, mover: Color, pos: &Position, mv: Move) -> bool {
        if mover != self.mover || mv.to() != self.to {
            return false;
        }
        if let Some(required) = self.from {
            if mv.from() != required {
                return false;
            }
        }
        match pos.piece_on(mv.from()) {
            Some(p) => piece_type_of(p) == self.piece_type,
            None => false,
        }
    }

    /// Pattern-only match when the caller already has the piece type
    /// in hand (e.g. replaying a move log).
    fn matches_parts(&self, mover: Color, piece_type: PieceType, from: Square, to: Square) -> bool {
        mover == self.mover
            && piece_type == self.piece_type
            && to == self.to
            && self.from.map_or(true, |required| required == from)
    }
}

fn piece_type_of(piece: Piece) -> PieceType {
    match piece {
        Piece::WhitePawn | Piece::BlackPawn => PieceType::Pawn,
        Piece::WhiteKnight | Piece::BlackKnight => PieceType::Knight,
        Piece::WhiteBishop | Piece::BlackBishop => PieceType::Bishop,
        Piece::WhiteRook | Piece::BlackRook => PieceType::Rook,
        Piece::WhiteQueen | Piece::BlackQueen => PieceType::Queen,
        Piece::WhiteKing | Piece::BlackKing => PieceType::King,
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitboard::square_bb;

    // ---- InvariantKind ----------------------------------------------

    #[test]
    fn piece_on_matches_an_actual_piece() {
        let pos = Position::startpos();
        assert!(check_invariant(
            &pos,
            &InvariantKind::PieceOn {
                square: Square::E1,
                piece: Piece::WhiteKing
            }
        ));
        assert!(!check_invariant(
            &pos,
            &InvariantKind::PieceOn {
                square: Square::E1,
                piece: Piece::BlackKing
            }
        ));
    }

    #[test]
    fn square_empty_and_all_empty_agree() {
        let pos = Position::startpos();
        assert!(check_invariant(
            &pos,
            &InvariantKind::SquareEmpty { square: Square::E4 }
        ));
        assert!(!check_invariant(
            &pos,
            &InvariantKind::SquareEmpty { square: Square::E2 }
        ));
        let mid_board = square_bb(Square::E4) | square_bb(Square::D4) | square_bb(Square::F4);
        assert!(check_invariant(
            &pos,
            &InvariantKind::AllEmpty { mask: mid_board }
        ));
    }

    #[test]
    fn any_piece_of_color_lights_up_friendly_squares() {
        let pos = Position::startpos();
        assert!(check_invariant(
            &pos,
            &InvariantKind::AnyPieceOfColor {
                color: Color::Black,
                square: Square::F8
            }
        ));
        assert!(!check_invariant(
            &pos,
            &InvariantKind::AnyPieceOfColor {
                color: Color::White,
                square: Square::F8
            }
        ));
    }

    #[test]
    fn piece_count_and_no_piece_in_mask() {
        let pos = Position::startpos();
        assert!(check_invariant(
            &pos,
            &InvariantKind::PieceCount {
                color: Color::White,
                piece_type: PieceType::Pawn,
                count: 8,
            }
        ));
        // No white knights on rank 4.
        let rank4 = crate::bitboard::rank_bb(crate::types::Rank::R4);
        assert!(check_invariant(
            &pos,
            &InvariantKind::NoPieceInMask {
                color: Color::White,
                piece_type: PieceType::Knight,
                mask: rank4,
            }
        ));
    }

    #[test]
    fn attacker_count_and_not_attacked_by() {
        // After 1.e4 e5 2.Nf3 f6: black's f6 pawn is the only
        // defender of e5, and h5 is currently not attacked by black.
        let mut pos = Position::startpos();
        for san_text in ["e4", "e5", "Nf3", "f6"] {
            let mv = san::parse(&mut pos, san_text).unwrap();
            let _ = pos.do_move(mv);
        }
        assert!(check_invariant(
            &pos,
            &InvariantKind::AttackerCountByColor {
                color: Color::Black,
                square: Square::E5,
                count: 1,
            }
        ));
        assert!(check_invariant(
            &pos,
            &InvariantKind::NotAttackedBy {
                color: Color::Black,
                square: Square::H5
            }
        ));
        assert!(check_invariant(
            &pos,
            &InvariantKind::AttackersEqual {
                color: Color::Black,
                square: Square::E5,
                mask: square_bb(Square::F6),
            }
        ));
    }

    #[test]
    fn ray_clear_sees_through_empty_squares() {
        // Startpos: a queen on d1 does NOT see h5 (path d1→e2 is
        // blocked by the white e-pawn).
        let pos = Position::startpos();
        assert!(!check_invariant(
            &pos,
            &InvariantKind::RayClear {
                from: Square::D1,
                to: Square::H5
            }
        ));

        // After 1.e4 e5 2.Nf3 f6, a queen on h5 WOULD see e5 along
        // rank 5 (f5 and g5 are empty), and would also see e8 along
        // the h5-e8 diagonal (g6 and f7 are empty since 2...f6
        // vacated f7).
        let mut pos = Position::startpos();
        for san_text in ["e4", "e5", "Nf3", "f6"] {
            let mv = san::parse(&mut pos, san_text).unwrap();
            let _ = pos.do_move(mv);
        }
        assert!(check_invariant(
            &pos,
            &InvariantKind::RayClear {
                from: Square::H5,
                to: Square::E5
            }
        ));
        assert!(check_invariant(
            &pos,
            &InvariantKind::RayClear {
                from: Square::H5,
                to: Square::E8
            }
        ));
    }

    #[test]
    fn ray_clear_rejects_non_aligned_squares() {
        // d1 and e3 are not on a shared rank / file / diagonal.
        let pos = Position::startpos();
        assert!(!check_invariant(
            &pos,
            &InvariantKind::RayClear {
                from: Square::D1,
                to: Square::E3
            }
        ));
    }

    // ---- TriggerPattern ---------------------------------------------

    #[test]
    fn trigger_pattern_matches_with_and_without_from() {
        let mut pos = Position::startpos();
        for san_text in ["e4", "e5", "Nf3"] {
            let mv = san::parse(&mut pos, san_text).unwrap();
            let _ = pos.do_move(mv);
        }
        // Build the Damiano trigger: black pawn to f6.
        let f6_move = san::parse(&mut pos.clone(), "f6").unwrap();
        let trigger_wildcard = TriggerPattern {
            mover: Color::Black,
            piece_type: PieceType::Pawn,
            to: Square::F6,
            from: None,
        };
        let trigger_strict = TriggerPattern {
            mover: Color::Black,
            piece_type: PieceType::Pawn,
            to: Square::F6,
            from: Some(Square::F7),
        };
        assert!(trigger_wildcard.matches(Color::Black, &pos, f6_move));
        assert!(trigger_strict.matches(Color::Black, &pos, f6_move));

        // Wrong side to move — should reject even if the move is
        // otherwise the right shape.
        assert!(!trigger_wildcard.matches(Color::White, &pos, f6_move));
    }

    // ---- Scan from the start position --------------------------------

    #[test]
    fn startpos_has_no_pre_move_threats() {
        // No trap in the library fires from the standard start position —
        // none of their triggers (e.g. Damiano's ...f6) are matched by
        // the side-to-move's legal set here.
        let pos = Position::startpos();
        assert!(scan_threats(&pos).is_empty());
    }

    #[test]
    fn scan_after_move_is_empty_when_no_trigger_matches() {
        // 1.Nc3 isn't any library trap's trigger, so nothing fires.
        let mut pos = Position::startpos();
        let mv = san::parse(&mut pos, "Nc3").unwrap();
        let after = {
            let mut p = pos.clone();
            let _ = p.do_move(mv);
            p
        };
        assert!(scan_after_move(
            &after,
            Color::White,
            PieceType::Knight,
            Square::B1,
            Square::C3
        )
        .is_empty());
    }
}
