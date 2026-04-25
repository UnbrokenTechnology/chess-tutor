//! King-safety narration — exposure and shelter shifts per side,
//! with flank-aware phrasing and endgame shelter suppression.

use std::io::{self, Write};

use chess_tutor_engine::analysis::KingSafetyOutcome;
use chess_tutor_engine::types::Square;

use super::util::format_shelter_pawns;

/// Engine-cp threshold for narrating a shelter shift. ~0.25 of a
/// pawn — small enough to catch a single pawn-shield break but
/// large enough that opening-phase noise (a tempo move that nudges
/// shelter by 5-10 cp) doesn't trigger a line every move.
const KING_SHELTER_DELTA_THRESHOLD_CP: i32 = 25;

/// Game-phase cutoff below which shelter narration is suppressed.
/// Phase is on the `[0, 128]` scale (128 = pure mg; 0 = pure eg);
/// at `< 32` we're deep into an endgame where pawn cover is no
/// longer the dominant king-safety concern, so pretending it is
/// would just add noise. Attackers-count narration still fires —
/// even kings in bare endgames care about being chased.
const KING_SHELTER_ENDGAME_PHASE_CUTOFF: i32 = 32;

/// True when the game phase is deep enough into an endgame that
/// shelter narration would be misleading — pawn cover matters far
/// less once queens/rooks have traded off.
fn shelter_narration_suppressed(o: &KingSafetyOutcome) -> bool {
    o.phase < KING_SHELTER_ENDGAME_PHASE_CUTOFF
}

/// Categorize a king's location as "kingside" (f-h files),
/// "queenside" (a-c files), or `None` when the king is on a
/// central file (d, e) where the flank concept doesn't cleanly
/// apply.
///
/// Mirrors Stockfish's `KING_FLANK[file]` partitioning: files 0..=2
/// map to the queenside flank, files 5..=7 to the kingside flank,
/// and files 3..=4 (d, e) land in the center where attackers don't
/// cluster along a single side. When the king is on a central
/// file, renderers fall back to the generic "king ring" phrasing.
fn flank_side_label(king_sq: Square) -> Option<&'static str> {
    match king_sq.file().index() {
        0..=2 => Some("queenside"),
        5..=7 => Some("kingside"),
        _ => None,
    }
}

/// Format the "N attackers ..." clause for an *exposure*
/// (worsening) line. When the king sits on an outside file the
/// clause names the flank; otherwise it falls back to the generic
/// "king ring."
fn attackers_clause_exposure(post_count: i32, pre_count: i32, king_sq: Square) -> String {
    let target = flank_side_label(king_sq).unwrap_or("king ring");
    format!("{post_count} attackers on the {target} (up from {pre_count})")
}

/// Format the "attackers down ..." clause for a *safer* (improving)
/// line. When the king sits on an outside file we prepend the
/// flank (e.g., "kingside attackers down to 1 (from 3)");
/// otherwise we just say "attackers down to 1 (from 3)" without
/// location.
fn attackers_clause_safer(post_count: i32, pre_count: i32, king_sq: Square) -> String {
    match flank_side_label(king_sq) {
        Some(side) => format!("{side} attackers down to {post_count} (from {pre_count})"),
        None => format!("attackers down to {post_count} (from {pre_count})"),
    }
}

/// Build a single-line summary describing how *our* king's safety
/// changed. Returns `None` when the change is too small to be worth
/// narrating.
fn our_king_exposure_line(o: &KingSafetyOutcome) -> Option<String> {
    let attackers_up = o.ours_attackers_delta() > 0;
    let shelter_down = !shelter_narration_suppressed(o)
        && o.ours_shelter_mg_delta() <= -KING_SHELTER_DELTA_THRESHOLD_CP;
    if !attackers_up && !shelter_down {
        return None;
    }
    let mut parts = Vec::new();
    if attackers_up {
        parts.push(attackers_clause_exposure(
            o.ours_post.attackers_count,
            o.ours_pre.attackers_count,
            o.ours_post.king_sq,
        ));
    }
    if shelter_down {
        parts.push(format!(
            "shelter weakened ({} → {})",
            format_shelter_pawns(o.ours_pre.shelter_mg),
            format_shelter_pawns(o.ours_post.shelter_mg),
        ));
    }
    Some(format!("Your king is more exposed: {}.", parts.join(", ")))
}

/// Mirror of [`our_king_exposure_line`] for the opponent's king.
fn their_king_exposure_line(o: &KingSafetyOutcome) -> Option<String> {
    let attackers_up = o.theirs_attackers_delta() > 0;
    let shelter_down = !shelter_narration_suppressed(o)
        && o.theirs_shelter_mg_delta() <= -KING_SHELTER_DELTA_THRESHOLD_CP;
    if !attackers_up && !shelter_down {
        return None;
    }
    let mut parts = Vec::new();
    if attackers_up {
        parts.push(attackers_clause_exposure(
            o.theirs_post.attackers_count,
            o.theirs_pre.attackers_count,
            o.theirs_post.king_sq,
        ));
    }
    if shelter_down {
        parts.push(format!(
            "shelter cracked ({} → {})",
            format_shelter_pawns(o.theirs_pre.shelter_mg),
            format_shelter_pawns(o.theirs_post.shelter_mg),
        ));
    }
    Some(format!(
        "You expose the opponent's king: {}.",
        parts.join(", ")
    ))
}

fn our_king_safer_line(o: &KingSafetyOutcome) -> Option<String> {
    let attackers_down = o.ours_attackers_delta() < 0;
    let shelter_up = !shelter_narration_suppressed(o)
        && o.ours_shelter_mg_delta() >= KING_SHELTER_DELTA_THRESHOLD_CP;
    if !attackers_down && !shelter_up {
        return None;
    }
    let mut parts = Vec::new();
    if attackers_down {
        parts.push(attackers_clause_safer(
            o.ours_post.attackers_count,
            o.ours_pre.attackers_count,
            o.ours_post.king_sq,
        ));
    }
    if shelter_up {
        parts.push(format!(
            "shelter strengthened ({} → {})",
            format_shelter_pawns(o.ours_pre.shelter_mg),
            format_shelter_pawns(o.ours_post.shelter_mg),
        ));
    }
    Some(format!("Your king is safer: {}.", parts.join(", ")))
}

fn their_king_safer_line(o: &KingSafetyOutcome) -> Option<String> {
    let attackers_down = o.theirs_attackers_delta() < 0;
    let shelter_up = !shelter_narration_suppressed(o)
        && o.theirs_shelter_mg_delta() >= KING_SHELTER_DELTA_THRESHOLD_CP;
    if !attackers_down && !shelter_up {
        return None;
    }
    let mut parts = Vec::new();
    if attackers_down {
        parts.push(attackers_clause_safer(
            o.theirs_post.attackers_count,
            o.theirs_pre.attackers_count,
            o.theirs_post.king_sq,
        ));
    }
    if shelter_up {
        parts.push(format!(
            "shelter strengthened ({} → {})",
            format_shelter_pawns(o.theirs_pre.shelter_mg),
            format_shelter_pawns(o.theirs_post.shelter_mg),
        ));
    }
    Some(format!(
        "The opponent's king is safer: {}.",
        parts.join(", ")
    ))
}

/// Render the king-safety lines (ours, theirs). Per side, exposure
/// (worsening) takes precedence over safer (improving) when both
/// fire — worsening is the more urgent teaching message.
pub(super) fn render_king_safety(
    out: &mut io::StdoutLock<'_>,
    outcome: &KingSafetyOutcome,
) -> io::Result<bool> {
    let mut wrote = false;

    let ours_line = our_king_exposure_line(outcome).or_else(|| our_king_safer_line(outcome));
    if let Some(line) = ours_line {
        writeln!(out, "                {line}")?;
        wrote = true;
    }

    let theirs_line = their_king_exposure_line(outcome).or_else(|| their_king_safer_line(outcome));
    if let Some(line) = theirs_line {
        writeln!(out, "                {line}")?;
        wrote = true;
    }

    Ok(wrote)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::analysis::KingSafetySnapshot;

    /// Helper: build a [`KingSafetyOutcome`] literal with all
    /// fields filled. Each side's pre/post snapshot comes from the
    /// supplied tuple — `(attackers, attacks, shelter_mg,
    /// shelter_eg)`. Both king squares default to central files
    /// (e1 / e8).
    fn ks_outcome(
        ours_pre: (i32, i32, i32, i32),
        ours_post: (i32, i32, i32, i32),
        theirs_pre: (i32, i32, i32, i32),
        theirs_post: (i32, i32, i32, i32),
    ) -> KingSafetyOutcome {
        ks_outcome_with_kings(
            (Square::E1, Square::E1),
            (Square::E8, Square::E8),
            ours_pre,
            ours_post,
            theirs_pre,
            theirs_post,
        )
    }

    fn ks_outcome_with_kings(
        ours_kings: (Square, Square),
        theirs_kings: (Square, Square),
        ours_pre: (i32, i32, i32, i32),
        ours_post: (i32, i32, i32, i32),
        theirs_pre: (i32, i32, i32, i32),
        theirs_post: (i32, i32, i32, i32),
    ) -> KingSafetyOutcome {
        let snap = |king_sq: Square, t: (i32, i32, i32, i32)| KingSafetySnapshot {
            king_sq,
            attackers_count: t.0,
            attacks_count: t.1,
            shelter_mg: t.2,
            shelter_eg: t.3,
        };
        KingSafetyOutcome {
            ours_pre: snap(ours_kings.0, ours_pre),
            ours_post: snap(ours_kings.1, ours_post),
            theirs_pre: snap(theirs_kings.0, theirs_pre),
            theirs_post: snap(theirs_kings.1, theirs_post),
            phase: 128,
        }
    }

    #[test]
    fn our_king_exposure_line_none_when_no_change() {
        let out = ks_outcome((1, 2, 80, 4), (1, 2, 80, 4), (0, 0, 80, 4), (0, 0, 80, 4));
        assert_eq!(our_king_exposure_line(&out), None);
        assert_eq!(their_king_exposure_line(&out), None);
    }

    #[test]
    fn our_king_exposure_line_attackers_only() {
        let out = ks_outcome((1, 2, 80, 4), (3, 4, 80, 4), (0, 0, 80, 4), (0, 0, 80, 4));
        let line = our_king_exposure_line(&out).expect("attackers up should fire");
        assert_eq!(
            line,
            "Your king is more exposed: 3 attackers on the king ring (up from 1)."
        );
    }

    #[test]
    fn our_king_exposure_line_shelter_only() {
        let out = ks_outcome((1, 2, 80, 4), (1, 2, 30, 4), (0, 0, 80, 4), (0, 0, 80, 4));
        let line = our_king_exposure_line(&out).expect("shelter drop should fire");
        assert_eq!(
            line,
            "Your king is more exposed: shelter weakened (+0.80 → +0.30)."
        );
    }

    #[test]
    fn our_king_exposure_line_shelter_below_threshold_does_not_fire() {
        let out = ks_outcome((1, 2, 80, 4), (1, 2, 60, 4), (0, 0, 80, 4), (0, 0, 80, 4));
        assert_eq!(our_king_exposure_line(&out), None);
    }

    #[test]
    fn our_king_exposure_line_combines_attackers_and_shelter() {
        let out = ks_outcome((1, 2, 80, 4), (3, 5, 30, 0), (0, 0, 80, 4), (0, 0, 80, 4));
        let line = our_king_exposure_line(&out).expect("both should fire");
        assert!(line.contains("3 attackers on the king ring (up from 1)"));
        assert!(line.contains("shelter weakened (+0.80 → +0.30)"));
    }

    #[test]
    fn their_king_exposure_line_uses_active_voice() {
        let out = ks_outcome((0, 0, 80, 4), (0, 0, 80, 4), (0, 0, 90, 4), (2, 3, 90, 4));
        let line = their_king_exposure_line(&out).expect("their exposure should fire");
        assert!(line.starts_with("You expose the opponent's king:"));
        assert!(line.contains("2 attackers on the king ring (up from 0)"));
    }

    #[test]
    fn their_king_exposure_line_shelter_uses_cracked_verb() {
        let out = ks_outcome((0, 0, 80, 4), (0, 0, 80, 4), (0, 0, 90, 4), (0, 0, 50, 4));
        let line = their_king_exposure_line(&out).expect("their shelter drop should fire");
        assert!(line.contains("shelter cracked (+0.90 → +0.50)"));
    }

    // ---- positive king-safety teaching ------------------------------

    #[test]
    fn our_king_safer_line_attackers_down() {
        let out = ks_outcome((3, 4, 80, 4), (1, 2, 80, 4), (0, 0, 80, 4), (0, 0, 80, 4));
        let line = our_king_safer_line(&out).expect("attackers down should fire");
        assert_eq!(line, "Your king is safer: attackers down to 1 (from 3).");
    }

    #[test]
    fn our_king_safer_line_shelter_up_from_castling() {
        let out = ks_outcome((0, 0, 30, 4), (0, 0, 80, 4), (0, 0, 80, 4), (0, 0, 80, 4));
        let line = our_king_safer_line(&out).expect("shelter up should fire");
        assert_eq!(
            line,
            "Your king is safer: shelter strengthened (+0.30 → +0.80)."
        );
    }

    #[test]
    fn our_king_safer_line_none_when_nothing_changed() {
        let out = ks_outcome((1, 2, 80, 4), (1, 2, 80, 4), (0, 0, 80, 4), (0, 0, 80, 4));
        assert_eq!(our_king_safer_line(&out), None);
    }

    #[test]
    fn their_king_safer_line_uses_third_person() {
        let out = ks_outcome((0, 0, 80, 4), (0, 0, 80, 4), (3, 4, 80, 4), (1, 2, 80, 4));
        let line = their_king_safer_line(&out).expect("their attackers down should fire");
        assert!(line.starts_with("The opponent's king is safer:"));
        assert!(line.contains("attackers down to 1 (from 3)"));
    }

    #[test]
    fn render_precedence_exposure_wins_over_safer_on_same_side() {
        let out = ks_outcome((1, 2, 30, 4), (3, 4, 80, 4), (0, 0, 80, 4), (0, 0, 80, 4));
        assert!(our_king_exposure_line(&out).is_some());
        assert!(our_king_safer_line(&out).is_some());
        let picked = our_king_exposure_line(&out).or_else(|| our_king_safer_line(&out));
        assert!(picked
            .as_deref()
            .unwrap()
            .starts_with("Your king is more exposed:"));
    }

    // ---- flank-side labeling -----------------------------------------

    #[test]
    fn flank_side_label_covers_all_file_zones() {
        assert_eq!(flank_side_label(Square::A1), Some("queenside"));
        assert_eq!(flank_side_label(Square::B1), Some("queenside"));
        assert_eq!(flank_side_label(Square::C1), Some("queenside"));
        assert_eq!(flank_side_label(Square::D1), None);
        assert_eq!(flank_side_label(Square::E1), None);
        assert_eq!(flank_side_label(Square::F1), Some("kingside"));
        assert_eq!(flank_side_label(Square::G1), Some("kingside"));
        assert_eq!(flank_side_label(Square::H1), Some("kingside"));
    }

    #[test]
    fn our_king_exposure_line_names_kingside_after_castling() {
        let out = ks_outcome_with_kings(
            (Square::E1, Square::G1),
            (Square::E8, Square::E8),
            (0, 0, 80, 4),
            (2, 3, 80, 4),
            (0, 0, 80, 4),
            (0, 0, 80, 4),
        );
        let line = our_king_exposure_line(&out).expect("attackers up should fire");
        assert!(line.contains("2 attackers on the kingside (up from 0)"));
    }

    #[test]
    fn their_king_exposure_line_names_queenside() {
        let out = ks_outcome_with_kings(
            (Square::E1, Square::E1),
            (Square::E8, Square::C8),
            (0, 0, 80, 4),
            (0, 0, 80, 4),
            (0, 0, 80, 4),
            (2, 3, 80, 4),
        );
        let line = their_king_exposure_line(&out).expect("their attackers up should fire");
        assert!(line.contains("2 attackers on the queenside (up from 0)"));
    }

    #[test]
    fn our_king_safer_line_uses_flank_prefix_when_attackers_drop() {
        let out = ks_outcome_with_kings(
            (Square::E1, Square::G1),
            (Square::E8, Square::E8),
            (3, 4, 80, 4),
            (1, 2, 80, 4),
            (0, 0, 80, 4),
            (0, 0, 80, 4),
        );
        let line = our_king_safer_line(&out).expect("attackers down should fire");
        assert!(line.contains("kingside attackers down to 1 (from 3)"));
    }

    #[test]
    fn central_king_falls_back_to_king_ring_wording() {
        let out = ks_outcome((1, 2, 80, 4), (3, 4, 80, 4), (0, 0, 80, 4), (0, 0, 80, 4));
        let line = our_king_exposure_line(&out).expect("attackers up should fire");
        assert!(line.contains("3 attackers on the king ring (up from 1)"));
    }

    // ---- endgame shelter suppression ---------------------------------

    #[test]
    fn shelter_clause_suppressed_in_endgame_phase() {
        let mut out = ks_outcome((1, 2, 80, 4), (1, 2, 20, 4), (0, 0, 80, 4), (0, 0, 80, 4));
        out.phase = 16;
        assert_eq!(our_king_exposure_line(&out), None);
    }

    #[test]
    fn attackers_clause_still_fires_in_endgame() {
        let mut out = ks_outcome((1, 2, 80, 4), (3, 4, 20, 4), (0, 0, 80, 4), (0, 0, 80, 4));
        out.phase = 16;
        let line = our_king_exposure_line(&out).expect("attackers up should still fire");
        assert!(line.contains("3 attackers on the king ring (up from 1)"));
        assert!(!line.contains("shelter"));
    }

    #[test]
    fn shelter_clause_fires_in_midgame() {
        let mut out = ks_outcome((1, 2, 80, 4), (1, 2, 20, 4), (0, 0, 80, 4), (0, 0, 80, 4));
        out.phase = 64;
        let line = our_king_exposure_line(&out).expect("shelter drop should fire in mg");
        assert!(line.contains("shelter weakened"));
    }

    #[test]
    fn safer_shelter_clause_also_suppressed_in_endgame() {
        let mut out = ks_outcome((1, 2, 20, 4), (1, 2, 80, 4), (0, 0, 80, 4), (0, 0, 80, 4));
        out.phase = 16;
        assert_eq!(our_king_safer_line(&out), None);
    }
}
