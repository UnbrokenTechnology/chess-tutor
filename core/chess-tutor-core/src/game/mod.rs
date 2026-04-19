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
    TimedOut { winner: Side },
}

/// Wall-clock state for a timed game. The core is wall-clock-agnostic: the UI
/// measures elapsed time between its turn and the move submission and passes
/// that into [`Game::apply_timed`]. Core deducts, adds increment, detects
/// flag. No ticking, no I/O.
///
/// Phase 1 supports Fischer increment only. Delay / Bronstein land in Phase 7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeControl {
    pub initial_ms: u64,
    pub increment_ms: u64,
    pub white_remaining_ms: u64,
    pub black_remaining_ms: u64,
}

impl TimeControl {
    /// Fischer clock: `initial` on each side's clock, add `increment` after
    /// each completed move.
    pub fn fischer(initial_ms: u64, increment_ms: u64) -> Self {
        Self {
            initial_ms,
            increment_ms,
            white_remaining_ms: initial_ms,
            black_remaining_ms: initial_ms,
        }
    }

    pub fn remaining(&self, side: Side) -> u64 {
        match side {
            Side::White => self.white_remaining_ms,
            Side::Black => self.black_remaining_ms,
        }
    }

    fn set_remaining(&mut self, side: Side, ms: u64) {
        match side {
            Side::White => self.white_remaining_ms = ms,
            Side::Black => self.black_remaining_ms = ms,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Game {
    position: Chess,
    history: Vec<HistoryEntry>,
    white_player: PlayerKind,
    black_player: PlayerKind,
    status: GameStatus,
    time_control: Option<TimeControl>,
}

impl Game {
    pub fn new_standard(white_player: PlayerKind, black_player: PlayerKind) -> Self {
        Self {
            position: Chess::default(),
            history: Vec::new(),
            white_player,
            black_player,
            status: GameStatus::Ongoing,
            time_control: None,
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
            time_control: None,
        })
    }

    /// Attach a time control to the game. Replaces any existing one.
    pub fn with_time_control(mut self, tc: TimeControl) -> Self {
        self.time_control = Some(tc);
        self
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

    pub fn time_control(&self) -> Option<&TimeControl> {
        self.time_control.as_ref()
    }

    pub fn has_time_control(&self) -> bool {
        self.time_control.is_some()
    }

    /// Milliseconds remaining for a side. `None` if the game has no clock.
    pub fn remaining_ms(&self, side: Side) -> Option<u64> {
        self.time_control.map(|tc| tc.remaining(side))
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

    /// All legal moves as SAN strings (e.g. "e4", "Nf3", "O-O"). More
    /// natural for UIs that let users type moves by algebraic notation.
    pub fn legal_moves_san(&self) -> Vec<String> {
        self.position
            .legal_moves()
            .into_iter()
            .map(|m| SanPlus::from_move(self.position.clone(), &m).to_string())
            .collect()
    }

    /// Accept a move in either SAN (`e4`, `Nf3`, `O-O`, `Qxf7#`) or UCI
    /// (`e2e4`, `g1f3`, `e1g1`) and return the canonical UCI form for the
    /// current position. SAN disambiguation uses the current position.
    ///
    /// Conveniences: `0-0` / `0-0-0` are normalised to `O-O` / `O-O-O`.
    pub fn parse_move(&self, input: &str) -> Result<String> {
        let trimmed = input.trim();
        let normalised = trimmed.replace('0', "O");

        // SAN first — more natural for a human typing in a terminal.
        if let Ok(san) = normalised.parse::<SanPlus>() {
            if let Ok(mv) = san.san.to_move(&self.position) {
                return Ok(mv.to_uci(CastlingMode::Standard).to_string());
            }
        }

        // UCI fallback.
        if let Ok(uci) = trimmed.parse::<Uci>() {
            if let Ok(mv) = uci.to_move(&self.position) {
                return Ok(mv.to_uci(CastlingMode::Standard).to_string());
            }
        }

        Err(Error::InvalidFen(format!(
            "could not parse '{trimmed}' as SAN or UCI"
        )))
    }

    /// Apply a move given as UCI (e.g. "e2e4", "e7e8q"). Rejects illegal
    /// moves. Returns a [`MoveReport`] with `class = Unclassified` and
    /// `deep_dive = None` until the Phase 1 analysis + classifier land —
    /// the history entry and FENs are correct today.
    ///
    /// For timed games use [`Game::apply_timed`]; plain `apply` does not
    /// touch the clock (useful for loading PGNs and analysis-only flows).
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

    /// Apply a move in a timed game. `elapsed_ms` is the wall-clock time the
    /// mover spent on this move (measured by the UI from "clock started for
    /// this side" to "move submitted"). Order of operations mirrors an OTB
    /// chess clock:
    ///
    /// 1. If the mover's remaining time after deduction would go negative,
    ///    they flag *before* the move registers — status becomes
    ///    [`GameStatus::TimedOut`] and the move is rejected.
    /// 2. Otherwise deduct `elapsed_ms`, add the increment, and apply the move.
    ///
    /// If the game has no time control attached, this errors — callers should
    /// use [`Game::apply`] for untimed play.
    pub fn apply_timed(&mut self, uci: &str, elapsed_ms: u64) -> Result<MoveReport> {
        let mut tc = self
            .time_control
            .ok_or_else(|| Error::Engine("apply_timed called on untimed game".into()))?;

        let mover = self.side_to_move();
        let remaining = tc.remaining(mover);
        if elapsed_ms > remaining {
            // Flagged. Opponent wins on time.
            let winner = match mover {
                Side::White => Side::Black,
                Side::Black => Side::White,
            };
            tc.set_remaining(mover, 0);
            self.time_control = Some(tc);
            self.status = GameStatus::TimedOut { winner };
            return Err(Error::Engine("flagged on the clock".into()));
        }

        // Deduct + add increment. Increment only applies if the move
        // completes successfully, so defer it until after `apply`.
        tc.set_remaining(mover, remaining - elapsed_ms);
        self.time_control = Some(tc);

        let report = self.apply(uci)?;

        // Post-move: add increment to the side that just moved.
        if let Some(mut tc) = self.time_control {
            let post = tc.remaining(mover) + tc.increment_ms;
            tc.set_remaining(mover, post);
            self.time_control = Some(tc);
        }

        Ok(report)
    }

    /// Explicitly flag a side on time (UI detected a flag without a move
    /// attempt, e.g. the user just let the clock run out).
    pub fn flag(&mut self, side: Side) {
        if let Some(mut tc) = self.time_control {
            tc.set_remaining(side, 0);
            self.time_control = Some(tc);
        }
        let winner = match side {
            Side::White => Side::Black,
            Side::Black => Side::White,
        };
        self.status = GameStatus::TimedOut { winner };
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

    #[test]
    fn apply_timed_deducts_and_adds_increment() {
        let mut g = Game::new_standard(PlayerKind::Human, PlayerKind::Human)
            .with_time_control(TimeControl::fischer(60_000, 2_000));
        g.apply_timed("e2e4", 5_000).unwrap();
        // White spent 5s, gained 2s increment = -3s net.
        assert_eq!(g.remaining_ms(Side::White), Some(57_000));
        assert_eq!(g.remaining_ms(Side::Black), Some(60_000));
    }

    #[test]
    fn apply_timed_on_untimed_game_errors() {
        let mut g = Game::new_standard(PlayerKind::Human, PlayerKind::Human);
        assert!(g.apply_timed("e2e4", 1_000).is_err());
    }

    #[test]
    fn flag_on_clock_ends_game() {
        let mut g = Game::new_standard(PlayerKind::Human, PlayerKind::Human)
            .with_time_control(TimeControl::fischer(3_000, 0));
        let err = g.apply_timed("e2e4", 5_000).unwrap_err();
        assert!(err.to_string().contains("flagged"));
        assert!(matches!(
            g.status(),
            GameStatus::TimedOut { winner: Side::Black }
        ));
        assert_eq!(g.remaining_ms(Side::White), Some(0));
    }

    #[test]
    fn parse_move_accepts_san_and_uci() {
        let g = Game::new_standard(PlayerKind::Human, PlayerKind::Human);
        assert_eq!(g.parse_move("e4").unwrap(), "e2e4");
        assert_eq!(g.parse_move("Nf3").unwrap(), "g1f3");
        assert_eq!(g.parse_move("e2e4").unwrap(), "e2e4");
        assert_eq!(g.parse_move("g1f3").unwrap(), "g1f3");
    }

    #[test]
    fn parse_move_normalises_castling() {
        // Quick middlegame where White can castle kingside.
        let fen = "r1bqk2r/pppp1ppp/2n2n2/2b1p3/2B1P3/2N2N2/PPPP1PPP/R1BQK2R w KQkq - 4 4";
        let g = Game::from_fen(fen, PlayerKind::Human, PlayerKind::Human).unwrap();
        assert_eq!(g.parse_move("O-O").unwrap(), "e1g1");
        assert_eq!(g.parse_move("0-0").unwrap(), "e1g1");
    }

    #[test]
    fn parse_move_rejects_garbage() {
        let g = Game::new_standard(PlayerKind::Human, PlayerKind::Human);
        assert!(g.parse_move("xx").is_err());
        assert!(g.parse_move("Nd5").is_err()); // illegal from startpos
    }

    #[test]
    fn legal_moves_san_covers_startpos() {
        let g = Game::new_standard(PlayerKind::Human, PlayerKind::Human);
        let moves = g.legal_moves_san();
        assert_eq!(moves.len(), 20);
        assert!(moves.contains(&"e4".to_string()));
        assert!(moves.contains(&"Nf3".to_string()));
    }

    #[test]
    fn explicit_flag_ends_game() {
        let mut g = Game::new_standard(PlayerKind::Human, PlayerKind::Human)
            .with_time_control(TimeControl::fischer(10_000, 0));
        g.flag(Side::Black);
        assert!(matches!(
            g.status(),
            GameStatus::TimedOut { winner: Side::White }
        ));
        assert_eq!(g.remaining_ms(Side::Black), Some(0));
    }
}
