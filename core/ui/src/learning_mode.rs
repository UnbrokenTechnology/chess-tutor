//! Learning-mode preferences — how much live help the student gets
//! during play, how (and whether) the session pauses on mistakes, and
//! whether the engine's preferred moves are ever revealed.
//!
//! The three axes are deliberately independent so the user can mix
//! them: "coached overlays during my turn + silent during mistakes +
//! blunder safety on" is just as valid as "silent everything." The
//! [`LearningPreset`] enum collapses common combinations into named
//! presets for the New Game UI; advanced users tune the axes directly.
//!
//! **Pedagogical principle.** The engine knows the best move; the
//! student needs to *develop* the skill of finding it. Revealing the
//! engine's choice short-circuits that practice and trains rote
//! memorisation — so every reveal is opt-in, every pause is gated to
//! genuine teaching moments (not every non-best move), and the
//! retrospective never tells the student what to do, only what they
//! missed.

/// Live overlays / cues shown during the user's turn, *before* they
/// commit a move.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AssistanceLevel {
    /// No live help during play. Pure decision-making — the user
    /// thinks through the position with only the board and their own
    /// candidate-move calculation.
    #[default]
    Off,
    /// Overlay opponent threats only (Dvoretsky-style prophylactic
    /// thinking). The user sees what the opponent is up to but never
    /// receives suggestions about their own play.
    Prophylactic,
    /// Surface position features — imbalances, weak squares, outposts —
    /// without naming any move (Silman-style imbalances trainer). The
    /// student is shown *what to look at*, not *what to do about it*.
    Coached,
}

/// What happens after the user commits a non-best move.
///
/// The gate for what counts as "non-best enough to interrupt for"
/// lives in [`chess_tutor_engine::analysis::classify_user_move`];
/// see that module for the dominant-term / share-of-drop rules.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MistakeHandling {
    /// Silent during play; everything goes to the retrospective. The
    /// strongest evidence-aligned mode (Dvoretsky's annotated-game
    /// method): the student plays through their own decisions and
    /// reviews afterward.
    #[default]
    SilentRetrospective,
    /// Pause on detected teaching moments — moves with a dominant,
    /// teachable eval-term shift. **Not every non-best move.** The
    /// classifier filters out noise / engine subtlety so the prompt
    /// only fires when there's a concrete chess concept the user
    /// could have spotted.
    TeachingMoments,
    /// Pause on every non-best move. High crutch risk; intended for
    /// short onboarding walkthroughs, not regular play.
    AllMistakes,
}

/// Safety net for material loss — independent of any pedagogical
/// dimension.
///
/// The student already *knows* when they hang a piece; the safety net
/// saves the game's time rather than teaching anything. Off-by-default
/// could be the right call if we want students to develop their own
/// blunder-check habit; on-by-default fits a "tool that respects my
/// time" framing. The user-facing toggle lets each player choose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlunderSafety {
    /// No safety net — every move stands, including blunders. Forces
    /// the student to develop their own pre-commit blunder check.
    #[default]
    Off,
    /// After a blunder is committed, offer "take back" with no
    /// teaching content (the student doesn't need to be told they
    /// hung a piece). Acceptance reverts the move; declining lets
    /// the game continue.
    OfferTakeback,
}

/// Bundle of all learning-mode preferences. Stored on
/// [`crate::session::Session`]; persists across moves within a game
/// and across games for the session. (Persistence to disk is a
/// future deliverable.)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LearningPreferences {
    pub assistance: AssistanceLevel,
    pub mistake_handling: MistakeHandling,
    pub blunder_safety: BlunderSafety,
    /// Whether the engine's preferred move is ever shown in the
    /// retrospective (text label and arrow on the board). Off by
    /// default to keep the retrospective focused on *why* the user's
    /// move was an inaccuracy rather than *what* they should have
    /// played — telling the student the answer trains memorisation,
    /// not understanding.
    pub reveal_best_moves: bool,
}

/// Named combinations of the three axes, surfaced as a single picker
/// in the New Game UI. "Custom" is the catch-all for users who tuned
/// the axes individually.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LearningPreset {
    /// Silent during play; retrospective only. The default for
    /// students above true beginner.
    #[default]
    Practicing,
    /// Silent during play, but pauses on detected teaching moments
    /// (with a takeback option after blunders).
    Supported,
    /// Live coaching overlay (features named, never moves) + teaching
    /// pauses + blunder safety. The most-help mode short of revealing
    /// engine moves.
    Coached,
    /// Bespoke combination of axes.
    Custom,
}

/// State of an active mid-game intervention, owned by
/// [`crate::session::Session`]. While `Some`, the engine reply is
/// held until the user dismisses or takes back.
///
/// Both the blunder and teaching dimensions can be populated
/// simultaneously (one move can trip both gates). The UI typically
/// shows the blunder prompt with priority and surfaces the teaching
/// dimension in the retrospective once the user continues.
#[derive(Clone, Debug)]
pub struct PendingIntervention {
    /// Index into `Session::history` of the move that triggered this
    /// intervention. Used so a take-back doesn't try to undo the wrong
    /// move if state shifts unexpectedly.
    pub at_history_index: usize,
    /// The move the user committed.
    pub original_move: chess_tutor_engine::types::Move,
    /// Structured assessment from the engine classifier.
    pub assessment: chess_tutor_engine::analysis::MoveAssessment,
    /// `true` once the user has clicked "Show me what I missed" so
    /// the renderer expands the concept-reveal panel. Game state is
    /// untouched.
    pub concept_revealed: bool,
}

/// Build the renderer-facing [`crate::view::InterventionPanelView`]
/// from a pending intervention. Blunder takes priority — when both
/// gates fired, the in-game prompt is about the material loss and
/// the teaching dimension surfaces in the retrospective. The
/// returned panel never names the engine's preferred move.
pub fn build_intervention_panel(
    pending: &PendingIntervention,
) -> crate::view::InterventionPanelView {
    use crate::view::{InterventionAction, InterventionPanelKind, InterventionPanelView};
    if let Some(b) = pending.assessment.blunder {
        let headline = match b.lost_piece_square {
            Some(sq) => format!(
                "Your piece on {} is in danger.",
                sq.to_algebraic()
            ),
            None => "That move loses material.".to_string(),
        };
        let summary = format!(
            "About {:.1} pawns at risk.",
            (b.material_loss_cp as f32) / 100.0
        );
        return InterventionPanelView {
            kind: InterventionPanelKind::BlunderSafety,
            headline,
            summary,
            concept: None,
            actions: vec![InterventionAction::TakeBack, InterventionAction::Continue],
        };
    }
    let teaching = pending
        .assessment
        .teaching
        .expect("intervention requires either blunder or teaching");
    let (headline, summary, concept) = teaching_prompt(teaching);
    let mut actions = vec![InterventionAction::TakeBack];
    if !pending.concept_revealed {
        actions.push(InterventionAction::RevealConcept);
    }
    actions.push(InterventionAction::Continue);
    crate::view::InterventionPanelView {
        kind: InterventionPanelKind::TeachingMoment,
        headline,
        summary,
        concept: if pending.concept_revealed {
            Some(concept)
        } else {
            None
        },
        actions,
    }
}

/// Build the (headline, summary, concept_reveal) triple for a
/// teaching-moment prompt. The headline never names the engine's
/// preferred move; the concept reveal names the *specific* concept
/// (one or two granular [`chess_tutor_engine::analysis::TermId`]s)
/// and frames it without naming squares or pieces by coordinate.
fn teaching_prompt(
    info: chess_tutor_engine::analysis::TeachingInfo,
) -> (String, String, String) {
    let (area_a, concept_a) = term_prompt_copy(info.dominant.term);
    match info.secondary {
        None => {
            let headline = format!("I noticed something about {}.", area_a);
            let summary = format!(
                "About {:.1} pawns of swing concentrated here.",
                (info.dominant.severity_cp as f32) / 100.0
            );
            (headline, summary, concept_a.to_string())
        }
        Some(secondary) => {
            let (area_b, concept_b) = term_prompt_copy(secondary.term);
            let headline = format!(
                "I noticed two things — {} and {}.",
                area_a, area_b
            );
            let combined_pawns =
                ((info.dominant.severity_cp + secondary.severity_cp) as f32) / 100.0;
            let summary = format!(
                "About {:.1} pawns of swing split between both.",
                combined_pawns
            );
            // Two separate concept paragraphs, prefixed so the student
            // can tell which is which when reading. Capitalise the
            // first letter of each area for the headers.
            let concept = format!(
                "{}: {}\n\n{}: {}",
                capitalize_first(area_a),
                concept_a,
                capitalize_first(area_b),
                concept_b,
            );
            (headline, summary, concept)
        }
    }
}

/// Uppercase the first character of `s`, leaving the rest unchanged.
/// Used to title-case the area phrase as a multi-term concept header.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Per-[`TermId`] copy for the intervention prompt. Returns
/// `(area_phrase, concept_paragraph)` — area drops into the headline
/// "I noticed something about {area}." and concept is the
/// expand-on-click body. Specific enough that the student can act on
/// it without the engine telling them the move.
pub(crate) fn term_prompt_copy(
    term: chess_tutor_engine::analysis::TermId,
) -> (&'static str, &'static str) {
    use chess_tutor_engine::analysis::TermId;
    match term {
        // MaterialPieceValue is excluded by the engine classifier — the
        // blunder gate handles it. This arm is unreachable in practice
        // but is here for exhaustiveness.
        TermId::MaterialPieceValue => (
            "the material balance",
            "Material balance shifted. (This shouldn't normally fire as \
             a teaching moment — please report if you see it.)",
        ),
        TermId::MaterialPsqPositional => (
            "where one of your pieces ended up",
            "One of your pieces moved to a square the piece-square \
             tables rank lower for that piece type. The opening's \
             textbook 'good square for a knight / bishop / rook' \
             prefers a different square. Common cases: knight on the \
             rim, bishop without scope, queen out too early.",
        ),
        TermId::Imbalance => (
            "the piece imbalance",
            "The mix of pieces on each side shifted in a way that \
             slightly favours your opponent's coordination — most \
             commonly: a knight-for-bishop trade in a position where \
             the bishop's diagonals will matter, or surrendering the \
             bishop pair.",
        ),
        TermId::Initiative => (
            "the initiative",
            "The initiative — who's forcing whom to react — moved \
             toward your opponent. Often this means you played a slow \
             move when there was a forcing alternative, or you \
             released tension that was holding their pieces back.",
        ),
        TermId::Space => (
            "central space",
            "The central space — the safe squares behind your pawns \
             where your pieces can reinforce — shifted against you. \
             Common causes: a pawn move that vacates a key central \
             square, or one that lets the opponent's pawns push \
             through.",
        ),
        TermId::KingPawnShield => (
            "your king's pawn shield",
            "The pawns directly in front of your king moved or were \
             traded, opening lines toward the king. Shield damage is \
             cheap to create and expensive to repair — once the \
             position opens up, even one missing shield pawn matters.",
        ),
        TermId::KingPawnStorm => (
            "the enemy pawn storm near your king",
            "The opponent's pawns near your king advanced (or got \
             closer to advancing). When kings are on opposite wings, \
             pawn storms are race conditions — meeting them with \
             your own counter-attack matters more than passive \
             defence.",
        ),
        TermId::KingPawnDistance => (
            "where your king sits relative to your pawns",
            "Your king ended up further from your own pawns. In the \
             endgame the king's job is to support pawns — when it \
             strays, the pawns lose a defender and the tempo to push.",
        ),
        TermId::KingDanger => (
            "your king's safety",
            "King safety swung against you — more enemy attackers \
             can reach the squares around your king, or a defender \
             just moved away. Check the squares directly around the \
             king and the diagonals and files leading toward it.",
        ),
        TermId::KingPawnlessFlank => (
            "the flank near your king",
            "A flank near your king ended up with no pawns of either \
             colour — wide-open files and diagonals that the \
             opponent's rooks and bishops can swing into. Trade \
             attackers before they double up.",
        ),
        TermId::KingFlankAttacks => (
            "attackers on your king's flank",
            "Enemy pieces accumulated attacks on the squares near \
             your king's flank. This is a build-up signal more than \
             an immediate threat — but build-ups become tactics if \
             you don't address them.",
        ),
        TermId::PassedRankBonus => (
            "a passed pawn's progress",
            "Either your opponent's passed pawn advanced (closer to \
             promotion) or yours regressed in priority. Passed pawns \
             are exponentially more valuable the further they go — \
             every rank matters.",
        ),
        TermId::PassedKingProximity => (
            "your king's role in the passed-pawn race",
            "Your king moved away from a passed pawn it should be \
             stopping (theirs) or supporting (yours). In endgames, \
             the king is the passed pawn's escort or executioner — \
             its position is decisive.",
        ),
        TermId::PassedFreeAdvance => (
            "a passed pawn's path",
            "Your opponent's passed pawn has a clearer path to \
             promotion (or yours got more blocked). 'Blockade or get \
             out of the way' — passed pawns need stoppers, not just \
             observers.",
        ),
        TermId::PassedStopperPenalty => (
            "the blockade in front of a passed pawn",
            "The blockade-on-passed-pawn balance shifted: their \
             stopper got heavier or yours got lighter. Stopping a \
             passed pawn needs a piece on the square in front of it, \
             not just attacking it from the side.",
        ),
        TermId::PawnsConnected => (
            "your pawn chain",
            "Your pawn chain broke or weakened — fewer pawns are \
             supporting each other diagonally. Connected pawns \
             defend each other; isolated ones rely on pieces, which \
             is expensive.",
        ),
        TermId::PawnsIsolated => (
            "an isolated pawn",
            "A pawn ended up isolated — no friendly pawn on either \
             adjacent file. Isolated pawns need pieces to defend \
             them, and the square in front of an isolated pawn is a \
             permanent hole for the opponent's pieces.",
        ),
        TermId::PawnsBackward => (
            "a backward pawn",
            "A pawn ended up backward — behind its file's neighbours \
             and unable to advance safely (an enemy pawn would \
             capture it). The square in front of a backward pawn is \
             a permanent weakness.",
        ),
        TermId::PawnsDoubled => (
            "doubled pawns",
            "You doubled one of your pawn files. Two pawns on the \
             same file can't support each other diagonally — that's \
             a structural concession that's hard to undo.",
        ),
        TermId::PawnsWeakUnopposed => (
            "a weak pawn",
            "A pawn became 'weak unopposed' — no enemy pawn opposes \
             it on its file, AND no friendly pawn defends it. Open \
             target for the opponent's rooks.",
        ),
        TermId::PawnsWeakLever => (
            "a pawn break against your pawns",
            "The opponent has a 'lever' — a pawn that can capture \
             into your structure to fix or break it. Look for \
             c4xb5 / d4xc5-style pawn captures they're now \
             threatening.",
        ),
        TermId::PiecesOutposts => (
            "an outpost",
            "An outpost — a square defended by your pawn that the \
             opponent's pawns can't kick away — changed hands. \
             Either your minor lost its outpost square, or the \
             opponent's minor reached one. Outposts are knight \
             country; bishops do well on them too.",
        ),
        TermId::PiecesReachableOutposts => (
            "an outpost route",
            "The outposts your knight can reach in a few hops \
             shrank — a pawn push or piece move blocked the route. \
             Knight routes matter as much as knight squares: a \
             knight that can't manoeuvre is wasted.",
        ),
        TermId::PiecesMinorBehindPawn => (
            "a minor piece's pawn cover",
            "Your minor piece is no longer directly behind one of \
             your own pawns. The pawn-cover is a small bonus: it \
             shields the minor from captures along its file and \
             tends to support pawn pushes.",
        ),
        TermId::PiecesKingProtector => (
            "your minor pieces' king defence",
            "One of your minors drifted further from your own king. \
             Knights and bishops within reach of home help shield \
             the king — when they wander, the king's defence thins.",
        ),
        TermId::PiecesBishopPawns => (
            "your bishop's diagonals",
            "Your bishop has more of your own pawns sitting on its \
             colour — those pawns block its diagonals. Either trade \
             the bishop, or push the blocking pawns off its colour.",
        ),
        TermId::PiecesLongDiagonalBishop => (
            "a bishop's long diagonal",
            "Your bishop left the long diagonal (or its long \
             diagonal got blocked). Bishops that hit both central \
             squares from the long diagonal exert massive pressure \
             on the centre from a single piece.",
        ),
        TermId::PiecesRookOnQueenFile => (
            "a rook lined up with the enemy queen",
            "A rook of yours left the file the enemy queen sits on \
             — that latent x-ray pressure is gone. Rooks on the \
             queen's file matter even with pieces in between, \
             because they become tactics the moment the file opens.",
        ),
        TermId::PiecesRookOnOpenFile => (
            "the open file",
            "A rook of yours left an open file (no pawns of either \
             colour) — or the opponent took the file. Open files \
             are the rook's natural element; whoever owns them \
             controls the rank and the squares they lead to.",
        ),
        TermId::PiecesRookOnSemiopenFile => (
            "the semi-open file",
            "A rook of yours left a semi-open file — one with no \
             friendly pawns but enemy pawns in the way. Semi-open \
             files are how you put pressure on enemy pawns \
             directly.",
        ),
        TermId::PiecesTrappedRook => (
            "a trapped rook",
            "One of your rooks got hemmed in behind its own king \
             (typically after losing castling rights). A trapped \
             rook has almost no mobility and blocks the king's \
             other escape squares.",
        ),
        TermId::PiecesWeakQueen => (
            "your queen under x-ray",
            "Your queen sees an x-ray threat from an enemy rook or \
             bishop through a single intervening piece. A discovered \
             attack from that line can win the queen unless you \
             defuse it.",
        ),
        TermId::MobilityKnight => (
            "your knight's activity",
            "Your knight covers fewer squares now. Knights live and \
             die by their squares — every step away from the centre \
             cuts roughly two moves from their reach. Check whether \
             the knight has a better route.",
        ),
        TermId::MobilityBishop => (
            "your bishop's activity",
            "Your bishop covers fewer squares now. Bishops want \
             open diagonals; blocked diagonals are wasted potential. \
             Look at what's in the way — your pawns or the \
             opponent's structure.",
        ),
        TermId::MobilityRook => (
            "your rook's activity",
            "Your rook covers fewer squares now. Rooks need files \
             and ranks — open ones if possible, but at least space \
             to swing across the board behind the pawns.",
        ),
        TermId::MobilityQueen => (
            "your queen's activity",
            "Your queen covers fewer squares now. Queens want both \
             diagonal and orthogonal reach; if one is blocked, the \
             other still needs to be open.",
        ),
        TermId::ThreatsByMinor => (
            "the minor-piece threat picture",
            "The opponent's minor pieces are attacking more of your \
             material (or yours less of theirs). Watch for forks and \
             outpost-based attacks — minor pieces attacking heavy \
             pieces is leverage.",
        ),
        TermId::ThreatsByRook => (
            "the rook-threat picture",
            "The opponent's rooks are attacking more of your pieces \
             (or yours less of theirs). Rook threats to minor pieces \
             along open files often force defensive retreats and \
             tempo loss.",
        ),
        TermId::ThreatsByKing => (
            "the king joining the attack",
            "A king walked closer to enemy material — typically in \
             the endgame, where the king is a fighting piece. Kings \
             attacking pawns is decisive when there are no checks \
             against the active king.",
        ),
        TermId::ThreatsHanging => (
            "a hanging piece",
            "A piece slipped into 'hanging' status — attacked and \
             undefended. Even if it's not the immediate move, a \
             hanging piece tends to fall to a discovered attack or \
             tactical sequence a tempo later.",
        ),
        TermId::ThreatsRestricted => (
            "your pieces being restricted",
            "More of the squares your pieces want to use are \
             attacked by opponent pawns or pieces. Restriction \
             compounds: every square your pieces can't visit \
             reroutes them through worse squares.",
        ),
        TermId::ThreatsBySafePawn => (
            "pawn attacks from safe squares",
            "Opponent pawns on safe squares are now attacking your \
             pieces (or yours less of theirs). A pawn attack on a \
             piece either wins the piece or forces a tempo-losing \
             retreat.",
        ),
        TermId::ThreatsByPawnPush => (
            "a pawn push that would attack a piece",
            "A pawn push the opponent can play soon will attack one \
             of your pieces — a one-tempo loss for you. Either move \
             the piece preemptively or set up a counter-threat that \
             overrides the push.",
        ),
        TermId::ThreatsKnightOnQueen => (
            "a knight one hop from your queen",
            "An opponent's knight is one move from attacking your \
             queen. Knight-on-queen forks are easy to miss because \
             the knight isn't the attacker yet; once it's on the \
             square, you lose a tempo at best.",
        ),
        TermId::ThreatsSliderOnQueen => (
            "a slider lined up on your queen",
            "An opponent rook or bishop is lined up against your \
             queen through a single intervening piece. A discovered \
             attack wins the queen unless you defuse it.",
        ),
    }
}

/// Decide whether `assessment` should pause the game given the
/// user's `prefs`. Returns `true` if either gate fires under the
/// preferences they've set.
pub fn intervention_required(
    assessment: &chess_tutor_engine::analysis::MoveAssessment,
    prefs: &LearningPreferences,
) -> bool {
    let blunder_active = matches!(prefs.blunder_safety, BlunderSafety::OfferTakeback)
        && assessment.blunder.is_some();
    let teaching_active = matches!(
        prefs.mistake_handling,
        MistakeHandling::TeachingMoments | MistakeHandling::AllMistakes
    ) && assessment.teaching.is_some();
    blunder_active || teaching_active
}

/// Build the [`chess_tutor_engine::analysis::GatingConfig`] that
/// matches the user's [`MistakeHandling`] preference. AllMistakes
/// loosens every gate so any non-best move with a teachable
/// component fires; TeachingMoments uses the default (strict) gates.
pub fn gating_config_for(
    handling: MistakeHandling,
) -> chess_tutor_engine::analysis::GatingConfig {
    use chess_tutor_engine::analysis::GatingConfig;
    match handling {
        MistakeHandling::AllMistakes => GatingConfig {
            noise_floor_cp: 1,
            dominant_term_share_min: 0.0,
            teaching_term_severity_min_cp: 1,
            teaching_term_severity_escape_cp: 1,
            multi_term_coverage_min: 0.0,
            ..GatingConfig::default()
        },
        MistakeHandling::TeachingMoments => GatingConfig::default(),
        // SilentRetrospective never calls the classifier; the return
        // value here is unused but we still need to satisfy the
        // function signature.
        MistakeHandling::SilentRetrospective => GatingConfig::default(),
    }
}

impl LearningPreset {
    /// Map a preset to its concrete axis settings.
    pub fn to_preferences(self) -> LearningPreferences {
        match self {
            LearningPreset::Practicing => LearningPreferences {
                assistance: AssistanceLevel::Off,
                mistake_handling: MistakeHandling::SilentRetrospective,
                blunder_safety: BlunderSafety::Off,
                reveal_best_moves: false,
            },
            LearningPreset::Supported => LearningPreferences {
                assistance: AssistanceLevel::Off,
                mistake_handling: MistakeHandling::TeachingMoments,
                blunder_safety: BlunderSafety::OfferTakeback,
                reveal_best_moves: false,
            },
            LearningPreset::Coached => LearningPreferences {
                assistance: AssistanceLevel::Coached,
                mistake_handling: MistakeHandling::TeachingMoments,
                blunder_safety: BlunderSafety::OfferTakeback,
                reveal_best_moves: false,
            },
            // Custom returns the default; callers using Custom should
            // have already populated their own preferences and ignore
            // this path.
            LearningPreset::Custom => LearningPreferences::default(),
        }
    }
}
