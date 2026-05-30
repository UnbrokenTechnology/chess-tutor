//! Standing (latent) threat detection — a pre-move static scan over
//! the **opponent's loaded tactics** against a given side.
//!
//! Parallel to [`super::overloading`]: same shape (a pure structural
//! observation about the board, not a "move you play"), same isolation
//! from the [`super::tactic_outcome`] played/missed/walked-into chain.
//! Where `find_overloaded` reports "this piece is doing two jobs,"
//! `find_latent_threats` reports "the opponent has this tactic
//! pre-loaded — if you don't address it, they execute it." The four
//! shapes covered:
//!
//! - **DiscoveredAttack** — enemy slider + enemy blocker (the
//!   "vehicle") + our piece on the slider's ray behind the blocker.
//!   Any move by the vehicle discovers the slider's attack on our
//!   piece. Example: `Qe6 / Be5 / Re1` on the discovered-attack
//!   case-study FEN — Bxh2+ fires it.
//! - **Pin** / **RelativePin** — enemy slider's ray passes through one
//!   of our pieces (the "vehicle") to a more valuable piece of ours.
//!   Our blocker cannot move without exposing the rear. A **king** rear
//!   is the absolute pin (`Pin`) — the vehicle literally may not move;
//!   any other rear is the relative pin (`RelativePin`) — the vehicle
//!   *may* move, it just concedes the rear if it does.
//! - **Skewer** — same ray shape, with our more-valuable piece in
//!   front and our cheaper piece behind. Slider's attack on the front
//!   forces it to move, then the rear falls.
//! - **RemovingDefender** — one of our pieces (the "target") is
//!   attacked AND held up by a sole defender; the enemy has an
//!   attacker on that defender. Capturing the defender unhooks the
//!   target. Example: White's Nf5 on the desperado case-study FEN,
//!   defended only by Pe4 which Nf6 attacks.
//!
//! Implementation notes:
//!
//! - Slider rays are walked via the classical bitboard x-ray trick:
//!   compute the slider's attacks under current occupancy (the
//!   "primary" set), then re-compute under occupancy with the candidate
//!   blocker removed; the new squares (`xray & !primary`) lie on the
//!   ray through that blocker. The first occupant on the new squares
//!   is the rear piece. [`crate::attacks::aligned`] is a belt-and-braces
//!   check that slider, blocker and rear are collinear.
//! - **`min_gain` is a thin SEE-ish heuristic, not a full SEE.**
//!   For DiscoveredAttack and Skewer — both of which fire with the
//!   slider capturing the target — we compute defenders of the target
//!   in the post-vehicle-move occupancy and gate the gain at:
//!   `target.value` when undefended, else `target.value − slider.value`
//!   (slider trades 1-for-1). This stops the predicate from claiming
//!   a "loaded discovered attack" in positions where the slider would
//!   just be blundering itself for a defended piece of lower value.
//!   For Pin we use `target.value − vehicle.value` as a rough proxy
//!   for "what's at stake if the pinned piece moves"; Pin is
//!   structural so a tighter gain calc would over-suppress. For
//!   RemovingDefender we still use `target.value` (the unhooked piece)
//!   — the desperado case-study sequence (`Nxe4 / Nxe4 / Qxf5`) nets
//!   only +1 cp under full SEE but the pre-move *shape* is the
//!   lesson, not the realized cp; the teaching surface refines from
//!   there.
//! - Gate: report only when `min_gain >= 3` (one minor piece), or when
//!   the target is the king (an absolute pin always lights).
//!
//! Caller convention: pass `defender_color = side_to_move` for "what
//! is the opponent threatening against me." The detector takes a
//! `defender_color` because both sides can have standing threats
//! against them; the CLI surfaces both colours symmetrically.

use crate::analysis::tactic_outcome::{Confidence, TacticPattern};
use crate::attacks::aligned;
use crate::bitboard::{square_bb, Bitboard};
use crate::magics::{bishop_attacks, rook_attacks};
use crate::movegen::legal_moves_vec;
use crate::position::Position;
use crate::types::{Color, Move, Piece, PieceType, Square};

#[cfg(test)]
#[path = "latent_threats_tests.rs"]
mod tests;

/// What the opponent must do for the threat to fire. The CLI / UI
/// reads this when narrating "fires on any forcing Bishop move" vs.
/// "if the defender on e4 is captured" vs. "if our knight moves" etc.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TriggerShape {
    /// The threat fires when [`LatentThreat::vehicle`] (an enemy
    /// piece blocking its own slider) moves. Used by
    /// [`TacticPattern::DiscoveredAttack`].
    VehicleMoves,
    /// The threat is the structural constraint itself —
    /// [`LatentThreat::vehicle`] (our piece) must stay (Pin) or must
    /// flee the slider's attack (Skewer); either way the rear piece is
    /// exposed. Used by [`TacticPattern::Pin`],
    /// [`TacticPattern::RelativePin`] and [`TacticPattern::Skewer`].
    VehicleConstrained,
    /// The threat fires when our [`Self::DefenderRemoved::defender`]
    /// is captured by [`LatentThreat::discoverer`] (the enemy attacker
    /// on the defender), unhooking
    /// [`LatentThreat::target`]. Used by
    /// [`TacticPattern::RemovingDefender`].
    DefenderRemoved { defender: Square },
}

/// One detected standing threat — read it as "if the opponent gets a
/// free tempo / the right trigger, they win [`Self::target`]." All
/// squares are absolute (board-coordinate) so callers can render piece
/// labels against the same position they passed in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LatentThreat {
    pub pattern: TacticPattern,
    /// The piece that *fires* — the slider for slider patterns, the
    /// attacker-on-defender for RemovingDefender.
    pub discoverer: Square,
    /// The blocker on the slider's ray (DA / Pin / Skewer). `None` for
    /// RemovingDefender — that pattern's "vehicle" is the defender
    /// itself, surfaced via [`TriggerShape::DefenderRemoved`].
    pub vehicle: Option<Square>,
    /// What gets attacked / won when the threat fires.
    pub target: Square,
    /// Conservative material gain estimate in **classical points**
    /// (P=1, N/B=3, R=5, Q=9). See module-level docs for why we keep
    /// this permissive ("value of the exposed piece") instead of full
    /// SEE. `0` means "structural only — pin / pressure pattern
    /// without a clean material claim;" the gate at `>= 3` (or king
    /// involvement) prevents the noise floor from drowning the signal.
    pub min_gain: i32,
    pub confidence: Confidence,
    pub trigger_shape: TriggerShape,
}

/// Scan `pos` for tactics the **attacker** (`!defender_color`) has
/// pre-loaded against the **defender** (`defender_color`).
///
/// Deterministic ordering: sorted by `(pattern, discoverer, target)`
/// so tests and CLI output are stable. Pure and side-effect-free.
pub fn find_latent_threats(pos: &Position, defender_color: Color) -> Vec<LatentThreat> {
    let mut out = Vec::new();
    find_latent_slider_alignments(pos, defender_color, &mut out);
    find_latent_remove_defender(pos, defender_color, &mut out);
    out.sort_by(|a, b| {
        (pattern_key(a.pattern), a.discoverer, a.target)
            .cmp(&(pattern_key(b.pattern), b.discoverer, b.target))
    });
    out
}

/// Sort key for stable output ordering. Lower = earlier. Matches the
/// most-instructive-first hierarchy a teaching CLI would want:
/// discovered attacks (silent dynamite) before remove-the-defender
/// (one trade away) before structural pins / skewers (movement
/// constraints without immediate material).
fn pattern_key(p: TacticPattern) -> u8 {
    match p {
        TacticPattern::DiscoveredAttack => 0,
        TacticPattern::RemovingDefender => 1,
        TacticPattern::Pin => 2,
        TacticPattern::RelativePin => 3,
        TacticPattern::Skewer => 4,
        _ => 99,
    }
}

// ------------------------------------------------------------------------
// Slider alignments — DiscoveredAttack, Pin, Skewer
// ------------------------------------------------------------------------

fn find_latent_slider_alignments(
    pos: &Position,
    defender_color: Color,
    out: &mut Vec<LatentThreat>,
) {
    let attacker = !defender_color;
    let occ = pos.occupied();
    let attacker_bb = pos.pieces_by_color(attacker);
    let defender_bb = pos.pieces_by_color(defender_color);

    let bishops_queens =
        (pos.pieces(PieceType::Bishop) | pos.pieces(PieceType::Queen)) & attacker_bb;
    let rooks_queens =
        (pos.pieces(PieceType::Rook) | pos.pieces(PieceType::Queen)) & attacker_bb;

    for slider_sq in bishops_queens {
        scan_one_slider(
            pos,
            slider_sq,
            /*orthogonal*/ false,
            occ,
            attacker_bb,
            defender_bb,
            out,
        );
    }
    for slider_sq in rooks_queens {
        scan_one_slider(
            pos,
            slider_sq,
            /*orthogonal*/ true,
            occ,
            attacker_bb,
            defender_bb,
            out,
        );
    }
}

fn scan_one_slider(
    pos: &Position,
    slider_sq: Square,
    orthogonal: bool,
    occ: Bitboard,
    attacker_bb: Bitboard,
    defender_bb: Bitboard,
    out: &mut Vec<LatentThreat>,
) {
    let primary = slider_attacks(slider_sq, occ, orthogonal);
    // First-hit pieces along the slider's rays.
    let blockers = primary & occ;
    for vehicle_sq in blockers {
        let xray_occ = occ ^ square_bb(vehicle_sq);
        let xray = slider_attacks(slider_sq, xray_occ, orthogonal);
        let extras = xray & !primary;
        let rear_candidates = extras & occ;
        for target_sq in rear_candidates {
            if !aligned(slider_sq, vehicle_sq, target_sq) {
                // Defensive — the x-ray trick should keep us on a single ray,
                // but `aligned` is the canonical predicate and a free check.
                continue;
            }
            classify_slider_triple(
                pos,
                slider_sq,
                vehicle_sq,
                target_sq,
                attacker_bb,
                defender_bb,
                out,
            );
        }
    }
}

fn slider_attacks(sq: Square, occ: Bitboard, orthogonal: bool) -> Bitboard {
    if orthogonal {
        rook_attacks(sq, occ)
    } else {
        bishop_attacks(sq, occ)
    }
}

/// Given (slider, vehicle, target) on a common ray, decide which
/// pattern (if any) fires and push it to `out`. The slider is always
/// attacker-color; `vehicle_color × target_color` selects the shape:
///
/// - `(attacker, defender)` → DiscoveredAttack
/// - `(defender, defender)` → Pin or Skewer (front-vs-back value)
/// - other combinations don't represent a threat against
///   `defender_color`.
fn classify_slider_triple(
    pos: &Position,
    slider_sq: Square,
    vehicle_sq: Square,
    target_sq: Square,
    attacker_bb: Bitboard,
    defender_bb: Bitboard,
    out: &mut Vec<LatentThreat>,
) {
    let vehicle_piece = match pos.piece_on(vehicle_sq) {
        Some(p) => p,
        None => return,
    };
    let target_piece = match pos.piece_on(target_sq) {
        Some(p) => p,
        None => return,
    };
    let target_value = points(target_piece);
    let vehicle_value = points(vehicle_piece);
    let slider_value = match pos.piece_on(slider_sq) {
        Some(p) => points(p),
        None => return,
    };

    let vehicle_attacker = attacker_bb.contains(vehicle_sq);
    let target_defender = defender_bb.contains(target_sq);

    if vehicle_attacker && target_defender {
        // DiscoveredAttack — slider captures target after the vehicle
        // moves. Compute defenders of target in the post-vehicle-move
        // occupancy (removing the vehicle can also *reveal* a defender
        // that was sitting behind it on a different ray, which the bare
        // current-occupancy check would miss). If undefended, slider
        // takes freely (gain = target.value); else the slider trades
        // 1-for-1 (gain = target.value - slider.value). Gate at >= 3
        // so the predicate doesn't falsely flag "loaded discovered
        // attack" in positions where the slider would just be
        // blundering itself for a cheaper defended piece.
        let gain = slider_capture_gain(
            pos,
            slider_sq,
            slider_value,
            target_sq,
            target_value,
            vehicle_sq,
            defender_bb,
        );
        if gain < 3 {
            return;
        }
        out.push(LatentThreat {
            pattern: TacticPattern::DiscoveredAttack,
            discoverer: slider_sq,
            vehicle: Some(vehicle_sq),
            target: target_sq,
            min_gain: gain,
            confidence: Confidence::High,
            trigger_shape: TriggerShape::VehicleMoves,
        });
        return;
    }

    if !vehicle_attacker && target_defender {
        // Pin or Skewer. The target-is-king case is always a Pin and
        // always reported (absolute pin); otherwise we sort by value.
        let target_is_king = target_piece.kind() == PieceType::King;
        let vehicle_is_king = vehicle_piece.kind() == PieceType::King;
        let (pattern, gain) = if target_is_king {
            (TacticPattern::Pin, target_value.max(9))
        } else if vehicle_is_king {
            // King in front, valuable piece behind — that's not a
            // legal position to begin with (the king couldn't sit on a
            // slider's ray with a friendly piece exposed behind it
            // without the position already being check, which would
            // mean the slider's been moved illegally). Skip
            // defensively; covered by movegen invariants in practice.
            return;
        } else if target_value > vehicle_value {
            // Rear is more valuable than front → vehicle is pinned to a
            // non-king piece: a *relative* pin. Unlike the absolute
            // (king-rear) case above, the vehicle may legally move; it
            // just concedes the rear. gain proxy = difference (rough
            // "stake if vehicle moves"). Intentionally not the SEE-ish
            // slider-captures-target calc: the pin's threat is structural
            // (vehicle shouldn't move), not "slider will swing in" —
            // over-tightening hides real pins where the slider would
            // itself be lost on the capture.
            (TacticPattern::RelativePin, target_value - vehicle_value)
        } else if vehicle_value > target_value {
            // Skewer — slider's attack forces vehicle to move,
            // exposing the rear. Slider then captures rear, so the
            // same SEE-ish gate as DiscoveredAttack applies. The
            // post-vehicle-move occupancy correctly models defenders
            // here too (vehicle has fled; rear's defenders are
            // unchanged in most cases, revealed in rare ones).
            let gain = slider_capture_gain(
                pos,
                slider_sq,
                slider_value,
                target_sq,
                target_value,
                vehicle_sq,
                defender_bb,
            );
            (TacticPattern::Skewer, gain)
        } else {
            // Equal values — neither side gains material from the
            // forcing line; the structural pin still constrains motion
            // but we suppress the report to keep noise low.
            return;
        };
        if gain < 3 && !target_is_king {
            return;
        }
        out.push(LatentThreat {
            pattern,
            discoverer: slider_sq,
            vehicle: Some(vehicle_sq),
            target: target_sq,
            min_gain: gain,
            confidence: Confidence::High,
            trigger_shape: TriggerShape::VehicleConstrained,
        });
    }
    // (attacker_color blocker, attacker_color rear) — slider's own
    // pieces stacked, no threat against the defender. Silently skip.
    // (defender_color blocker, attacker_color rear) — slider would be
    // hitting its own piece behind an enemy; no defender-side threat.
}

/// SEE-ish gain for the slider capturing target once vehicle moves.
/// Computes target's defenders in the post-vehicle-move occupancy so
/// the calc correctly accounts for defenders whose lines run through
/// the vehicle (rare, but possible). Undefended → full target.value;
/// defended → `max(0, target.value - slider.value)` (slider trades
/// 1-for-1, no further recapture chain).
fn slider_capture_gain(
    pos: &Position,
    slider_sq: Square,
    slider_value: i32,
    target_sq: Square,
    target_value: i32,
    vehicle_sq: Square,
    defender_bb: Bitboard,
) -> i32 {
    // `attackers_to` reads piece-type bitboards from the BOARD STATE
    // (not from the supplied occupancy), so removing the vehicle from
    // `occ` correctly extends rays *through* it but does NOT remove
    // the vehicle's own attacks. The vehicle has moved in our
    // hypothetical, so explicitly subtract it from the defender set.
    // The slider itself is attacker-color so `& defender_bb` already
    // excludes it; the `_ = slider_sq` keeps the signature honest
    // (the slider square is conceptually part of the calc and may be
    // needed if we ever extend to a full SEE recursion).
    let _ = slider_sq;
    let post_move_occ = pos.occupied() ^ square_bb(vehicle_sq);
    let defenders =
        (pos.attackers_to(target_sq, post_move_occ) & defender_bb).without(vehicle_sq);
    if defenders.is_empty() {
        target_value
    } else {
        (target_value - slider_value).max(0)
    }
}

// ------------------------------------------------------------------------
// RemovingDefender — non-ray pattern
// ------------------------------------------------------------------------

fn find_latent_remove_defender(
    pos: &Position,
    defender_color: Color,
    out: &mut Vec<LatentThreat>,
) {
    let attacker = !defender_color;
    let occ = pos.occupied();
    let attacker_bb = pos.pieces_by_color(attacker);
    let defender_bb = pos.pieces_by_color(defender_color);

    // For each of *our* (defender-color) pieces X that is currently
    // under attack and held up by exactly one defender, check whether
    // any enemy piece is hitting that defender — if so, capturing the
    // defender unhooks X.
    for x_sq in defender_bb {
        let x_piece = match pos.piece_on(x_sq) {
            Some(p) => p,
            None => continue,
        };
        let x_value = points(x_piece);
        // Pawns aren't worth the report; kings can't be "won."
        if x_value < 3 || x_piece.kind() == PieceType::King {
            continue;
        }
        let attackers_of_x = pos.attackers_to(x_sq, occ) & attacker_bb;
        if attackers_of_x.is_empty() {
            // Must currently be under attack — otherwise removing the
            // defender doesn't immediately threaten anything.
            continue;
        }
        let defenders_of_x = pos.attackers_to(x_sq, occ) & defender_bb;
        if defenders_of_x.popcount() != 1 {
            // Strict sole-defender predicate keeps misfires low; the
            // multi-defender case needs full SEE to know whether
            // removing *one* defender is enough. Documented follow-on.
            continue;
        }
        let y_sq = defenders_of_x.lsb();
        // The defender can't be a king — kings don't "defend" in the
        // square-attacked sense the predicate cares about.
        let y_piece = match pos.piece_on(y_sq) {
            Some(p) => p,
            None => continue,
        };
        if y_piece.kind() == PieceType::King {
            continue;
        }
        let attackers_of_y = pos.attackers_to(y_sq, occ) & attacker_bb;
        for z_sq in attackers_of_y {
            // No further filter on Z — even a Knight × Pawn trade can
            // be the right move when the unhooked target is a knight
            // (the desperado case-study FEN: Black's Nxe4 trades a
            // knight for a pawn but unhooks Nf5). The teaching layer
            // / search refines from there.
            out.push(LatentThreat {
                pattern: TacticPattern::RemovingDefender,
                discoverer: z_sq,
                vehicle: None,
                target: x_sq,
                min_gain: x_value,
                confidence: Confidence::High,
                trigger_shape: TriggerShape::DefenderRemoved { defender: y_sq },
            });
        }
    }
}

// ------------------------------------------------------------------------
// Forcing-move synthesis — does the geometric restraint actually hold?
// ------------------------------------------------------------------------
//
// The slider scans above report *geometry*: "this bishop is the vehicle of
// a discovered attack" / "this bishop is pinned." Read in isolation those
// two facts look contradictory — a pinned piece reads as "can't move," yet
// the discovered attack fires when it *does* move. The reconciliation is a
// single question these helpers answer: **does the vehicle have a forcing
// move (a check) that defeats the pin?** If it does, the pin never gets to
// bite (the forcing move must be answered before the pinning side could
// punish the vehicle's departure), so the discovered attack is genuinely
// live. This is the load-bearing insight on the
// `discovered-attack-after-qxe6` case study: `…Bxh2+` springs the
// discovery on `Re1` *because* it is a check, even though the bishop is
// nominally pinned to the queen by that same rook.

/// How a discovered-attack standing threat actually fires, beyond the bare
/// "the vehicle moves" geometry. Produced by [`describe_discovery_firing`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DiscoveryFiring {
    /// The opponent move that springs the discovery, chosen
    /// most-forcing-first (check+capture, then check, then capture, then
    /// any quiet vehicle move that vacates the ray).
    pub firing_move: Move,
    /// The firing move gives check — the property that lets it beat a
    /// counter-pin on the vehicle.
    pub gives_check: bool,
    /// The firing move is also a capture (e.g. `…Bxh2+` grabs a pawn on
    /// the way).
    pub is_capture: bool,
    /// The vehicle is counter-pinned by one of the *defender's* sliders
    /// (a quiet vehicle move would lose the rear piece). When this is
    /// `true` **and** [`Self::gives_check`] is `true`, the forcing nature
    /// of the firing move is what overrides the counter-pin — the
    /// teaching point the danger block must spell out.
    pub vehicle_counter_pinned: bool,
}

/// For a [`TacticPattern::DiscoveredAttack`] standing `threat` against
/// `defender_color`, work out the concrete opponent move that fires it and
/// whether the defender's counter-pin on the vehicle (if any) actually
/// restrains it. Returns `None` for non-discovered-attack patterns, when
/// the defender is in check (the standing-threat framing doesn't apply),
/// or when no vehicle move actually springs the discovery.
///
/// Pure with respect to `pos` (works on clones). The opponent is given the
/// move via a null pivot when it isn't already their turn — the same
/// "free tempo" model the `tactics` opponent scan uses.
pub fn describe_discovery_firing(
    pos: &Position,
    defender_color: Color,
    threat: &LatentThreat,
) -> Option<DiscoveryFiring> {
    if threat.pattern != TacticPattern::DiscoveredAttack || pos.in_check() {
        return None;
    }
    let vehicle = threat.vehicle?;
    let discoverer = threat.discoverer;
    let target = threat.target;

    let mut scratch = pos.clone();
    let saved = (scratch.side_to_move() == defender_color).then(|| scratch.do_null_move());

    // Most-forcing vehicle move that actually fires the discovery (after
    // it, the discoverer attacks the target square).
    let mut best: Option<(u8, Move, bool, bool)> = None;
    for mv in legal_moves_vec(&mut scratch) {
        if mv.from() != vehicle {
            continue;
        }
        let gives_check = scratch.gives_check(mv);
        let is_capture = scratch.is_capture(mv);
        let mut after = scratch.clone();
        after.do_move(mv);
        let fires = after
            .attackers_to(target, after.occupied())
            .contains(discoverer);
        if !fires {
            continue;
        }
        let rank = forcing_rank(gives_check, is_capture);
        if best.is_none_or(|(r, ..)| rank < r) {
            best = Some((rank, mv, gives_check, is_capture));
        }
    }
    if let Some(s) = saved {
        scratch.undo_null_move(s);
    }
    let (_, firing_move, gives_check, is_capture) = best?;

    // Counter-pin: does one of the defender's own sliders pin/skewer the
    // vehicle to a rear piece? If so, a *quiet* vehicle move would concede
    // the rear — which is exactly why the forcing (checking) firing move
    // matters. Reuse the same scan, pointed the other way.
    let vehicle_counter_pinned = find_latent_threats(pos, !defender_color).iter().any(|t| {
        matches!(
            t.pattern,
            TacticPattern::Pin | TacticPattern::RelativePin | TacticPattern::Skewer
        ) && t.vehicle == Some(vehicle)
    });

    Some(DiscoveryFiring {
        firing_move,
        gives_check,
        is_capture,
        vehicle_counter_pinned,
    })
}

/// The most forcing legal move by the (relatively) pinned piece on
/// `pinned_sq` that **breaks the pin** — a checking relocation after which
/// the pinner (on `pinner_sq`) attacks the rear (`rear_sq`), i.e. the pin's
/// restraint is exposed as illusory. Returns `None` when the piece has no
/// such move: the normal case for an absolute (king-rear) pin, where the
/// piece legally cannot leave the ray and the pin genuinely holds.
///
/// "Forcing" is deliberately narrow — only a check defeats the pin, because
/// only a check denies the pinning side the free tempo it needs to capture
/// the rear. Captures are preferred over quiet checks for the tiebreak so
/// the named move is the one a player would actually reach for.
///
/// `owner` is the pinned piece's colour; the position is null-pivoted when
/// it isn't already `owner`'s turn. Returns `None` if the defender is in
/// check (the pivot would be unsound).
pub fn pin_forcing_escape(
    pos: &Position,
    pinned_sq: Square,
    pinner_sq: Square,
    rear_sq: Square,
    owner: Color,
) -> Option<Move> {
    if pos.in_check() {
        return None;
    }
    let mut scratch = pos.clone();
    let saved = (scratch.side_to_move() != owner).then(|| scratch.do_null_move());

    let mut best: Option<(u8, Move)> = None;
    for mv in legal_moves_vec(&mut scratch) {
        if mv.from() != pinned_sq || !scratch.gives_check(mv) {
            continue;
        }
        let is_capture = scratch.is_capture(mv);
        let mut after = scratch.clone();
        after.do_move(mv);
        // The escape only breaks the pin if it exposes the rear — i.e. the
        // pinner now bears on the rear square. A checking move that stays
        // on the pin ray leaves the rear shielded and doesn't count.
        if !after
            .attackers_to(rear_sq, after.occupied())
            .contains(pinner_sq)
        {
            continue;
        }
        let rank = if is_capture { 0 } else { 1 };
        if best.is_none_or(|(r, _)| rank < r) {
            best = Some((rank, mv));
        }
    }
    if let Some(s) = saved {
        scratch.undo_null_move(s);
    }
    best.map(|(_, mv)| mv)
}

/// Rank a forcing move for "which would the opponent actually play": a
/// checking capture first, then a check, then a quiet capture, then a
/// quiet move. Lower is more forcing.
fn forcing_rank(gives_check: bool, is_capture: bool) -> u8 {
    match (gives_check, is_capture) {
        (true, true) => 0,
        (true, false) => 1,
        (false, true) => 2,
        (false, false) => 3,
    }
}

// ------------------------------------------------------------------------
// helpers
// ------------------------------------------------------------------------

fn points(p: Piece) -> i32 {
    p.kind().classical_points() as i32
}
