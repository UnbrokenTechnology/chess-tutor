//! The language-free **Claim IR**.
//!
//! A [`Claim`] is one salient teaching point about a played move. It carries
//! exactly the structured data the translator ([`crate::phrasing::phrase`])
//! needs to produce prose — **no strings, no "you"**. Direction is stored in
//! mover-relative terms (`mover: Color`, signed mover-POV cp, a
//! [`TacticRole`]); the "you" vs "they" reframe is applied *only* inside
//! `phrase`, never here.
//!
//! A move's retrospective is an ordered `Vec<Claim>` (verdict first, then the
//! category claims in the existing card order), produced by
//! [`claims_for`]. Both the GUI and the CLI consume the same `Vec<Claim>`,
//! killing the duplicated salience logic.
//!
//! Variants are pinned per migration step; this scaffold stands up the enum
//! and its companion roles. Category payloads carrying `/* … */` in the plan
//! are filled in their own migration step.

use chess_tutor_engine::analysis::{
    compute_tactic_outcome, cumulative_prefix, find_desperado, gave_away_advantage,
    is_sacrifice, is_silent_sequencing, list_hanging, list_see_losing, BlockedCenterOutcome,
    CaptureEvent, CastlingOutcome, EscapeKind, HangingPiece, InitiativeOutcome, KingSafetyOutcome,
    MaterialOutcome, MobilityOutcome, MoveAnalysis, MoveVerdict, PassedPawnsOutcome,
    PawnStructureOutcome, PieceLocation, PiecesPositionalOutcome, PressureKind, PressuredPiece,
    win_chances, PriorMove, SpaceOutcome, SurpriseKind, TacticEscape, TacticHit, TacticsOutcome,
    ThreatsOutcome, TermId,
};
use chess_tutor_engine::eval::{
    evaluate, EvalTrace, MobilityBreakdown, PassedBreakdown, PawnsBreakdown, PiecesBreakdown,
};
use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::search::stm_after_ply;
use chess_tutor_engine::types::{Color, Move, PieceType, Score, Value};

/// One salient teaching point about a played move, in mover-relative terms.
///
/// A `Claim` **never** encodes "you": perspective is applied only inside
/// [`crate::phrasing::phrase`].
#[derive(Clone, Debug)]
pub enum Claim {
    /// Headline verdict for the move. `only_good_move` / `sacrifice` are the
    /// inputs the *translator* maps to chess.com's presentation tiers:
    ///   "Great"     = verdict==Best + only_good_move
    ///   "Brilliant" = verdict==Best + only_good_move + sacrifice
    /// `only_good_move` (not just "still winning") is what kills the chess.com
    /// false-positive where hanging a piece at +25 reads as "brilliant": at
    /// +25 the second-best move is also winning, so it fails `only_good_move`'s
    /// "second-best clearly worse" gate. [`MoveVerdict`] itself stays the
    /// engine-truth ladder — the chess.com label mapping lives in `phrase`.
    Verdict {
        verdict: MoveVerdict,
        mover: Color,
        san: String,
        score: Value,
        best_score: Value,
        gap: Value,
        only_good_move: bool,
        sacrifice: bool,
        /// Only populated when `PhrasingContext::reveal_moves`.
        best_san: Option<String>,
    },

    /// Material resolved along the realised window.
    Material {
        mover: Color,
        events: Vec<CaptureEvent>,
        net_points: i32,
        net_mg_cp: i32,
        net_eg_cp: i32,
    },

    /// A named tactic, mover-relative role.
    ///
    /// `hit` carries the pattern, targets, material gain, and mate
    /// geometry — all renderable to prose without a board. `escape` is
    /// the refuting/defending resource, already resolved to SAN at
    /// build time (the builder has the board; `phrase` does not) — same
    /// posture as [`Claim::Verdict`]'s pre-resolved `san`. `allowed`
    /// carries the ALLOWED-not-MISSED reframe (eval swing + opponent's
    /// punishing line) when the move *also* gave away the advantage; it
    /// is `Some` only on a [`TacticRole::WalkedInto`] claim.
    Tactic {
        mover: Color,
        role: TacticRole,
        hit: TacticHit,
        escape: Option<TacticEscapeInfo>,
        allowed: Option<AllowedReframe>,
    },

    /// One king's safety shift across the move, mover-relative.
    ///
    /// `side == KingSide::Mover` is the king of the side that moved
    /// (the engine's `ours_*` snapshots); `KingSide::Opponent` is the
    /// other king (`theirs_*`). `phrase` applies the "you" / "they"
    /// reframe and the directional flip — exposing the *opponent's*
    /// king is *your* gain, exposing *your own* king is the warning.
    ///
    /// `direction` is the salience verdict (more exposed vs safer)
    /// already resolved by the builder (which owns the threshold
    /// gating, the exposure-over-safer precedence, and the endgame
    /// shelter suppression). The structured shifts `phrase` reads for
    /// the clauses live in `attackers` / `shield` / `pressure`;
    /// `king_sq` is the post-move king square, for the flank-aware
    /// wording. `pressure` is the fallback signal — set only when the
    /// distinct attacker count was flat but king-danger rose/fell — and
    /// drives the number-free "under more pressure" wording.
    KingSafety {
        side: KingSide,
        direction: SafetyDirection,
        attackers: Option<CountShift>,
        shield: Option<ShelterShift>,
        pressure: Option<PressureShift>,
        king_sq: chess_tutor_engine::types::Square,
    },

    /// One piece type's mobility ("activity") shift across the move,
    /// on one side, mover-relative.
    ///
    /// `side == MobilitySide::Mover` is the side that moved (the
    /// engine's `ours_*` snapshots); `MobilitySide::Opponent` is the
    /// other side (`theirs_*`). `phrase` applies the "you" / "they"
    /// reframe and the directional flip — improving *your own* piece's
    /// reach is good, restricting the *opponent's* is *your* gain. The
    /// raw pre/post midgame engine-cp values (`pre_cp` / `post_cp`) are
    /// what `phrase` renders; the sign of `post_cp - pre_cp` is the
    /// direction. The builder ([`mobility_claims`]) owns the threshold
    /// gating and the biggest-first ordering.
    Mobility {
        side: MobilitySide,
        piece: PieceType,
        pre_cp: i32,
        post_cp: i32,
    },

    /// One side's pawn-structure shift across the move, mover-relative.
    ///
    /// `side == PawnSide::Mover` is the side that moved (the engine's
    /// `ours_*` snapshots); `PawnSide::Opponent` is the other side
    /// (`theirs_*`). `phrase` applies the "you" / "they" reframe and the
    /// directional flip — weakening *your own* structure is the warning,
    /// weakening the *opponent's* is *your* gain.
    ///
    /// `direction` is the salience verdict (worsened vs improved) already
    /// resolved by the builder (which owns the per-side worsened-over-
    /// improved precedence). `categories` is the language-free list of
    /// which sub-terms moved in that direction; `phrase` maps each to its
    /// worsened/improved wording. Non-empty by construction.
    PawnStructure {
        side: PawnSide,
        direction: StructureDirection,
        categories: Vec<PawnCategory>,
    },

    /// One side's passed-pawn shift across the move, mover-relative.
    ///
    /// `side == PawnSide::Mover` is the side that moved (the engine's
    /// `ours_*` snapshots); `PawnSide::Opponent` is the other side
    /// (`theirs_*`). `phrase` applies the "you" / "they" reframe and the
    /// directional flip — your own passers advancing is good, the
    /// opponent's passers advancing is the warning (and blunting theirs is
    /// *your* gain). `direction` is the salience verdict (advanced vs lost
    /// ground); `delta_mg` is the side's aggregate midgame passed-pawn cp
    /// shift (signed, side-relative: positive = that side's passers grew).
    PassedPawns {
        side: PawnSide,
        direction: StructureDirection,
        delta_mg: i32,
    },

    /// One piece-placement sub-term's shift across the move, on one
    /// side, mover-relative.
    ///
    /// `side == PlacementSide::Mover` is the side that moved (the
    /// engine's `ours_*` snapshots); `PlacementSide::Opponent` is the
    /// other side (`theirs_*`). `phrase` applies the "you" / "they"
    /// reframe and the directional flip — your own knight claiming an
    /// outpost is good, weakening the *opponent's* placement is *your*
    /// gain. `direction` is the salience verdict (improved vs
    /// worsened) already resolved by the builder; `category` names the
    /// sub-term and `phrase` maps it to its direction-correct wording.
    PiecePlacement {
        side: PlacementSide,
        category: PlacementCategory,
        direction: StructureDirection,
        delta_mg: i32,
    },

    /// One side's space shift across the move, mover-relative.
    ///
    /// `side == SpaceSide::Mover` is the side that moved (the engine's
    /// `ours_*` snapshots); `SpaceSide::Opponent` is the other side
    /// (`theirs_*`). `phrase` applies the "you" / "they" reframe and
    /// the directional flip — gaining your own space is good, squeezing
    /// the *opponent's* is *your* gain. `direction` is the salience
    /// verdict (gained vs lost); `delta_mg` is the side's signed midgame
    /// space cp shift (positive = that side gained space).
    Space {
        side: SpaceSide,
        direction: SpaceDirection,
        delta_mg: i32,
    },

    /// One threatened-piece group — a hanging piece, an SEE-losing
    /// exchange, or a piece under Stockfish-pattern pressure, on one
    /// side. Mover-relative: `side == ThreatSide::Mover` is a threat
    /// against the side that moved (the engine's `ours_*` lists),
    /// `ThreatSide::Opponent` is a threat against the other side
    /// (`theirs_*`). `phrase` applies the "you" / "they" reframe and the
    /// directional flip (a threat against the opponent is *your*
    /// opportunity). The structured per-piece geometry the renderers
    /// paint (squares, attackers) stays in `pieces`; `phrase` reads it
    /// for the prose only.
    Threats {
        side: ThreatSide,
        kind: ThreatKind,
        /// Non-empty by construction (the builder skips empty groups).
        pieces: Vec<ThreatTarget>,
    },

    /// The forcing-hierarchy story relating the mover's move to the
    /// opponent's best reply (checks > captures > threats > quiet).
    ///
    /// Always mover-relative: `reply_san` is the *non-moving* side's
    /// reply, already resolved to SAN by the builder (which has the
    /// board; `phrase` does not). `template` is the salience verdict —
    /// the move's threat reinforced, refuted, or held-despite — already
    /// resolved by the builder (which owns the swing gating and the
    /// favoured-side precedence). `reply_is_check` selects the
    /// check-vs-capture wording (a reply that is *both* narrates as a
    /// check — checks dominate captures in the hierarchy). `phrase`
    /// applies the "you" / "they" reframe.
    Initiative {
        mover: Color,
        template: InitiativeTemplate,
        reply_san: String,
        reply_is_check: bool,
    },

    /// A structural concession the *non-moving* side's best reply
    /// creates **on its own side** — the "yes Bxh6 was an even trade,
    /// *and* it doubles their h-pawns" story. Always mover-relative:
    /// `reply_san` is the non-moving side's reply (pre-resolved to SAN
    /// by the builder, which has the board; `phrase` does not), and
    /// `category` is the pawn weakness the reply concedes. `delta_mg`
    /// is the *signed* engine-cp shift in the replier's pawn structure
    /// after the reply — always negative here (a concession), carried
    /// so `phrase` can render the magnitude. `phrase` applies the "you"
    /// / "they" reframe: the concession lands on the replier, so it is
    /// *your* gain when the player moved, *their* gain when the
    /// opponent moved. Never says "this forces" — only "if they reply
    /// X".
    ForcedConsequence {
        mover: Color,
        reply_san: String,
        category: ForcedConcession,
        delta_mg: i32,
    },

    /// The desperado note — a doomed piece that cashes material with a
    /// same-tempo capture-with-check before it falls (`Nxg7+`).
    /// Mover-relative: `san` is the mover's desperado move (pre-resolved
    /// to SAN by the builder), `recovered_cp` the midgame cp the doomed
    /// piece pockets on the way down. `phrase` applies the "you" /
    /// "they" reframe — the desperado is the mover's resource.
    Desperado {
        mover: Color,
        san: String,
        recovered_cp: i32,
    },

    /// The static-vs-search override note — the move where the per-term
    /// ledger *lies*: it would point you toward the mover's move, yet
    /// search prefers the engine's pick by a real margin because the
    /// static terms are built on something the one-ply diff can't see.
    /// Mover-relative: `static_pawns` is how much prettier the mover's
    /// move looks on the static ledger, `search_pawns` how much search
    /// favours the engine's pick — both positive pawns. `phrase`
    /// applies the "you" / "they" reframe and never names a positional
    /// virtue for the recommended move.
    OverrideNote {
        mover: Color,
        static_pawns: f32,
        search_pawns: f32,
    },

    /// The depth-honesty note — a move the engine dislikes whose reason
    /// resolves only past practical calculation depth, with no detector
    /// firing. The retrospective is honest about its own limits: no
    /// blunder stamp, no fabricated mechanism. Mover-relative (carries
    /// `mover` for API uniformity; the content is the same either way —
    /// "there's no shorter, teachable reason"). `phrase` applies the
    /// "you" / "they" reframe to the "you didn't miss anything"
    /// framing.
    DepthHonesty {
        mover: Color,
    },

    /// The shallow-vs-deep surprise tag — a move whose first-glance
    /// read and its deeper evaluation disagree. Verdict-relative: the
    /// `(verdict, kind)` pair decides *whether* (and how) to surface the
    /// tag (the salience lives in [`surprise_claim`]); `phrase` renders
    /// the perspective-correct sentence ("looks risky but pays off" for
    /// the player vs "they found a move that looks risky but pays off"
    /// for the opponent). `mover` carries the reframe target.
    Surprise {
        mover: Color,
        verdict: MoveVerdict,
        kind: SurpriseKind,
    },

    /// The fallback "other shifts" list — the residual eval-term deltas
    /// (after the specialised claims consumed their own terms), in
    /// **mover-POV** signed cp (positive = helped the mover). `phrase`
    /// applies the "you" / "they" reframe to the helped/hurt framing;
    /// the per-term labels are perspective-neutral. Non-empty by
    /// construction (the builder returns `None` for an empty list).
    Secondary {
        terms: Vec<(TermId, i32)>,
    },

    /// A centre-structure cross-term shift — the closed-centre and
    /// own-piece-barricade stories that ride Stockfish's `bishop_pawns`
    /// multiplier. Both are board-state facts the *mover* brought about,
    /// so the reframe is purely the subject ("You closed the centre" vs
    /// "They closed the centre"); the structural consequence is the same
    /// for whoever moved. `kind` is the salience verdict (closed / opened
    /// / barricaded / cleared) already resolved by the builder
    /// ([`center_structure_claims`]), which owns the amplifier gate and
    /// the no-change suppression. `mover` carries the reframe subject.
    CenterStructure {
        mover: Color,
        kind: CenterShift,
    },

    /// The castling-loss × trapped-rook cross-term story — a side just
    /// forfeited its last castling rights while a rook is boxed in by its
    /// own king, so Stockfish doubles the trapped-rook penalty.
    /// Mover-relative: `side == CastleSide::Mover` is the side that moved
    /// losing its own castling (a warning); `CastleSide::Opponent` is the
    /// mover stripping the opponent's castling (*your* gain — the
    /// reframe). `phrase` applies the "you" / "they" flip.
    CastlingLoss {
        side: CastleSide,
    },

    /// The sound-sacrifice justification — the engine's best move ends
    /// **down material** yet the search rates it at least equal, because
    /// a purely *positional* term (king danger, a frozen enemy rook, …)
    /// swings hard in the mover's favour across the forcing tail. The
    /// "you're down a point but it's worth it" lesson, driven off the
    /// STATIC eval term diff (the search under-sells these), explicitly
    /// **excluding** any regained material so the compensation is honest.
    ///
    /// Mover-relative: `sacrificed_points` is the mover's material
    /// balance at the climax minus the baseline (negative = down
    /// material), `dominant_term` the non-material term that carries the
    /// compensation, `term_pre_cp` / `term_post_cp` its mover-POV tapered
    /// engine-cp at the baseline and climax. `phrase` applies the "you" /
    /// "they" reframe (played = praise, missed = "you had this").
    PositionalWin {
        mover: Color,
        sacrificed_points: i32,
        dominant_term: TermId,
        term_pre_cp: i32,
        term_post_cp: i32,
    },

    /// The missed-prophylaxis lesson — the user's move allowed a deep
    /// punishing line that the engine's best move would have **prevented**.
    /// Surfaces *what they needed to stop and why*: "you needed `Ra8` to
    /// stop `Rxe7+` — otherwise king safety collapses." Reframes the flat
    /// "ALLOWED, NOT MISSED" detection into a teachable lesson by naming
    /// the prophylactic move, the punisher, and the static term that
    /// explodes along the user's PV.
    ///
    /// By construction `user.mv != best.mv` and the best line does *not*
    /// explode — confirmed by a replay/disambiguation test (apply the best
    /// move, then re-try the punisher: it must now fail; if it still works
    /// the best move was a *deferred own-tactic*, not prophylaxis, and this
    /// claim is not built).
    ///
    /// Mover-relative: `mover` is the side that played the sub-optimal move,
    /// `prophylactic_san` the best move (pre-resolved to SAN by the builder,
    /// honouring `reveal_moves`), `punisher_san` the opponent reply that
    /// explodes the user's line, `exploded_term` the static term that
    /// worsened at the explosion, `swing_cp` the mover-POV cp lost across it
    /// (sentiment input, not necessarily shown). `phrase` applies the "you"
    /// / "they" reframe — under `Opponent` it is the *opportunity* reframe
    /// ("your opponent skipped {prophylactic}; {punisher} now wins").
    MissedProphylaxis {
        mover: Color,
        /// Best move SAN, or `None` when `reveal_moves` is off (the prose
        /// then teaches the concept without naming the move).
        prophylactic_san: Option<String>,
        punisher_san: String,
        exploded_term: TermId,
        swing_cp: i32,
    },
}

/// Which centre-structure shift a [`Claim::CenterStructure`] carries,
/// language-free. The salience verdict — the builder resolves it from
/// the locked / barricaded count deltas (and the amplifier gate), so
/// `phrase` just renders. Mirrors the two distinct chess concepts the
/// `bishop_pawns` multiplier amplifies:
///
/// - **Closed / Opened** — own central pawn meets an enemy pawn directly
///   ahead (a pawn-on-pawn lock) appearing or dissolving.
/// - **Barricaded / Cleared** — a (usually friendly) piece sitting in
///   front of an own central pawn appearing or moving off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CenterShift {
    /// A new pawn-on-pawn lock appeared in the centre.
    Closed,
    /// A pawn-on-pawn lock dissolved — the centre opened.
    Opened,
    /// A piece landed in front of an own central pawn.
    Barricaded,
    /// A blocker moved off the front of an own central pawn.
    Cleared,
}

/// Which side a [`Claim::CastlingLoss`] bears on, in mover-relative
/// terms. `phrase` maps these to "you" / "they": the mover losing its
/// own castling is the warning, the mover stripping the opponent's is
/// *your* gain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastleSide {
    /// The side that moved forfeited its own castling rights.
    Mover,
    /// The mover stripped the opponent's castling rights.
    Opponent,
}

/// The mover's relationship to a tactic. Mover-relative — `phrase` maps these
/// to "you played / you missed / you walked into" or the opponent reframe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TacticRole {
    /// The mover executed the tactic.
    Played,
    /// The mover failed to play an available tactic.
    Missed,
    /// The mover allowed the opponent's tactic.
    WalkedInto,
}

/// The pawn-structure concession a [`Claim::ForcedConsequence`] reply
/// creates on the replier's side, language-free. A subset of the pawn
/// sub-terms — only the penalty terms that read as a clear concession
/// ("they get doubled pawns"). `phrase` maps each to its wording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForcedConcession {
    Doubled,
    Isolated,
    Backward,
    WeakUnopposed,
}

/// Which side a threat group bears on, in mover-relative terms.
/// `phrase` maps these to "you" / "they" plus the directional reframe
/// (a threat against the opponent is *your* opportunity).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreatSide {
    /// A threat against the side that moved (the engine's `ours_*`
    /// lists — the mover left this piece vulnerable).
    Mover,
    /// A threat against the other side (`theirs_*` — the move exposed
    /// an opponent piece).
    Opponent,
}

/// What flavour of threat a group represents — the three the
/// retrospective surfaces (hanging / SEE-losing / Stockfish-pattern
/// pressure). The pressure kind is carried so `phrase` can pick the
/// pattern-specific verb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreatKind {
    /// Attacked and undefended — a free piece.
    Hanging,
    /// Defended, but the SEE-assessed exchange still loses material.
    SeeLosing,
    /// A Stockfish-evaluator positional threat pattern.
    Pressured(PressureKind),
}

/// One threatened piece plus the enemy pieces hitting it — the common
/// shape of [`HangingPiece`] and [`PressuredPiece`]. Carries only the
/// geometry `phrase` needs (no colour: the side is on the group).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreatTarget {
    pub location: PieceLocation,
    pub attackers: Vec<PieceLocation>,
}

impl From<&HangingPiece> for ThreatTarget {
    fn from(h: &HangingPiece) -> Self {
        ThreatTarget {
            location: h.location,
            attackers: h.attackers.clone(),
        }
    }
}

impl From<&PressuredPiece> for ThreatTarget {
    fn from(p: &PressuredPiece) -> Self {
        ThreatTarget {
            location: p.location,
            attackers: p.attackers.clone(),
        }
    }
}

/// Which king a [`Claim::KingSafety`] bears on, in mover-relative
/// terms. `phrase` maps these to "you" / "they" plus the directional
/// flip (exposing the opponent's king is *your* gain).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KingSide {
    /// The king of the side that moved (the engine's `ours_*`
    /// snapshots).
    Mover,
    /// The other side's king (`theirs_*`).
    Opponent,
}

/// Whether a king got *more exposed* or *safer* over the move. The
/// salience verdict — the builder resolves it (exposure wins over
/// safer when both could fire), so `phrase` just renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SafetyDirection {
    /// Attackers up and/or pawn shield weakened.
    MoreExposed,
    /// Attackers down and/or pawn shield strengthened.
    Safer,
}

/// Which side a [`Claim::Mobility`] bears on, in mover-relative
/// terms. `phrase` maps these to "you" / "they" plus the directional
/// flip (restricting the opponent's piece is *your* gain).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MobilitySide {
    /// The side that moved (the engine's `ours_*` snapshots).
    Mover,
    /// The other side (`theirs_*`).
    Opponent,
}

/// Which side a [`Claim::PawnStructure`] / [`Claim::PassedPawns`] bears
/// on, in mover-relative terms. `phrase` maps these to "you" / "they"
/// plus the directional flip (weakening the opponent's structure, or
/// blunting their passers, is *your* gain).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PawnSide {
    /// The side that moved (the engine's `ours_*` snapshots).
    Mover,
    /// The other side (`theirs_*`).
    Opponent,
}

/// Whether a side's pawn structure / passed pawns got *worse* or
/// *better* over the move. The salience verdict — the builder resolves
/// it (worsening wins over improving when both could fire), so `phrase`
/// just renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StructureDirection {
    /// Structure weakened / passers lost ground.
    Worsened,
    /// Structure improved / passers advanced.
    Improved,
}

/// One pawn-structure sub-term that moved past the narration threshold,
/// language-free. `phrase` maps each to its worsened/improved wording.
/// Mirrors Stockfish's pawn sub-term decomposition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PawnCategory {
    Connected,
    Isolated,
    Backward,
    Doubled,
    WeakUnopposed,
    WeakLever,
}

/// Which side a [`Claim::PiecePlacement`] bears on, in mover-relative
/// terms. `phrase` maps these to "you" / "they" plus the directional
/// flip (weakening the opponent's piece placement is *your* gain).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacementSide {
    /// The side that moved (the engine's `ours_*` snapshots).
    Mover,
    /// The other side (`theirs_*`).
    Opponent,
}

/// One piece-placement sub-term, language-free. Mirrors Stockfish's
/// 11-sub-term per-piece positional decomposition
/// ([`chess_tutor_engine::eval::PiecesBreakdown`]). `phrase` maps each
/// to its worsened/improved wording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacementCategory {
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

/// Which side a [`Claim::Space`] bears on, in mover-relative terms.
/// `phrase` maps these to "you" / "they" plus the directional flip
/// (squeezing the opponent's space is *your* gain).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpaceSide {
    /// The side that moved (the engine's `ours_*` snapshots).
    Mover,
    /// The other side (`theirs_*`).
    Opponent,
}

/// Whether a side *gained* or *lost* space over the move. The salience
/// verdict — the builder resolves it from the signed delta, so `phrase`
/// just renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpaceDirection {
    /// Space score grew for this side.
    Gained,
    /// Space score shrank for this side.
    Lost,
}

/// The three forcing-hierarchy teaching templates a [`Claim::Initiative`]
/// can carry. The salience verdict — the builder selects it (swing
/// gating, favoured-side precedence), so `phrase` just renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitiativeTemplate {
    /// The mover's threat lands — the opponent has no check or capture
    /// to outrank it. Reinforces the rule by showing it works.
    Reinforcement,
    /// The opponent's reply is a check / capture and the line settled
    /// against the mover — the higher-ranking forcing reply the mover
    /// missed. Teaches the rule by showing what went wrong.
    Refutation,
    /// The opponent's reply is a check / capture but the mover stays
    /// favoured. Teaches the limit of the rule: the hierarchy is a
    /// processing order, not a winning rule.
    HeldDespite,
}

/// A king-ring attacker-count shift — pre/post raw counts. Present on
/// a [`Claim::KingSafety`] only when the count actually moved in the
/// claim's direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountShift {
    pub pre: i32,
    pub post: i32,
}

/// A pawn-shield (shelter) shift — pre/post engine-cp midgame
/// components. Present on a [`Claim::KingSafety`] only when the shift
/// clears the narration threshold and the phase isn't deep endgame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShelterShift {
    pub pre_mg: i32,
    pub post_mg: i32,
}

/// A king-*pressure* shift — present on a [`Claim::KingSafety`] only
/// when the **distinct attacker count was flat** but the pressure rose
/// (or fell): more enemy attacks landing on the squares right next to
/// the king, and/or a higher aggregate king-danger score. It's the
/// signal that catches a move which adds a closer, more dangerous
/// attacker while the bare count stays put (a piece blocks one attacker
/// just as another takes its place). `pre` / `post` are the counts of
/// enemy attacks on the king's immediately-adjacent squares — the king's
/// own escape squares; `phrase` renders them only as an expandable
/// detail, never the heading (the visual attacker arrows carry the
/// "pressure" intuition, so the heading stays number-free).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PressureShift {
    pub pre: i32,
    pub post: i32,
}

/// A tactic's defensive/refuting resource, resolved to SAN at build
/// time. For a played/missed tactic this is the *opponent's* out (the
/// tactic isn't forced); for a walked-into tactic it is the *mover's*
/// own escape. SAN is pre-resolved here because rendering a `Move` to
/// SAN needs the board, which the claim builder has and `phrase` does
/// not — the same posture as [`Claim::Verdict`]'s `san`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TacticEscapeInfo {
    /// The refuting move in SAN (`Qxe5`).
    pub san: String,
    /// Why the refutation works — `phrase` glosses this.
    pub kind: EscapeKind,
}

/// The ALLOWED-not-MISSED reframe for a walked-into tactic that *also*
/// gave away the advantage: the eval swing and the opponent's punishing
/// continuation. The swing numbers and the SAN line are structured data
/// the builder resolves (board + scores in hand); `phrase` turns them
/// into the perspective-correct lead. Mirrors the CLI's
/// `print_allowed_banner` content.
#[derive(Clone, Debug, PartialEq)]
pub struct AllowedReframe {
    /// Best line's score in pawns, mover-POV (positive = mover better).
    pub best_pawns: f32,
    /// Played move's score in pawns, mover-POV.
    pub played_pawns: f32,
    /// Magnitude of the swing in pawns (always positive).
    pub swing_pawns: f32,
    /// The opponent's punishing line in SAN, joined with spaces
    /// (`"Qc5+ Kf7 b3 …"`). Empty when the PV was too short.
    pub continuation: String,
}

/// Build the ordered tactic claims for one analysed move — played,
/// missed, and walked-into — from [`compute_tactic_outcome`]. One call
/// covers all three slots; the salience (which slots fire, the escape
/// resolution, the ALLOWED reframe gate) lives here so both the GUI and
/// CLI share it. The returned claims are mover-relative; `phrase`
/// applies perspective.
///
/// - `best_ma` / `user_ma` — the engine's best line and the user's line.
/// - `pre_move_pos` — the position the move was played from (`root_stm`
///   to move).
/// - `root_stm` — the side that moved.
/// - `prior_move` — the opponent's move into `pre_move_pos`, for the
///   hanging-capture recapture guard. Pass `None` with no history.
pub fn tactic_claims(
    pre_move_pos: &Position,
    best_ma: &MoveAnalysis,
    user_ma: &MoveAnalysis,
    root_stm: Color,
    prior_move: Option<PriorMove>,
) -> Vec<Claim> {
    let outcome =
        compute_tactic_outcome(best_ma, user_ma, pre_move_pos, root_stm, prior_move);
    let TacticsOutcome {
        user_played_tactic,
        user_missed_tactic,
        user_walked_into,
        user_played_escape,
        user_missed_escape,
        user_walked_into_escape,
    } = outcome;

    let mut claims = Vec::new();

    if let Some(hit) = user_played_tactic {
        let escape =
            user_played_escape.map(|e| escape_info(pre_move_pos, &user_ma.pv, &hit, &e));
        claims.push(Claim::Tactic {
            mover: root_stm,
            role: TacticRole::Played,
            hit,
            escape,
            allowed: None,
        });
    }
    if let Some(hit) = user_missed_tactic {
        let escape =
            user_missed_escape.map(|e| escape_info(pre_move_pos, &best_ma.pv, &hit, &e));
        claims.push(Claim::Tactic {
            mover: root_stm,
            role: TacticRole::Missed,
            hit,
            escape,
            allowed: None,
        });
    }
    if let Some(hit) = user_walked_into {
        let escape =
            user_walked_into_escape.map(|e| escape_info(pre_move_pos, &user_ma.pv, &hit, &e));
        // ALLOWED-not-MISSED reframe: when the move *also* gave away the
        // advantage, carry the swing + the opponent's punishing line so
        // `phrase` can lead with "what did I let them do?" instead of the
        // bare "if they reply" framing.
        let allowed = gave_away_advantage(best_ma.score, user_ma.score)
            .then(|| allowed_reframe(pre_move_pos, best_ma.score, user_ma.score, &user_ma.pv));
        claims.push(Claim::Tactic {
            mover: root_stm,
            role: TacticRole::WalkedInto,
            hit,
            escape,
            allowed,
        });
    }
    claims
}

/// Resolve a [`TacticEscape`] for `hit` to a [`TacticEscapeInfo`] —
/// formatting the refutation in SAN from the position it is played in
/// (the pre-move position advanced through `full_pv[..hit.pv_ply]` then
/// the hit's key move).
fn escape_info(
    pre: &Position,
    full_pv: &[Move],
    hit: &TacticHit,
    esc: &TacticEscape,
) -> TacticEscapeInfo {
    let mut board = pre.clone();
    for &mv in full_pv.iter().take(hit.pv_ply) {
        board.do_move(mv);
    }
    if let Some(km) = hit.key_move {
        board.do_move(km);
    }
    TacticEscapeInfo {
        san: san::format(&board, esc.refutation),
        kind: esc.kind,
    }
}

/// Build the [`AllowedReframe`] for a walked-into tactic: the eval swing
/// (mover-POV) and the opponent's punishing continuation from the played
/// move's own PV. `MoveAnalysis::score` is already root-STM (mover) POV,
/// so this is a straight cp→pawn conversion on the PawnEG scale.
fn allowed_reframe(
    pre: &Position,
    best_score: Value,
    user_score: Value,
    user_pv: &[Move],
) -> AllowedReframe {
    let pawn = Value::PAWN_EG.0 as f32;
    AllowedReframe {
        best_pawns: best_score.0 as f32 / pawn,
        played_pawns: user_score.0 as f32 / pawn,
        swing_pawns: (best_score.0 - user_score.0) as f32 / pawn,
        continuation: pv_to_san(pre, user_pv).join(" "),
    }
}

/// Walk a PV from `root`, emitting SAN per ply.
fn pv_to_san(root: &Position, pv: &[Move]) -> Vec<String> {
    let mut out = Vec::with_capacity(pv.len());
    let mut pos = root.clone();
    for &mv in pv {
        out.push(san::format(&pos, mv));
        pos.do_move(mv);
    }
    out
}

/// Build the headline [`Claim::Verdict`] for `user`'s move.
///
/// The salience layer both the GUI and CLI headline share. Computes the
/// chess.com `only_good_move` signal (a *valid* use of a best-vs-second
/// PV gap — see PLAN §"Analysis config") and the `sacrifice` flag the
/// translator maps to "Great" / "Brilliant".
///
/// `analyses` is the retrospective slice (`analyses[0]` = engine best,
/// plus the true second-best at `multi_pv = 2`, plus the force-included
/// user line). `best` / `user` are the chosen entries. `verdict` is the
/// already-classified [`MoveVerdict`] (material-aware). `reveal_moves`
/// gates whether the engine's preferred SAN is carried for the
/// best-move-reveal detail.
pub fn verdict_claim(
    pre_move_pos: &Position,
    analyses: &[MoveAnalysis],
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    verdict: MoveVerdict,
    reveal_moves: bool,
) -> Claim {
    let mover = pre_move_pos.side_to_move();
    let san = san::format(pre_move_pos, user.mv);
    let gap = Value(user.score.0 - best.score.0);

    let only_good_move = only_good_move(pre_move_pos, analyses, best);
    let sacrifice = is_sacrifice(pre_move_pos, &user.pv, mover);

    // Best-move SAN carried only when the caller opted into the reveal
    // and the engine actually preferred a different move.
    let best_san = if reveal_moves && best.mv != user.mv {
        Some(san::format(pre_move_pos, best.mv))
    } else {
        None
    };

    Claim::Verdict {
        verdict,
        mover,
        san,
        score: user.score,
        best_score: best.score,
        gap,
        only_good_move,
        sacrifice,
        best_san,
    }
}

/// Build a [`Claim::Material`] from a set of capture `events`, in
/// `mover`-relative terms.
///
/// `events` is the caller-selected capture slice the claim should
/// phrase — the GUI passes `MaterialOutcome::realized_events` (past-tense
/// "you won material"), the CLI passes the full PV `events` (hypothetical
/// "Best line: … you win material"). The salience question *which* events
/// to phrase and *whether* to suppress the card lives with the caller
/// (the GUI suppresses pure hangs and empty exchanges); this builder only
/// packages the structured nets `phrase` needs.
///
/// Nets are computed from `mover`'s POV: positive = the mover came out
/// ahead, negative = the mover came out behind. `net_points` uses
/// classical values (P:1, N:3, B:3, R:5, Q:9) — what a human reads when
/// judging "even trade?"; `net_mg_cp` / `net_eg_cp` are the engine's
/// tapered cp valuation for the slight B-vs-N / phase asymmetries.
pub fn material_claim(events: &[CaptureEvent], mover: Color) -> Claim {
    let (net_points, net_mg_cp, net_eg_cp) = events.iter().fold((0, 0, 0), |(pts, mg, eg), ev| {
        let sign = if ev.captor == mover { 1 } else { -1 };
        (
            pts + sign * ev.captured_piece.classical_points() as i32,
            mg + sign * ev.value_mg,
            eg + sign * ev.value_eg,
        )
    });
    Claim::Material {
        mover,
        events: events.to_vec(),
        net_points,
        net_mg_cp,
        net_eg_cp,
    }
}

/// Build the realized-window [`Claim::Material`] for the GUI's past-tense
/// material card, applying the card's salience rules. Returns `None` when
/// there is nothing to narrate as a settled fact:
///
/// - **No realized captures** — the move and its forced recapture took
///   nothing; the SAN / other cards carry the story.
/// - **The first realized capture is the opponent's** — i.e. the mover's
///   own move took nothing and the opponent's best reply grabs a piece.
///   That's a *hanging* piece (present-tense, the threats card's job, and
///   the opponent may still miss it), not a settled loss.
///
/// "Realized" = captures at ply ≤ 1 (the mover's own move plus a forced
/// recapture) — deeper PV captures are speculative continuation and don't
/// belong in a past-tense framing. See [`MaterialOutcome::realized_events`].
pub fn material_claim_realized(outcome: &MaterialOutcome, mover: Color) -> Option<Claim> {
    let events: Vec<CaptureEvent> = outcome.realized_events().copied().collect();
    if events.is_empty() {
        return None;
    }
    if events.first().is_some_and(|ev| ev.captor != mover) {
        return None;
    }
    Some(material_claim(&events, mover))
}

/// Build a single threat-group [`Claim::Threats`]. `pieces` is empty ⇒
/// `None` (a group with no targets has nothing to phrase). The
/// constructor every renderer uses once it has applied its own salience
/// (the GUI's misleading-hang filter, the CLI's de-dup); the shared
/// list-level salience lives in [`threats_claims`].
pub fn threats_claim_group(
    side: ThreatSide,
    kind: ThreatKind,
    pieces: Vec<ThreatTarget>,
) -> Option<Claim> {
    if pieces.is_empty() {
        return None;
    }
    Some(Claim::Threats { side, kind, pieces })
}

/// Build the ordered threat claims for one analysed move — the shared
/// salience the CLI retrospective consumes (and the GUI can reuse for
/// the lists it doesn't filter further). Mirrors the old
/// `render_threats` decision tree, minus the prose:
///
/// - **Mover-side** (`ours_*`) hanging / SEE-losing fire on a positive
///   delta (the move *created* the threat).
/// - **Opponent-side** (`theirs_*`) hanging / SEE-losing fire on a
///   positive delta but only off the *guaranteed* lists — the static
///   `theirs_*` snapshot would mis-teach defensible threats (1.Nf3
///   attacks e5 but …Nc6 defends).
/// - **Pressure** (both sides) fires on a positive delta, de-duped
///   against the hanging / SEE-losing targets already surfaced so the
///   same piece isn't narrated twice.
///
/// Order matches the old narrator: ours-hanging, theirs-hanging,
/// ours-SEE, theirs-SEE, ours-pressured, theirs-pressured.
pub fn threats_claims(outcome: &ThreatsOutcome) -> Vec<Claim> {
    let mut claims = Vec::new();

    let push = |claims: &mut Vec<Claim>, side, kind, pieces: Vec<ThreatTarget>| {
        if let Some(c) = threats_claim_group(side, kind, pieces) {
            claims.push(c);
        }
    };

    // Hanging — most visceral, narrated first.
    if outcome.ours_hanging_delta > 0 {
        push(
            &mut claims,
            ThreatSide::Mover,
            ThreatKind::Hanging,
            targets(&outcome.ours_hanging),
        );
    }
    // "you can win material" only off the guaranteed list.
    if outcome.theirs_hanging_delta > 0 {
        push(
            &mut claims,
            ThreatSide::Opponent,
            ThreatKind::Hanging,
            targets(&outcome.theirs_hanging_guaranteed),
        );
    }

    // SEE-losing (defended but unequal exchange).
    if outcome.ours_see_losing_delta > 0 {
        push(
            &mut claims,
            ThreatSide::Mover,
            ThreatKind::SeeLosing,
            targets(&outcome.ours_see_losing),
        );
    }
    if outcome.theirs_see_losing_delta > 0 {
        push(
            &mut claims,
            ThreatSide::Opponent,
            ThreatKind::SeeLosing,
            targets(&outcome.theirs_see_losing_guaranteed),
        );
    }

    // Pressure — de-duped against the hanging / SEE-losing targets
    // already surfaced on the same side, so we don't say the same
    // thing twice in different words.
    let ours_rendered = rendered_squares(&outcome.ours_hanging, &outcome.ours_see_losing);
    let theirs_rendered =
        rendered_squares(&outcome.theirs_hanging, &outcome.theirs_see_losing);
    if outcome.ours_pressured_delta > 0 {
        push_pressured(&mut claims, ThreatSide::Mover, &outcome.ours_pressured, &ours_rendered);
    }
    if outcome.theirs_pressured_delta > 0 {
        push_pressured(
            &mut claims,
            ThreatSide::Opponent,
            &outcome.theirs_pressured,
            &theirs_rendered,
        );
    }

    claims
}

/// Emit one `Claim::Threats` per *pressure kind* present on `side`
/// (after de-dup), so each kind gets its pattern-specific verb. Skips
/// targets already surfaced as hanging / SEE-losing on this side.
fn push_pressured(
    claims: &mut Vec<Claim>,
    side: ThreatSide,
    pressured: &[PressuredPiece],
    already_rendered: &[chess_tutor_engine::types::Square],
) {
    let filtered: Vec<&PressuredPiece> = pressured
        .iter()
        .filter(|p| !already_rendered.contains(&p.location.square))
        .collect();
    // Group by pressure kind so the heading verb ("harried" / "kicked"
    // / "pressured") is uniform within a card.
    for kind in [
        PressureKind::MinorOnMajor,
        PressureKind::RookOnQueen,
        PressureKind::SafePawnThreat,
    ] {
        let pieces: Vec<ThreatTarget> = filtered
            .iter()
            .filter(|p| p.kind == kind)
            .map(|p| ThreatTarget::from(*p))
            .collect();
        if let Some(c) = threats_claim_group(side, ThreatKind::Pressured(kind), pieces) {
            claims.push(c);
        }
    }
}

/// Squares already surfaced as hanging / SEE-losing, for pressure de-dup.
fn rendered_squares(
    hanging: &[HangingPiece],
    see_losing: &[HangingPiece],
) -> Vec<chess_tutor_engine::types::Square> {
    hanging
        .iter()
        .chain(see_losing.iter())
        .map(|h| h.location.square)
        .collect()
}

/// Convert a hanging-piece list to threat targets.
fn targets(hangs: &[HangingPiece]) -> Vec<ThreatTarget> {
    hangs.iter().map(ThreatTarget::from).collect()
}

/// Engine-cp threshold for surfacing a pawn-shield shift. ~0.25 of a
/// pawn — small enough to catch a single shield-pawn break, large
/// enough that an opening tempo nudging shelter by 5-10 cp doesn't
/// fire a line every move.
const KING_SHELTER_DELTA_THRESHOLD_CP: i32 = 25;

/// Game-phase cutoff below which the pawn-shield clause is suppressed.
/// Phase is `[0, 128]` (128 = pure midgame, 0 = pure endgame); below
/// 32 we're deep into an endgame where pawn cover is no longer the
/// dominant king-safety concern, so a shelter story would just be
/// noise. The attacker-count clause still fires — even bare-board
/// kings care about being chased.
const KING_SHELTER_ENDGAME_PHASE_CUTOFF: i32 = 32;

/// Extra enemy attacks on the king's *adjacent* squares (its escape
/// squares) needed to call the pressure shift worth a card on its own.
/// `2` skips the single-square wobble a quiet developing move causes
/// while catching a real escalation (a knight landing two new attacks
/// next to the king).
const KING_PRESSURE_ATTACKS_THRESHOLD: i32 = 2;

/// Aggregate king-danger swing (engine-cp, mg) that independently
/// triggers a pressure card — the holistic backstop for pressure the
/// adjacent-attack count misses (attacker-*weight* changes, weak ring
/// squares two ranks out, new safe checks). ~0.5 of a pawn at the
/// PawnEG≈213 scale: big enough to mean a real shift in how hard the
/// king is being hit, small enough to fire on the knight-closing-in
/// case (the king_danger nearly doubled there).
const KING_PRESSURE_DANGER_THRESHOLD_CP: i32 = 100;

/// Build the ordered king-safety claims for one analysed move — the
/// shared salience both the GUI and CLI consume, minus the prose.
/// Mirrors the old narrator's decision tree:
///
/// - Per side, **exposure wins over safer** when both could fire
///   (worsening is the more urgent teaching message), so each side
///   emits at most one claim.
/// - The **attacker-count** clause fires on any change in the claim's
///   direction; the **pawn-shield** clause fires only when the shift
///   clears [`KING_SHELTER_DELTA_THRESHOLD_CP`] and the phase is above
///   [`KING_SHELTER_ENDGAME_PHASE_CUTOFF`].
/// - A claim is emitted only when at least one clause is present.
///
/// Order matches the old narrator: mover-side king first, then the
/// opponent's. The claims are mover-relative; `phrase` applies the
/// "you" / "they" reframe and the directional flip.
pub fn king_safety_claims(outcome: &KingSafetyOutcome) -> Vec<Claim> {
    let mut claims = Vec::new();
    let shelter_relevant = outcome.phase >= KING_SHELTER_ENDGAME_PHASE_CUTOFF;

    if let Some(c) = king_safety_side_claim(
        KingSide::Mover,
        outcome.ours_attackers_delta(),
        outcome.ours_pawn_shield_mg_delta(),
        &outcome.ours_pre,
        &outcome.ours_post,
        shelter_relevant,
    ) {
        claims.push(c);
    }
    if let Some(c) = king_safety_side_claim(
        KingSide::Opponent,
        outcome.theirs_attackers_delta(),
        outcome.theirs_pawn_shield_mg_delta(),
        &outcome.theirs_pre,
        &outcome.theirs_post,
        shelter_relevant,
    ) {
        claims.push(c);
    }
    claims
}

/// Resolve one king's safety claim — direction (exposure over safer),
/// the attacker-count clause, the threshold-gated shelter clause, and
/// the pressure fallback. `None` when nothing clears the bar on this
/// side.
///
/// The **pressure** trigger is what catches the "knight closes in"
/// case: a move can leave the *distinct attacker count* unchanged (one
/// attacker gets blocked just as a closer one arrives) while the king is
/// demonstrably under more fire — more attacks on its escape squares,
/// and a higher aggregate king-danger score. When the count moved we
/// keep the precise "N attackers" clause; the pressure clause is the
/// number-free fallback for when only the closeness/intensity changed.
fn king_safety_side_claim(
    side: KingSide,
    attackers_delta: i32,
    shield_delta: i32,
    pre: &chess_tutor_engine::analysis::KingSafetySnapshot,
    post: &chess_tutor_engine::analysis::KingSafetySnapshot,
    shelter_relevant: bool,
) -> Option<Claim> {
    let shield_moved = shelter_relevant && shield_delta.abs() >= KING_SHELTER_DELTA_THRESHOLD_CP;

    // Pressure: more attacks on the king's own escape squares, and/or a
    // higher aggregate king-danger score. Either crossing its threshold
    // counts (adjacent attacks is the explainable headline; king-danger
    // is the holistic backstop). Only consulted when the attacker count
    // is flat — a count change is the clearer story and owns the wording.
    let attacks_delta = post.attacks_count - pre.attacks_count;
    let danger_delta = post.king_danger_mg - pre.king_danger_mg;
    let pressure_up = attacks_delta >= KING_PRESSURE_ATTACKS_THRESHOLD
        || danger_delta >= KING_PRESSURE_DANGER_THRESHOLD_CP;
    let pressure_down = attacks_delta <= -KING_PRESSURE_ATTACKS_THRESHOLD
        || danger_delta <= -KING_PRESSURE_DANGER_THRESHOLD_CP;
    let count_flat = attackers_delta == 0;

    // Exposure wins over safer when both could fire.
    let exposed =
        attackers_delta > 0 || (shield_moved && shield_delta < 0) || (count_flat && pressure_up);
    let safer =
        attackers_delta < 0 || (shield_moved && shield_delta > 0) || (count_flat && pressure_down);

    let direction = if exposed {
        SafetyDirection::MoreExposed
    } else if safer {
        SafetyDirection::Safer
    } else {
        return None;
    };

    // The attacker clause fires only when the count moved in the
    // claim's direction (exposed ⇒ up, safer ⇒ down).
    let attackers = if (direction == SafetyDirection::MoreExposed && attackers_delta > 0)
        || (direction == SafetyDirection::Safer && attackers_delta < 0)
    {
        Some(CountShift {
            pre: pre.attackers_count,
            post: post.attackers_count,
        })
    } else {
        None
    };

    let shield = if shield_moved
        && ((direction == SafetyDirection::MoreExposed && shield_delta < 0)
            || (direction == SafetyDirection::Safer && shield_delta > 0))
    {
        Some(ShelterShift {
            pre_mg: pre.pawn_shield_mg,
            post_mg: post.pawn_shield_mg,
        })
    } else {
        None
    };

    // Pressure is the fallback clause: only when the count was flat (no
    // attacker clause) yet pressure moved in the claim's direction.
    let pressure = if attackers.is_none()
        && ((direction == SafetyDirection::MoreExposed && pressure_up)
            || (direction == SafetyDirection::Safer && pressure_down))
    {
        Some(PressureShift {
            pre: pre.attacks_count,
            post: post.attacks_count,
        })
    } else {
        None
    };

    if attackers.is_none() && shield.is_none() && pressure.is_none() {
        return None;
    }

    Some(Claim::KingSafety {
        side,
        direction,
        attackers,
        shield,
        pressure,
        king_sq: post.king_sq,
    })
}

/// Build the ordered mobility claims for one analysed move — the
/// shared salience both the GUI and CLI consume, minus the prose.
///
/// One [`Claim::Mobility`] per piece type whose midgame mobility shift
/// clears `threshold_cp` on either side, biggest-first within a side,
/// mover-side first then the opponent (matching the old narrator's
/// order). `threshold_cp` is the caller's reporting floor — the CLI
/// retrospective uses a higher floor (one line per side), the GUI uses
/// a lower one (and drops it further for its "show all" expansion). The
/// claims are mover-relative; `phrase` applies the "you" / "they"
/// reframe and the directional flip (restricting the opponent's piece
/// is *your* gain).
///
/// "Activity" rather than "mobility" in the prose: Stockfish's mobility
/// term is a weighted count of squares a piece attacks inside the safe-
/// area bitmap, not the number of legal moves the piece has — which is
/// what a student hears in "mobility."
pub fn mobility_claims(outcome: &MobilityOutcome, threshold_cp: i32) -> Vec<Claim> {
    let mut claims = Vec::new();
    for (side, pre, post) in [
        (MobilitySide::Mover, &outcome.ours_pre, &outcome.ours_post),
        (MobilitySide::Opponent, &outcome.theirs_pre, &outcome.theirs_post),
    ] {
        for (piece, pre_cp, post_cp) in mobility_shifts(pre, post, threshold_cp) {
            claims.push(Claim::Mobility {
                side,
                piece,
                pre_cp,
                post_cp,
            });
        }
    }
    claims
}

/// All per-piece-type midgame mobility shifts whose `|delta|` clears
/// `threshold_cp`, sorted biggest-|delta|-first. Returns up to four
/// entries: `(piece_type, pre_mg, post_mg)`.
fn mobility_shifts(
    pre: &MobilityBreakdown,
    post: &MobilityBreakdown,
    threshold_cp: i32,
) -> Vec<(PieceType, i32, i32)> {
    let candidates = [
        (PieceType::Knight, pre.knight.mg().0, post.knight.mg().0),
        (PieceType::Bishop, pre.bishop.mg().0, post.bishop.mg().0),
        (PieceType::Rook, pre.rook.mg().0, post.rook.mg().0),
        (PieceType::Queen, pre.queen.mg().0, post.queen.mg().0),
    ];
    let mut shifts: Vec<(PieceType, i32, i32)> = candidates
        .into_iter()
        .filter(|(_, pre_cp, post_cp)| (post_cp - pre_cp).abs() >= threshold_cp)
        .collect();
    shifts.sort_by_key(|(_, pre_cp, post_cp)| std::cmp::Reverse((post_cp - pre_cp).abs()));
    shifts
}

/// Engine-cp threshold per pawn-structure sub-term for calling a shift
/// worth narrating. ~0.15 of a pawn — big enough to skip the 1-2 cp
/// wobble from tapered rescoring but small enough to catch single-pawn-
/// scale events like a new doubled pawn. Matches the old narrator.
const PAWN_STRUCTURE_DELTA_THRESHOLD_CP: i32 = 15;

impl PawnCategory {
    const ALL: [PawnCategory; 6] = [
        PawnCategory::Connected,
        PawnCategory::Isolated,
        PawnCategory::Backward,
        PawnCategory::Doubled,
        PawnCategory::WeakUnopposed,
        PawnCategory::WeakLever,
    ];

    /// `post.mg() - pre.mg()` for this sub-term. Positive = improved
    /// (bonus grew or penalty shrank); negative = worsened.
    fn delta_mg(self, pre: &PawnsBreakdown, post: &PawnsBreakdown) -> i32 {
        match self {
            PawnCategory::Connected => post.connected.mg().0 - pre.connected.mg().0,
            PawnCategory::Isolated => post.isolated.mg().0 - pre.isolated.mg().0,
            PawnCategory::Backward => post.backward.mg().0 - pre.backward.mg().0,
            PawnCategory::Doubled => post.doubled.mg().0 - pre.doubled.mg().0,
            PawnCategory::WeakUnopposed => post.weak_unopposed.mg().0 - pre.weak_unopposed.mg().0,
            PawnCategory::WeakLever => post.weak_lever.mg().0 - pre.weak_lever.mg().0,
        }
    }
}

/// Sub-terms that worsened past the threshold for one side.
fn worsened_pawn_categories(pre: &PawnsBreakdown, post: &PawnsBreakdown) -> Vec<PawnCategory> {
    PawnCategory::ALL
        .into_iter()
        .filter(|st| st.delta_mg(pre, post) <= -PAWN_STRUCTURE_DELTA_THRESHOLD_CP)
        .collect()
}

/// Sub-terms that improved past the threshold for one side.
fn improved_pawn_categories(pre: &PawnsBreakdown, post: &PawnsBreakdown) -> Vec<PawnCategory> {
    PawnCategory::ALL
        .into_iter()
        .filter(|st| st.delta_mg(pre, post) >= PAWN_STRUCTURE_DELTA_THRESHOLD_CP)
        .collect()
}

/// Build one side's pawn-structure claim — worsened wins over improved
/// when both could fire (worsening is the more urgent teaching message).
/// `None` when nothing on this side clears the threshold.
fn pawn_structure_side_claim(
    side: PawnSide,
    pre: &PawnsBreakdown,
    post: &PawnsBreakdown,
) -> Option<Claim> {
    let worsened = worsened_pawn_categories(pre, post);
    if !worsened.is_empty() {
        return Some(Claim::PawnStructure {
            side,
            direction: StructureDirection::Worsened,
            categories: worsened,
        });
    }
    let improved = improved_pawn_categories(pre, post);
    if !improved.is_empty() {
        return Some(Claim::PawnStructure {
            side,
            direction: StructureDirection::Improved,
            categories: improved,
        });
    }
    None
}

/// Build the ordered pawn-structure claims for one analysed move — the
/// shared salience both the GUI and CLI consume, minus the prose.
///
/// Per side, **worsening wins over improving** when both could fire
/// (mirroring the king-safety precedence), so each side emits at most
/// one claim. Order matches the old narrator: mover-side first, then the
/// opponent's. The claims are mover-relative; `phrase` applies the "you"
/// / "they" reframe and the directional flip (weakening the opponent's
/// structure is *your* gain).
pub fn pawn_structure_claims(outcome: &PawnStructureOutcome) -> Vec<Claim> {
    let mut claims = Vec::new();
    if let Some(c) =
        pawn_structure_side_claim(PawnSide::Mover, &outcome.ours_pre, &outcome.ours_post)
    {
        claims.push(c);
    }
    if let Some(c) =
        pawn_structure_side_claim(PawnSide::Opponent, &outcome.theirs_pre, &outcome.theirs_post)
    {
        claims.push(c);
    }
    claims
}

/// Engine-cp threshold for narrating an aggregate passed-pawn shift on
/// one side. Passed-pawn swings scale hard with rank — a rank-5 passer
/// alone puts ~170 cp of MG rank-bonus on the board — so a ~20 cp floor
/// suppresses noise while catching meaningful events. Matches the GUI
/// card's prior floor.
const PASSED_DELTA_THRESHOLD_CP: i32 = 20;

/// Aggregate midgame passed-pawn cp for one side's breakdown.
fn passed_total_mg(bd: &PassedBreakdown) -> i32 {
    bd.rank_bonus.mg().0
        + bd.king_proximity.mg().0
        + bd.free_advance.mg().0
        + bd.stopper_penalty.mg().0
}

/// Build one side's passed-pawn claim from the aggregate midgame shift.
/// `None` when the shift is below [`PASSED_DELTA_THRESHOLD_CP`].
fn passed_pawns_side_claim(
    side: PawnSide,
    pre: &PassedBreakdown,
    post: &PassedBreakdown,
) -> Option<Claim> {
    let delta_mg = passed_total_mg(post) - passed_total_mg(pre);
    if delta_mg.abs() < PASSED_DELTA_THRESHOLD_CP {
        return None;
    }
    let direction = if delta_mg > 0 {
        StructureDirection::Improved
    } else {
        StructureDirection::Worsened
    };
    Some(Claim::PassedPawns {
        side,
        direction,
        delta_mg,
    })
}

/// Build the passed-pawn claims for one analysed move — the shared
/// salience both the GUI and CLI consume, minus the prose. One claim per
/// side whose aggregate midgame passed-pawn shift clears the threshold,
/// mover-side first then the opponent's (matching the old narrator). The
/// claims are mover-relative; `phrase` applies the "you" / "they" reframe
/// and the directional flip (blunting the opponent's passers is *your*
/// gain).
pub fn passed_pawns_claims(outcome: &PassedPawnsOutcome) -> Vec<Claim> {
    let mut claims = Vec::new();
    if let Some(c) =
        passed_pawns_side_claim(PawnSide::Mover, &outcome.ours_pre, &outcome.ours_post)
    {
        claims.push(c);
    }
    if let Some(c) =
        passed_pawns_side_claim(PawnSide::Opponent, &outcome.theirs_pre, &outcome.theirs_post)
    {
        claims.push(c);
    }
    claims
}

/// "Only good move" predicate — the chess.com "Great" / "Brilliant"
/// gate. True when the position offered **more than one** legal reply
/// (never flag a *forced* move), the engine's best line **holds** the
/// position (≥ roughly equal), and the **second-best** line is clearly
/// worse on an **absolute** threshold (crosses toward losing).
///
/// The absolute second-best test — not a raw best-vs-second gap — is what
/// keeps "+5 vs +3, both crushing" from false-positiving: at +25 the
/// second move is *also* winning, so it never reads as the only move (and
/// a piece-hang at +25 never reads as Brilliant). See PLAN §"Analysis
/// config".
fn only_good_move(pre_move_pos: &Position, analyses: &[MoveAnalysis], best: &MoveAnalysis) -> bool {
    // ≥ ~equal (allow a small minus): best must keep the position.
    const BEST_HOLDS_FLOOR_CP: i32 = -Value::PAWN_EG.0 / 2;
    // Second-best must be clearly losing-ish: worse than ~one pawn down.
    const SECOND_LOSES_CEIL_CP: i32 = -Value::PAWN_EG.0;

    let mut scratch = pre_move_pos.clone();
    if legal_moves_vec(&mut scratch).len() <= 1 {
        return false;
    }
    if best.score.0 < BEST_HOLDS_FLOOR_CP {
        return false;
    }
    // The true second-best *distinct* move: the highest-scoring analysis
    // whose move differs from `best`. With `multi_pv = 2` + force-include,
    // this is the natural runner-up (or the user's own line if it ranks
    // ahead of any other). Absent (single candidate) ⇒ not "only good".
    let Some(second) = analyses.iter().find(|a| a.mv != best.mv) else {
        return false;
    };
    second.score.0 <= SECOND_LOSES_CEIL_CP
}

// =========================================================================
// Piece placement
// =========================================================================

/// Engine-cp threshold per piece-positional sub-term for calling a
/// shift worth narrating. ~0.15 of a pawn — each individual positional
/// term is typically 20-40 cp when it fires (knight outpost ~30, rook
/// on open file ~45), so 15 catches one piece moving in or out of the
/// pattern while skipping 1-2 cp tapered-rescoring wobble. Matches the
/// old narrator.
const PIECES_POSITIONAL_DELTA_THRESHOLD_CP: i32 = 15;

impl PlacementCategory {
    const ALL: [PlacementCategory; 11] = [
        PlacementCategory::Outposts,
        PlacementCategory::ReachableOutposts,
        PlacementCategory::MinorBehindPawn,
        PlacementCategory::KingProtector,
        PlacementCategory::BishopPawns,
        PlacementCategory::LongDiagonalBishop,
        PlacementCategory::RookOnQueenFile,
        PlacementCategory::RookOnOpenFile,
        PlacementCategory::RookOnSemiopenFile,
        PlacementCategory::TrappedRook,
        PlacementCategory::WeakQueen,
    ];

    /// `post.mg() - pre.mg()` for this sub-term. Positive = improved
    /// (bonus grew or penalty shrank); negative = worsened.
    fn delta_mg(self, pre: &PiecesBreakdown, post: &PiecesBreakdown) -> i32 {
        match self {
            PlacementCategory::Outposts => post.outposts.mg().0 - pre.outposts.mg().0,
            PlacementCategory::ReachableOutposts => {
                post.reachable_outposts.mg().0 - pre.reachable_outposts.mg().0
            }
            PlacementCategory::MinorBehindPawn => {
                post.minor_behind_pawn.mg().0 - pre.minor_behind_pawn.mg().0
            }
            PlacementCategory::KingProtector => {
                post.king_protector.mg().0 - pre.king_protector.mg().0
            }
            PlacementCategory::BishopPawns => post.bishop_pawns.mg().0 - pre.bishop_pawns.mg().0,
            PlacementCategory::LongDiagonalBishop => {
                post.long_diagonal_bishop.mg().0 - pre.long_diagonal_bishop.mg().0
            }
            PlacementCategory::RookOnQueenFile => {
                post.rook_on_queen_file.mg().0 - pre.rook_on_queen_file.mg().0
            }
            PlacementCategory::RookOnOpenFile => {
                post.rook_on_open_file.mg().0 - pre.rook_on_open_file.mg().0
            }
            PlacementCategory::RookOnSemiopenFile => {
                post.rook_on_semiopen_file.mg().0 - pre.rook_on_semiopen_file.mg().0
            }
            PlacementCategory::TrappedRook => post.trapped_rook.mg().0 - pre.trapped_rook.mg().0,
            PlacementCategory::WeakQueen => post.weak_queen.mg().0 - pre.weak_queen.mg().0,
        }
    }
}

/// Skip `BishopPawns` narration when bishop geometry didn't change on
/// the side. Without this filter, a central pawn push (1.e4 e5) that
/// merely doubles the blocked-centre multiplier would fire phantom
/// "a bishop got stuck behind its pawn chain" claims on both sides —
/// none of which describe anything a 1200-ELO student can act on.
fn include_bishop_pawns(c: PlacementCategory, bishop_geometry_changed: bool) -> bool {
    c != PlacementCategory::BishopPawns || bishop_geometry_changed
}

/// One side's piece-placement sub-term shifts past the threshold,
/// honouring the BishopPawns geometry suppression. Returns the
/// `(category, delta_mg)` for each that fired.
fn placement_shifts(
    pre: &PiecesBreakdown,
    post: &PiecesBreakdown,
    bishop_geometry_changed: bool,
) -> Vec<(PlacementCategory, i32)> {
    PlacementCategory::ALL
        .into_iter()
        .filter(|c| include_bishop_pawns(*c, bishop_geometry_changed))
        .filter_map(|c| {
            let d = c.delta_mg(pre, post);
            (d.abs() >= PIECES_POSITIONAL_DELTA_THRESHOLD_CP).then_some((c, d))
        })
        .collect()
}

/// Build the ordered piece-placement claims for one analysed move — the
/// shared salience both the GUI and CLI consume, minus the prose. One
/// [`Claim::PiecePlacement`] per sub-term whose midgame shift clears the
/// threshold on either side, in [`PlacementCategory::ALL`] order,
/// mover-side first then the opponent's (matching the old narrator). The
/// claims are mover-relative; `phrase` applies the "you" / "they"
/// reframe and the directional flip (weakening the opponent's placement
/// is *your* gain).
///
/// Capture-aware suppression (a captured rook didn't "escape its trap",
/// a minor leaving the board "improving" its mates' king-distance by
/// arithmetic) is the GUI card's own concern — it owns the realised
/// capture events — so it filters the returned claims itself, mirroring
/// the prior split.
pub fn pieces_positional_claims(outcome: &PiecesPositionalOutcome) -> Vec<Claim> {
    let mut claims = Vec::new();
    for (cat, delta) in placement_shifts(
        &outcome.ours_pre,
        &outcome.ours_post,
        outcome.ours_bishop_pawn_count_changed(),
    ) {
        claims.push(placement_claim(PlacementSide::Mover, cat, delta));
    }
    for (cat, delta) in placement_shifts(
        &outcome.theirs_pre,
        &outcome.theirs_post,
        outcome.theirs_bishop_pawn_count_changed(),
    ) {
        claims.push(placement_claim(PlacementSide::Opponent, cat, delta));
    }
    claims
}

fn placement_claim(side: PlacementSide, category: PlacementCategory, delta_mg: i32) -> Claim {
    let direction = if delta_mg > 0 {
        StructureDirection::Improved
    } else {
        StructureDirection::Worsened
    };
    Claim::PiecePlacement {
        side,
        category,
        direction,
        delta_mg,
    }
}

// =========================================================================
// Space
// =========================================================================

/// Minimum |delta_mg| to bother narrating a space shift. Space is a
/// small term to begin with — most shifts are <20 cp — so we threshold
/// modestly. One reinforced-square change at full piece count moves the
/// score by ~14 cp.
const SPACE_DELTA_THRESHOLD_CP: i32 = 15;

/// Build one side's space claim from the signed midgame shift. `None`
/// when the shift is below `threshold_cp`. The caller's reporting floor:
/// the GUI's "show all signals" expansion drops it to 1; the default
/// floor is [`SPACE_DELTA_THRESHOLD_CP`].
fn space_side_claim(side: SpaceSide, delta_mg: i32, threshold_cp: i32) -> Option<Claim> {
    if delta_mg.abs() < threshold_cp {
        return None;
    }
    let direction = if delta_mg > 0 {
        SpaceDirection::Gained
    } else {
        SpaceDirection::Lost
    };
    Some(Claim::Space {
        side,
        direction,
        delta_mg,
    })
}

/// Build the space claims for one analysed move — the shared salience
/// both the GUI and CLI consume, minus the prose. Up to one claim per
/// side whose midgame space shift clears `threshold_cp`, mover-side
/// first then the opponent's. The claims are mover-relative; `phrase`
/// applies the "you" / "they" reframe and the directional flip
/// (squeezing the opponent's space is *your* gain). The board-highlight
/// bitboards (`*_safe_post` / `*_reinforced_post`) stay on the outcome
/// for the GUI to read directly — they're a render concern, not prose.
pub fn space_claims(outcome: &SpaceOutcome, threshold_cp: i32) -> Vec<Claim> {
    let mut claims = Vec::new();
    if let Some(c) = space_side_claim(SpaceSide::Mover, outcome.ours_space_delta_mg(), threshold_cp)
    {
        claims.push(c);
    }
    if let Some(c) = space_side_claim(
        SpaceSide::Opponent,
        outcome.theirs_space_delta_mg(),
        threshold_cp,
    ) {
        claims.push(c);
    }
    claims
}

// =========================================================================
// Initiative
// =========================================================================

/// Minimum |swing| (ply 1 → settled, mover POV, engine cp) for the
/// refutation template to fire. Below this, the opponent's check /
/// capture didn't materially change the line — no point pretending the
/// mover's threat got "refuted." Matches the old narrator.
const REFUTATION_SWING_GATE: i32 = 50;

/// Build the initiative / forcing-hierarchy claim for one analysed
/// move — the shared salience both the GUI and CLI consume, minus the
/// prose. `None` when the mover's move didn't create a threat, when
/// there's no named opponent reply to anchor a template, or (for the
/// refutation case) when the swing is too small to claim anything got
/// refuted. The claim is mover-relative; `phrase` applies the "you" /
/// "they" reframe.
pub fn initiative_claim(outcome: &InitiativeOutcome, mover: Color) -> Option<Claim> {
    if !outcome.user_move_was_threat {
        return None;
    }
    // Without a named opponent reply we can't form any template.
    let reply_san = outcome.opponent_reply_san.clone()?;
    let opponent_forces =
        outcome.opponent_reply_is_check || outcome.opponent_reply_is_capture;

    let template = if !opponent_forces {
        InitiativeTemplate::Reinforcement
    } else if outcome.user_still_favored {
        InitiativeTemplate::HeldDespite
    } else {
        // Refutation — suppress when the swing is too small.
        if outcome.eval_swing_cp > -REFUTATION_SWING_GATE {
            return None;
        }
        InitiativeTemplate::Refutation
    };

    Some(Claim::Initiative {
        mover,
        template,
        reply_san,
        reply_is_check: outcome.opponent_reply_is_check,
    })
}

// =========================================================================
// Secondary terms
// =========================================================================

/// Cumulative-coverage threshold for the fallback term list. Lower than
/// the `search --analyze` default (75%) because real-game output showed
/// 7–9 rows per move at 75%, most of which were noise; 50% keeps the
/// list tight and usually lands on the 2–4 terms that drove the swing.
const SECONDARY_TOP_PERCENT: f32 = 50.0;

/// Build the fallback "other shifts" [`Claim::Secondary`] for one
/// analysed move — the residual eval-term deltas after the specialised
/// claims consumed their own terms.
///
/// `skip` is the set of [`TermId`]s already narrated by a specialised
/// claim (so they aren't double-counted here). `top_percent` is the
/// cumulative-coverage trim — the caller passes a higher value (e.g.
/// 100.0) to surface every residual term in a "show all" expansion.
/// Deltas are stored **mover-POV** (positive = helped the mover), so
/// `phrase` can apply the perspective reframe; the engine's raw deltas
/// are white-POV, so this sign-flips for a black mover. Returns `None`
/// when nothing survives the trim.
pub fn secondary_claim(
    user: &MoveAnalysis,
    mover: Color,
    skip: &[TermId],
    top_percent: f32,
) -> Option<Claim> {
    let prefix = cumulative_prefix(&user.term_deltas, top_percent);
    // Sign-flip so positive = helped the mover (their POV), not raw
    // white-POV — `phrase` then reframes to "you" / "they".
    let sign = match mover {
        Color::White => 1,
        Color::Black => -1,
    };
    let terms: Vec<(TermId, i32)> = prefix
        .iter()
        .filter(|d| !skip.contains(&d.term) && d.delta_tapered != 0)
        .map(|d| (d.term, d.delta_tapered * sign))
        .collect();
    if terms.is_empty() {
        return None;
    }
    Some(Claim::Secondary { terms })
}

/// The default fallback-term cumulative-coverage trim — exposed so
/// callers that don't override it (the CLI, the GUI's collapsed card)
/// share one floor.
pub const SECONDARY_DEFAULT_TOP_PERCENT: f32 = SECONDARY_TOP_PERCENT;

/// The default space-card firing floor — exposed so the GUI's default
/// (non-"show all") card and the CLI share one threshold.
pub const SPACE_DEFAULT_THRESHOLD_CP: i32 = SPACE_DELTA_THRESHOLD_CP;

// =========================================================================
// Special UI narratives (forced-consequences, desperado, override, depth
// honesty, surprise tag)
// =========================================================================

impl ForcedConcession {
    /// `post.mg() - pre.mg()` for this concession's pawn sub-term on the
    /// replier's breakdown. All four are penalty terms (≤ 0 score), so a
    /// *more-negative* delta means the structure got worse — the
    /// concession we're looking for.
    fn delta_mg(self, pre: &PawnsBreakdown, post: &PawnsBreakdown) -> i32 {
        match self {
            ForcedConcession::Doubled => post.doubled.mg().0 - pre.doubled.mg().0,
            ForcedConcession::Isolated => post.isolated.mg().0 - pre.isolated.mg().0,
            ForcedConcession::Backward => post.backward.mg().0 - pre.backward.mg().0,
            ForcedConcession::WeakUnopposed => {
                post.weak_unopposed.mg().0 - pre.weak_unopposed.mg().0
            }
        }
    }

    const ALL: [ForcedConcession; 4] = [
        ForcedConcession::Doubled,
        ForcedConcession::Isolated,
        ForcedConcession::Backward,
        ForcedConcession::WeakUnopposed,
    ];
}

/// Engine-cp threshold for surfacing a forced-consequence pawn
/// concession. Lower than the regular pawn-structure card's gate: SF11's
/// Doubled penalty is only ~11 cp at full middlegame phase, yet a
/// doubled / isolated / backward pawn is a pedagogically valuable
/// long-term concession even at small cp. Matches the prior GUI card.
const FORCED_CONSEQUENCES_THRESHOLD_CP: i32 = 8;

/// Build the forced-consequences claims for one analysed move — the
/// structural concessions the *non-moving* side's best reply creates on
/// its **own** side. Walks the user's PV one ply past the move and diffs
/// the replier's pawn breakdown `post-user-move → post-reply`. Mirrors
/// the prior GUI `build_forced_consequences_items`, minus the prose.
///
/// `None`-shaped: an empty `Vec` when the PV is too short or no
/// concession clears the threshold. The claims are mover-relative;
/// `phrase` applies the "you" / "they" reframe (a concession on the
/// replier's side is *your* gain when the player moved).
pub fn forced_consequence_claims(
    pre_move_pos: &Position,
    user: &MoveAnalysis,
    root_stm: Color,
) -> Vec<Claim> {
    if user.pv.len() < 2 {
        return Vec::new();
    }
    let mut after_user = pre_move_pos.clone();
    after_user.do_move(user.pv[0]);
    let reply = user.pv[1];
    let reply_san = san::format(&after_user, reply);
    let mut after_reply = after_user.clone();
    after_reply.do_move(reply);

    // The replier is the non-moving side; read *their* pawn breakdown.
    let replier = !root_stm;
    let before = chess_tutor_engine::pawns::evaluate(&after_user).breakdowns[replier.index()];
    let after = chess_tutor_engine::pawns::evaluate(&after_reply).breakdowns[replier.index()];

    // When the reply is a capture, the opponent may have other ways to
    // recapture on the same square. A structural concession is only a real
    // weakness if every *materially-sound* recapture incurs it; if a sound
    // alternative preserves the structure, assume the opponent plays that
    // one and stay silent — the search's PV often shows one of several
    // equal-value recaptures arbitrarily, and we shouldn't claim a weakness
    // they can simply sidestep. ("Only way to capture" or "the engine's
    // best/only sound way" → show; otherwise → don't.)
    let target = reply.to();
    let reply_is_capture = after_user.piece_on(target).is_some();
    let alt_breakdowns: Vec<PawnsBreakdown> = if reply_is_capture {
        let mut scratch = after_user.clone();
        legal_moves_vec(&mut scratch)
            .into_iter()
            // Other recaptures on the same square (target is occupied, so
            // any move landing there captures), that don't lose material.
            .filter(|m| *m != reply && m.to() == target && after_user.see_ge(*m, Value::ZERO))
            .map(|m| {
                let mut b = after_user.clone();
                b.do_move(m);
                chess_tutor_engine::pawns::evaluate(&b).breakdowns[replier.index()]
            })
            .collect()
    } else {
        Vec::new()
    };

    let mut claims = Vec::new();
    for category in ForcedConcession::ALL {
        let delta_mg = category.delta_mg(&before, &after);
        // A concession = the replier's structure getting worse (a
        // more-negative delta past the threshold).
        if delta_mg > -FORCED_CONSEQUENCES_THRESHOLD_CP {
            continue;
        }
        // Suppress if a sound alternative recapture avoids *this*
        // concession — the opponent isn't forced to damage their structure.
        let avoidable = alt_breakdowns
            .iter()
            .any(|alt| category.delta_mg(&before, alt) > -FORCED_CONSEQUENCES_THRESHOLD_CP);
        if avoidable {
            continue;
        }
        claims.push(Claim::ForcedConsequence {
            mover: root_stm,
            reply_san: reply_san.clone(),
            category,
            delta_mg,
        });
    }
    claims
}

/// Build a desperado [`Claim::Desperado`] for the user's move, or `None`
/// when the move isn't a same-tempo capture-with-check by a genuinely
/// doomed piece. Mirrors the prior GUI `build_desperado_item`, minus the
/// prose.
///
/// Two gates keep this honest (the original whole-PV walk over-fired):
///   1. only the user's actual ply-0 move is considered, and
///   2. the moving piece must be genuinely doomed where it stands —
///      hanging or SEE-losing on its origin square (a *safe* piece
///      making a winning capture-with-check is not "grabbing material on
///      the way down").
///
/// The claim is mover-relative; `phrase` applies the "you" / "they"
/// reframe (the desperado is the mover's resource).
pub fn desperado_claim(
    pre_move_pos: &Position,
    user: &MoveAnalysis,
    root_stm: Color,
) -> Option<Claim> {
    let mv = user.mv;
    let from = mv.from();

    // Gate 2: the moving piece must be genuinely doomed. The two lists
    // are disjoint by construction, so check both.
    let doomed = list_hanging(pre_move_pos, root_stm)
        .iter()
        .chain(list_see_losing(pre_move_pos, root_stm).iter())
        .any(|h| h.location.square == from);
    if !doomed {
        return None;
    }

    let d = find_desperado(pre_move_pos, from, root_stm)?;
    if mv.from() != d.piece || mv.to() != d.captures_on {
        return None;
    }
    Some(Claim::Desperado {
        mover: root_stm,
        san: san::format(pre_move_pos, mv),
        recovered_cp: d.recovered_cp,
    })
}

/// Minimum disagreement magnitude (engine-internal cp) on *each* axis for
/// the override note to fire — both the static gap and the search gap
/// must clear this so we don't narrate ledger/search noise as a "lie".
/// 100 cp ≈ a half-pawn on the PawnEG=213 scale. Matches the prior GUI
/// card.
const OVERRIDE_MARGIN_CP: i32 = 100;

/// Build the static-vs-search override [`Claim::OverrideNote`], or `None`
/// when the term ledger and the search agree (the common case). Mirrors
/// the prior GUI `build_override_note_item`, minus the prose.
///
/// `best` / `user` are the engine's preferred move and the user's move
/// from the same root; `root_stm` is the side that moved. Both must carry
/// a post-move trace (`ply_traces[0]`); without it we can't read the
/// static picture and stay silent.
pub fn override_note_claim(
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    root_stm: Color,
) -> Option<Claim> {
    if best.mv == user.mv {
        return None;
    }
    let best_static = post_move_static_root_pov(best, root_stm)?;
    let user_static = post_move_static_root_pov(user, root_stm)?;

    // By construction search prefers `best` (search_gap > 0); the override
    // fires only when the *static* ledger disagrees (static_gap < 0 — it
    // ranks the user's move higher).
    let search_gap = best.score.0 - user.score.0;
    let static_gap = best_static.0 - user_static.0;

    if search_gap <= OVERRIDE_MARGIN_CP {
        return None; // search doesn't meaningfully prefer best
    }
    if static_gap >= -OVERRIDE_MARGIN_CP {
        return None; // the ledger agrees (or is close) — no lie to flag
    }

    Some(Claim::OverrideNote {
        mover: root_stm,
        static_pawns: (-static_gap) as f32 / Value::PAWN_EG.0 as f32,
        search_pawns: search_gap as f32 / Value::PAWN_EG.0 as f32,
    })
}

/// The move's post-move static net total, root-STM POV, in engine cp.
/// `ply_traces[0]` is the trace right after the user's move (opponent to
/// move), so the stm at that trace is `!root_stm`; `white_pov_value`
/// strips tempo and orients to white, and we flip to root-STM POV.
fn post_move_static_root_pov(ma: &MoveAnalysis, root_stm: Color) -> Option<Value> {
    let trace = ma.ply_traces.first()?;
    let white_pov = trace.white_pov_value(!root_stm);
    Some(if root_stm == Color::White {
        white_pov
    } else {
        Value(-white_pov.0)
    })
}

/// Build the depth-honesty [`Claim::DepthHonesty`], or `None` when the
/// move isn't silent sequencing (the overwhelmingly common case).
/// Mirrors the prior GUI `build_depth_honesty_item`, minus the prose.
///
/// `best` / `user` are the engine's preferred move and the user's move
/// from the same root; `pre_move_pos` is the position they were played
/// from; `prior_move` feeds the detector chain's recapture guard.
pub fn depth_honesty_claim(
    pre_move_pos: &Position,
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    root_stm: Color,
    prior_move: Option<PriorMove>,
) -> Option<Claim> {
    if best.mv == user.mv {
        return None;
    }
    let deep_gap_cp = best.score.0 - user.score.0;
    if !is_silent_sequencing(pre_move_pos, user.mv, best.mv, deep_gap_cp, prior_move) {
        return None;
    }
    Some(Claim::DepthHonesty { mover: root_stm })
}

/// Build the shallow-vs-deep surprise tag [`Claim::Surprise`], or `None`
/// when the `(verdict, kind)` pair would contradict or over-narrate the
/// verdict. The salience (which combinations fire) lives here, mirroring
/// the prior `select_surprise_phrase` gate; `phrase` renders the
/// perspective-correct sentence.
///
/// `surprise` is the engine's raw [`SurpriseKind`] for the move (or
/// `None`); `mover` carries the reframe target.
pub fn surprise_claim(
    verdict: MoveVerdict,
    surprise: Option<SurpriseKind>,
    mover: Color,
) -> Option<Claim> {
    let kind = surprise?;
    // Only the two non-contradictory combinations surface — see the
    // `phrase_surprise` doc for the suppression rationale.
    matches!(
        (verdict, kind),
        (MoveVerdict::Best | MoveVerdict::Good, SurpriseKind::LooksBadButGood)
            | (MoveVerdict::Inaccuracy | MoveVerdict::Mistake, SurpriseKind::LooksGoodButBad)
    )
    .then_some(Claim::Surprise {
        mover,
        verdict,
        kind,
    })
}

// =========================================================================
// Cross-term multipliers (centre structure, castling × trapped rook)
// =========================================================================

/// Build the centre-structure claims for one analysed move — the
/// closed-centre and own-piece-barricade stories that ride Stockfish's
/// `bishop_pawns` multiplier. The shared salience both renderers consume,
/// minus the prose. Mirrors the prior `blocked_center_narration`:
///
/// - Both stories stay silent unless at least one side actually has a
///   bishop / same-coloured-pawn pair for the multiplier to amplify
///   (`*_amplifies_bishop_penalty`).
/// - A locked-count delta yields a [`CenterShift::Closed`] (a lock
///   appeared) or [`CenterShift::Opened`] (a lock dissolved); a
///   barricade-count delta yields [`CenterShift::Barricaded`] /
///   [`CenterShift::Cleared`]. Zero delta on either axis stays silent.
///
/// The claims are mover-relative; `phrase` applies only the "you" /
/// "they" subject flip — the structural consequence is the same for
/// whoever moved.
pub fn center_structure_claims(outcome: &BlockedCenterOutcome, mover: Color) -> Vec<Claim> {
    let amplifies =
        outcome.ours_amplifies_bishop_penalty || outcome.theirs_amplifies_bishop_penalty;
    if !amplifies {
        return Vec::new();
    }

    let mut claims = Vec::new();
    match outcome.locked_total_delta().signum() {
        1 => claims.push(Claim::CenterStructure {
            mover,
            kind: CenterShift::Closed,
        }),
        -1 => claims.push(Claim::CenterStructure {
            mover,
            kind: CenterShift::Opened,
        }),
        _ => {}
    }
    match outcome.barricaded_total_delta().signum() {
        1 => claims.push(Claim::CenterStructure {
            mover,
            kind: CenterShift::Barricaded,
        }),
        -1 => claims.push(Claim::CenterStructure {
            mover,
            kind: CenterShift::Cleared,
        }),
        _ => {}
    }
    claims
}

/// A trapped-rook penalty smaller in magnitude than this is too small to
/// bother teaching about — the doubling-from-castling-loss adds only a
/// few cp. Matches the prior `castling_narration` threshold.
const TRAPPED_ROOK_NARRATE_THRESHOLD_MG: i32 = 20;

/// Build the castling-loss × trapped-rook claims for one analysed move —
/// the shared salience both renderers consume, minus the prose. Mirrors
/// the prior `castling_narration`: a claim fires per side that just
/// forfeited its last castling rights *and* still has a rook boxed in by
/// its own king (the trapped-rook penalty clears the threshold).
///
/// The claims are mover-relative; `phrase` applies the "you" / "they"
/// flip — the mover losing its own castling is the warning, the mover
/// stripping the opponent's is *your* gain.
pub fn castling_claims(outcome: &CastlingOutcome) -> Vec<Claim> {
    let mut claims = Vec::new();
    if outcome.ours_lost_castling()
        && outcome.ours_trapped_rook_post_mg.abs() >= TRAPPED_ROOK_NARRATE_THRESHOLD_MG
    {
        claims.push(Claim::CastlingLoss {
            side: CastleSide::Mover,
        });
    }
    if outcome.theirs_lost_castling()
        && outcome.theirs_trapped_rook_post_mg.abs() >= TRAPPED_ROOK_NARRATE_THRESHOLD_MG
    {
        claims.push(Claim::CastlingLoss {
            side: CastleSide::Opponent,
        });
    }
    claims
}

/// Magnitude floor (engine-cp at the climax phase) the dominant
/// non-material term must clear for a [`Claim::PositionalWin`] to fire.
/// ~0.7 of a pawn on the PawnEG≈213 scale — well above the 1-2 cp
/// tapered-rescoring wobble, comfortably below the case study's
/// king-danger swing (`Rxe7+`: +286 → +3211 mover-POV mg, hundreds of cp
/// even after the phase discount). Tuned so the card only fires when a
/// *real* positional return is paying for the sacrificed material.
pub const POSITIONAL_WIN_TERM_FLOOR_CP: i32 = 150;

/// Build the sound-sacrifice justification claim for the engine's best
/// move, or `None` when the gate doesn't hold.
///
/// Fires when ALL of:
/// - `best.pv` is a [`is_sacrifice`] for `root_stm` (down ≥ 2 points of
///   material by the mover's 2nd move), AND
/// - the line is sound — [`win_chances`] of `best.score` ≥ 0 (at least
///   equal), AND
/// - a dominant **non-material** term swings ≥ [`POSITIONAL_WIN_TERM_FLOOR_CP`]
///   in the mover's favour from the baseline trace to the climax trace.
///
/// The compensation is computed by diffing `best.pre_move_trace`
/// (baseline) against `best.ply_traces[settled_ply]` (climax) over
/// [`TermId::ALL`] **excluding every `Material*` variant** — the
/// material-recapture filter that keeps the card honest (it must justify
/// the sacrifice *without* counting regained material). Each candidate
/// term's `(mg, eg)` delta is tapered at the climax phase + scale factor,
/// signed to the mover's POV; the largest mover-favourable one wins.
///
/// `sacrificed_points` is the mover's material balance at the climax
/// minus the baseline (negative = down material), read from piece counts
/// — the same shape [`is_sacrifice`] gates on.
pub fn positional_win_claim(best: &MoveAnalysis, pre_move_pos: &Position, root_stm: Color) -> Option<Claim> {
    // Gate 1: the best line is a sacrifice for the mover.
    if !is_sacrifice(pre_move_pos, &best.pv, root_stm) {
        return None;
    }
    // Gate 2: the line is sound — search rates it at least equal.
    if win_chances(best.score) < 0.0 {
        return None;
    }
    // The mover's POV sign for net (white − black) term scores.
    let sign = if root_stm == Color::White { 1 } else { -1 };

    let baseline = &best.pre_move_trace;
    // Climax = the end of the *forcing tail*, NOT the settled ply. The
    // settled ply walks all the way to where the search score quiesces —
    // by then the attack has been *converted into material* and the
    // game has thinned into an endgame (phase → 0), which discounts the
    // very middlegame king-danger term that is the lesson. The forcing
    // tail (the contiguous run of checks / captures / forced replies from
    // the root) is where the sacrifice is paid and the positional
    // compensation peaks while the mover is still down material — exactly
    // the case study's Position 3 (after `…Qxf8`, down a point, phase 22,
    // king danger raging). See the case-study doc's "comparison endpoints"
    // note.
    let climax_idx = forcing_tail_climax(pre_move_pos, &best.pv).min(best.ply_traces.len().saturating_sub(1));
    let climax = best.ply_traces.get(climax_idx)?;

    // Gate 3 + compensation: the dominant non-material term. Diff every
    // TermId except the Material* variants (the recapture filter),
    // tapered at the climax phase, signed to the mover. Largest
    // mover-favourable delta wins; require it to clear the floor.
    let mut best_term: Option<(TermId, i32, i32, i32)> = None; // (term, delta, pre, post)
    for &term in &TermId::ALL {
        if is_material_term(term) {
            continue;
        }
        let pre_cp = sign * tapered_cp(term.net_score(baseline), climax.phase, climax.scale_factor);
        let post_cp = sign * tapered_cp(term.net_score(climax), climax.phase, climax.scale_factor);
        let delta = post_cp - pre_cp;
        if best_term.map_or(true, |(_, d, _, _)| delta > d) {
            best_term = Some((term, delta, pre_cp, post_cp));
        }
    }
    let (dominant_term, delta, term_pre_cp, term_post_cp) = best_term?;
    if delta < POSITIONAL_WIN_TERM_FLOOR_CP {
        return None;
    }

    // Sacrificed material = mover's point balance at the climax minus the
    // baseline (negative = down material). Read from the climax position
    // (apply the climax PV prefix to a clone of pre_move_pos) vs the
    // pre-move balance — the same shape `is_sacrifice` gates on.
    let baseline_pts = material_diff_points_pov(pre_move_pos, root_stm);
    let mut scratch = pre_move_pos.clone();
    for &mv in best.pv.iter().take(climax_idx + 1) {
        scratch.do_move(mv);
    }
    let climax_pts = material_diff_points_pov(&scratch, root_stm);
    let sacrificed_points = climax_pts - baseline_pts;

    Some(Claim::PositionalWin {
        mover: root_stm,
        sacrificed_points,
        dominant_term,
        term_pre_cp,
        term_post_cp,
    })
}

/// The ply index that ends the *forcing tail* of `pv` from `root` — the
/// last ply of the contiguous prefix of forcing moves. A move counts as
/// forcing when the side to move was already in check (a forced reply),
/// the move is a capture, or it gives check. The first move that is none
/// of these (a genuinely quiet, optional move) ends the tail.
///
/// This is the natural "after the forcing sequence quiesces" endpoint the
/// sacrifice-justification card reads its static compensation at — the
/// king hunt's climax, before the attack gets converted into material and
/// the position thins into an endgame. Clamped to ≥ 0 (ply 0 is always
/// the sacrifice itself, by construction a capture or check).
///
/// Public so the retrospective UI builder can paint its king-ring /
/// trapped-rook annotations on the *same* climax board the claim's
/// compensation was read from.
pub fn forcing_tail_climax(root: &Position, pv: &[Move]) -> usize {
    let mut pos = root.clone();
    let mut last_forcing = 0usize;
    for (i, &mv) in pv.iter().enumerate() {
        let forcing = pos.in_check() || pos.is_capture(mv) || pos.gives_check(mv);
        pos.do_move(mv);
        if forcing {
            last_forcing = i;
        } else {
            break;
        }
    }
    last_forcing
}

/// Whether `term` is a material-derived term — the recapture filter for
/// [`positional_win_claim`]. Covers the two `Material*` variants (raw
/// piece values + the PSQ positional half) **and `Imbalance`**: the
/// imbalance polynomial is a non-linear function of piece *counts*
/// (bishop pair, rook redundancy, …), so it shifts purely because
/// material left the board. Counting it as "compensation" would let the
/// card justify a sacrifice with the very material it spent — exactly the
/// double-count the filter exists to prevent (the case study's `Rxe7+`
/// swings Imbalance just from the exchange, while the *teachable* story is
/// king danger). Matched by enum identity so a new material sub-term is a
/// compile-time prompt.
fn is_material_term(term: TermId) -> bool {
    matches!(
        term,
        TermId::MaterialPieceValue | TermId::MaterialPsqPositional | TermId::Imbalance
    )
}

/// `color`'s material balance in lichess point units (P1 N3 B3 R5 Q9),
/// mover-POV (positive = `color` ahead). Mirrors the engine-private
/// `material_diff_points`; lives here because the climax balance is read
/// from a UI-side position walk.
fn material_diff_points_pov(pos: &Position, color: Color) -> i32 {
    const VALUES: [(PieceType, i32); 5] = [
        (PieceType::Pawn, 1),
        (PieceType::Knight, 3),
        (PieceType::Bishop, 3),
        (PieceType::Rook, 5),
        (PieceType::Queen, 9),
    ];
    VALUES
        .iter()
        .map(|&(pt, v)| (pos.count(color, pt) as i32 - pos.count(!color, pt) as i32) * v)
        .sum()
}

/// Mover-POV cp drop (relative to the line's starting level) that counts
/// as an "explosion" along a PV — the ply where the static eval lurches
/// against the side that played the sub-optimal move. ~0.85 pawn on the
/// PawnEG≈213 scale: above the per-ply tapered-rescoring wobble, below the
/// case study's `Qxf8` king-danger eruption. The plan's "≥150–200 cp"
/// band, expressed on the engine scale.
pub const PROPHYLAXIS_EXPLOSION_CP: i32 = 180;

/// Mover-POV cp the *best* line is allowed to drift before it no longer
/// counts as "stays roughly level." Half the explosion floor — the best
/// line must hold the position while the user's line collapses, but a
/// small honest drift (the engine's pick isn't always perfectly flat)
/// shouldn't disqualify a real prophylaxis lesson.
const PROPHYLAXIS_BEST_LEVEL_CP: i32 = PROPHYLAXIS_EXPLOSION_CP / 2;

/// How much the punisher must *fail to* explode the position after the
/// best (prophylactic) move for the replay test to confirm prophylaxis.
/// The replay applies `best.mv` then the punisher and reads the static
/// mover-POV eval: if it no longer drops by at least this much (vs the
/// explosion the user's line suffered), the best move *removed* the
/// tactic — that's prophylaxis, not a deferred own-tactic. Same scale as
/// the explosion floor; we require the refuted line to recover to within
/// the best-level band.
const PROPHYLAXIS_REFUTED_CP: i32 = PROPHYLAXIS_EXPLOSION_CP / 2;

/// Build the missed-prophylaxis claim for the user's sub-optimal move, or
/// `None` when the gate doesn't hold.
///
/// Fires when ALL of:
/// - `user.mv != best.mv` (the whole mechanism is "the user's PV explodes
///   where the best PV doesn't"; a correctly-played prophylactic move has
///   no exploding line to show and is out of scope), AND
/// - the move genuinely gave away the advantage
///   ([`gave_away_advantage`] — the same eval-delta gate the headline /
///   ALLOWED reframe already uses; no absolute-level gate), AND
/// - the **user's line explodes**: walking `user.ply_traces` by mover-POV
///   static eval, some ply `k ≥ 1` drops ≥ [`PROPHYLAXIS_EXPLOSION_CP`]
///   below ply 0. `user.pv[k]` is the punisher (the opponent's move), AND
/// - the **best line does NOT explode**: `best.ply_traces` stays within
///   [`PROPHYLAXIS_BEST_LEVEL_CP`] of its start (the best move prevents
///   the disaster — distinguishing prophylaxis from "everything loses"),
///   AND
/// - the **replay/disambiguation test** passes: apply `best.mv` to a clone
///   of `pre_move_pos`, then re-try the punisher (same `from → to`). If it
///   is now illegal, or its static aftermath no longer explodes (recovers
///   to within [`PROPHYLAXIS_REFUTED_CP`] of level), the best move
///   *removed* the tactic ⇒ prophylaxis. If the punisher still works, the
///   best move was a *deferred own-tactic* (Feature 1's framing) ⇒ `None`.
///
/// The exploded term is the dominant non-material term that worsened (in
/// mover-POV) at the explosion ply — the human-legible "why," read off the
/// STATIC term diff (`TermId::ALL`, excluding `Material*`) exactly as
/// Feature 1 does. `reveal_moves` gates whether the prophylactic move's
/// SAN is carried (same posture as [`Claim::Verdict`]'s `best_san`).
pub fn missed_prophylaxis_claim(
    pre_move_pos: &Position,
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    root_stm: Color,
    reveal_moves: bool,
) -> Option<Claim> {
    // Gate 1: structural — the user's move differs from the engine's pick.
    if user.mv == best.mv {
        return None;
    }
    // Gate 2: the move genuinely gave away the advantage (the existing
    // eval-delta gate — no absolute-level reasoning).
    if !gave_away_advantage(best.score, user.score) {
        return None;
    }

    // Gate 3: the user's line explodes. The opponent has a *forcing*
    // punishing reply (the punisher) whose forcing tail drives the
    // mover-POV static eval down by ≥ the floor by the time it quiesces.
    // Measuring at the tail's endpoint — not the first ply — is what skips
    // the mid-exchange recapture bounce (a sac's ply-1 trace shows the
    // attacker temporarily up before the recapture restores parity). See
    // the case-study doc's "comparison endpoints" note.
    let explosion = prophylaxis_explosion(pre_move_pos, user, root_stm)?;
    let ProphylaxisExplosion {
        punisher_ply,
        endpoint_ply,
        swing_cp,
    } = explosion;
    let punisher = *user.pv.get(punisher_ply)?;

    // Gate 4: the best line does NOT explode — it holds the position. This
    // is what separates "the best move prevents it" from "everything
    // loses." Measure the worst sustained drop the same way (the forcing
    // tail of the best line, mover-POV); it must stay within the level band.
    if let Some(best_drop) = best_line_worst_drop(pre_move_pos, best, root_stm) {
        if best_drop >= PROPHYLAXIS_BEST_LEVEL_CP {
            return None;
        }
    }

    // Gate 5: replay/disambiguation — apply the best move, then replay the
    // punisher's forcing line with the mover defending best. Prophylaxis ⇒
    // it now fails (the best move removed the tactic); deferred own-tactic
    // ⇒ it still collapses (and belongs to Feature 1's framing, not here).
    let _ = punisher; // squares carried by the SAN; the replay uses the PV
    if !punisher_refuted_by_best(pre_move_pos, best.mv, &user.pv, punisher_ply, root_stm) {
        return None;
    }

    // The exploded term: the dominant non-material term that worsened (in
    // mover-POV) from the level baseline to the explosion endpoint. Same
    // TermId::ALL diff Feature 1 uses, excluding Material* (the recapture
    // filter — the lesson is the positional collapse, not regained
    // material).
    let pre_trace = best.pre_move_trace; // a level baseline; the explosion is read against it
    let climax = user.ply_traces.get(endpoint_ply)?;
    let exploded_term = dominant_worsened_term(&pre_trace, climax, root_stm)?;

    let prophylactic_san = reveal_moves.then(|| san::format(pre_move_pos, best.mv));

    Some(Claim::MissedProphylaxis {
        mover: root_stm,
        prophylactic_san,
        punisher_san: punisher_san(pre_move_pos, &user.pv, punisher_ply),
        exploded_term,
        swing_cp,
    })
}

/// The result of the explosion scan on the user's punished line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProphylaxisExplosion {
    /// The ply of the opponent's *forcing* reply that springs the
    /// punishment — the move the prophylaxis needed to stop. This is what
    /// the card names ("you needed Ra8 to stop **Rxe7+**").
    pub punisher_ply: usize,
    /// The ply at which the mover-POV static eval has fully collapsed —
    /// the end of the punisher's forcing tail, where the static terms
    /// (king danger, …) are read. Skips the mid-exchange recapture bounce.
    pub endpoint_ply: usize,
    /// Mover-POV cp lost from ply 0 to the endpoint (always ≥
    /// [`PROPHYLAXIS_EXPLOSION_CP`] when `Some`).
    pub swing_cp: i32,
}

/// Detect the punisher + explosion endpoint for `user`'s line, or `None`
/// when the line isn't a forcing-punishment shape that collapses the
/// mover-POV static eval.
///
/// The punisher is the opponent's first reply (ply 1) **only when it is
/// forcing** (a check or capture) — a quiet ply-1 reply isn't a punishing
/// tactic the prophylaxis "stops." The endpoint is the end of that reply's
/// forcing tail (the contiguous run of checks / captures / forced replies),
/// where the static eval has resolved past the recapture bounce. Fires when
/// the mover-POV eval at the endpoint is ≥ [`PROPHYLAXIS_EXPLOSION_CP`]
/// below ply 0.
///
/// Public so the retrospective UI builder can recover the punisher move's
/// squares for its trigger arrow without re-implementing the scan.
pub fn prophylaxis_explosion(
    pre_move_pos: &Position,
    user: &MoveAnalysis,
    root_stm: Color,
) -> Option<ProphylaxisExplosion> {
    let pov = ply_pov_values(&user.ply_traces, root_stm);
    if pov.len() < 2 || user.pv.len() < 2 {
        return None;
    }

    // The punisher is the opponent's reply at ply 1; it must be forcing.
    let mut board = pre_move_pos.clone();
    board.do_move(user.pv[0]);
    let punisher_mv = user.pv[1];
    let punisher_forcing =
        board.in_check() || board.is_capture(punisher_mv) || board.gives_check(punisher_mv);
    if !punisher_forcing {
        return None;
    }

    // Walk the forcing tail from the punisher (ply 1): the contiguous run of
    // checks / captures / forced replies. The last such ply is the endpoint
    // where the collapse has resolved.
    let mut endpoint_ply = 1usize;
    for (i, &mv) in user.pv.iter().enumerate().skip(1) {
        let forcing = board.in_check() || board.is_capture(mv) || board.gives_check(mv);
        board.do_move(mv);
        if forcing {
            endpoint_ply = i;
        } else {
            break;
        }
    }
    let endpoint_ply = endpoint_ply.min(pov.len() - 1);

    let swing_cp = pov[0] - pov[endpoint_ply];
    if swing_cp < PROPHYLAXIS_EXPLOSION_CP {
        return None;
    }

    Some(ProphylaxisExplosion {
        punisher_ply: 1,
        endpoint_ply,
        swing_cp,
    })
}

/// The punisher move (and the ply it lands on) for `user`'s exploding
/// line, or `None` when no explosion is present. The UI builder uses this
/// to arrow the punisher on the board; the same scan the claim builder
/// gates on, exposed so the annotation and the prose agree on the move.
pub fn prophylaxis_punisher_move(
    pre_move_pos: &Position,
    user: &MoveAnalysis,
    root_stm: Color,
) -> Option<(usize, Move)> {
    let e = prophylaxis_explosion(pre_move_pos, user, root_stm)?;
    user.pv.get(e.punisher_ply).copied().map(|m| (e.punisher_ply, m))
}

/// The worst sustained mover-POV drop along the *best* line's forcing
/// tail, measured the same way as the user line — so Gate 4 compares
/// like-for-like (the recapture-bounce-resistant endpoint, not a transient
/// dip). `None` when the best line is too short. A best line whose forcing
/// tail still collapses the mover's eval means "everything loses," not
/// "the best move prevents it" — that disqualifies the prophylaxis card.
fn best_line_worst_drop(
    pre_move_pos: &Position,
    best: &MoveAnalysis,
    root_stm: Color,
) -> Option<i32> {
    let pov = ply_pov_values(&best.ply_traces, root_stm);
    if pov.len() < 2 {
        return None;
    }
    // Walk the best line's forcing tail from ply 0 and take the eval at its
    // resolved endpoint, the same recapture-resistant point the user-line
    // scan uses.
    let mut board = pre_move_pos.clone();
    let mut endpoint = 0usize;
    for (i, &mv) in best.pv.iter().enumerate() {
        let forcing = board.in_check() || board.is_capture(mv) || board.gives_check(mv);
        board.do_move(mv);
        if forcing {
            endpoint = i;
        } else {
            break;
        }
    }
    let endpoint = endpoint.min(pov.len() - 1);
    Some(pov[0] - pov[endpoint])
}

/// Per-ply mover-POV (root-STM POV) static eval values along a PV's
/// `ply_traces`. `ply_traces[i]` is evaluated at the position after
/// `pv[0..=i]`; [`stm_after_ply`] gives the side to move there, and
/// `white_pov_value` strips tempo + the side flip, leaving a white-POV
/// number we re-sign to the mover.
fn ply_pov_values(traces: &[EvalTrace], root_stm: Color) -> Vec<i32> {
    let sign = if root_stm == Color::White { 1 } else { -1 };
    traces
        .iter()
        .enumerate()
        .map(|(i, t)| sign * t.white_pov_value(stm_after_ply(root_stm, i)).0)
        .collect()
}

/// Render the punisher move (the explosion-ply move) to SAN from the
/// position it is played in — `pre_move_pos` advanced through the user's
/// PV up to (not including) the explosion ply.
fn punisher_san(pre_move_pos: &Position, user_pv: &[Move], explosion_ply: usize) -> String {
    let mut board = pre_move_pos.clone();
    for &mv in user_pv.iter().take(explosion_ply) {
        board.do_move(mv);
    }
    san::format(&board, user_pv[explosion_ply])
}

/// The dominant non-material term that *worsened* in the mover's POV from
/// `baseline` to `climax`. Mirrors Feature 1's compensation scan but in
/// reverse: the most mover-*un*favourable tapered delta (excluding the
/// `Material*` recapture terms). `None` when nothing actually worsened.
fn dominant_worsened_term(
    baseline: &EvalTrace,
    climax: &EvalTrace,
    root_stm: Color,
) -> Option<TermId> {
    let sign = if root_stm == Color::White { 1 } else { -1 };
    let mut worst: Option<(TermId, i32)> = None; // (term, delta) — most negative
    for &term in &TermId::ALL {
        if is_material_term(term) {
            continue;
        }
        let pre = sign * tapered_cp(term.net_score(baseline), climax.phase, climax.scale_factor);
        let post = sign * tapered_cp(term.net_score(climax), climax.phase, climax.scale_factor);
        let delta = post - pre;
        if worst.map_or(true, |(_, d)| delta < d) {
            worst = Some((term, delta));
        }
    }
    let (term, delta) = worst?;
    (delta < 0).then_some(term)
}

/// How many plies of the punisher's forcing line the replay test walks.
/// Long enough to reach the case-study refutation (`Rxe7+ Kxe7 Re1+ Kd7
/// Qxf8 Rxf8` — 6 plies) with headroom, short enough to stay cheap (no
/// search; one greedy static-eval pick per ply).
const PROPHYLAXIS_REPLAY_PLIES: usize = 8;

/// The replay/disambiguation test: does playing `best_mv` *refute* the
/// punishing line? Apply `best_mv` to a clone of `pre_move_pos`, then
/// replay the punisher's forcing tail — but on the **new** position, where
/// `best_mv` may have changed the defence.
///
/// The replay is a tiny greedy walk, not a blind PV replay: the opponent
/// plays its forcing punisher moves (taken from the user's PV while they
/// stay legal — the same checks / captures that exploded the original
/// line), and the **mover plays its static-eval-best legal reply each
/// turn**. That is what lets `best_mv`'s contribution show: in the case
/// study `Ra8` puts a rook on the 8th rank, so when the line reaches
/// `Qxf8+` the mover's best reply is now `Rxf8` — capturing the queen and
/// refuting the sac. A blind PV replay would force the original `…Kc7` and
/// miss it.
///
/// Refuted (⇒ prophylaxis) when, after the walk, the mover-POV static eval
/// no longer collapsed — it holds within [`PROPHYLAXIS_REFUTED_CP`] of the
/// post-`best_mv` level, i.e. the punisher's swing has been answered. NOT
/// refuted (⇒ deferred own-tactic, Feature 1's framing) when the eval still
/// collapses despite the mover's best defence.
fn punisher_refuted_by_best(
    pre_move_pos: &Position,
    best_mv: Move,
    user_pv: &[Move],
    punisher_ply: usize,
    root_stm: Color,
) -> bool {
    let mut board = pre_move_pos.clone();
    board.do_move(best_mv);
    // Level baseline from the mover's POV after the best move (opponent to
    // move). `evaluate` is side-to-move POV; re-sign to the mover.
    let level = mover_pov_eval(&board, root_stm);

    // Greedy forcing replay. The opponent's intended forcing moves come
    // from the user PV (from `punisher_ply` onward); the mover answers with
    // its eval-best legal reply. Stop when the line goes quiet (the
    // opponent's intended move is no longer forcing / not legal) or after
    // the ply budget.
    let mut opp_idx = punisher_ply;
    for _ in 0..PROPHYLAXIS_REPLAY_PLIES {
        if board.side_to_move() == root_stm {
            // Mover's turn: pick its static-eval-best legal reply (the
            // defence). This is where `best_mv`'s resource — e.g. the a8
            // rook capturing on f8 — gets to fire.
            let Some(reply) = mover_best_reply(&mut board, root_stm) else {
                break; // no legal move (mate / stalemate) — leave eval as is
            };
            board.do_move(reply);
        } else {
            // Opponent's turn: replay its intended forcing punisher move
            // while it stays legal *and* forcing; otherwise the punishment
            // has run out of forcing steam, so stop.
            let Some(&intended) = user_pv.get(opp_idx) else {
                break;
            };
            opp_idx += 1;
            // Validate legality on THIS (divergent) board BEFORE probing the
            // move. The mover answered with its own eval-best reply rather
            // than the user PV, so `intended`'s origin square may be empty
            // here — and `gives_check`/`is_capture` panic on an empty origin
            // (`moved_piece` expects a piece on the from-square).
            let Some(mv) = legal_matching(&mut board, intended) else {
                // The best move made the opponent's intended forcing move
                // illegal — the punishment is structurally gone.
                return true;
            };
            let forcing = board.in_check() || board.is_capture(mv) || board.gives_check(mv);
            if !forcing {
                break;
            }
            board.do_move(mv);
        }
    }

    // After the greedy defence, did the mover hold? Read the mover-POV
    // static eval (re-signed). If it never collapsed past the level, the
    // best move refuted the punisher ⇒ prophylaxis.
    let held = mover_pov_eval(&board, root_stm);
    level - held < PROPHYLAXIS_REFUTED_CP
}

/// The mover's static-eval-best legal reply in `pos` (a one-ply greedy
/// pick, no search), or `None` when there are no legal moves. Used by the
/// replay test so the defender gets to play its refuting resource (the
/// capture `best_mv` enabled) rather than a blindly-replayed PV move.
fn mover_best_reply(pos: &mut Position, mover: Color) -> Option<Move> {
    legal_moves_vec(pos)
        .into_iter()
        .map(|mv| {
            let mut after = pos.clone();
            after.do_move(mv);
            // After the mover's reply it's the opponent's turn; re-sign the
            // side-to-move eval to the mover's POV.
            (mv, mover_pov_eval(&after, mover))
        })
        .max_by_key(|&(_, v)| v)
        .map(|(mv, _)| mv)
}

/// `pos`'s static eval in `mover` (root-STM) POV. `evaluate` returns a
/// side-to-move-POV value; flip it when the side to move isn't the mover.
fn mover_pov_eval(pos: &Position, mover: Color) -> i32 {
    let v = evaluate(pos).0;
    if pos.side_to_move() == mover {
        v
    } else {
        -v
    }
}

/// Find the legal move matching `wanted`'s `from → to` in `pos`, if any.
/// The punisher recovered from a PV carries its full encoding; re-matching
/// against freshly generated legal moves confirms it is still playable
/// after the best move (and picks up the correct flags, e.g. promotion).
fn legal_matching(pos: &mut Position, wanted: Move) -> Option<Move> {
    legal_moves_vec(pos)
        .into_iter()
        .find(|m| m.from() == wanted.from() && m.to() == wanted.to())
}

/// Taper a packed `(mg, eg)` [`Score`] to engine-cp at `phase` +
/// `scale_factor`, matching the evaluator's blend formula (the same one
/// `analysis::term_delta::tapered_cp` applies). Replicated here because
/// that function is engine-private and the positional-win card needs to
/// taper individual net term scores at the *climax* phase, not the ply-1
/// phase the precomputed `term_deltas` use.
fn tapered_cp(score: Score, phase: i32, scale_factor: i32) -> i32 {
    const PHASE_MAX: i32 = 128;
    const SCALE_NORMAL: i32 = 64;
    let mg_part = score.mg().0 * phase;
    let eg_part = score.eg().0 * (PHASE_MAX - phase) * scale_factor / SCALE_NORMAL;
    (mg_part + eg_part) / PHASE_MAX
}

#[cfg(test)]
#[path = "claim_tests.rs"]
mod tests;
