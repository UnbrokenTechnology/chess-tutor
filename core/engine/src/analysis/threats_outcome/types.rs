//! Structured threat-snapshot types for [`ThreatsOutcome`].

use crate::types::{PieceType, Square};

/// One square + the piece on it. Colour is implicit from the
/// containing context (which list/field the location appears in).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PieceLocation {
    pub square: Square,
    pub piece: PieceType,
}

/// A hanging piece plus every enemy piece attacking it. The hanging
/// piece's colour is implicit from which list on [`ThreatsOutcome`]
/// contains this entry (`ours_hanging` vs `theirs_hanging`); the
/// attackers are always the opposite colour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HangingPiece {
    pub location: PieceLocation,
    /// Enemy pieces attacking `location.square`. Non-empty by
    /// construction — a piece with zero enemy attackers wouldn't be
    /// on the hanging list. Ordered by ascending square index so
    /// renderers produce deterministic output.
    pub attackers: Vec<PieceLocation>,
}

/// A piece under "pressure": attacked in a way that would force it
/// to move or concede material, but *not* already on the hanging
/// or SEE-losing lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PressuredPiece {
    pub location: PieceLocation,
    /// Enemy pieces applying this specific pressure kind. Ordered
    /// by ascending square index for deterministic output.
    pub attackers: Vec<PieceLocation>,
    pub kind: PressureKind,
}

/// Which Stockfish-evaluator threat pattern this pressure entry
/// represents.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PressureKind {
    /// A knight or bishop attacks an enemy rook or queen.
    MinorOnMajor,
    /// A rook attacks the enemy queen.
    RookOnQueen,
    /// A pawn on a safe square attacks an enemy non-pawn piece.
    SafePawnThreat,
}

/// Structured snapshot of threatened pieces in the position
/// immediately after the user's move, compared against the pre-move
/// baseline.
///
/// `*_delta` counts compare against the same measure at the
/// pre-move position, so callers can answer "did this move
/// *create* a threat on our side, or *resolve* one?"
///
/// POV convention: `ours_*` fields refer to the user's side
/// (`root_stm`); `theirs_*` fields refer to the opponent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreatsOutcome {
    /// Our pieces after the user's move that are attacked and
    /// undefended.
    pub ours_hanging: Vec<HangingPiece>,
    /// Their pieces after the user's move that are attacked and
    /// undefended. **Raw / static snapshot** — does not check
    /// whether the opponent's next move can refute the threat. Use
    /// [`theirs_hanging_guaranteed`](Self::theirs_hanging_guaranteed)
    /// for teaching surfaces.
    pub theirs_hanging: Vec<HangingPiece>,
    /// Our pieces after the user's move that are defended but still
    /// lose material in an SEE-assessed exchange initiated by the
    /// enemy.
    pub ours_see_losing: Vec<HangingPiece>,
    /// Their pieces where the same SEE assessment favours us. Raw /
    /// static, like `theirs_hanging`. Use
    /// [`theirs_see_losing_guaranteed`](Self::theirs_see_losing_guaranteed)
    /// for teaching surfaces.
    pub theirs_see_losing: Vec<HangingPiece>,
    /// Subset of [`theirs_hanging`](Self::theirs_hanging) that
    /// survives *every* legal opponent response — the target piece
    /// stays on its square AND our cheapest attacker remains
    /// SEE-positive after every reply. This is the honest "you can
    /// win material" surface: phrasing the static list as a winnable
    /// claim mis-teaches when the opponent's reply (defend, move the
    /// target, capture an attacker, or pose a bigger counter-threat)
    /// refutes the win.
    pub theirs_hanging_guaranteed: Vec<HangingPiece>,
    /// Subset of [`theirs_see_losing`](Self::theirs_see_losing) that
    /// survives every legal opponent response, by the same logic as
    /// `theirs_hanging_guaranteed`.
    pub theirs_see_losing_guaranteed: Vec<HangingPiece>,
    /// Our pieces under Stockfish-style positional pressure.
    pub ours_pressured: Vec<PressuredPiece>,
    /// Their pieces under the same form of positional pressure from
    /// our side.
    pub theirs_pressured: Vec<PressuredPiece>,
    /// `ours_hanging.len() − (count at pre-move)`. Positive means
    /// this move *created* a hanging piece on our side.
    pub ours_hanging_delta: i32,
    pub theirs_hanging_delta: i32,
    pub ours_see_losing_delta: i32,
    pub theirs_see_losing_delta: i32,
    pub ours_pressured_delta: i32,
    pub theirs_pressured_delta: i32,
}
