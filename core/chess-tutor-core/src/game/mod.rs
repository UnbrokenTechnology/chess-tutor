//! Live game state.
//!
//! The core's primary runtime object. Unit-testable from any FEN; static
//! analysis is just "construct a `Game` from this FEN and inspect the current
//! position."
//!
//! Play loop:
//! 1. UI asks `legal_moves()` or `legal_moves_from(square)` to highlight.
//! 2. User picks a move; UI calls [`Game::apply`].
//! 3. [`Game::apply`] returns a [`MoveReport`] carrying the classification,
//!    the pre-move [`PositionAnalysis`], and the missed-idea comparison.
//! 4. If it's the bot's turn, the UI asks the bot for its move and applies
//!    it the same way.
//!
//! Phase 1 stub: the state machinery is in place; the [`MoveReport`]
//! classification is populated by the Phase 1 analysis + classifier work,
//! and the engine-check / bot fields arrive in Phase 2.

use serde::{Deserialize, Serialize};
use shakmaty::fen::Fen;
use shakmaty::san::SanPlus;
use shakmaty::uci::Uci;
use shakmaty::{CastlingMode, Chess, Color, EnPassantMode, Outcome, Position};

use crate::{Error, PositionAnalysis, Result, ANALYSIS_SCHEMA_VERSION};

/// Who controls a given colour. Persisted with the game so save/restore
/// preserves it; the UI drives turn sequencing based on the current side
/// to move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlayerKind {
    Human,
    Bot,
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Side {
    White,
    Black,
}

impl From<Color> for Side {
    fn from(c: Color) -> Self {
        match c {
            Color::White => Side::White,
            Color::Black => Side::Black,
        }
    }
}

/// Coarse classification of a move. Shown to the user as a one-line tag with
/// the full deep-dive a click away. Populated from the eval-delta between
/// the played move and the best candidate, combined with our own heuristics
/// (e.g. Book is a library lookup, Forced means only legal reply).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MoveClass {
    Best,
    Excellent,
    Good,
    Inaccuracy,
    Mistake,
    Blunder,
    Book,
    Forced,
    Unclassified,
}

/// Single entry in the move list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub san: String,
    pub uci: String,
    pub fen_before: String,
    pub fen_after: String,
    pub class: MoveClass,
}

/// The teaching artefact returned by [`Game::apply`]. The UI renders the
/// `class` + `tagline` immediately; `deep_dive` is hydrated on demand (it
/// carries the pre-move analysis, which is expensive).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveReport {
    pub entry: HistoryEntry,
    pub class: MoveClass,
    pub tagline: String,
    pub deep_dive: Option<DeepDive>,
}

/// Full context for "here's what you should have seen" — the pre-move
/// analysis plus a paired view of what the user played vs. what the engine /
/// our analyser preferred.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepDive {
    pub pre_move_analysis: PositionAnalysis,
    pub played_uci: String,
    pub best_uci: Option<String>,
    /// The missed idea in one prioritised sentence — populated by the
    /// explainer once templates are wired up.
    pub missed_idea: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GameStatus {
    Ongoing,
    Checkmate { winner: Side },
    Stalemate,
    InsufficientMaterial,
    FiftyMoveRule,
    ThreefoldRepetition,
    Resigned { winner: Side },
    DrawAgreed,
}

/// Placeholder for clocks. Carried on the game so it survives save/restore,
/// but the UI ignores it in v1 (see `PLAN.md` → Phase 7).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TimeControl {
    pub initial_ms: Option<u64>,
    pub increment_ms: Option<u64>,
    pub white_remaining_ms: Option<u64>,
    pub black_remaining_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Game {
    position: Chess,
    history: Vec<HistoryEntry>,
    white_player: PlayerKind,
    black_player: PlayerKind,
    status: GameStatus,
    time_control: TimeControl,
}

impl Game {
    pub fn new_standard(white_player: PlayerKind, black_player: PlayerKind) -> Self {
        Self {
            position: Chess::default(),
            history: Vec::new(),
            white_player,
            black_player,
            status: GameStatus::Ongoing,
            time_control: TimeControl::default(),
        }
    }

    pub fn from_fen(fen: &str, white: PlayerKind, black: PlayerKind) -> Result<Self> {
        let parsed: Fen = fen.parse().map_err(|e| Error::InvalidFen(format!("{e}")))?;
        let position: Chess = parsed
            .into_position(CastlingMode::Standard)
            .map_err(|e| Error::InvalidFen(format!("{e}")))?;
        Ok(Self {
            position,
            history: Vec::new(),
            white_player: white,
            black_player: black,
            status: GameStatus::Ongoing,
            time_control: TimeControl::default(),
        })
    }

    pub fn side_to_move(&self) -> Side {
        self.position.turn().into()
    }

    pub fn player_to_move(&self) -> PlayerKind {
        match self.position.turn() {
            Color::White => self.white_player,
            Color::Black => self.black_player,
        }
    }

    pub fn status(&self) -> GameStatus {
        self.status
    }

    pub fn history(&self) -> &[HistoryEntry] {
        &self.history
    }

    pub fn time_control(&self) -> &TimeControl {
        &self.time_control
    }

    pub fn fen(&self) -> String {
        Fen::from_position(&self.position, EnPassantMode::Legal).to_string()
    }

    /// All legal moves as UCI strings. Shells use this for generic legality
    /// checks; `legal_moves_from` is cheaper for per-square highlighting.
    pub fn legal_moves(&self) -> Vec<String> {
        self.position
            .legal_moves()
            .into_iter()
            .map(|m| m.to_uci(CastlingMode::Standard).to_string())
            .collect()
    }

    /// Apply a move given as UCI (e.g. "e2e4", "e7e8q"). Rejects illegal
    /// moves. Returns a [`MoveReport`] with `class = Unclassified` and
    /// `deep_dive = None` until the Phase 1 analysis + classifier land —
    /// the history entry and FENs are correct today.
    pub fn apply(&mut self, uci: &str) -> Result<MoveReport> {
        if !matches!(self.status, GameStatus::Ongoing) {
            return Err(Error::Engine(format!(
                "cannot apply move — game status is {:?}",
                self.status
            )));
        }

        let fen_before = self.fen();

        let parsed: Uci = uci
            .parse()
            .map_err(|e| Error::InvalidFen(format!("bad UCI {uci}: {e}")))?;
        let mv = parsed
            .to_move(&self.position)
            .map_err(|e| Error::InvalidFen(format!("illegal move {uci}: {e}")))?;

        // `SanPlus::from_move_and_play_unchecked` both builds the SAN (with
        // `+` / `#` suffix) *and* applies the move; one call instead of two.
        let san = SanPlus::from_move_and_play_unchecked(&mut self.position, &mv).to_string();
        let fen_after = self.fen();

        self.refresh_status();

        let entry = HistoryEntry {
            san,
            uci: uci.to_string(),
            fen_before,
            fen_after,
            class: MoveClass::Unclassified,
        };
        self.history.push(entry.clone());

        Ok(MoveReport {
            entry,
            class: MoveClass::Unclassified,
            tagline: String::new(),
            deep_dive: None,
        })
    }

    /// Pop the last move. Intentionally loses any cached explanation tied to
    /// the popped position — cheap enough to recompute lazily.
    pub fn undo(&mut self) -> Option<HistoryEntry> {
        // shakmaty has no direct `pop` — replay from startpos minus the last
        // move. Fine for interactive pacing; we'll persist an undo stack
        // alongside the position later if the perf shows up on a profile.
        if self.history.is_empty() {
            return None;
        }
        let popped = self.history.pop().expect("non-empty above");
        let mut replay = Chess::default();
        for entry in &self.history {
            let parsed: Uci = entry.uci.parse().expect("stored UCI is valid");
            let mv = parsed.to_move(&replay).expect("stored move is legal");
            replay.play_unchecked(&mv);
        }
        self.position = replay;
        self.status = GameStatus::Ongoing;
        Some(popped)
    }

    pub fn resign(&mut self, side: Side) {
        let winner = match side {
            Side::White => Side::Black,
            Side::Black => Side::White,
        };
        self.status = GameStatus::Resigned { winner };
    }

    pub fn agree_draw(&mut self) {
        self.status = GameStatus::DrawAgreed;
    }

    fn refresh_status(&mut self) {
        self.status = match self.position.outcome() {
            Some(Outcome::Decisive { winner }) => GameStatus::Checkmate {
                winner: winner.into(),
            },
            Some(Outcome::Draw) => {
                if self.position.is_stalemate() {
                    GameStatus::Stalemate
                } else if self.position.is_insufficient_material() {
                    GameStatus::InsufficientMaterial
                } else {
                    // shakmaty doesn't distinguish 50-move vs. threefold in
                    // `outcome()`; refine later from the move history.
                    GameStatus::DrawAgreed
                }
            }
            None => GameStatus::Ongoing,
        };
    }
}

/// Analyse the current position of a game. Hook used by the per-move
/// classifier once the Phase 1 analysis lands.
pub fn analyse_game_position(game: &Game) -> Result<PositionAnalysis> {
    let mut pa = PositionAnalysis::empty(game.fen());
    pa.schema_version = ANALYSIS_SCHEMA_VERSION;
    Ok(pa)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startpos_has_20_legal_moves() {
        let g = Game::new_standard(PlayerKind::Human, PlayerKind::Human);
        assert_eq!(g.legal_moves().len(), 20);
    }

    #[test]
    fn applies_and_undoes_e4() {
        let mut g = Game::new_standard(PlayerKind::Human, PlayerKind::Human);
        let report = g.apply("e2e4").expect("e4 is legal from startpos");
        assert_eq!(report.entry.san, "e4");
        assert_eq!(g.side_to_move(), Side::Black);
        assert_eq!(g.history().len(), 1);

        let undone = g.undo().expect("history non-empty");
        assert_eq!(undone.san, "e4");
        assert_eq!(g.side_to_move(), Side::White);
        assert!(g.history().is_empty());
    }

    #[test]
    fn rejects_illegal_move() {
        let mut g = Game::new_standard(PlayerKind::Human, PlayerKind::Human);
        assert!(g.apply("e2e5").is_err());
    }

    #[test]
    fn detects_fools_mate() {
        let mut g = Game::new_standard(PlayerKind::Human, PlayerKind::Human);
        for uci in ["f2f3", "e7e5", "g2g4", "d8h4"] {
            g.apply(uci).unwrap();
        }
        assert!(matches!(
            g.status(),
            GameStatus::Checkmate { winner: Side::Black }
        ));
    }
}
