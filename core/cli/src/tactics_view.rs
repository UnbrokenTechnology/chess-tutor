//! `chess-tutor tactics <FEN>` — engine tactic-detector chain run for
//! both sides, plus the overloaded-defender scan. The agent's
//! "what's actually tactically going on in this position" entry point.
//!
//! Composition:
//!
//! - For the side to move we call
//!   [`find_best_tactic_in_position`] directly. The detector chain
//!   enumerates every legal move and picks the best `Confidence::High`
//!   hit (mate trumps material gain, then pattern severity breaks ties).
//! - For the side not to move we **null-move the position first** so the
//!   detectors see the right side-to-move. This is the "one-ply ahead"
//!   reading of the opponent's standing threats — "if granted a free
//!   tempo, what would they play?" When the actual side to move is in
//!   check, the null-move pivot is unsound (the in-check side can't
//!   legally pass) and we skip the opponent scan with an explanatory
//!   note.
//! - [`find_overloaded`] runs unconditionally for both sides — it's a
//!   pure structural scan over the board, independent of whose turn it
//!   is.
//!
//! `prior_move` flows into the side-to-move detector for the
//! recapture guard (a `HangingCapture` that's actually completing an
//! exchange shouldn't fire). It is intentionally **not** passed to the
//! null-pivot opponent scan: in the pivot frame, the "opponent's last
//! move" is the side-to-move's last move, which is whatever the caller
//! supplied — but the recapture guard semantics don't line up across
//! the null, so we pass `None` and document the limitation.

use chess_tutor_engine::analysis::{
    find_best_tactic_in_position, find_check_followups, find_latent_threats, find_overloaded,
    CheckFollowup, Confidence, LatentThreat, MatePattern, OverloadedPiece, PriorMove,
    ReplyFollowup, TacticHit, TacticPattern, TriggerShape,
};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::types::Color;
use serde::Serialize;

use crate::piece_fmt::{color_name, piece_label};

/// Full tactics report, one [`SideTactics`] per colour, ready for text
/// or JSON rendering. The optional blocks ([`Self::latent`],
/// [`Self::check_followups`]) populate only when the caller asks for
/// `--latent` / `--check-followups`.
#[derive(Debug, Clone, Serialize)]
pub struct TacticsView {
    pub white: SideTactics,
    pub black: SideTactics,
    /// `None` when `--latent` was not requested; `Some` with the
    /// standing-threat scan per defender side when it was.
    pub latent: Option<LatentView>,
    /// `None` when `--check-followups` was not requested. `Some` with
    /// the two-step forcing-line scan per mover side ("after my
    /// check, what tactic do I have").
    pub check_followups: Option<CheckFollowupsView>,
}

/// Per-mover-side block of multi-step forcing-line tactics.
#[derive(Debug, Clone, Serialize)]
pub struct CheckFollowupsView {
    pub for_white: CheckFollowupsSide,
    pub for_black: CheckFollowupsSide,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckFollowupsSide {
    pub mover_side: String,
    pub sequences: Vec<CheckFollowupView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckFollowupView {
    /// The check move in SAN — easier to read than UCI when grepping
    /// agent output ("Nd3+" vs "c5d3").
    pub check_san: String,
    pub check_uci: String,
    /// All of opponent's legal responses to the check, with the
    /// follow-up tactic (if any) that fires after each.
    pub replies: Vec<ReplyFollowupView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplyFollowupView {
    pub reply_san: String,
    pub reply_uci: String,
    /// `Some` when a Confidence::High or Confidence::Medium tactic
    /// fires for the check-giver on the post-reply position. `None`
    /// means this reply defuses the threat — pedagogically important
    /// because it names the *escape route*.
    pub followup: Option<TacticHitView>,
}

/// Standing-threat report, one [`LatentSide`] per defender colour.
/// Read "against_white" as "threats the *opponent of White* (= Black)
/// has pre-loaded against White."
#[derive(Debug, Clone, Serialize)]
pub struct LatentView {
    pub against_white: LatentSide,
    pub against_black: LatentSide,
}

#[derive(Debug, Clone, Serialize)]
pub struct LatentSide {
    /// The colour the listed threats are aimed at.
    pub defender_side: String,
    pub threats: Vec<LatentThreatView>,
}

/// Flattened view of [`LatentThreat`] with all squares / pieces
/// resolved to printable labels. `pattern` mirrors the engine enum
/// (`DiscoveredAttack` / `RemovingDefender` / `Pin` / `Skewer`).
#[derive(Debug, Clone, Serialize)]
pub struct LatentThreatView {
    pub pattern: String,
    /// The firing piece — the slider for slider patterns; the
    /// enemy-attacker-of-the-defender for RemovingDefender.
    pub discoverer: String,
    pub discoverer_square: String,
    /// Slider-ray blocker (for DA / Pin / Skewer). `None` for
    /// RemovingDefender — its "vehicle" is the defender, surfaced
    /// under [`Self::trigger`].
    pub vehicle: Option<String>,
    pub vehicle_square: Option<String>,
    pub target: String,
    pub target_square: String,
    /// Classical-points gain estimate (P=1 / N/B=3 / R=5 / Q=9). See
    /// [`LatentThreat::min_gain`].
    pub min_gain: i32,
    pub confidence: String,
    /// Human-readable summary of what triggers the threat. Mirrors
    /// [`TriggerShape`] but renders to text agents can parse without
    /// a JSON decoder (e.g. `"any move by the bishop on e5"`).
    pub trigger: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SideTactics {
    pub side: String,
    /// `true` iff this side is the FEN's side-to-move.
    pub to_move: bool,
    /// When the side-to-move is in check, the null-move pivot used to
    /// query the opponent's standing tactic is unsound — we skip it.
    /// `Some(reason)` records the skip; `None` means the scan ran.
    pub skipped: Option<String>,
    /// `Some` when a high-confidence tactic exists for this side; `None`
    /// when the chain found nothing.
    pub best_tactic: Option<TacticHitView>,
    pub overloaded: Vec<OverloadedView>,
}

/// Flattened view of [`TacticHit`] with all squares / pieces resolved
/// to printable labels so the JSON consumer never has to second-guess
/// what `primary_piece: 28` means.
#[derive(Debug, Clone, Serialize)]
pub struct TacticHitView {
    pub pattern: String,
    pub mate_pattern: Option<String>,
    pub sacrifice: bool,
    /// Destination square of the key move (where the mover's piece
    /// lands). Algebraic, e.g. `"e4"`. We don't try to name the moved
    /// piece — for capture patterns the *captured* piece sits there
    /// pre-move and labelling it (`"Pe4"`) reads as if the pawn were
    /// the attacker, which is the opposite of the truth. The board
    /// view alongside this output (or the `targets` list below) is
    /// the right surface for "which piece does what".
    pub primary_square: String,
    /// Whatever the pre-move position has on [`Self::primary_square`],
    /// in `Nf3` / `qe6` style — `None` for empty squares (Fork,
    /// most TrappedPiece cases). Useful to JSON consumers as
    /// metadata; the text renderer omits it because it's pattern-
    /// dependent.
    pub primary_square_pre_move_piece: Option<String>,
    /// Pieces / squares the pattern bears on — the forked enemies, the
    /// hanging piece, the pinned piece, the checked king. Same
    /// `Nf3` / `qe6` formatting; ascending square order matches the
    /// engine's deterministic ordering.
    pub targets: Vec<String>,
    /// Engine-cp midgame material gain over the first four plies of the
    /// line, from the tactic owner's POV. `None` when the line was too
    /// short to assess.
    pub material_gain: Option<i32>,
    pub confidence: String,
    pub pv_ply: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct OverloadedView {
    pub piece: String,
    pub square: String,
    pub duties: Vec<String>,
}

/// Build the full tactics report. `prior_move` is the *actual* side-to-
/// move's opponent's last move (the one that produced `pos`), used by
/// the hanging-capture recapture guard for the side-to-move scan only.
/// `with_latent = true` populates [`TacticsView::latent`] with the
/// Phase-D standing-threat scan; `with_check_followups = true`
/// populates [`TacticsView::check_followups`] with the Phase-E
/// two-step forcing-line scan. Both default to `None` when off.
pub fn build(
    pos: &Position,
    prior_move: Option<PriorMove>,
    with_latent: bool,
    with_check_followups: bool,
) -> TacticsView {
    let stm = pos.side_to_move();
    let stm_in_check = pos.in_check();
    let latent = if with_latent {
        Some(LatentView {
            against_white: build_latent_side(pos, Color::White),
            against_black: build_latent_side(pos, Color::Black),
        })
    } else {
        None
    };
    let check_followups = if with_check_followups {
        Some(CheckFollowupsView {
            for_white: build_check_followups_side(pos, Color::White),
            for_black: build_check_followups_side(pos, Color::Black),
        })
    } else {
        None
    };
    TacticsView {
        white: build_side(pos, Color::White, stm == Color::White, stm_in_check, prior_move),
        black: build_side(pos, Color::Black, stm == Color::Black, stm_in_check, prior_move),
        latent,
        check_followups,
    }
}

fn build_check_followups_side(pos: &Position, mover: Color) -> CheckFollowupsSide {
    let sequences = find_check_followups(pos, mover, None)
        .into_iter()
        .map(|cf| build_check_followup_view(pos, mover, &cf))
        .collect();
    CheckFollowupsSide {
        mover_side: color_name(mover).to_lowercase(),
        sequences,
    }
}

fn build_check_followup_view(pos: &Position, mover: Color, cf: &CheckFollowup) -> CheckFollowupView {
    // SAN rendering needs the position the move was played FROM. The
    // mover side's checks were enumerated from either `pos` (when
    // mover == stm) or `pos` after a null pivot (otherwise). For SAN
    // labelling we just need a position where it's the mover's turn —
    // a clone with side-to-move flipped works whether or not we
    // actually need the pivot.
    let san_for_check = san_for_move(pos, mover, cf.check_move);
    let mut post = pos.clone();
    if post.side_to_move() != mover {
        post.do_null_move();
    }
    // Replay the check to get the SAN context for replies.
    let _saved = post.do_move(cf.check_move);

    let replies = cf
        .replies
        .iter()
        .map(|r| build_reply_followup_view(&post, mover, r))
        .collect();

    CheckFollowupView {
        check_san: san_for_check,
        check_uci: crate::uci::format(cf.check_move),
        replies,
    }
}

fn build_reply_followup_view(
    post_check_pos: &Position,
    mover: Color,
    r: &ReplyFollowup,
) -> ReplyFollowupView {
    let mut scratch = post_check_pos.clone();
    let reply_san = san::format(&scratch, r.reply);
    let _saved = scratch.do_move(r.reply);
    // After the reply, the position is in mover's frame; use it to
    // resolve square labels for the followup hit.
    let followup = r
        .followup
        .as_ref()
        .map(|h| build_tactic_hit_view(&scratch, h));
    let _ = mover;
    ReplyFollowupView {
        reply_san,
        reply_uci: crate::uci::format(r.reply),
        followup,
    }
}

/// SAN for `mv` when `mv` is played by `mover` (which may not be
/// `pos.side_to_move()`). Null-pivots when needed.
fn san_for_move(pos: &Position, mover: Color, mv: chess_tutor_engine::types::Move) -> String {
    let mut scratch = pos.clone();
    if scratch.side_to_move() != mover {
        scratch.do_null_move();
    }
    san::format(&scratch, mv)
}

fn build_latent_side(pos: &Position, defender: Color) -> LatentSide {
    LatentSide {
        defender_side: color_name(defender).to_lowercase(),
        threats: find_latent_threats(pos, defender)
            .into_iter()
            .map(|t| build_latent_threat_view(pos, &t))
            .collect(),
    }
}

fn build_latent_threat_view(pos: &Position, t: &LatentThreat) -> LatentThreatView {
    let discoverer = pos
        .piece_on(t.discoverer)
        .map(|p| piece_label(p, t.discoverer))
        .unwrap_or_else(|| t.discoverer.to_algebraic());
    let (vehicle, vehicle_square) = match t.vehicle {
        Some(v_sq) => (
            Some(
                pos.piece_on(v_sq)
                    .map(|p| piece_label(p, v_sq))
                    .unwrap_or_else(|| v_sq.to_algebraic()),
            ),
            Some(v_sq.to_algebraic()),
        ),
        None => (None, None),
    };
    let target = pos
        .piece_on(t.target)
        .map(|p| piece_label(p, t.target))
        .unwrap_or_else(|| t.target.to_algebraic());
    let trigger = match t.trigger_shape {
        TriggerShape::VehicleMoves => {
            // The vehicle is the slider's own blocker — any move by it
            // unmasks the discovery.
            format!(
                "any move by {} discovers the slider's attack on {}",
                vehicle.clone().unwrap_or_else(|| "?".to_string()),
                target,
            )
        }
        TriggerShape::VehicleConstrained => format!(
            "{} can't move without exposing {}",
            vehicle.clone().unwrap_or_else(|| "?".to_string()),
            target,
        ),
        TriggerShape::DefenderRemoved { defender } => {
            let def_label = pos
                .piece_on(defender)
                .map(|p| piece_label(p, defender))
                .unwrap_or_else(|| defender.to_algebraic());
            format!(
                "{} captures defender {}, leaving {} unhooked",
                discoverer, def_label, target,
            )
        }
    };
    LatentThreatView {
        pattern: pattern_name(t.pattern).to_string(),
        discoverer,
        discoverer_square: t.discoverer.to_algebraic(),
        vehicle,
        vehicle_square,
        target,
        target_square: t.target.to_algebraic(),
        min_gain: t.min_gain,
        confidence: confidence_name(t.confidence).to_string(),
        trigger,
    }
}

fn build_side(
    pos: &Position,
    side: Color,
    is_stm: bool,
    stm_in_check: bool,
    prior_move: Option<PriorMove>,
) -> SideTactics {
    let overloaded = find_overloaded(pos, side)
        .into_iter()
        .map(|o| build_overloaded_view(pos, o))
        .collect();

    let (best_tactic, skipped) = if is_stm {
        let hit = find_best_tactic_in_position(pos, side, prior_move);
        (hit.map(|h| build_tactic_hit_view(pos, &h)), None)
    } else if stm_in_check {
        // The side-to-move is in check; a null-move pivot to ask "what
        // could the opponent play with a free tempo" is unsound (you
        // can't legally null a check). Document the skip so the agent
        // knows the section is silent on purpose, not by oversight.
        (
            None,
            Some(
                "side-to-move is in check; null-move pivot unsound — opponent scan skipped"
                    .to_string(),
            ),
        )
    } else {
        // Null-move pivot: flip side-to-move so the detector chain
        // enumerates the opponent's pseudo-free-tempo moves. We work on
        // a clone so the caller's position is untouched.
        let mut scratch = pos.clone();
        let saved = scratch.do_null_move();
        // `prior_move` does not survive the null pivot — its recapture
        // semantics line up with the original side-to-move, not the
        // opponent's pseudo-turn. Pass `None`.
        let hit = find_best_tactic_in_position(&scratch, side, None);
        scratch.undo_null_move(saved);
        (hit.map(|h| build_tactic_hit_view(pos, &h)), None)
    };

    SideTactics {
        side: color_name(side).to_lowercase(),
        to_move: is_stm,
        skipped,
        best_tactic,
        overloaded,
    }
}

fn build_tactic_hit_view(pos: &Position, hit: &TacticHit) -> TacticHitView {
    let primary_square_pre_move_piece = pos
        .piece_on(hit.primary_piece)
        .map(|p| piece_label(p, hit.primary_piece));
    let targets = hit
        .targets
        .iter()
        .map(|sq| {
            pos.piece_on(*sq)
                .map(|p| piece_label(p, *sq))
                .unwrap_or_else(|| sq.to_algebraic())
        })
        .collect();
    TacticHitView {
        pattern: pattern_name(hit.pattern).to_string(),
        mate_pattern: hit.mate_pattern.map(|mp| mate_pattern_name(mp).to_string()),
        sacrifice: hit.sacrifice,
        primary_square: hit.primary_piece.to_algebraic(),
        primary_square_pre_move_piece,
        targets,
        material_gain: hit.material_gain,
        confidence: confidence_name(hit.confidence).to_string(),
        pv_ply: hit.pv_ply,
    }
}

fn build_overloaded_view(pos: &Position, o: OverloadedPiece) -> OverloadedView {
    let piece = pos
        .piece_on(o.piece)
        .map(|p| piece_label(p, o.piece))
        .unwrap_or_else(|| o.piece.to_algebraic());
    OverloadedView {
        piece,
        square: o.piece.to_algebraic(),
        duties: o
            .duties
            .iter()
            .map(|sq| {
                pos.piece_on(*sq)
                    .map(|p| piece_label(p, *sq))
                    .unwrap_or_else(|| sq.to_algebraic())
            })
            .collect(),
    }
}

fn pattern_name(p: TacticPattern) -> &'static str {
    match p {
        TacticPattern::Fork => "Fork",
        TacticPattern::HangingCapture => "HangingCapture",
        TacticPattern::RemovingDefender => "RemovingDefender",
        TacticPattern::TrappedPiece => "TrappedPiece",
        TacticPattern::Pin => "Pin",
        TacticPattern::Skewer => "Skewer",
        TacticPattern::DiscoveredAttack => "DiscoveredAttack",
        TacticPattern::DiscoveredCheck => "DiscoveredCheck",
        TacticPattern::DoubleCheck => "DoubleCheck",
        TacticPattern::Sacrifice => "Sacrifice",
        TacticPattern::Intermezzo => "Intermezzo",
        TacticPattern::Deflection => "Deflection",
        TacticPattern::Attraction => "Attraction",
        TacticPattern::Interference => "Interference",
        TacticPattern::Clearance => "Clearance",
        TacticPattern::XRay => "XRay",
        TacticPattern::AttackingF2F7 => "AttackingF2F7",
        TacticPattern::UnderPromotion => "UnderPromotion",
        TacticPattern::Checkmate => "Checkmate",
    }
}

fn mate_pattern_name(m: MatePattern) -> &'static str {
    match m {
        MatePattern::BackRank => "BackRank",
        MatePattern::Smothered => "Smothered",
        MatePattern::Anastasia => "Anastasia",
        MatePattern::Hook => "Hook",
        MatePattern::Arabian => "Arabian",
        MatePattern::Boden => "Boden",
        MatePattern::DoubleBishop => "DoubleBishop",
        MatePattern::Dovetail => "Dovetail",
    }
}

fn confidence_name(c: Confidence) -> &'static str {
    match c {
        Confidence::High => "High",
        Confidence::Medium => "Medium",
    }
}

/// Multi-line human-readable rendering. The header per side calls out
/// `(to move)` so the agent sees side-of-turn at a glance, even when
/// the position summary above this output is also showing it.
pub fn render_text(view: &TacticsView) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for side in [&view.white, &view.black] {
        render_side(&mut out, side);
        writeln!(out).unwrap();
    }
    if let Some(latent) = &view.latent {
        render_latent(&mut out, latent);
    }
    if let Some(cf) = &view.check_followups {
        render_check_followups(&mut out, cf);
    }
    out
}

fn render_check_followups(out: &mut String, cf: &CheckFollowupsView) {
    use std::fmt::Write;
    writeln!(out, "check-followups (one ply past the check):").unwrap();
    for side in [&cf.for_white, &cf.for_black] {
        if side.sequences.is_empty() {
            writeln!(out, "  for {}: (no two-step forcing sequences detected)", side.mover_side).unwrap();
            continue;
        }
        writeln!(out, "  for {} ({}):", side.mover_side, side.sequences.len()).unwrap();
        for seq in &side.sequences {
            writeln!(out, "    {} ({}):", seq.check_san, seq.check_uci).unwrap();
            for r in &seq.replies {
                match &r.followup {
                    Some(hit) => {
                        let gain = match hit.material_gain {
                            Some(g) => format!(", gain {g} engine-cp"),
                            None => String::new(),
                        };
                        writeln!(
                            out,
                            "      {} ({}) -> {} on {} ({}{})",
                            r.reply_san,
                            r.reply_uci,
                            hit.pattern,
                            hit.primary_square,
                            hit.confidence,
                            gain,
                        )
                        .unwrap();
                    }
                    None => writeln!(
                        out,
                        "      {} ({}) -> defuses (no followup tactic)",
                        r.reply_san, r.reply_uci,
                    )
                    .unwrap(),
                }
            }
        }
    }
}

fn render_latent(out: &mut String, latent: &LatentView) {
    use std::fmt::Write;
    writeln!(out, "standing (latent) threats:").unwrap();
    for side in [&latent.against_white, &latent.against_black] {
        if side.threats.is_empty() {
            writeln!(out, "  against {}: (none detected)", side.defender_side).unwrap();
        } else {
            writeln!(
                out,
                "  against {} ({}):",
                side.defender_side,
                side.threats.len()
            )
            .unwrap();
            for t in &side.threats {
                // First line names the pattern and the structural triple;
                // second line spells out the trigger so the agent can
                // tell what move the opponent would play to fire it.
                match (&t.vehicle, &t.vehicle_square) {
                    (Some(_), Some(vsq)) => writeln!(
                        out,
                        "    {} via {}/{}({}) -> {}  (gain {})",
                        t.pattern, t.discoverer_square, vsq, t.vehicle.as_deref().unwrap_or("?"), t.target_square, t.min_gain,
                    )
                    .unwrap(),
                    _ => writeln!(
                        out,
                        "    {} via {} -> {}  (gain {})",
                        t.pattern, t.discoverer_square, t.target_square, t.min_gain,
                    )
                    .unwrap(),
                }
                writeln!(out, "      trigger:   {}", t.trigger).unwrap();
            }
        }
    }
}

fn render_side(out: &mut String, side: &SideTactics) {
    use std::fmt::Write;
    let role = if side.to_move {
        "to move"
    } else {
        "one-ply ahead"
    };
    writeln!(out, "{} ({}):", side.side, role).unwrap();
    match (&side.skipped, &side.best_tactic) {
        (Some(reason), _) => writeln!(out, "  best tactic: skipped ({reason})").unwrap(),
        (None, None) => writeln!(out, "  best tactic: (no high-confidence pattern detected)").unwrap(),
        (None, Some(hit)) => render_hit(out, hit),
    }
    if side.overloaded.is_empty() {
        writeln!(out, "  overloaded: (none)").unwrap();
    } else {
        writeln!(out, "  overloaded ({}):", side.overloaded.len()).unwrap();
        for o in &side.overloaded {
            writeln!(
                out,
                "    {} — sole defender of {}",
                o.piece,
                o.duties.join(", "),
            )
            .unwrap();
        }
    }
}

fn render_hit(out: &mut String, hit: &TacticHitView) {
    use std::fmt::Write;
    let sac = if hit.sacrifice { " (sacrifice)" } else { "" };
    let mate_suffix = match &hit.mate_pattern {
        Some(mp) => format!(" + {mp} mate"),
        None => String::new(),
    };
    writeln!(out, "  best tactic: {}{}{}", hit.pattern, mate_suffix, sac).unwrap();
    // Just the destination square — see the field doc on
    // [`TacticHitView::primary_square`] for why we don't try to label
    // the moving piece.
    writeln!(out, "    key sq:     {}", hit.primary_square).unwrap();
    let targets = if hit.targets.is_empty() {
        "(none)".to_string()
    } else {
        hit.targets.join(", ")
    };
    writeln!(out, "    targets:    {targets}").unwrap();
    let gain = match hit.material_gain {
        Some(g) => format!("{g} engine-cp"),
        None => "n/a".to_string(),
    };
    writeln!(out, "    gain:       {gain}").unwrap();
    writeln!(out, "    confidence: {}", hit.confidence).unwrap();
}

#[cfg(test)]
#[path = "tactics_view_tests.rs"]
mod tests;
