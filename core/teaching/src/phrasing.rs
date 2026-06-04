//! The translation layer: [`Claim`] → human-readable [`Phrasing`].
//!
//! This is the **only** home for the "you" vs "they" reframe, the directional
//! chess.com-style reframe (an opponent's blunder becomes *your* chance),
//! verbosity, and i18n. A [`Claim`] is language-free and mover-relative;
//! [`phrase`] applies the [`PhrasingContext`] to produce final, short,
//! mobile-first prose.
//!
//! English-only today (decision C): `locale` and `verbosity` are present in
//! the API and threaded through `phrase`, but each has a single
//! implementation. Matching on them inside `phrase` with one arm now makes
//! adding a variant later a compile-error-driven checklist, not a hunt.

use chess_tutor_engine::analysis::{
    CaptureEvent, EscapeKind, MatePattern, MoveVerdict, PressureKind, SurpriseKind, TacticHit,
    TacticPattern, TermId,
};
use chess_tutor_engine::types::{Color, PieceType, Square, Value};

use crate::claim::{
    AllowedReframe, CastleSide, CenterShift, Claim, CountShift, ForcedConcession,
    InitiativeTemplate, KingSide, MobilitySide, PawnCategory, PawnSide, PlacementCategory,
    PlacementSide, SafetyDirection, ShelterShift, SpaceDirection, SpaceSide, StructureDirection,
    TacticEscapeInfo, TacticRole, ThreatKind, ThreatSide, ThreatTarget,
};
use crate::util::{
    format_attackers, format_delta_pawns, format_score_pawns, format_shelter_pawns, piece_name,
    verdict_label,
};

/// Whose move is being narrated, relative to the user.
///
/// `Player` = the user moved; `Opponent` = the engine/other side moved
/// (`moved_by == user_color` selects `Player`). The directional reframe is a
/// function of `(Claim, Perspective)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Perspective {
    /// The user made the move ("you …").
    Player,
    /// The opponent made the move ("they …", with the reframe to your benefit).
    Opponent,
}

/// Output language. English-only today; the field exists so the seam
/// accommodates future locales without an API change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locale {
    En,
}

/// How much text to emit. `Normal` only today; `Terse` / `Detailed` later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verbosity {
    Normal,
}

/// Everything `phrase` needs beyond the [`Claim`] itself to pick the right
/// wording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhrasingContext {
    /// You vs them — drives the directional reframe.
    pub perspective: Perspective,
    /// Output language (English-only today).
    pub locale: Locale,
    /// Amount of text (Normal-only today).
    pub verbosity: Verbosity,
    /// Mirror of `LearningPreferences.reveal_best_moves`: whether concrete
    /// best-move SAN may be shown.
    pub reveal_moves: bool,
}

/// A rendered teaching point. Short and mobile-first (decision B): a one-line
/// `summary` and an optional `detail` for the expanded view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Phrasing {
    pub summary: String,
    pub detail: Option<String>,
}

/// The chess.com-style **presentation tier** for a verdict.
///
/// This is the *only* place [`MoveVerdict`] (the engine-truth ladder) is
/// remapped to chess.com's headline vocabulary:
///   - `Best + only_good_move`             → "Great"
///   - `Best + only_good_move + sacrifice` → "Brilliant"
///   - everything else                     → the plain [`verdict_label`]
///
/// `only_good_move` gating both tiers is what kills chess.com's classic
/// false-positive: hanging a piece while still up +25 leaves a *second*
/// move that's also winning, so it fails `only_good_move` and never
/// reads as "Brilliant". The label is **perspective-neutral** — only the
/// surrounding sentence flips for "you" vs "they".
pub fn verdict_tier_label(verdict: MoveVerdict, only_good_move: bool, sacrifice: bool) -> &'static str {
    match verdict {
        MoveVerdict::Best if only_good_move && sacrifice => "Brilliant",
        MoveVerdict::Best if only_good_move => "Great",
        other => verdict_label(other),
    }
}

/// Translate a single [`Claim`] into final prose under the given context.
///
/// English-only / `Verbosity::Normal` today (decision C); both are
/// matched with a single arm so adding a variant later is a
/// compile-error-driven checklist, not a hunt.
pub fn phrase(claim: &Claim, ctx: &PhrasingContext) -> Phrasing {
    // Single-arm matches today; widen per-variant in the matching step.
    let Locale::En = ctx.locale;
    let Verbosity::Normal = ctx.verbosity;

    match claim {
        Claim::Verdict {
            verdict,
            mover: _,
            san,
            score,
            best_score,
            gap,
            only_good_move,
            sacrifice,
            best_san,
        } => phrase_verdict(
            ctx,
            *verdict,
            san,
            *score,
            *best_score,
            *gap,
            *only_good_move,
            *sacrifice,
            best_san.as_deref(),
        ),
        Claim::Material {
            mover,
            events,
            net_points,
            net_mg_cp,
            net_eg_cp,
        } => phrase_material(ctx, *mover, events, *net_points, *net_mg_cp, *net_eg_cp),
        Claim::Tactic {
            mover: _,
            role,
            hit,
            escape,
            allowed,
        } => phrase_tactic(ctx, *role, hit, escape.as_ref(), allowed.as_ref()),
        Claim::Threats { side, kind, pieces } => phrase_threats(ctx, *side, *kind, pieces),
        Claim::KingSafety {
            side,
            direction,
            attackers,
            shield,
            king_sq,
        } => phrase_king_safety(ctx, *side, *direction, attackers.as_ref(), shield.as_ref(), *king_sq),
        Claim::Mobility {
            side,
            piece,
            pre_cp,
            post_cp,
        } => phrase_mobility(ctx, *side, *piece, *pre_cp, *post_cp),
        Claim::PawnStructure {
            side,
            direction,
            categories,
        } => phrase_pawn_structure(ctx, *side, *direction, categories),
        Claim::PassedPawns {
            side,
            direction,
            delta_mg,
        } => phrase_passed_pawns(ctx, *side, *direction, *delta_mg),
        Claim::PiecePlacement {
            side,
            category,
            direction,
            delta_mg,
        } => phrase_piece_placement(ctx, *side, *category, *direction, *delta_mg),
        Claim::Space {
            side,
            direction,
            delta_mg,
        } => phrase_space(ctx, *side, *direction, *delta_mg),
        Claim::Initiative {
            mover: _,
            template,
            reply_san,
            reply_is_check,
        } => phrase_initiative(ctx, *template, reply_san, *reply_is_check),
        Claim::Secondary { terms } => phrase_secondary(ctx, terms),
        Claim::ForcedConsequence {
            mover: _,
            reply_san,
            category,
            delta_mg,
        } => phrase_forced_consequence(ctx, reply_san, *category, *delta_mg),
        Claim::Desperado {
            mover: _,
            san,
            recovered_cp,
        } => phrase_desperado(ctx, san, *recovered_cp),
        Claim::OverrideNote {
            mover: _,
            static_pawns,
            search_pawns,
        } => phrase_override_note(ctx, *static_pawns, *search_pawns),
        Claim::DepthHonesty { mover: _ } => phrase_depth_honesty(ctx),
        Claim::Surprise {
            mover: _,
            verdict,
            kind,
        } => phrase_surprise(ctx, *verdict, *kind),
        Claim::CenterStructure { mover: _, kind } => phrase_center_structure(ctx, *kind),
        Claim::CastlingLoss { side } => phrase_castling_loss(ctx, *side),
        Claim::PositionalWin {
            mover: _,
            sacrificed_points,
            dominant_term,
            term_pre_cp,
            term_post_cp,
        } => phrase_positional_win(ctx, *sacrificed_points, *dominant_term, *term_pre_cp, *term_post_cp),
        Claim::MissedProphylaxis {
            mover: _,
            prophylactic_san,
            punisher_san,
            exploded_term,
            swing_cp: _,
        } => phrase_missed_prophylaxis(
            ctx,
            prophylactic_san.as_deref(),
            punisher_san,
            *exploded_term,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn phrase_verdict(
    ctx: &PhrasingContext,
    verdict: MoveVerdict,
    san: &str,
    score: chess_tutor_engine::types::Value,
    best_score: chess_tutor_engine::types::Value,
    gap: chess_tutor_engine::types::Value,
    only_good_move: bool,
    sacrifice: bool,
    best_san: Option<&str>,
) -> Phrasing {
    let tier = verdict_tier_label(verdict, only_good_move, sacrifice);
    // "you" vs "they" is the only directional switch; the verdict word
    // itself is perspective-neutral (see `verdict_tier_label`).
    let subject = match ctx.perspective {
        Perspective::Player => "You played",
        Perspective::Opponent => "They played",
    };
    let user_score = format_score_pawns(score);
    // Verdict-derived SAN annotation glyph for the prose form
    // ("Qxf7?? — Blunder"). The sharp `!` is a separate, surprise-driven
    // signal handled by callers, so it isn't applied here.
    let annotated_san = format!(
        "{san}{}",
        crate::util::sharp_or_verdict_annotation(verdict, false)
    );

    // The directional reframe: a mover's *blunder / mistake / miss* is the
    // opponent's *opportunity* — from your side, "they … (your chance)".
    let opponent_chance = matches!(ctx.perspective, Perspective::Opponent)
        && matches!(
            verdict,
            MoveVerdict::Mistake | MoveVerdict::Blunder | MoveVerdict::Miss
        );

    let summary = match verdict {
        MoveVerdict::Best | MoveVerdict::Good => {
            format!("{subject} {annotated_san} — {tier} ({user_score}).")
        }
        MoveVerdict::BestAvailable => {
            format!("{subject} {annotated_san} — {tier}.")
        }
        MoveVerdict::Inaccuracy
        | MoveVerdict::Mistake
        | MoveVerdict::Blunder
        | MoveVerdict::Miss => {
            let best_score_str = format_score_pawns(best_score);
            let gap_str = format_delta_pawns(gap.0);
            let base = format!(
                "{subject} {annotated_san} — {tier} ({user_score} vs {best_score_str} best, Δ {gap_str})."
            );
            if opponent_chance {
                format!("{base} Your chance.")
            } else {
                base
            }
        }
    };

    let detail = match verdict {
        MoveVerdict::BestAvailable => Some(format!(
            "Position was already lost ({}).",
            format_score_pawns(best_score)
        )),
        MoveVerdict::Miss => Some(match ctx.perspective {
            Perspective::Player => {
                "A stronger move won material here — this one let it slip.".to_string()
            }
            Perspective::Opponent => {
                "A stronger move won material for them — they let it slip.".to_string()
            }
        }),
        _ => None,
    };

    // Best-move reveal: append the engine's preferred line to the detail
    // only when the caller opted in and a distinct alternative exists.
    let detail = match (ctx.reveal_moves, best_san) {
        (true, Some(bs)) => {
            let reveal = format!("Engine preferred {bs} ({}).", format_score_pawns(best_score));
            Some(match detail {
                Some(d) => format!("{d} {reveal}"),
                None => reveal,
            })
        }
        _ => detail,
    };

    Phrasing { summary, detail }
}

/// Phrase a [`Claim::Material`]. The nets are already in `mover`-relative
/// terms (positive = the mover came out ahead). This is the only place the
/// directional reframe lives:
///
/// | net           | Player (you moved)        | Opponent (they moved)                 |
/// |---------------|---------------------------|---------------------------------------|
/// | net > 0 (won) | "You won a bishop"        | "They won a bishop"                   |
/// | net < 0 (lost)| "You lost a bishop"       | "They lost a bishop — you win material"|
/// | net == 0      | "Even trade"              | "Even trade"                          |
///
/// The mover's *gain* stays "they …" from the player's side; the mover's
/// *loss* reframes to the player's benefit. A `Color` is never read here —
/// "you" vs "they" comes purely from `ctx.perspective`.
fn phrase_material(
    ctx: &PhrasingContext,
    mover: Color,
    events: &[CaptureEvent],
    net_points: i32,
    net_mg_cp: i32,
    net_eg_cp: i32,
) -> Phrasing {
    let mover_moved = match ctx.perspective {
        Perspective::Player => "You",
        Perspective::Opponent => "They",
    };

    let summary = match net_points.signum() {
        0 => "Even trade".to_string(),
        1 => {
            // Mover gained — never reframed; just "you/they won …".
            let gain = describe_swing(net_points, events, mover, /* mover_is_winner */ true);
            format!("{mover_moved} won {gain}")
        }
        _ => {
            // Mover lost. From the player's own side it's a plain loss; from
            // the opponent's side it's the player's gain (the reframe).
            let loss = describe_swing(-net_points, events, mover, /* mover_is_winner */ false);
            match ctx.perspective {
                Perspective::Player => format!("You lost {loss}"),
                Perspective::Opponent => format!("They lost {loss} — you win material"),
            }
        }
    };

    // Detail: the engine's tapered-cp read, surfaced only when it adds
    // information the classical point count hides — an even-by-points trade
    // that the engine still leans on (a B-for-N swap, a phase-dependent
    // imbalance). On a clear point swing the headline already tells the
    // story, so we stay quiet.
    let detail = if net_points == 0 && events.is_empty() {
        None
    } else if net_points == 0 {
        let lean = if net_mg_cp == 0 && net_eg_cp == 0 {
            format!("{} captures, balanced.", events.len())
        } else {
            format!(
                "{} captures, even by point value ({} engine cp).",
                events.len(),
                format_delta_pawns(net_mg_cp)
            )
        };
        Some(lean)
    } else {
        None
    };

    Phrasing { summary, detail }
}

/// Describe a material swing of `magnitude` classical points: a clean
/// single-piece win/loss reads as "a bishop" / "a pawn"; anything else
/// reads as "N points (knight for bishop + pawn)" — the captured-piece
/// ledger from the winning side's POV.
///
/// `mover_is_winner` selects whose captures are the "won" pile: when the
/// mover came out ahead its captures are the gains; when it came out behind
/// the opponent's captures are. The reframe to "you/they" never reaches
/// here — this is pure piece bookkeeping.
fn describe_swing(
    magnitude: i32,
    events: &[CaptureEvent],
    mover: Color,
    mover_is_winner: bool,
) -> String {
    let winner: Color = if mover_is_winner { mover } else { !mover };
    let won: Vec<PieceType> = events
        .iter()
        .filter(|ev| ev.captor == winner)
        .map(|ev| ev.captured_piece)
        .collect();
    let lost: Vec<PieceType> = events
        .iter()
        .filter(|ev| ev.captor != winner)
        .map(|ev| ev.captured_piece)
        .collect();

    // Clean single-piece swing with nothing given back: "a bishop".
    if lost.is_empty() {
        if let [only] = won.as_slice() {
            return article(piece_name(*only));
        }
    }
    let headline = if magnitude == 1 {
        "a pawn".to_string()
    } else {
        format!("{magnitude} points")
    };
    if lost.is_empty() {
        headline
    } else {
        format!(
            "{headline} ({} for {})",
            list_pieces(&won),
            list_pieces(&lost)
        )
    }
}

/// Comma-joined captured-piece phrase: `[Knight, Pawn]` → `"knight + pawn"`.
/// Empty → `"nothing"` (defensive; callers gate on a non-empty winner pile).
fn list_pieces(pieces: &[PieceType]) -> String {
    if pieces.is_empty() {
        return "nothing".to_string();
    }
    pieces
        .iter()
        .map(|p| piece_name(*p))
        .collect::<Vec<_>>()
        .join(" + ")
}

/// `"a bishop"` / `"an enemy"` — the indefinite article for a piece name.
fn article(name: &str) -> String {
    let first = name.chars().next().unwrap_or('x').to_ascii_lowercase();
    if matches!(first, 'a' | 'e' | 'i' | 'o' | 'u') {
        format!("an {name}")
    } else {
        format!("a {name}")
    }
}

// =========================================================================
// Tactic
// =========================================================================

/// Phrase a [`Claim::Tactic`]. The role + perspective together pick the
/// directional reframe — this is the **only** place it lives:
///
/// | role        | Player (you moved)        | Opponent (they moved)                        |
/// |-------------|---------------------------|----------------------------------------------|
/// | Played      | "You played a fork"       | "They forked you"                            |
/// | Missed      | "You missed a fork"       | "They missed a fork"                         |
/// | WalkedInto  | "You walked into a fork"  | "You get a chance — they walked into a fork" |
///
/// The mover's *gain* (Played) stays "they …" from the player's side; the
/// mover's *loss* (WalkedInto) reframes to the player's benefit. The
/// per-pattern lesson, the escape note, and the mate detail go in
/// `detail` and are perspective-neutral.
fn phrase_tactic(
    ctx: &PhrasingContext,
    role: TacticRole,
    hit: &TacticHit,
    escape: Option<&TacticEscapeInfo>,
    allowed: Option<&AllowedReframe>,
) -> Phrasing {
    let summary = tactic_summary(ctx.perspective, role, hit, allowed);
    let detail = tactic_detail(ctx.perspective, role, hit, escape, allowed);
    Phrasing {
        summary,
        detail: Some(detail),
    }
}

/// The card heading — the perspective-correct, role-correct one-liner.
fn tactic_summary(
    perspective: Perspective,
    role: TacticRole,
    hit: &TacticHit,
    allowed: Option<&AllowedReframe>,
) -> String {
    let pattern = pattern_phrase(hit.pattern);
    let mate = mate_suffix(hit);
    match (role, perspective) {
        // -------- Played: the mover executed the tactic ----------------
        (TacticRole::Played, Perspective::Player) => {
            format!("You played {pattern}{mate}")
        }
        (TacticRole::Played, Perspective::Opponent) => {
            // "They forked you" reads better than "They played a fork";
            // fall back to the generic frame for patterns without a verb.
            match tactic_verb_against_you(hit.pattern) {
                Some(verb) => format!("They {verb} you{mate}"),
                None => format!("They played {pattern}{mate}"),
            }
        }
        // -------- Missed: the mover failed to play the tactic ----------
        (TacticRole::Missed, Perspective::Player) => {
            format!("You missed {pattern}{mate}")
        }
        (TacticRole::Missed, Perspective::Opponent) => {
            format!("They missed {pattern}{mate}")
        }
        // -------- WalkedInto: the mover allowed the opponent's tactic --
        // From the player's own side, the mover (you) walked into it — a
        // plain warning. From the opponent's side, the mover (they)
        // walked into it, which is *your* opportunity — the reframe.
        (TacticRole::WalkedInto, Perspective::Player) => match allowed {
            Some(_) => format!("You allowed {pattern}{mate}"),
            None => format!("You walked into {pattern}{mate}"),
        },
        (TacticRole::WalkedInto, Perspective::Opponent) => {
            format!("You get a chance — they walked into {pattern}{mate}")
        }
    }
}

/// The expanded prose — the per-pattern lesson, the escape note, the
/// mate detail, and (for an ALLOWED walked-into) the swing lead.
fn tactic_detail(
    perspective: Perspective,
    role: TacticRole,
    hit: &TacticHit,
    escape: Option<&TacticEscapeInfo>,
    allowed: Option<&AllowedReframe>,
) -> String {
    let lesson = pattern_lesson(hit.pattern);
    let mate = mate_detail(hit);
    match role {
        TacticRole::Played => {
            let sac = if hit.sacrifice && hit.pattern != TacticPattern::Sacrifice {
                " The line also gives up material — a sacrifice — but the engine \
                 confirms the combination is sound: at least equal once it resolves."
            } else {
                ""
            };
            let escape_note = match escape {
                Some(e) => format!(
                    " Watch out, though — it isn't forced: the opponent can wriggle \
                     out with {} ({}).",
                    e.san,
                    escape_kind_phrase(e.kind),
                ),
                None => String::new(),
            };
            format!("{lesson}{mate}{sac}{escape_note}")
        }
        TacticRole::Missed => {
            let escape_note = match escape {
                Some(e) => format!(
                    " That said, it wasn't a clean win: the opponent could have met \
                     it with {} ({}).",
                    e.san,
                    escape_kind_phrase(e.kind),
                ),
                None => String::new(),
            };
            format!("{lesson}{mate}{escape_note}")
        }
        TacticRole::WalkedInto => {
            let escape_note = match escape {
                Some(e) => format!(
                    " The good news: it isn't forced — {} ({}) gets out of it.",
                    e.san,
                    escape_kind_phrase(e.kind),
                ),
                None => String::new(),
            };
            match allowed {
                // ALLOWED-not-MISSED: lead with the swing + the opponent's
                // punishing line, then the per-pattern lesson.
                Some(r) => {
                    let swing = allowed_swing_line(perspective, r);
                    let cont = if r.continuation.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " How it turns out: {}. Read past the move to see the reply that \
                             does the damage.",
                            r.continuation
                        )
                    };
                    format!("{swing}{cont} {lesson}{mate}{escape_note}")
                }
                None => {
                    let (mover_subj, reply_owner) = match perspective {
                        Perspective::Player => ("your move", "the opponent's"),
                        Perspective::Opponent => ("their move", "your"),
                    };
                    format!(
                        "After {mover_subj}, {reply_owner} best response lands this pattern. \
                         {lesson}{mate} It may not be played, but if it is, this is the \
                         cost.{escape_note}"
                    )
                }
            }
        }
    }
}

/// The ALLOWED swing lead, perspective-correct. From the player's side
/// it's a warning about what they conceded; from the opponent's side the
/// same swing is the player's opportunity.
fn allowed_swing_line(perspective: Perspective, r: &AllowedReframe) -> String {
    match perspective {
        Perspective::Player => format!(
            "eval {:+.2} → {:+.2} (your POV) — a {:.1}-pawn swing in the opponent's favour. \
             Your move didn't lose to a better move you missed; it ALLOWED the opponent a \
             strong reply you didn't address.",
            r.best_pawns, r.played_pawns, r.swing_pawns,
        ),
        Perspective::Opponent => format!(
            "eval {:+.2} → {:+.2} (their POV) — a {:.1}-pawn swing your way. Their move \
             ALLOWED you a strong reply; this is your chance.",
            r.best_pawns, r.played_pawns, r.swing_pawns,
        ),
    }
}

/// Lower-case sentence-fragment form of a pattern's name, with article
/// ("a fork", "interference") — used inside the heading templates.
fn pattern_phrase(pattern: TacticPattern) -> &'static str {
    match pattern {
        TacticPattern::Fork => "a fork",
        TacticPattern::HangingCapture => "a free piece",
        TacticPattern::RemovingDefender => "removing the defender",
        TacticPattern::TrappedPiece => "a trapped piece",
        TacticPattern::Pin => "a pin",
        TacticPattern::RelativePin => "a relative pin",
        TacticPattern::Skewer => "a skewer",
        TacticPattern::DiscoveredAttack => "a discovered attack",
        TacticPattern::DiscoveredCheck => "a discovered check",
        TacticPattern::DoubleCheck => "double check",
        TacticPattern::Sacrifice => "a sound sacrifice",
        TacticPattern::Intermezzo => "an in-between move",
        TacticPattern::Deflection => "a deflection",
        TacticPattern::Attraction => "an attraction",
        TacticPattern::Interference => "interference",
        TacticPattern::Clearance => "a clearance",
        TacticPattern::XRay => "an x-ray",
        TacticPattern::AttackingF2F7 => "an attack on f2/f7",
        TacticPattern::UnderPromotion => "an under-promotion",
        TacticPattern::Checkmate => "checkmate",
    }
}

/// A transitive verb for the "They <verb> you" played-tactic frame from
/// the player's defending side. `None` for patterns where no natural
/// "<verb> you" reads cleanly — those fall back to "They played <a …>".
fn tactic_verb_against_you(pattern: TacticPattern) -> Option<&'static str> {
    match pattern {
        TacticPattern::Fork => Some("forked"),
        TacticPattern::Pin | TacticPattern::RelativePin => Some("pinned"),
        TacticPattern::Skewer => Some("skewered"),
        // The rest don't have a clean "<verb> you" form a 1200 reads
        // unambiguously — keep the generic "They played …" frame.
        _ => None,
    }
}

/// " — back-rank mate" / " — smothered mate" suffix for the heading,
/// only for the everyday mates surfaced by default.
fn mate_suffix(hit: &TacticHit) -> String {
    match hit.mate_pattern {
        Some(mp) if mp.surfaced_by_default() => format!(" — {}", mate_phrase(mp)),
        _ => String::new(),
    }
}

fn mate_phrase(mate: MatePattern) -> &'static str {
    match mate {
        MatePattern::BackRank => "back-rank mate",
        MatePattern::Smothered => "smothered mate",
        MatePattern::Anastasia => "Anastasia's mate",
        MatePattern::Hook => "hook mate",
        MatePattern::Arabian => "Arabian mate",
        MatePattern::Boden => "Boden's mate",
        MatePattern::DoubleBishop => "double-bishop mate",
        MatePattern::Dovetail => "dovetail mate",
    }
}

/// Per-pattern teaching paragraph. Short — one or two sentences; the goal
/// is for the student to recognise the *shape*, not memorise a definition.
fn pattern_lesson(pattern: TacticPattern) -> &'static str {
    match pattern {
        TacticPattern::Fork => {
            "A fork is one piece attacking two or more enemy pieces at \
             once — they can't all be defended, so one falls."
        }
        TacticPattern::HangingCapture => {
            "An enemy piece was attacked and undefended — a free piece. \
             Spotting hanging pieces is the most reliable source of \
             material at any level."
        }
        TacticPattern::RemovingDefender => {
            "Capturing the only piece defending another enemy piece — \
             the defender goes, the piece it was guarding falls next."
        }
        TacticPattern::TrappedPiece => {
            "An enemy piece with no safe square: every move it can make \
             loses material or is impossible. Often the player who lands \
             the trapped piece recovers the cost of any sacrifice needed \
             to seal off escape squares."
        }
        TacticPattern::Pin => {
            "An absolute pin: the pinned piece can't move at all, because \
             its own king sits directly behind it. Pin a defender and the \
             piece it defends becomes free; pin an attacker and its threat \
             goes away."
        }
        TacticPattern::RelativePin => {
            "A relative pin: a more valuable piece (not the king) sits \
             behind the pinned one, so moving it loses material. Unlike an \
             absolute pin it's only *usually* binding — the opponent will \
             break it with a forcing move (a check or a winning capture) \
             when that's worth more than the piece behind. Always check \
             for that forcing escape before you count on the pin."
        }
        TacticPattern::Skewer => {
            "A piece attacks two enemy pieces in a line — the more \
             valuable one in front must move, exposing the one behind. \
             The opposite of a pin geometrically; usually decisive when \
             the front piece is the king."
        }
        TacticPattern::DiscoveredAttack => {
            "Moving one piece unmasks an attack from a friend behind it. \
             Discovered attacks combine the threat of the moved piece \
             with the threat of the unmasked one — both must be \
             addressed at once."
        }
        TacticPattern::DiscoveredCheck => {
            "Moving one piece unmasks a check from a friend behind it. \
             Because the king must respond to the check, the moved \
             piece is free to grab something — discovered checks are \
             one of the most powerful tactical motifs."
        }
        TacticPattern::DoubleCheck => {
            "Two pieces give check at the same time. The king must \
             move — blocking and capturing don't work against double \
             check — which usually narrows the response to a single \
             move (or none)."
        }
        TacticPattern::Sacrifice => {
            "Giving up material to gain something more important: \
             a winning attack, a decisive position, or recovering more \
             material later. The engine confirms this combination is \
             sound — at least equal after it resolves."
        }
        TacticPattern::Intermezzo => {
            "An in-between move: instead of making the expected \
             recapture, you insert a forcing move elsewhere first. The \
             opponent must respond to the threat before the original \
             sequence completes — often turning an even trade into a \
             material gain."
        }
        TacticPattern::Deflection => {
            "Pulling an enemy defender off its duty square. The piece \
             it was guarding then falls, or the square it was \
             controlling becomes available."
        }
        TacticPattern::Attraction => {
            "Drawing an enemy piece — usually the king — onto a square \
             where it can be checked or attacked decisively. Often by \
             offering a piece the opponent can't resist taking."
        }
        TacticPattern::Interference => {
            "Cutting the line between an enemy piece and what it was \
             defending. The defender's reach is broken; the piece it \
             was guarding becomes vulnerable."
        }
        TacticPattern::Clearance => {
            "Moving a piece off a square to clear the line for a \
             friend behind it. The clearing move usually carries a \
             threat of its own so the opponent can't take advantage of \
             the tempo lost."
        }
        TacticPattern::XRay => {
            "A battery: two of your pieces line up on the same file or \
             diagonal so when the front one captures, the back one is \
             ready to recapture — usually winning the exchange."
        }
        TacticPattern::AttackingF2F7 => {
            "The f2 (white) and f7 (black) squares are the weakest \
             points near an uncastled king — only the king itself \
             defends them. A piece landing on f7 with the enemy king \
             on e8 is the classic beginner combination."
        }
        TacticPattern::UnderPromotion => {
            "Promoting to a knight (or rook / bishop) instead of a \
             queen — used when the under-promoted piece does something \
             a queen can't: a knight giving immediate mate, or a rook / \
             bishop avoiding stalemate."
        }
        TacticPattern::Checkmate => {
            "A forced sequence ending the game. From here, every \
             opponent reply is met by a continuation that leads to mate."
        }
    }
}

fn mate_detail(hit: &TacticHit) -> &'static str {
    match hit.mate_pattern {
        Some(MatePattern::BackRank) => {
            " The mate uses the back-rank pattern — the enemy king is \
             trapped on its first rank by its own pieces, with the \
             mating piece sliding along that rank."
        }
        Some(MatePattern::Smothered) => {
            " The mate is smothered — the enemy king has no flight \
             squares because every neighbour is occupied by its own \
             pieces, and a knight delivers the final blow."
        }
        _ => "",
    }
}

/// Perspective-neutral phrase for why a refutation works.
fn escape_kind_phrase(kind: EscapeKind) -> &'static str {
    match kind {
        EscapeKind::ForcingCheck => "a forcing check",
        EscapeKind::Zwischenzug => "an in-between capture",
        EscapeKind::DefendsBothTargets => "a single move defending both threats",
        EscapeKind::AdequateRetreat => "moving the attacked piece to safety",
        EscapeKind::CounterAttack => "a counter-threat",
    }
}

// =========================================================================
// Threats
// =========================================================================

/// Phrase a [`Claim::Threats`] group. The reframe is a function of
/// *who owns the threatened piece* relative to the user — this is the
/// **only** place it lives:
///
/// | victim       | hanging                                   | SEE-losing                                       |
/// |--------------|-------------------------------------------|--------------------------------------------------|
/// | your piece   | "Your piece is hanging"                   | "Your piece loses to a trade"                    |
/// | their piece  | "You can win material" (their piece hangs)| "Their piece loses to a trade"                   |
///
/// `ThreatSide::Mover` is the side that moved; combined with the
/// perspective it resolves to whether the threatened piece is the
/// user's (a warning) or the opponent's (an opportunity). The
/// per-piece geometry (squares + attackers) goes in `detail` and is
/// perspective-neutral.
fn phrase_threats(
    ctx: &PhrasingContext,
    side: ThreatSide,
    kind: ThreatKind,
    pieces: &[ThreatTarget],
) -> Phrasing {
    // The threatened piece is the user's when the side that moved is the
    // user (Mover + Player) or when the non-moving side is the user
    // (Opponent + Opponent). Otherwise it's the opponent's — the user's
    // opportunity.
    let victim_is_user = (side == ThreatSide::Mover) == (ctx.perspective == Perspective::Player);

    let summary = threats_summary(victim_is_user, kind, pieces);
    let detail = threats_detail(victim_is_user, kind, pieces);
    Phrasing {
        summary,
        detail: Some(detail),
    }
}

/// The card heading — perspective-correct, kind-correct one-liner.
fn threats_summary(victim_is_user: bool, kind: ThreatKind, pieces: &[ThreatTarget]) -> String {
    let count = pieces.len();
    match kind {
        ThreatKind::Hanging => {
            if victim_is_user {
                if count == 1 {
                    format!("Your {} is hanging", piece_at(&pieces[0]))
                } else {
                    format!("{count} of your pieces are hanging")
                }
            } else if count == 1 {
                format!("You can win material — their {} is hanging", piece_at(&pieces[0]))
            } else {
                format!("You can win material — {count} of their pieces hang")
            }
        }
        ThreatKind::SeeLosing => {
            if victim_is_user {
                if count == 1 {
                    format!("Your {} loses to a trade", piece_at(&pieces[0]))
                } else {
                    format!("{count} of your pieces lose to a trade")
                }
            } else if count == 1 {
                format!("Their {} loses to a trade", piece_at(&pieces[0]))
            } else {
                format!("{count} of their pieces lose to a trade")
            }
        }
        ThreatKind::Pressured(pk) => {
            let verb = pressure_verb_passive(pk);
            let owner = if victim_is_user { "Your" } else { "Their" };
            if count == 1 {
                format!("{owner} {} is being {verb}", piece_at(&pieces[0]))
            } else {
                format!("{owner} pieces are being {verb}")
            }
        }
    }
}

/// The expanded prose — the per-piece attacker geometry. Perspective-
/// neutral; the heading carries the "you" / "they" reframe.
fn threats_detail(victim_is_user: bool, kind: ThreatKind, pieces: &[ThreatTarget]) -> String {
    let lead = match (kind, victim_is_user) {
        (ThreatKind::Hanging, true) => {
            "Attacked and undefended — a free piece for the opponent unless you defend it, \
             move it, or create a bigger threat."
        }
        (ThreatKind::Hanging, false) => {
            "Attacked and undefended — a free piece, and it survives every legal reply, \
             so the win is real."
        }
        (ThreatKind::SeeLosing, true) => {
            "Defended, but the exchange still loses material — the attackers are worth less \
             than what they win."
        }
        (ThreatKind::SeeLosing, false) => {
            "Defended, but the exchange still wins material for you, and it holds against \
             every legal reply."
        }
        (ThreatKind::Pressured(_), _) => {
            "Under pressure — it will have to move or concede something, costing a tempo \
             even if no material is lost."
        }
    };

    let mut lines = vec![lead.to_string()];
    for p in pieces {
        lines.push(format!(
            "{} on {} — {}.",
            capitalize_piece(p.location.piece),
            p.location.square.to_algebraic(),
            format_attackers(&p.attackers),
        ));
    }
    lines.join(" ")
}

/// "knight on d2" — the piece name + its square, for inline headings.
fn piece_at(t: &ThreatTarget) -> String {
    format!("{} on {}", piece_name(t.location.piece), t.location.square.to_algebraic())
}

/// Capitalized piece name for the start of a detail line.
fn capitalize_piece(pt: PieceType) -> String {
    let name = piece_name(pt);
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// Passive verb describing how a piece is being pressured ("harried",
/// "pressured", "kicked"), per Stockfish pattern. Matches the old
/// narrator's vocabulary.
fn pressure_verb_passive(kind: PressureKind) -> &'static str {
    match kind {
        PressureKind::MinorOnMajor => "harried",
        PressureKind::RookOnQueen => "pressured",
        PressureKind::SafePawnThreat => "kicked",
    }
}

// =========================================================================
// King safety
// =========================================================================

/// Phrase a [`Claim::KingSafety`] group. The reframe is a function of
/// *whose king* shifted, relative to the user — this is the **only**
/// place it lives:
///
/// | king        | MoreExposed                                | Safer                                     |
/// |-------------|--------------------------------------------|-------------------------------------------|
/// | your king   | "Your king is more exposed"                | "Your king is safer"                      |
/// | their king  | "You expose the opponent's king"           | "The opponent's king is safer"            |
///
/// `KingSide::Mover` is the side that moved; combined with the
/// perspective it resolves to whether the king is the user's (a
/// warning when exposed, good when safer) or the opponent's (the
/// reframe — exposing it is *your* gain). The attacker / shield
/// clauses are perspective-neutral and go in `detail`.
fn phrase_king_safety(
    ctx: &PhrasingContext,
    side: KingSide,
    direction: SafetyDirection,
    attackers: Option<&CountShift>,
    shield: Option<&ShelterShift>,
    king_sq: Square,
) -> Phrasing {
    // The shifted king is the user's when the moving side is the user
    // (Mover + Player) or the non-moving side is the user (Opponent +
    // Opponent). Otherwise it's the opponent's king.
    let king_is_user = (side == KingSide::Mover) == (ctx.perspective == Perspective::Player);

    let summary = king_safety_summary(king_is_user, direction, attackers, shield, king_sq);
    let detail = king_safety_detail(direction, attackers, shield);
    Phrasing {
        summary,
        detail,
    }
}

/// The card heading — perspective-correct, direction-correct, with the
/// flank-aware attacker clause and the shelter clause.
fn king_safety_summary(
    king_is_user: bool,
    direction: SafetyDirection,
    attackers: Option<&CountShift>,
    shield: Option<&ShelterShift>,
    king_sq: Square,
) -> String {
    let lead = match (direction, king_is_user) {
        (SafetyDirection::MoreExposed, true) => "Your king is more exposed",
        (SafetyDirection::MoreExposed, false) => "You expose the opponent's king",
        (SafetyDirection::Safer, true) => "Your king is safer",
        (SafetyDirection::Safer, false) => "The opponent's king is safer",
    };

    let mut parts = Vec::new();
    if let Some(c) = attackers {
        parts.push(attackers_clause(direction, c, king_sq));
    }
    if let Some(s) = shield {
        parts.push(shelter_clause(king_is_user, direction, s));
    }

    if parts.is_empty() {
        format!("{lead}.")
    } else {
        format!("{lead}: {}.", parts.join(", "))
    }
}

/// The attacker-count clause. For an *exposure* it names the flank
/// when the king sits on an outside file ("3 attackers on the kingside
/// (up from 1)"); for a *safer* shift it prefixes the flank only when
/// known ("kingside attackers down to 1 (from 3)").
fn attackers_clause(direction: SafetyDirection, c: &CountShift, king_sq: Square) -> String {
    match direction {
        SafetyDirection::MoreExposed => {
            let target = flank_side_label(king_sq).unwrap_or("king ring");
            format!("{} attackers on the {target} (up from {})", c.post, c.pre)
        }
        SafetyDirection::Safer => match flank_side_label(king_sq) {
            Some(side) => format!("{side} attackers down to {} (from {})", c.post, c.pre),
            None => format!("attackers down to {} (from {})", c.post, c.pre),
        },
    }
}

/// The pawn-shield clause. The verb depends on direction *and* whose
/// king: weakening your own shield "weakened", cracking the opponent's
/// "cracked"; strengthening either reads "strengthened".
fn shelter_clause(king_is_user: bool, direction: SafetyDirection, s: &ShelterShift) -> String {
    let verb = match (direction, king_is_user) {
        (SafetyDirection::MoreExposed, true) => "pawn shield weakened",
        (SafetyDirection::MoreExposed, false) => "pawn shield cracked",
        (SafetyDirection::Safer, _) => "pawn shield strengthened",
    };
    format!(
        "{verb} ({} → {})",
        format_shelter_pawns(s.pre_mg),
        format_shelter_pawns(s.post_mg),
    )
}

/// The expanded prose — the raw pre→post numbers. Perspective-neutral;
/// the heading carries the "you" / "they" reframe. `None` when no
/// clause fired (defensive; the builder gates on at least one).
fn king_safety_detail(
    direction: SafetyDirection,
    attackers: Option<&CountShift>,
    shield: Option<&ShelterShift>,
) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(c) = attackers {
        lines.push(format!(
            "Attackers on the king ring: {} → {}.",
            c.pre, c.post
        ));
    }
    if let Some(s) = shield {
        lines.push(format!(
            "Pawn shield: {} → {}.",
            format_shelter_pawns(s.pre_mg),
            format_shelter_pawns(s.post_mg),
        ));
    }
    let _ = direction;
    if lines.is_empty() {
        None
    } else {
        Some(lines.join(" "))
    }
}

/// Categorize a king's file as "kingside" (f-h), "queenside" (a-c), or
/// `None` for a central file (d, e) where the flank concept doesn't
/// cleanly apply. Mirrors Stockfish's `KING_FLANK[file]` partitioning;
/// central kings fall back to the generic "king ring" wording.
fn flank_side_label(king_sq: Square) -> Option<&'static str> {
    match king_sq.file().index() {
        0..=2 => Some("queenside"),
        5..=7 => Some("kingside"),
        _ => None,
    }
}

// =========================================================================
// Mobility
// =========================================================================

/// Phrase a [`Claim::Mobility`] shift. The reframe is a function of
/// *whose piece* gained or lost activity, relative to the user — this is
/// the **only** place it lives:
///
/// | piece        | improved                                  | dropped                                       |
/// |--------------|-------------------------------------------|-----------------------------------------------|
/// | your piece   | "Your knight is more active"              | "Your knight is less active"                  |
/// | their piece  | "The opponent's knight is more active"    | "You restrict the opponent's knight"          |
///
/// `MobilitySide::Mover` is the side that moved; combined with the
/// perspective it resolves to whether the piece is the user's (improving
/// it is good, losing reach is the warning) or the opponent's (the
/// reframe — restricting it is *your* gain). "Activity" rather than
/// "mobility": Stockfish's term is a weighted count of squares the piece
/// attacks inside the safe-area bitmap, not the number of legal moves.
/// The raw pre→post numbers go in `detail` and are perspective-neutral.
fn phrase_mobility(
    ctx: &PhrasingContext,
    side: MobilitySide,
    piece: PieceType,
    pre_cp: i32,
    post_cp: i32,
) -> Phrasing {
    // The piece is the user's when the moving side is the user
    // (Mover + Player) or the non-moving side is the user (Opponent +
    // Opponent). Otherwise it's the opponent's — restricting it is the
    // user's gain.
    let piece_is_user = (side == MobilitySide::Mover) == (ctx.perspective == Perspective::Player);
    let improved = post_cp - pre_cp >= 0;
    let name = piece_name(piece);

    let summary = match (piece_is_user, improved) {
        (true, true) => format!("Your {name} is more active"),
        (true, false) => format!("Your {name} is less active"),
        // The opponent's piece improving is a warning; restricting it is
        // the player's opportunity — the reframe.
        (false, true) => format!("The opponent's {name} is more active"),
        (false, false) => format!("You restrict the opponent's {name}"),
    };

    let detail = format!(
        "Activity {} → {} (the squares this {name} attacks inside its safe-area bitmap).",
        format_shelter_pawns(pre_cp),
        format_shelter_pawns(post_cp),
    );

    Phrasing {
        summary,
        detail: Some(detail),
    }
}

// =========================================================================
// Pawn structure
// =========================================================================

/// Phrase a [`Claim::PawnStructure`]. The reframe is a function of
/// *whose structure* shifted, relative to the user — this is the
/// **only** place it lives:
///
/// | structure   | Worsened                                  | Improved                                  |
/// |-------------|-------------------------------------------|-------------------------------------------|
/// | yours       | "Your pawn structure weakened: …"         | "Your pawn structure improved: …"         |
/// | theirs      | "You weakened the opponent's pawns: …"    | "The opponent's pawn structure improved"  |
///
/// `PawnSide::Mover` is the side that moved; combined with the
/// perspective it resolves to whether the structure is the user's (a
/// warning when worsened) or the opponent's (the reframe — weakening it
/// is *your* gain). The sub-term clause list goes in both the heading
/// and the detail; `phrase` maps each [`PawnCategory`] to its
/// direction-correct wording here.
fn phrase_pawn_structure(
    ctx: &PhrasingContext,
    side: PawnSide,
    direction: StructureDirection,
    categories: &[PawnCategory],
) -> Phrasing {
    // The structure is the user's when the moving side is the user
    // (Mover + Player) or the non-moving side is the user (Opponent +
    // Opponent). Otherwise it's the opponent's — weakening it is the
    // user's gain.
    let structure_is_user =
        (side == PawnSide::Mover) == (ctx.perspective == Perspective::Player);

    let clauses: Vec<&'static str> = categories
        .iter()
        .map(|c| pawn_category_phrase(*c, direction))
        .collect();
    let clause_list = clauses.join(", ");

    let lead = match (direction, structure_is_user) {
        (StructureDirection::Worsened, true) => "Your pawn structure weakened",
        (StructureDirection::Worsened, false) => "You weakened the opponent's pawn structure",
        (StructureDirection::Improved, true) => "Your pawn structure improved",
        (StructureDirection::Improved, false) => "The opponent's pawn structure improved",
    };

    let summary = format!("{lead}: {clause_list}.");
    let detail = "Pawn structure is the skeleton of the position — connected, \
                  passed, and healthy pawns support pieces and squares; isolated, \
                  backward, and doubled pawns become long-term weaknesses to defend."
        .to_string();

    Phrasing {
        summary,
        detail: Some(detail),
    }
}

/// Direction-correct wording for one pawn sub-term. Worsened reads as
/// the harm done ("doubled a pawn"), improved as the repair ("resolved a
/// doubled pawn"). Matches the old narrator's vocabulary.
fn pawn_category_phrase(category: PawnCategory, direction: StructureDirection) -> &'static str {
    match (direction, category) {
        (StructureDirection::Worsened, PawnCategory::Connected) => "broke pawn connections",
        (StructureDirection::Worsened, PawnCategory::Isolated) => "isolated a pawn",
        (StructureDirection::Worsened, PawnCategory::Backward) => "created a backward pawn",
        (StructureDirection::Worsened, PawnCategory::Doubled) => "doubled a pawn",
        (StructureDirection::Worsened, PawnCategory::WeakUnopposed) => "exposed a weak pawn",
        (StructureDirection::Worsened, PawnCategory::WeakLever) => "walked into a pawn lever",
        (StructureDirection::Improved, PawnCategory::Connected) => "connected pawns",
        (StructureDirection::Improved, PawnCategory::Isolated) => "reconnected an isolated pawn",
        (StructureDirection::Improved, PawnCategory::Backward) => "freed a backward pawn",
        (StructureDirection::Improved, PawnCategory::Doubled) => "resolved a doubled pawn",
        (StructureDirection::Improved, PawnCategory::WeakUnopposed) => "covered a weak pawn",
        (StructureDirection::Improved, PawnCategory::WeakLever) => "resolved a pawn lever",
    }
}

// =========================================================================
// Passed pawns
// =========================================================================

/// Phrase a [`Claim::PassedPawns`]. The reframe is a function of *whose
/// passers* shifted, relative to the user — this is the **only** place it
/// lives:
///
/// | passers     | Improved (advanced)                       | Worsened (lost ground)                    |
/// |-------------|-------------------------------------------|-------------------------------------------|
/// | yours       | "Your passed pawns advanced"              | "Your passed pawns lost ground"           |
/// | theirs      | "The opponent's passed pawns advanced"    | "You blunted the opponent's passed pawns" |
///
/// `PawnSide::Mover` is the side that moved; combined with the
/// perspective it resolves to whether the passers are the user's (good
/// when they advance) or the opponent's (the reframe — blunting theirs is
/// *your* gain). The signed cp shift goes in the detail (always rendered
/// from the owning side's POV, so a positive number reads as "their
/// passers grew" on an opponent claim).
fn phrase_passed_pawns(
    ctx: &PhrasingContext,
    side: PawnSide,
    direction: StructureDirection,
    delta_mg: i32,
) -> Phrasing {
    // The passers are the user's when the moving side is the user
    // (Mover + Player) or the non-moving side is the user (Opponent +
    // Opponent). Otherwise they're the opponent's — blunting them is the
    // user's gain.
    let passers_are_user =
        (side == PawnSide::Mover) == (ctx.perspective == Perspective::Player);

    let summary = match (direction, passers_are_user) {
        (StructureDirection::Improved, true) => "Your passed pawns advanced.",
        (StructureDirection::Worsened, true) => "Your passed pawns lost ground.",
        (StructureDirection::Improved, false) => "The opponent's passed pawns advanced.",
        (StructureDirection::Worsened, false) => "You blunted the opponent's passed pawns.",
    }
    .to_string();

    let detail = format!(
        "Passed pawns have no enemy pawns ahead on their file or the adjacent files, \
         so nothing blocks the run to promotion. The engine scores them by rank, king \
         proximity, and a clear-path bonus — this side's passed-pawn value moved {}.",
        format_shelter_pawns(delta_mg),
    );

    Phrasing {
        summary,
        detail: Some(detail),
    }
}

// =========================================================================
// Piece placement
// =========================================================================

/// Phrase a [`Claim::PiecePlacement`]. The reframe is a function of
/// *whose piece* shifted, relative to the user — this is the **only**
/// place it lives:
///
/// | placement   | Improved                                   | Worsened                                    |
/// |-------------|--------------------------------------------|---------------------------------------------|
/// | yours       | "Your knight reached an outpost"           | "Your knight lost its outpost"              |
/// | theirs      | "Opponent's knight reached an outpost"     | "You denied the opponent's knight an outpost"|
///
/// `PlacementSide::Mover` is the side that moved; combined with the
/// perspective it resolves to whether the piece is the user's (good
/// when it improves) or the opponent's (the reframe — worsening it is
/// *your* gain). The category + direction together pick the
/// concept-specific wording.
fn phrase_piece_placement(
    ctx: &PhrasingContext,
    side: PlacementSide,
    category: PlacementCategory,
    direction: StructureDirection,
    delta_mg: i32,
) -> Phrasing {
    // The piece is the user's when the moving side is the user
    // (Mover + Player) or the non-moving side is the user (Opponent +
    // Opponent). Otherwise it's the opponent's — worsening it is the
    // user's gain.
    let piece_is_user =
        (side == PlacementSide::Mover) == (ctx.perspective == Perspective::Player);

    let summary = placement_heading(piece_is_user, direction, category).to_string();
    let detail = placement_detail(category).to_string();

    Phrasing {
        summary,
        // Carry the raw shift so a renderer that wants a numeric chip
        // has it; phrasing keeps the prose self-contained otherwise.
        detail: Some(format!(
            "{detail} ({} this side).",
            format_shelter_pawns(delta_mg)
        )),
    }
}

/// The concept-specific, perspective-correct, direction-correct
/// one-liner per piece-placement sub-term. Mirrors the old narrator's
/// and GUI card's vocabulary.
fn placement_heading(
    piece_is_user: bool,
    direction: StructureDirection,
    category: PlacementCategory,
) -> &'static str {
    use PlacementCategory as C;
    use StructureDirection::{Improved, Worsened};
    match (piece_is_user, direction, category) {
        // -------- your piece, improved ----------------------------------
        (true, Improved, C::Outposts) => "Your knight reached an outpost",
        (true, Improved, C::ReachableOutposts) => "Your knight has a route to an outpost",
        (true, Improved, C::MinorBehindPawn) => "Your minor gained pawn cover",
        (true, Improved, C::KingProtector) => "Your minor rallied to defend the king",
        (true, Improved, C::BishopPawns) => "Your bishop freed itself from its pawn chain",
        (true, Improved, C::LongDiagonalBishop) => "Your bishop took the long diagonal",
        (true, Improved, C::RookOnQueenFile) => "Your rook reached the queen's file",
        (true, Improved, C::RookOnOpenFile) => "Your rook took the open file",
        (true, Improved, C::RookOnSemiopenFile) => "Your rook took a semi-open file",
        (true, Improved, C::TrappedRook) => "Your rook escaped its trap",
        (true, Improved, C::WeakQueen) => "Your queen shook off pressure",
        // -------- your piece, worsened ----------------------------------
        (true, Worsened, C::Outposts) => "Your knight lost its outpost",
        (true, Worsened, C::ReachableOutposts) => "Your knight's outpost route closed",
        (true, Worsened, C::MinorBehindPawn) => "Your minor lost its pawn cover",
        (true, Worsened, C::KingProtector) => "Your minor drifted away from the king",
        (true, Worsened, C::BishopPawns) => "Your bishop is blocked by its own pawns",
        (true, Worsened, C::LongDiagonalBishop) => "Your bishop left the long diagonal",
        (true, Worsened, C::RookOnQueenFile) => "Your rook left the queen's file",
        (true, Worsened, C::RookOnOpenFile) => "Your rook left the open file",
        (true, Worsened, C::RookOnSemiopenFile) => "Your rook left a semi-open file",
        (true, Worsened, C::TrappedRook) => "Your rook got trapped",
        (true, Worsened, C::WeakQueen) => "Your queen is under x-ray pressure",
        // -------- opponent's piece, improved (a warning) ----------------
        (false, Improved, C::Outposts) => "Opponent's knight reached an outpost",
        (false, Improved, C::ReachableOutposts) => "Opponent's knight has a route to an outpost",
        (false, Improved, C::MinorBehindPawn) => "Opponent's minor gained pawn cover",
        (false, Improved, C::KingProtector) => "Opponent's minor rallied to defend their king",
        (false, Improved, C::BishopPawns) => "Opponent's bishop freed itself from its pawn chain",
        (false, Improved, C::LongDiagonalBishop) => "Opponent's bishop took the long diagonal",
        (false, Improved, C::RookOnQueenFile) => "Opponent's rook reached your queen's file",
        (false, Improved, C::RookOnOpenFile) => "Opponent's rook took the open file",
        (false, Improved, C::RookOnSemiopenFile) => "Opponent's rook took a semi-open file",
        (false, Improved, C::TrappedRook) => "Opponent's rook escaped its trap",
        (false, Improved, C::WeakQueen) => "Opponent's queen shook off pressure",
        // -------- opponent's piece, worsened (the reframe) --------------
        (false, Worsened, C::Outposts) => "You denied the opponent's knight an outpost",
        (false, Worsened, C::ReachableOutposts) => "You closed the opponent's outpost route",
        (false, Worsened, C::MinorBehindPawn) => "You stripped the opponent's pawn cover",
        (false, Worsened, C::KingProtector) => "Opponent's minor drifted from their king",
        (false, Worsened, C::BishopPawns) => "Opponent's bishop is blocked by their own pawns",
        (false, Worsened, C::LongDiagonalBishop) => "Opponent's bishop left the long diagonal",
        (false, Worsened, C::RookOnQueenFile) => "Opponent's rook left your queen's file",
        (false, Worsened, C::RookOnOpenFile) => "Opponent's rook left the open file",
        (false, Worsened, C::RookOnSemiopenFile) => "Opponent's rook left a semi-open file",
        (false, Worsened, C::TrappedRook) => "You trapped the opponent's rook",
        (false, Worsened, C::WeakQueen) => "You put the opponent's queen under x-ray pressure",
    }
}

/// Short prose explaining what a piece-placement sub-term measures —
/// the card's expand-on-click detail. Perspective-neutral.
fn placement_detail(category: PlacementCategory) -> &'static str {
    match category {
        PlacementCategory::Outposts => {
            "An outpost is a square defended by your own pawn that the opponent's \
             pawns can't kick away. Knights and bishops are powerful on outposts \
             because no minor piece can dislodge them with a single move."
        }
        PlacementCategory::ReachableOutposts => {
            "Your knight is one move away from an outpost — a square defended by \
             your pawn that the opponent's pawns can't reach. Outposts are \
             strongest with a knight on them; this means the route is open."
        }
        PlacementCategory::MinorBehindPawn => {
            "A minor piece directly behind one of your pawns is shielded from \
             captures along its file and tends to support pawn pushes."
        }
        PlacementCategory::KingProtector => {
            "Minor pieces lose a small bonus the further they sit from your own \
             king. Knights and bishops near home help shield the king from attacks."
        }
        PlacementCategory::BishopPawns => {
            "A bishop is penalised for each friendly pawn sitting on its color — \
             those pawns block the bishop's diagonals. Either trade the bishop or \
             push the pawns off its color."
        }
        PlacementCategory::LongDiagonalBishop => {
            "A bishop attacking both central squares along its long diagonal exerts \
             pressure on the center from a single piece."
        }
        PlacementCategory::RookOnQueenFile => {
            "A rook on the same file as the enemy queen exerts latent pressure even \
             with pawns in the way — when the file opens it becomes a tactic."
        }
        PlacementCategory::RookOnOpenFile => {
            "A rook on a file with no pawns of either color controls the entire \
             file. Open files are the rook's natural element."
        }
        PlacementCategory::RookOnSemiopenFile => {
            "A rook on a file with no friendly pawns but enemy pawns can pressure \
             those pawns directly — useful for attacking weak pawns."
        }
        PlacementCategory::TrappedRook => {
            "A rook stuck behind its own king after castling rights are gone has \
             almost no mobility. It blocks the king and contributes nothing."
        }
        PlacementCategory::WeakQueen => {
            "The queen sees a slider x-ray threat against it — a rook or bishop \
             aimed through one intervening piece. A discovered attack can win the \
             queen unless you defuse it."
        }
    }
}

// =========================================================================
// Space
// =========================================================================

/// Phrase a [`Claim::Space`]. The reframe is a function of *whose
/// space* shifted, relative to the user — this is the **only** place it
/// lives:
///
/// | space       | Gained                                     | Lost                                        |
/// |-------------|--------------------------------------------|---------------------------------------------|
/// | yours       | "You gained space"                         | "You lost space"                            |
/// | theirs      | "Opponent gained space"                    | "You squeezed the opponent's space"         |
///
/// `SpaceSide::Mover` is the side that moved; combined with the
/// perspective it resolves to whether the space is the user's (good
/// when gained) or the opponent's (the reframe — squeezing theirs is
/// *your* gain).
fn phrase_space(
    ctx: &PhrasingContext,
    side: SpaceSide,
    direction: SpaceDirection,
    delta_mg: i32,
) -> Phrasing {
    // The space is the user's when the moving side is the user
    // (Mover + Player) or the non-moving side is the user (Opponent +
    // Opponent). Otherwise it's the opponent's.
    let space_is_user = (side == SpaceSide::Mover) == (ctx.perspective == Perspective::Player);

    let summary = match (direction, space_is_user) {
        (SpaceDirection::Gained, true) => "You gained space",
        (SpaceDirection::Lost, true) => "You lost space",
        (SpaceDirection::Gained, false) => "The opponent gained space",
        (SpaceDirection::Lost, false) => "You squeezed the opponent's space",
    }
    .to_string();

    let detail = format!(
        "Stockfish's space term scores the central c–f files across the three ranks in \
         front of the back row. Squares the enemy pawns attack don't count; squares on or \
         behind a friendly pawn that no enemy piece attacks count twice. The bonus is \
         squared by piece count, so space matters most when the board is still full — this \
         side's space value moved {}.",
        format_shelter_pawns(delta_mg),
    );

    Phrasing {
        summary,
        detail: Some(detail),
    }
}

// =========================================================================
// Initiative
// =========================================================================

/// Phrase a [`Claim::Initiative`] — the forcing-hierarchy story. The
/// reframe is a function of `(template, perspective)`: from the player's
/// side it's "your move creates a threat …"; from the opponent's side
/// the same hierarchy is described in "their / your" terms (and a
/// refutation of *their* threat is *your* opportunity).
///
/// `reply_is_check` selects the check-vs-capture wording; a reply that
/// is both narrates as a check (checks dominate captures in the
/// hierarchy — the builder doesn't strip the capture flag, so we honour
/// the precedence here).
fn phrase_initiative(
    ctx: &PhrasingContext,
    template: InitiativeTemplate,
    reply_san: &str,
    reply_is_check: bool,
) -> Phrasing {
    let mover = match ctx.perspective {
        Perspective::Player => "Your",
        Perspective::Opponent => "Their",
    };
    // The forcing reply belongs to the *non-moving* side.
    let replier = match ctx.perspective {
        Perspective::Player => "the opponent",
        Perspective::Opponent => "you",
    };

    let summary = match template {
        InitiativeTemplate::Reinforcement => format!(
            "{mover} move creates a threat {replier} must address — there's no check or \
             capture available to play first."
        ),
        InitiativeTemplate::Refutation => {
            if reply_is_check {
                format!(
                    "{mover} move creates a threat — but {replier} can play {reply_san}, a check \
                     that takes priority. Checks must be answered before any other threat, \
                     so the threat doesn't get a chance to land."
                )
            } else {
                format!(
                    "{mover} move creates a threat — but {replier} can play {reply_san}, a capture \
                     that takes priority. Captures change the material picture immediately, \
                     so the threat sits behind it."
                )
            }
        }
        InitiativeTemplate::HeldDespite => {
            if reply_is_check {
                format!(
                    "{mover} move creates a threat. {} {reply_san} has to be answered first — \
                     checks come before threats — but after the dust settles, the threat still \
                     lands.",
                    capitalize_first(replier),
                )
            } else {
                format!(
                    "{mover} move creates a threat. {} {reply_san} addresses the material \
                     first, but it doesn't actually save the position — the threat still lands.",
                    capitalize_first(replier),
                )
            }
        }
    };

    let detail = "Checks, captures, and threats form a forcing hierarchy: a check must be \
                  answered before any other threat, a capture changes the material picture \
                  before a quiet threat resolves. Processing replies in that order is the \
                  habit that turns calculation into a reliable skill."
        .to_string();

    Phrasing {
        summary,
        detail: Some(detail),
    }
}

/// Capitalize the first character of a word (`"the opponent"` →
/// `"The opponent"`, `"you"` → `"You"`).
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

// =========================================================================
// Secondary terms
// =========================================================================

/// Phrase a [`Claim::Secondary`] — the fallback "other shifts" list.
/// The deltas are already mover-POV (positive = helped the mover), so
/// the helped/hurt split is perspective-neutral in *content*; only the
/// heading flips ("you" framing isn't used — the headings stay terse
/// "Also helped" / "Also hurt", matching the prior renderers). The
/// perspective is accepted for API uniformity and future "you/they"
/// wording without an API change.
fn phrase_secondary(ctx: &PhrasingContext, terms: &[(TermId, i32)]) -> Phrasing {
    let _ = ctx.perspective;
    let (helped, hurt): (Vec<_>, Vec<_>) =
        terms.iter().copied().partition(|(_, cp)| *cp > 0);

    let mut lines = Vec::new();
    if !helped.is_empty() {
        lines.push(format!("Also helped: {}", format_term_list(&helped)));
    }
    if !hurt.is_empty() {
        lines.push(format!("Also hurt: {}", format_term_list(&hurt)));
    }

    let summary = match (helped.is_empty(), hurt.is_empty()) {
        (false, false) => format!("{} helped, {} hurt", helped.len(), hurt.len()),
        (false, true) => format!("{} helped", helped.len()),
        (true, false) => format!("{} hurt", hurt.len()),
        // Defensive: the builder returns `None` for an empty list.
        (true, true) => "no other shifts".to_string(),
    };

    Phrasing {
        summary,
        detail: Some(lines.join("\n")),
    }
}

/// Render a term-delta list, biggest-|cp|-first, as
/// `"King safety +0.80, Mobility +0.40"`. The cp values are on the
/// canonical pawn=100 scale the term ledger uses.
fn format_term_list(rows: &[(TermId, i32)]) -> String {
    let mut sorted: Vec<&(TermId, i32)> = rows.iter().collect();
    sorted.sort_by_key(|(_, cp)| std::cmp::Reverse(cp.abs()));
    sorted
        .iter()
        .map(|(term, cp)| format!("{} {:+.2}", term.pretty_label(), *cp as f32 / 100.0))
        .collect::<Vec<_>>()
        .join(", ")
}

// =========================================================================
// Special UI narratives
// =========================================================================

/// Phrase a [`Claim::ForcedConsequence`] — a structural concession the
/// *non-moving* side's best reply creates on its own side. The reframe
/// is a function of `(perspective)`: the replier is the non-mover, so
/// from the player's side the concession lands on "them" (your gain),
/// from the opponent's side it lands on "you". Never says "this forces"
/// — only "if they reply X".
fn phrase_forced_consequence(
    ctx: &PhrasingContext,
    reply_san: &str,
    category: ForcedConcession,
    delta_mg: i32,
) -> Phrasing {
    // The replier is the *non-moving* side.
    let (replier_subj, replier_poss) = match ctx.perspective {
        Perspective::Player => ("they", "their"),
        Perspective::Opponent => ("you", "your"),
    };
    let concession = forced_concession_phrase(category);

    let summary =
        format!("If {replier_subj} reply {reply_san}, {replier_subj} get {concession}.");
    let detail = format!(
        "After the move and {replier_poss} best response {reply_san}, {replier_poss} pawn \
         structure picks up {concession} — about {} pawns by the engine's count. It's a \
         long-term concession on {replier_poss} side: {replier_subj} may decide not to reply \
         this way, but if {replier_subj} do, this is the structural cost.",
        format_shelter_pawns(delta_mg),
    );

    Phrasing {
        summary,
        detail: Some(detail),
    }
}

/// The pawn-weakness wording for a [`ForcedConcession`].
fn forced_concession_phrase(category: ForcedConcession) -> &'static str {
    match category {
        ForcedConcession::Doubled => "doubled pawns",
        ForcedConcession::Isolated => "an isolated pawn",
        ForcedConcession::Backward => "a backward pawn",
        ForcedConcession::WeakUnopposed => "a weak pawn on a half-open file",
    }
}

/// Phrase a [`Claim::Desperado`] — a doomed piece cashing material with a
/// same-tempo capture-with-check before it falls. The reframe is a
/// function of `(perspective)`: the desperado is the *mover's* resource,
/// so it's "your" piece for the player, "their" piece for the opponent.
fn phrase_desperado(ctx: &PhrasingContext, san: &str, recovered_cp: i32) -> Phrasing {
    // `recovered_cp` is a midgame piece value (PAWN_MG-scaled), so a
    // captured pawn = one human "pawn".
    let recovered_pawns = recovered_cp as f32 / Value::PAWN_MG.0 as f32;
    let (mover_subj, mover_poss) = match ctx.perspective {
        Perspective::Player => ("you", "you"),
        Perspective::Opponent => ("they", "they"),
    };

    let summary = format!(
        "Desperado — {san} grabs a pawn on the way down (~{recovered_pawns:.0})."
    );
    let detail = format!(
        "That piece was going to be lost, so before it falls it captures with check ({san}). The \
         check has to be answered first, which buys the tempo to recover the piece — so instead of \
         losing it for nothing, {mover_subj} trade it off having pocketed ~{recovered_pawns:.0} \
         pawn(s). In the ledger that turns a clean loss into a roughly even one: {mover_subj} go \
         down the piece but {mover_poss} collect material on the way, rather than '{mover_subj}'re \
         fine'."
    );

    Phrasing {
        summary,
        detail: Some(detail),
    }
}

/// Phrase a [`Claim::OverrideNote`] — the move where the per-term ledger
/// would point you the *other* way, yet search overrules it. Never names
/// a positional virtue for the recommended move (the static price it
/// pays is real). The reframe is a function of `(perspective)`: the
/// "prettier static" move is the *mover's*.
fn phrase_override_note(
    ctx: &PhrasingContext,
    static_pawns: f32,
    search_pawns: f32,
) -> Phrasing {
    let mover_poss = match ctx.perspective {
        Perspective::Player => "your",
        Perspective::Opponent => "their",
    };

    let summary = format!(
        "The term breakdown is misleading here — it favours {mover_poss} move (~{static_pawns:.1}), \
         but the search favours the other (~{search_pawns:.1})."
    );
    let detail = format!(
        "Read the term breakdown alone and it would tell you the opposite — {mover_poss} move keeps \
         the prettier static eval (by about {static_pawns:.1} pawns of named terms: king attack, \
         mobility, piece placement). The search overrules it by about {search_pawns:.1} pawns, \
         because those terms are built on something the one-ply breakdown can't see: {mover_poss} \
         move lets the opponent equalise, and the activity the static score is crediting \
         evaporates. This is the case to trust the search over the ledger — the recommended move \
         pays a real, visible positional price to deny the opponent that resource. Don't read it \
         as the prettier positional move; read it as the correctly cautious one."
    );

    Phrasing {
        summary,
        detail: Some(detail),
    }
}

/// Phrase a [`Claim::DepthHonesty`] — a move worse only past practical
/// calculation depth, with no detector firing. Honest about the layer's
/// own limits: no blunder stamp, no fabricated mechanism. The reframe is
/// a function of `(perspective)`: the "you didn't miss anything" framing
/// is for the player; for the opponent it's a neutral observation.
fn phrase_depth_honesty(ctx: &PhrasingContext) -> Phrasing {
    let summary =
        "No shorter lesson here — the engine sees trouble several moves out, beyond practical \
         calculation depth."
            .to_string();
    let detail = match ctx.perspective {
        Perspective::Player => {
            "The engine evaluates this as worse than its top choice, but the difference doesn't \
             resolve until well past the depth a person can calculate over the board, and no \
             tactic, pin, fork, or loose piece explains it. This isn't a move you should feel you \
             missed — there isn't a shorter, teachable reason. Not every move the engine dislikes \
             has a lesson a human could have used."
                .to_string()
        }
        Perspective::Opponent => {
            "The engine evaluates this as worse than its top choice, but the difference doesn't \
             resolve until well past the depth a person can calculate over the board, and no \
             tactic, pin, fork, or loose piece explains it. There isn't a shorter, teachable \
             reason here — not every move the engine dislikes has a lesson a human could have used."
                .to_string()
        }
    };

    Phrasing {
        summary,
        detail: Some(detail),
    }
}

/// Phrase a [`Claim::Surprise`] — the shallow-vs-deep tag. The salience
/// (which `(verdict, kind)` combinations surface) lives in
/// [`crate::claim::surprise_claim`]; here we only render the two it
/// passes:
///   - `Best / Good` + `LooksBadButGood` → a positive surprise (the move
///     looks risky but pays off);
///   - `Inaccuracy / Mistake` + `LooksGoodButBad` → the main teaching
///     case (looks reasonable, the follow-up favours the opponent).
///
/// Phrasing intentionally avoids strong chess terminology ("refutes"):
/// the shallow-vs-deep delta threshold is low enough that a tagged move
/// isn't necessarily being refuted in any formal sense — it's just
/// "deeper analysis doesn't like it as much". The reframe flips the
/// subject ("you" vs "they") only.
fn phrase_surprise(
    ctx: &PhrasingContext,
    verdict: MoveVerdict,
    kind: SurpriseKind,
) -> Phrasing {
    let subj = match ctx.perspective {
        Perspective::Player => "you",
        Perspective::Opponent => "they",
    };
    let summary = match (verdict, kind) {
        (MoveVerdict::Best | MoveVerdict::Good, SurpriseKind::LooksBadButGood) => match ctx
            .perspective
        {
            Perspective::Player => {
                "Well spotted — this looks risky at first glance, the longer line pays off."
                    .to_string()
            }
            Perspective::Opponent => {
                "They found a move that looks risky at first glance — the longer line pays off."
                    .to_string()
            }
        },
        (MoveVerdict::Inaccuracy | MoveVerdict::Mistake, SurpriseKind::LooksGoodButBad) => {
            format!(
                "This looked reasonable, but the deeper line doesn't hold up — {subj}'ll be on \
                 the defensive."
            )
        }
        // Defensive: `surprise_claim` only emits the two pairs above.
        _ => String::new(),
    };

    Phrasing {
        summary,
        detail: None,
    }
}

// =========================================================================
// Centre structure (cross-term multiplier)
// =========================================================================

/// Phrase a [`Claim::CenterStructure`] — the closed-centre / barricade
/// story that rides Stockfish's `bishop_pawns` multiplier. Only the
/// *closing* / *barricading* directions name a subject (the mover brought
/// the lock about); the *opening* / *clearing* directions are neutral
/// board-state facts, so the perspective flip only swaps "You" ↔ "They".
fn phrase_center_structure(ctx: &PhrasingContext, kind: CenterShift) -> Phrasing {
    let mover = match ctx.perspective {
        Perspective::Player => "You",
        Perspective::Opponent => "They",
    };
    let summary = match kind {
        CenterShift::Closed => format!(
            "{mover} closed the center: pawn play is locked, cramping bishops behind their own pawns."
        ),
        CenterShift::Opened => {
            "The center opened: bishops and rooks gain scope.".to_string()
        }
        CenterShift::Barricaded => {
            "A piece now sits in front of a central pawn: the pawn can't advance, so the bishop \
             diagonals it would clear stay constrained until the blocker moves."
                .to_string()
        }
        CenterShift::Cleared => {
            "A central pawn's path cleared: the pawn can advance now, freeing the bishop \
             diagonals it had been holding back."
                .to_string()
        }
    };
    Phrasing {
        summary,
        detail: None,
    }
}

// =========================================================================
// Castling loss × trapped rook (cross-term multiplier)
// =========================================================================

/// Phrase a [`Claim::CastlingLoss`] — the castling-loss × trapped-rook
/// doubling. The reframe is a function of `(side, perspective)`: the
/// mover losing its own castling is the warning, the mover stripping the
/// opponent's is *your* gain.
fn phrase_castling_loss(ctx: &PhrasingContext, side: CastleSide) -> Phrasing {
    // The forfeiting side is the user when the mover is the user and it's
    // the mover's own castling (Mover + Player), or when the opponent
    // moved and stripped *your* castling (Opponent + Opponent).
    let forfeiter_is_user =
        (side == CastleSide::Mover) == (ctx.perspective == Perspective::Player);

    let summary = if forfeiter_is_user {
        "You forfeited castling: a rook is locked in by your king with no way to free it — \
         the trapped-rook penalty just doubled."
            .to_string()
    } else {
        "You stripped the opponent of castling: a rook of theirs is locked in by their king \
         with no way out."
            .to_string()
    };
    Phrasing {
        summary,
        detail: None,
    }
}

/// Phrase a [`Claim::PositionalWin`] — the sound-sacrifice justification.
/// Leads with the material cost, then names the compensating positional
/// term; the pre→post term swing (in pawns) goes in the detail. No raw
/// search cp ever appears.
///
/// | perspective | wording                                            |
/// |-------------|----------------------------------------------------|
/// | Player      | "Worth it: you give up {material}, but {term} …"   |
/// | Opponent    | "Worth it for them: they give up {material}, …"    |
///
/// `sacrificed_points` is mover-POV (negative = down material); the card
/// only fires on a real sacrifice, so it's negative here.
fn phrase_positional_win(
    ctx: &PhrasingContext,
    sacrificed_points: i32,
    dominant_term: TermId,
    term_pre_cp: i32,
    term_post_cp: i32,
) -> Phrasing {
    let material = describe_points_lost(-sacrificed_points);
    let term = dominant_term.pretty_label();
    let subject = match ctx.perspective {
        Perspective::Player => "you give up",
        Perspective::Opponent => "they give up",
    };
    let lead = match ctx.perspective {
        Perspective::Player => "Worth it",
        Perspective::Opponent => "Worth it for them",
    };
    let possessive = match ctx.perspective {
        Perspective::Player => "your",
        Perspective::Opponent => "their",
    };
    let summary = format!(
        "{lead}: {subject} {material}, but {term} swings hard in {possessive} favour."
    );
    // Detail: the pre→post swing of the dominant term, in pawns. Never
    // the raw search number — the static term diff is the teaching point.
    let detail = Some(format!(
        "{} goes {} → {} (positional compensation, excluding any material won back).",
        capitalize(term),
        format_delta_pawns(term_pre_cp),
        format_delta_pawns(term_post_cp),
    ));
    Phrasing { summary, detail }
}

/// Phrase a [`Claim::MissedProphylaxis`] — the user's move allowed a deep
/// punishing line the engine's best move would have prevented. Names the
/// punisher (that IS the teaching — see it coming), the static term that
/// collapses, and the prophylactic move when `reveal_moves` is on.
///
/// | perspective | wording                                                       |
/// |-------------|---------------------------------------------------------------|
/// | Player      | "You needed to stop {punisher} — otherwise {term} collapses." |
/// | Opponent    | "Your opponent left {punisher} on — now it wins ({term})."    |
///
/// `prophylactic_san` is `Some` only when the caller opted into the
/// best-move reveal; when present, the Player wording names the move
/// ("You needed Ra8 to stop Rxe7+"), otherwise it teaches the concept
/// without spoiling the move.
fn phrase_missed_prophylaxis(
    ctx: &PhrasingContext,
    prophylactic_san: Option<&str>,
    punisher_san: &str,
    exploded_term: TermId,
) -> Phrasing {
    let term = exploded_term.pretty_label();
    let summary = match ctx.perspective {
        Perspective::Player => match prophylactic_san {
            Some(prophy) => {
                format!("You needed {prophy} to stop {punisher_san} — otherwise {term} collapses.")
            }
            None => {
                format!("You needed a quiet move to stop {punisher_san} — otherwise {term} collapses.")
            }
        },
        // Opponent perspective is the *opportunity* reframe: the opponent
        // skipped the defence, so the punisher is now *your* winning move.
        Perspective::Opponent => match prophylactic_san {
            Some(prophy) => {
                format!("Your opponent skipped {prophy}; {punisher_san} now wins — {term}.")
            }
            None => {
                format!("Your opponent skipped the defence; {punisher_san} now wins — {term}.")
            }
        },
    };
    // Detail: name the lesson — prophylaxis is "stop their move," not
    // "build your own." Short and concrete; the punisher is the receipt.
    let detail = Some(match ctx.perspective {
        Perspective::Player => format!(
            "{punisher_san} was the move to prevent — a quiet defence removes it; this move left it on."
        ),
        Perspective::Opponent => format!(
            "{punisher_san} was theirs to stop — they left it on, so it's yours to play."
        ),
    });
    Phrasing { summary, detail }
}

/// Describe a material deficit of `points` (always ≥ 0 here) as a short
/// human phrase — "a pawn" for 1, "N points" otherwise. Mirrors the
/// material card's terse point bookkeeping without a captured-piece
/// ledger (the sacrifice card doesn't have the per-piece events).
fn describe_points_lost(points: i32) -> String {
    match points {
        n if n <= 1 => "a pawn".to_string(),
        n => format!("{n} points"),
    }
}

/// Capitalize the first character of a label for sentence-leading prose.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
#[path = "phrasing_tests.rs"]
mod tests;
