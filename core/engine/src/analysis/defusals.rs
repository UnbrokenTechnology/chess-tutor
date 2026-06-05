//! Search-backed **defusal** enumeration for standing (latent) threats.
//!
//! [`super::latent_threats`] answers *"what has the opponent loaded
//! against me?"* This module answers the follow-on the agent always
//! needs next: *"which of my legal moves actually neutralise it without
//! throwing the game away?"*
//!
//! The motivating failure mode (see
//! `teaching-positions/discovered-attack-after-qxe6.md`): White is
//! winning, Black has a loaded discovered attack on the rook, and the
//! natural-looking `Qc5+` ignores it — the eval collapses from +2 to
//! −2. The lesson isn't "you missed a better move," it's "you had to
//! defuse a standing threat and didn't." There were exactly three moves
//! that defused it *and* held the advantage (`Qxe6`, `Qe4`, `Qe2`);
//! every other move walked into the loss. This module computes that set.
//!
//! ## Two-stage filter
//!
//! 1. **Geometric candidacy (free).** A legal move *neutralises* a
//!    specific threat if, after playing it, the threat's target square
//!    no longer carries any standing threat (re-scan
//!    [`find_latent_threats`], following the target piece if the move
//!    relocated it). This catches every mechanism uniformly —
//!    capture-the-discoverer, block-the-ray, relocate-the-target,
//!    over-defend — without enumerating them by hand.
//! 2. **Search-backed holding check.** Neutralising a threat geometrically
//!    is not enough: relocating the rook off the e-file *does* defuse the
//!    discovered attack, but it drops a separately-hanging queen. So we
//!    score every candidate with a real search and mark each as
//!    **holds** (keeps the eval on the winning side of the line) or
//!    "addresses the threat but loses elsewhere." The CLI leads with the
//!    holders and shows the rest as a cautionary list.
//!
//! Stage 1 shrinks the legal-move set to a handful before any search
//! runs, so the cost is one MultiPV search over ~3–6 forced candidates,
//! not over all 40-odd legal moves.

use std::collections::HashMap;

use crate::analysis::latent_threats::{find_latent_threats, LatentThreat};
use crate::analysis::tactic_outcome::TacticPattern;
use crate::attacks::between_bb;
use crate::engine::{Engine, SearchParams};
use crate::movegen::legal_moves_vec;
use crate::position::Position;
use crate::types::{Move, Square, Value};

#[cfg(test)]
#[path = "defusals_tests.rs"]
mod tests;

/// How a move neutralises a standing threat. Purely descriptive — the
/// candidacy test is the geometric re-scan; this label just lets the
/// teaching surface say *why* the move works in plain English.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DefusalMechanism {
    /// The move captures the firing piece (the slider for a discovered
    /// attack / pin / skewer, the attacker-of-the-defender for a
    /// remove-the-defender). With the threat's engine gone, the pattern
    /// evaporates. This is the cleanest defusal.
    CaptureDiscoverer,
    /// The move captures the blocker on the slider's ray (the
    /// "vehicle"). Rare, but it removes the discovery vehicle outright.
    CaptureVehicle,
    /// The move interposes a piece on the firing ray, strictly between
    /// the slider and the target, so the target stays shielded even after
    /// the vehicle moves.
    Block,
    /// The move walks the threatened piece off the firing line / out of
    /// the attack entirely.
    RelocateTarget,
    /// The move neutralises the threat by some other means the re-scan
    /// confirms — typically adding a defender so the capture is no longer
    /// profitable, or removing the attacker indirectly.
    OverDefend,
}

/// One threat a given move neutralises, with the mechanism and enough
/// identity (pattern + target square) for the renderer to point at the
/// matching `danger:` line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefusedThreat {
    /// Index into the `threats` slice passed to [`find_threat_defusals`]
    /// — lets the caller cross-reference the exact `danger:` entry.
    pub threat_index: usize,
    pub pattern: TacticPattern,
    /// The piece the threat bore on (its square in the *pre-move*
    /// position).
    pub target: Square,
    pub mechanism: DefusalMechanism,
}

/// A legal move that neutralises at least one standing threat, with its
/// searched score and whether it holds the advantage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreatDefusal {
    pub mv: Move,
    /// Side-to-move-POV score from the backing search (same scale as
    /// [`Value`] everywhere else — PawnEG = 213, tempo included).
    pub score: Value,
    /// `true` when the move keeps the side to move on the winning side of
    /// the line (see [`holds`]). `false` means it addresses the threat
    /// but concedes the advantage elsewhere — the cautionary case.
    pub holds: bool,
    /// Every standing threat this move neutralises (usually one).
    pub defuses: Vec<DefusedThreat>,
}

/// The full defusal report for a position's standing threats.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefusalReport {
    /// The best line's score from the backing search (side-to-move POV).
    /// The baseline a defusal is measured against.
    pub best_score: Value,
    /// The best move overall, for the "what you should actually play"
    /// headline. `None` only in a terminal position.
    pub best_move: Option<Move>,
    /// Candidates sorted holders-first, then by score descending.
    pub defusals: Vec<ThreatDefusal>,
}

/// A defusal "holds" if its score stays at or above the winning side of
/// the line, within this slack. We compare against `min(best, 0)` rather
/// than `best` directly: in a winning position the second-best defence
/// can be a pawn or two below best yet still completely winning (the
/// case study's `Qe4` is ~2 pawns under `Qxe6` but both keep White up a
/// piece), so a relative-to-best band would wrongly reject valid
/// defences. The real question is "does this move keep me on the right
/// side of equality," which `min(best, 0) − slack` captures: when best
/// is winning the bar is ~equality; when best is already losing the bar
/// relaxes to "within slack of best available."
const HOLD_SLACK_CP: i32 = 150;

/// Compute which legal moves defuse the supplied standing `threats` and
/// which of those hold the advantage. `threats` should be the side-to-
/// move's standing threats (i.e. `find_latent_threats(pos, side_to_move)`);
/// the `threat_index` on each [`DefusedThreat`] indexes into it.
///
/// Runs exactly one MultiPV search (over the geometric candidates) to
/// score them, so cost scales with the candidate count, not the legal-
/// move count. Pure with respect to `pos` (searches on the engine's own
/// state; `pos` is restored).
pub fn find_threat_defusals(
    engine: &mut Engine,
    pos: &mut Position,
    threats: &[LatentThreat],
    depth: u32,
) -> DefusalReport {
    let side = pos.side_to_move();
    let legal = legal_moves_vec(&mut pos.clone());

    // Stage 1: geometric candidacy. For each legal move, collect the
    // threats it neutralises. A move with an empty list isn't a
    // candidate.
    let mut candidate_defuses: HashMap<Move, Vec<DefusedThreat>> = HashMap::new();
    for &mv in &legal {
        let mut after = pos.clone();
        after.do_move(mv);
        let post = find_latent_threats(&after, side);
        for (idx, threat) in threats.iter().enumerate() {
            if move_neutralises(pos, mv, threat, &post) {
                candidate_defuses
                    .entry(mv)
                    .or_default()
                    .push(DefusedThreat {
                        threat_index: idx,
                        pattern: threat.pattern,
                        target: threat.target,
                        mechanism: classify_mechanism(pos, mv, threat),
                    });
            }
        }
    }

    let candidates: Vec<Move> = candidate_defuses.keys().copied().collect();

    // Stage 2: one search to score the candidates (force_include) and to
    // get the best line as the baseline.
    let params = SearchParams {
        max_depth: depth,
        max_nodes: None,
        max_time: None,
        multi_pv: 1,
        game_history: Vec::new(),
        force_include: candidates.clone(),
        verbose_progress: false,
        threads: 1,
        eval_mask: crate::opponent::EvalMask::EMPTY,
        qsearch_max_plies: None,
        endgame_skill: crate::endgame::EndgameSkill::Full,
    };
    let lines = engine.search(pos, params);
    if lines.is_empty() {
        return DefusalReport {
            best_score: Value::ZERO,
            best_move: None,
            defusals: Vec::new(),
        };
    }

    let best_score = lines[0].score;
    let best_move = lines[0].pv.first().copied();

    // Map each scored root move (by its first PV move) to its score.
    let mut score_of: HashMap<Move, Value> = HashMap::new();
    for line in &lines {
        if let Some(&first) = line.pv.first() {
            score_of.entry(first).or_insert(line.score);
        }
    }

    let mut defusals: Vec<ThreatDefusal> = candidates
        .into_iter()
        .filter_map(|mv| {
            let score = *score_of.get(&mv)?;
            let mut defuses = candidate_defuses.remove(&mv).unwrap_or_default();
            defuses.sort_by_key(|d| d.threat_index);
            Some(ThreatDefusal {
                mv,
                score,
                holds: holds(score, best_score),
                defuses,
            })
        })
        .collect();

    // Holders first, then by score descending. Tie-break on the move's
    // raw bits so the ordering is deterministic for tests / CLI.
    defusals.sort_by(|a, b| {
        b.holds
            .cmp(&a.holds)
            .then(b.score.cmp(&a.score))
            .then((a.mv.from(), a.mv.to()).cmp(&(b.mv.from(), b.mv.to())))
    });

    DefusalReport {
        best_score,
        best_move,
        defusals,
    }
}

/// Whether `score` keeps the side to move on the winning side of the
/// line relative to `best`. See [`HOLD_SLACK_CP`] for the rationale
/// behind the `min(best, 0)` bar.
fn holds(score: Value, best: Value) -> bool {
    let bar = best.0.min(0) - HOLD_SLACK_CP;
    score.0 >= bar
}

/// A move neutralises `threat` if, after the move, the threat's target
/// square no longer carries any standing threat. We follow the target
/// piece: if the move relocated it (its `from` square was the target),
/// we look at where it landed; otherwise we look at the original square.
///
/// `post` is the re-scanned standing-threat list for the same defender
/// side, computed once per move by the caller.
fn move_neutralises(
    pos: &Position,
    mv: Move,
    threat: &LatentThreat,
    post: &[LatentThreat],
) -> bool {
    let _ = pos;
    let new_target = if mv.from() == threat.target {
        mv.to()
    } else {
        threat.target
    };
    !post.iter().any(|t| t.target == new_target)
}

/// Best-effort label for *how* the move defuses the threat. The
/// candidacy decision is the re-scan in [`move_neutralises`]; this is
/// purely for the human-readable surface.
fn classify_mechanism(pos: &Position, mv: Move, threat: &LatentThreat) -> DefusalMechanism {
    let _ = pos;
    if mv.to() == threat.discoverer {
        DefusalMechanism::CaptureDiscoverer
    } else if threat.vehicle == Some(mv.to()) {
        DefusalMechanism::CaptureVehicle
    } else if mv.from() == threat.target {
        DefusalMechanism::RelocateTarget
    } else if threat.vehicle.is_some()
        && between_bb(threat.discoverer, threat.target).contains(mv.to())
    {
        DefusalMechanism::Block
    } else {
        DefusalMechanism::OverDefend
    }
}
