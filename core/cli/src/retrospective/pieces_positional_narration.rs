//! Piece-placement narration — per-sub-term worsening / improving
//! per side, drawing on the 11-sub-term [`PiecesBreakdown`].

use std::io::{self, Write};

use chess_tutor_engine::analysis::PiecesPositionalOutcome;
use chess_tutor_engine::eval::PiecesBreakdown;

/// Engine-cp threshold per piece-positional sub-term for narrating
/// a shift. ~0.15 of a pawn — each individual positional term is
/// typically 20-40 cp when it fires (knight outpost ~30, rook on
/// open file ~45), so 15 catches one piece moving in or out of the
/// pattern while skipping 1-2 cp tapered-rescoring wobble.
const PIECES_POSITIONAL_DELTA_THRESHOLD_CP: i32 = 15;

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

    fn worsened_phrase(self) -> &'static str {
        match self {
            PieceSubTerm::Outposts => "a minor lost its outpost",
            PieceSubTerm::ReachableOutposts => "an outpost route closed",
            PieceSubTerm::MinorBehindPawn => "a minor stepped out from behind its pawn",
            PieceSubTerm::KingProtector => "a minor drifted away from the king",
            PieceSubTerm::BishopPawns => "a bishop got stuck behind its pawn chain",
            PieceSubTerm::LongDiagonalBishop => "a bishop left the long diagonal",
            PieceSubTerm::RookOnQueenFile => "a rook left the queen's file",
            PieceSubTerm::RookOnOpenFile => "a rook left the open file",
            PieceSubTerm::RookOnSemiopenFile => "a rook left a semi-open file",
            PieceSubTerm::TrappedRook => "a rook got trapped",
            PieceSubTerm::WeakQueen => "the queen came under minor-piece pressure",
        }
    }

    fn improved_phrase(self) -> &'static str {
        match self {
            PieceSubTerm::Outposts => "a minor claimed an outpost",
            PieceSubTerm::ReachableOutposts => "an outpost route opened",
            PieceSubTerm::MinorBehindPawn => "a minor tucked behind its pawn",
            PieceSubTerm::KingProtector => "a minor rallied to the king",
            PieceSubTerm::BishopPawns => "a bishop freed itself from its pawn chain",
            PieceSubTerm::LongDiagonalBishop => "a bishop claimed the long diagonal",
            PieceSubTerm::RookOnQueenFile => "a rook reached the queen's file",
            PieceSubTerm::RookOnOpenFile => "a rook claimed the open file",
            PieceSubTerm::RookOnSemiopenFile => "a rook claimed a semi-open file",
            PieceSubTerm::TrappedRook => "a rook escaped its trap",
            PieceSubTerm::WeakQueen => "the queen shook off minor-piece pressure",
        }
    }
}

/// When false, skip `BishopPawns` narration on this side because its
/// Score delta is driven purely by the blocked-centre multiplier
/// (e.g., 1.e4 e5 creating locked central pawns) rather than by any
/// bishop physically relocating or any pawn on a bishop's colour
/// being captured / promoted / pushed.
fn include_bishop_pawns(st: PieceSubTerm, bishop_geometry_changed: bool) -> bool {
    st != PieceSubTerm::BishopPawns || bishop_geometry_changed
}

fn worsened_pieces_categories(
    pre: &PiecesBreakdown,
    post: &PiecesBreakdown,
    bishop_geometry_changed: bool,
) -> Vec<&'static str> {
    PieceSubTerm::ALL
        .iter()
        .filter(|st| include_bishop_pawns(**st, bishop_geometry_changed))
        .filter(|st| st.delta_mg(pre, post) <= -PIECES_POSITIONAL_DELTA_THRESHOLD_CP)
        .map(|st| st.worsened_phrase())
        .collect()
}

fn improved_pieces_categories(
    pre: &PiecesBreakdown,
    post: &PiecesBreakdown,
    bishop_geometry_changed: bool,
) -> Vec<&'static str> {
    PieceSubTerm::ALL
        .iter()
        .filter(|st| include_bishop_pawns(**st, bishop_geometry_changed))
        .filter(|st| st.delta_mg(pre, post) >= PIECES_POSITIONAL_DELTA_THRESHOLD_CP)
        .map(|st| st.improved_phrase())
        .collect()
}

fn our_pieces_worsened_line(o: &PiecesPositionalOutcome) -> Option<String> {
    let clauses = worsened_pieces_categories(
        &o.ours_pre,
        &o.ours_post,
        o.ours_bishop_pawn_count_changed(),
    );
    if clauses.is_empty() {
        return None;
    }
    Some(format!(
        "Your piece placement weakened: {}.",
        clauses.join(", ")
    ))
}

fn our_pieces_improved_line(o: &PiecesPositionalOutcome) -> Option<String> {
    let clauses = improved_pieces_categories(
        &o.ours_pre,
        &o.ours_post,
        o.ours_bishop_pawn_count_changed(),
    );
    if clauses.is_empty() {
        return None;
    }
    Some(format!(
        "Your piece placement improved: {}.",
        clauses.join(", ")
    ))
}

fn their_pieces_worsened_line(o: &PiecesPositionalOutcome) -> Option<String> {
    let clauses = worsened_pieces_categories(
        &o.theirs_pre,
        &o.theirs_post,
        o.theirs_bishop_pawn_count_changed(),
    );
    if clauses.is_empty() {
        return None;
    }
    Some(format!(
        "You weakened the opponent's piece placement: {}.",
        clauses.join(", "),
    ))
}

fn their_pieces_improved_line(o: &PiecesPositionalOutcome) -> Option<String> {
    let clauses = improved_pieces_categories(
        &o.theirs_pre,
        &o.theirs_post,
        o.theirs_bishop_pawn_count_changed(),
    );
    if clauses.is_empty() {
        return None;
    }
    Some(format!(
        "The opponent's piece placement improved: {}.",
        clauses.join(", "),
    ))
}

pub(super) fn render_pieces_positional(
    out: &mut io::StdoutLock<'_>,
    outcome: &PiecesPositionalOutcome,
) -> io::Result<bool> {
    let mut wrote = false;

    let ours = our_pieces_worsened_line(outcome).or_else(|| our_pieces_improved_line(outcome));
    if let Some(line) = ours {
        writeln!(out, "                {line}")?;
        wrote = true;
    }

    let theirs =
        their_pieces_worsened_line(outcome).or_else(|| their_pieces_improved_line(outcome));
    if let Some(line) = theirs {
        writeln!(out, "                {line}")?;
        wrote = true;
    }

    Ok(wrote)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::types::Score;

    fn pib_zero() -> PiecesBreakdown {
        PiecesBreakdown {
            outposts: Score::ZERO,
            reachable_outposts: Score::ZERO,
            minor_behind_pawn: Score::ZERO,
            king_protector: Score::ZERO,
            bishop_pawns: Score::ZERO,
            long_diagonal_bishop: Score::ZERO,
            rook_on_queen_file: Score::ZERO,
            rook_on_open_file: Score::ZERO,
            rook_on_semiopen_file: Score::ZERO,
            trapped_rook: Score::ZERO,
            weak_queen: Score::ZERO,
        }
    }

    fn pieces_outcome(
        ours_pre: PiecesBreakdown,
        ours_post: PiecesBreakdown,
        theirs_pre: PiecesBreakdown,
        theirs_post: PiecesBreakdown,
    ) -> PiecesPositionalOutcome {
        // Default: both sides' bishop geometry counted as changed, so
        // tests that aren't specifically about the suppression path
        // see every sub-term fire uniformly.
        pieces_outcome_with_geometry(ours_pre, ours_post, theirs_pre, theirs_post, true, true)
    }

    fn pieces_outcome_with_geometry(
        ours_pre: PiecesBreakdown,
        ours_post: PiecesBreakdown,
        theirs_pre: PiecesBreakdown,
        theirs_post: PiecesBreakdown,
        ours_geometry_changed: bool,
        theirs_geometry_changed: bool,
    ) -> PiecesPositionalOutcome {
        PiecesPositionalOutcome {
            ours_pre,
            ours_post,
            theirs_pre,
            theirs_post,
            ours_bishop_pawn_count_pre: 0,
            ours_bishop_pawn_count_post: u32::from(ours_geometry_changed),
            theirs_bishop_pawn_count_pre: 0,
            theirs_bishop_pawn_count_post: u32::from(theirs_geometry_changed),
        }
    }

    #[test]
    fn our_pieces_improved_line_fires_when_outpost_claimed() {
        let mut post = pib_zero();
        post.outposts = Score::new(30, 0);
        let out = pieces_outcome(pib_zero(), post, pib_zero(), pib_zero());
        assert_eq!(
            our_pieces_improved_line(&out),
            Some("Your piece placement improved: a minor claimed an outpost.".to_string())
        );
    }

    #[test]
    fn our_pieces_worsened_line_fires_when_rook_leaves_open_file() {
        let mut pre = pib_zero();
        pre.rook_on_open_file = Score::new(45, 0);
        let out = pieces_outcome(pre, pib_zero(), pib_zero(), pib_zero());
        let line = our_pieces_worsened_line(&out).expect("should fire");
        assert!(line.contains("a rook left the open file"));
    }

    #[test]
    fn pieces_delta_below_threshold_does_not_fire() {
        let mut post = pib_zero();
        post.outposts = Score::new(10, 0);
        let out = pieces_outcome(pib_zero(), post, pib_zero(), pib_zero());
        assert_eq!(our_pieces_improved_line(&out), None);
        assert_eq!(our_pieces_worsened_line(&out), None);
    }

    #[test]
    fn their_pieces_worsened_line_uses_active_voice() {
        let mut their_pre = pib_zero();
        their_pre.rook_on_open_file = Score::new(45, 0);
        let out = pieces_outcome(pib_zero(), pib_zero(), their_pre, pib_zero());
        let line = their_pieces_worsened_line(&out).expect("should fire");
        assert!(line.starts_with("You weakened the opponent's piece placement:"));
    }

    #[test]
    fn pieces_worsened_line_combines_multiple_clauses() {
        let mut pre = pib_zero();
        pre.outposts = Score::new(30, 0);
        pre.bishop_pawns = Score::new(0, 0);
        let mut post = pib_zero();
        post.outposts = Score::new(0, 0);
        post.bishop_pawns = Score::new(-20, 0);
        let out = pieces_outcome(pre, post, pib_zero(), pib_zero());
        let line = our_pieces_worsened_line(&out).expect("should fire");
        assert!(line.contains("a minor lost its outpost"));
        assert!(line.contains("a bishop got stuck behind its pawn chain"));
    }

    #[test]
    fn bishop_pawns_suppressed_when_geometry_unchanged() {
        // Models 1.e4 e5: the central pawn push doubles the
        // blocked-centre multiplier on both sides, so `bishop_pawns`
        // mg jumps negatively (~−24 cp) without any bishop moving or
        // any pawn on a bishop's colour being captured / promoted.
        // With `ours_geometry_changed = false`, narration for the
        // BishopPawns sub-term must stay silent — firing "a bishop
        // got stuck behind its pawn chain" here is the exact false
        // positive a 1200-ELO student flagged as nonsense.
        let mut pre = pib_zero();
        pre.bishop_pawns = Score::new(-24, -64);
        let mut post = pib_zero();
        post.bishop_pawns = Score::new(-48, -128);
        let out = pieces_outcome_with_geometry(
            pre,
            post,
            pib_zero(),
            pib_zero(),
            false,
            false,
        );
        assert_eq!(our_pieces_worsened_line(&out), None);
        assert_eq!(their_pieces_worsened_line(&out), None);
    }

    #[test]
    fn bishop_pawns_still_fires_when_geometry_changes() {
        // Same Score shift as above, but now `ours_geometry_changed =
        // true` — e.g., a real pawn capture that removed a pawn on a
        // bishop's colour. The narrator still fires.
        let mut pre = pib_zero();
        pre.bishop_pawns = Score::new(-24, -64);
        let mut post = pib_zero();
        post.bishop_pawns = Score::new(-48, -128);
        let out =
            pieces_outcome_with_geometry(pre, post, pib_zero(), pib_zero(), true, false);
        let line = our_pieces_worsened_line(&out).expect("should fire");
        assert!(line.contains("a bishop got stuck behind its pawn chain"));
    }
}
