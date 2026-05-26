//! Piece-placement card builders (one card per sub-signal x side).
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::PiecesPositionalOutcome;
use chess_tutor_engine::eval::PiecesBreakdown;
use chess_tutor_engine::types::Color;

use crate::view::{
    RetrospectiveCategory,
    RetrospectiveItem, Sentiment,
};


// ---------------------------------------------------------------------
// Piece placement — one card per sub-signal × side
// ---------------------------------------------------------------------

const PIECES_DELTA_THRESHOLD_CP: i32 = 20;

/// One per `PiecesBreakdown` sub-term — each describes a distinct
/// chess concept (outpost claimed, rook on open file, bishop blocked
/// by own pawns, etc.) and gets its own card. Mirrors the narration
/// crate's `PieceSubTerm` (core/ui doesn't depend on narration).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum PieceSubTerm {
    Outposts,
    ReachableOutposts,
    MinorBehindPawn,
    KingProtector,
    BishopPawns,
    LongDiagonalBishop,
    RookOnQueenFile,
    RookOnOpenFile,
    RookOnSemiopenFile,
    TrappedRook,
    WeakQueen,
}

impl PieceSubTerm {
    const ALL: [PieceSubTerm; 11] = [
        PieceSubTerm::Outposts,
        PieceSubTerm::ReachableOutposts,
        PieceSubTerm::MinorBehindPawn,
        PieceSubTerm::KingProtector,
        PieceSubTerm::BishopPawns,
        PieceSubTerm::LongDiagonalBishop,
        PieceSubTerm::RookOnQueenFile,
        PieceSubTerm::RookOnOpenFile,
        PieceSubTerm::RookOnSemiopenFile,
        PieceSubTerm::TrappedRook,
        PieceSubTerm::WeakQueen,
    ];

    fn delta_mg(self, pre: &PiecesBreakdown, post: &PiecesBreakdown) -> i32 {
        match self {
            PieceSubTerm::Outposts => post.outposts.mg().0 - pre.outposts.mg().0,
            PieceSubTerm::ReachableOutposts => {
                post.reachable_outposts.mg().0 - pre.reachable_outposts.mg().0
            }
            PieceSubTerm::MinorBehindPawn => {
                post.minor_behind_pawn.mg().0 - pre.minor_behind_pawn.mg().0
            }
            PieceSubTerm::KingProtector => post.king_protector.mg().0 - pre.king_protector.mg().0,
            PieceSubTerm::BishopPawns => post.bishop_pawns.mg().0 - pre.bishop_pawns.mg().0,
            PieceSubTerm::LongDiagonalBishop => {
                post.long_diagonal_bishop.mg().0 - pre.long_diagonal_bishop.mg().0
            }
            PieceSubTerm::RookOnQueenFile => {
                post.rook_on_queen_file.mg().0 - pre.rook_on_queen_file.mg().0
            }
            PieceSubTerm::RookOnOpenFile => {
                post.rook_on_open_file.mg().0 - pre.rook_on_open_file.mg().0
            }
            PieceSubTerm::RookOnSemiopenFile => {
                post.rook_on_semiopen_file.mg().0 - pre.rook_on_semiopen_file.mg().0
            }
            PieceSubTerm::TrappedRook => post.trapped_rook.mg().0 - pre.trapped_rook.mg().0,
            PieceSubTerm::WeakQueen => post.weak_queen.mg().0 - pre.weak_queen.mg().0,
        }
    }

    /// Heading when our side's sub-term improved (good for the user).
    fn ours_improved_heading(self) -> &'static str {
        match self {
            PieceSubTerm::Outposts => "Your knight reached an outpost",
            PieceSubTerm::ReachableOutposts => "Your knight has a route to an outpost",
            PieceSubTerm::MinorBehindPawn => "Your minor gained pawn cover",
            PieceSubTerm::KingProtector => "Your minor rallied to defend the king",
            PieceSubTerm::BishopPawns => "Your bishop freed itself from its pawn chain",
            PieceSubTerm::LongDiagonalBishop => "Your bishop took the long diagonal",
            PieceSubTerm::RookOnQueenFile => "Your rook reached the queen's file",
            PieceSubTerm::RookOnOpenFile => "Your rook took the open file",
            PieceSubTerm::RookOnSemiopenFile => "Your rook took a semi-open file",
            PieceSubTerm::TrappedRook => "Your rook escaped its trap",
            PieceSubTerm::WeakQueen => "Your queen shook off pressure",
        }
    }

    /// Heading when our side's sub-term worsened (bad for the user).
    fn ours_worsened_heading(self) -> &'static str {
        match self {
            PieceSubTerm::Outposts => "Your knight lost its outpost",
            PieceSubTerm::ReachableOutposts => "Your knight's outpost route closed",
            PieceSubTerm::MinorBehindPawn => "Your minor lost its pawn cover",
            PieceSubTerm::KingProtector => "Your minor drifted away from the king",
            PieceSubTerm::BishopPawns => "Your bishop is blocked by its own pawns",
            PieceSubTerm::LongDiagonalBishop => "Your bishop left the long diagonal",
            PieceSubTerm::RookOnQueenFile => "Your rook left the queen's file",
            PieceSubTerm::RookOnOpenFile => "Your rook left the open file",
            PieceSubTerm::RookOnSemiopenFile => "Your rook left a semi-open file",
            PieceSubTerm::TrappedRook => "Your rook got trapped",
            PieceSubTerm::WeakQueen => "Your queen is under x-ray pressure",
        }
    }

    /// Heading when their side's sub-term improved (bad for the user).
    fn theirs_improved_heading(self) -> &'static str {
        match self {
            PieceSubTerm::Outposts => "Opponent's knight reached an outpost",
            PieceSubTerm::ReachableOutposts => "Opponent's knight has a route to an outpost",
            PieceSubTerm::MinorBehindPawn => "Opponent's minor gained pawn cover",
            PieceSubTerm::KingProtector => "Opponent's minor rallied to defend their king",
            PieceSubTerm::BishopPawns => "Opponent's bishop freed itself from its pawn chain",
            PieceSubTerm::LongDiagonalBishop => "Opponent's bishop took the long diagonal",
            PieceSubTerm::RookOnQueenFile => "Opponent's rook reached your queen's file",
            PieceSubTerm::RookOnOpenFile => "Opponent's rook took the open file",
            PieceSubTerm::RookOnSemiopenFile => "Opponent's rook took a semi-open file",
            PieceSubTerm::TrappedRook => "Opponent's rook escaped its trap",
            PieceSubTerm::WeakQueen => "Opponent's queen shook off pressure",
        }
    }

    /// Heading when their side's sub-term worsened (good for the user).
    fn theirs_worsened_heading(self) -> &'static str {
        match self {
            PieceSubTerm::Outposts => "You denied the opponent's knight an outpost",
            PieceSubTerm::ReachableOutposts => "You closed the opponent's outpost route",
            PieceSubTerm::MinorBehindPawn => "You stripped the opponent's pawn cover",
            PieceSubTerm::KingProtector => "Opponent's minor drifted from their king",
            PieceSubTerm::BishopPawns => "Opponent's bishop is blocked by their own pawns",
            PieceSubTerm::LongDiagonalBishop => "Opponent's bishop left the long diagonal",
            PieceSubTerm::RookOnQueenFile => "Opponent's rook left your queen's file",
            PieceSubTerm::RookOnOpenFile => "Opponent's rook left the open file",
            PieceSubTerm::RookOnSemiopenFile => "Opponent's rook left a semi-open file",
            PieceSubTerm::TrappedRook => "You trapped the opponent's rook",
            PieceSubTerm::WeakQueen => "You put the opponent's queen under x-ray pressure",
        }
    }

    /// Short prose explaining what this sub-term measures. Renders in
    /// the card's expand-on-click detail.
    fn detail(self) -> &'static str {
        match self {
            PieceSubTerm::Outposts => {
                "An outpost is a square defended by your own pawn that the opponent's \
                 pawns can't kick away. Knights and bishops are powerful on outposts \
                 because no minor piece can dislodge them with a single move."
            }
            PieceSubTerm::ReachableOutposts => {
                "Your knight is one move away from an outpost — a square defended by \
                 your pawn that the opponent's pawns can't reach. Outposts are \
                 strongest with a knight on them; this card means the route is open."
            }
            PieceSubTerm::MinorBehindPawn => {
                "A minor piece directly behind one of your pawns is shielded from \
                 captures along its file and tends to support pawn pushes."
            }
            PieceSubTerm::KingProtector => {
                "Minor pieces lose a small bonus the further they sit from your own \
                 king. Knights and bishops near home help shield the king from attacks."
            }
            PieceSubTerm::BishopPawns => {
                "A bishop is penalised for each friendly pawn sitting on its color — \
                 those pawns block the bishop's diagonals. Either trade the bishop or \
                 push the pawns off its color."
            }
            PieceSubTerm::LongDiagonalBishop => {
                "A bishop attacking both central squares (e4/e5 or d4/d5) along its \
                 long diagonal exerts pressure on the center from a single piece."
            }
            PieceSubTerm::RookOnQueenFile => {
                "A rook on the same file as the enemy queen exerts latent pressure \
                 even with pawns in the way — when the file opens it becomes a tactic."
            }
            PieceSubTerm::RookOnOpenFile => {
                "A rook on a file with no pawns of either color controls the entire \
                 file. Open files are the rook's natural element."
            }
            PieceSubTerm::RookOnSemiopenFile => {
                "A rook on a file with no friendly pawns but enemy pawns can pressure \
                 those pawns directly — useful for attacking weak pawns."
            }
            PieceSubTerm::TrappedRook => {
                "A rook stuck behind its own king after castling rights are gone has \
                 almost no mobility. It blocks the king and contributes nothing."
            }
            PieceSubTerm::WeakQueen => {
                "The queen sees a slider x-ray threat against it — a rook or bishop \
                 aimed through one intervening piece. A discovered attack can win the \
                 queen unless you defuse it."
            }
        }
    }
}

/// Skip `BishopPawns` narration when bishop geometry didn't change on
/// `side`. Without this filter, a central pawn push (1.e4 e5) that
/// merely doubles the blocked-centre multiplier would fire phantom
/// "bishop blocked by own pawns" cards on both sides — none of which
/// describe anything a 1200-ELO student can act on. Mirrors the
/// narration crate's `include_bishop_pawns`.
fn include_sub_term(st: PieceSubTerm, bishop_geometry_changed: bool) -> bool {
    st != PieceSubTerm::BishopPawns || bishop_geometry_changed
}

/// Capture-aware suppression flags for the king-protector
/// sub-term — built from the realized capture events so the
/// per-sub-term loop can drop arithmetic-noise variants of the
/// card.
#[derive(Copy, Clone, Debug, Default)]
pub(super) struct KingProtectorSuppression {
    /// `true` when at least one of our minors was captured at ply
    /// ≤ 1. Their average king-distance "improves" purely because a
    /// minor came off the board — no actual repositioning happened.
    /// Drop the ours-side KP card (in either direction).
    pub(super) ours_minor_captured: bool,
    /// Same logic for the opponent's side.
    pub(super) theirs_minor_captured: bool,
    /// `true` when *our* ply-0 move was a capture made *by* a minor.
    /// The minor's "drift away from the king" is what enabled the
    /// capture; framing it as a cost mis-teaches. Drops the
    /// `ours_worsened` direction only — improvements (a minor
    /// rallying back to the king) still surface normally.
    pub(super) our_minor_capturing: bool,
}

pub(super) fn king_protector_suppression(
    material: &chess_tutor_engine::analysis::MaterialOutcome,
    root_stm: Color,
) -> KingProtectorSuppression {
    let mut out = KingProtectorSuppression::default();
    for ev in material.realized_events() {
        if ev.captured_piece.is_minor() {
            if ev.captor == root_stm {
                // We captured one of their minors.
                out.theirs_minor_captured = true;
            } else {
                // They captured one of ours.
                out.ours_minor_captured = true;
            }
        }
        if ev.ply == 0 && ev.captor == root_stm && ev.captor_piece.is_minor() {
            out.our_minor_capturing = true;
        }
    }
    out
}

pub(super) fn build_pieces_positional_items(
    outcome: &PiecesPositionalOutcome,
    _root_stm: Color,
    kp_supp: KingProtectorSuppression,
) -> Vec<RetrospectiveItem> {
    let mut items = Vec::new();

    for st in PieceSubTerm::ALL {
        // Our side.
        if include_sub_term(st, outcome.ours_bishop_pawn_count_changed()) {
            let delta = st.delta_mg(&outcome.ours_pre, &outcome.ours_post);
            if delta.abs() >= PIECES_DELTA_THRESHOLD_CP {
                let (heading, sentiment) = if delta > 0 {
                    (st.ours_improved_heading(), Sentiment::Positive)
                } else {
                    (st.ours_worsened_heading(), Sentiment::Negative)
                };
                // King-protector capture-aware suppression. Two cases
                // for our side:
                //   - If one of our minors was just captured, the
                //     remaining minors' average king-distance
                //     "improved" purely by arithmetic — not a
                //     defensive move. Drop in either direction.
                //   - If our ply-0 capture was BY a minor, the "drift
                //     away" variant misleadingly frames the
                //     capture-enabling move as a cost. Drop only the
                //     worsened direction; if the captor ends up
                //     closer to our king (the engine likes the
                //     square defensively), keep the improved card.
                if st == PieceSubTerm::KingProtector {
                    if kp_supp.ours_minor_captured {
                        continue;
                    }
                    if kp_supp.our_minor_capturing && delta < 0 {
                        continue;
                    }
                }
                items.push(RetrospectiveItem {
                    category: RetrospectiveCategory::PiecePlacement,
                    heading: heading.to_string(),
                    summary: format!("{:+.2} pawns", delta as f32 / 100.0),
                    detail: st.detail().to_string(),
                    // User-POV delta: our improvement is positive for
                    // us; our worsening is negative.
                    score_delta_pawns: Some(delta as f32 / 100.0),
                    sentiment,
                    annotations: Vec::new(),
                });
            }
        }

        // Their side.
        if include_sub_term(st, outcome.theirs_bishop_pawn_count_changed()) {
            let delta = st.delta_mg(&outcome.theirs_pre, &outcome.theirs_post);
            if delta.abs() >= PIECES_DELTA_THRESHOLD_CP {
                let (heading, sentiment) = if delta > 0 {
                    (st.theirs_improved_heading(), Sentiment::Negative)
                } else {
                    (st.theirs_worsened_heading(), Sentiment::Positive)
                };
                // Same KP capture-aware suppression for their side:
                // if a minor of theirs came off the board, the
                // delta is arithmetic — not their pieces "rallying."
                if st == PieceSubTerm::KingProtector && kp_supp.theirs_minor_captured {
                    continue;
                }
                items.push(RetrospectiveItem {
                    category: RetrospectiveCategory::PiecePlacement,
                    heading: heading.to_string(),
                    summary: format!("{:+.2} pawns", delta as f32 / 100.0),
                    detail: st.detail().to_string(),
                    // User-POV delta: their improvement hurts us; their
                    // worsening helps us — both flip sign vs. raw delta.
                    score_delta_pawns: Some(-delta as f32 / 100.0),
                    sentiment,
                    annotations: Vec::new(),
                });
            }
        }
    }

    items
}

