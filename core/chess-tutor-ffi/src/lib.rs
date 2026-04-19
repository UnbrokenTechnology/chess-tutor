//! FFI surface for Swift (via uniffi), Kotlin (via uniffi), and Web (via
//! wasm-bindgen, phase 2).
//!
//! Intentionally narrow. Two flavours:
//!
//! 1. **Analyse-a-FEN**: one-shot analysis for static puzzles and "analyse
//!    this position" features. Returns a JSON-encoded
//!    [`chess_tutor_core::PositionAnalysis`].
//!
//! 2. **Game loop**: create/serialise/restore a [`chess_tutor_core::game::Game`],
//!    apply moves, get per-move reports. Shells decode the JSON into native
//!    types — keeps the bindings trivial and version drift across platforms
//!    cheap to resolve.

use chess_tutor_core::{
    analyze,
    game::{Game, PlayerKind, TimeControl},
    Error,
};

/// Analyse a FEN and return the resulting [`PositionAnalysis`] as JSON.
pub fn analyze_fen(fen: String) -> Result<String, String> {
    let report = analyze(&fen).map_err(|e: Error| e.to_string())?;
    serde_json::to_string(&report).map_err(|e| e.to_string())
}

/// Start a fresh standard game, H vs H. The shell persists the opaque
/// handle (for now just the JSON game state) and passes it back on each
/// subsequent call. Keeping things stateless-across-FFI-calls makes the
/// Swift/Kotlin side trivial — no retained native handle, no lifecycle to
/// manage.
pub fn game_new_standard() -> Result<String, String> {
    let g = Game::new_standard(PlayerKind::Human, PlayerKind::Human);
    snapshot(&g)
}

/// Start a fresh timed H vs H game with a Fischer clock. `initial_ms` goes
/// on each side, `increment_ms` is added after each completed move.
pub fn game_new_timed(initial_ms: u64, increment_ms: u64) -> Result<String, String> {
    let g = Game::new_standard(PlayerKind::Human, PlayerKind::Human)
        .with_time_control(TimeControl::fischer(initial_ms, increment_ms));
    snapshot(&g)
}

/// Apply a UCI move to a serialised game, returning `(new_game_json,
/// move_report_json)`.
pub fn game_apply(game_json: String, uci: String) -> Result<(String, String), String> {
    apply_inner(game_json, uci, None)
}

/// Apply a UCI move in a timed game. `elapsed_ms` is the wall-clock time
/// the mover spent. Returns `(new_game_json, move_report_json)`. If the
/// mover flags on the clock, returns an error and the returned snapshot
/// will show `TimedOut` status.
pub fn game_apply_timed(
    game_json: String,
    uci: String,
    elapsed_ms: u64,
) -> Result<(String, String), String> {
    apply_inner(game_json, uci, Some(elapsed_ms))
}

fn apply_inner(
    game_json: String,
    uci: String,
    elapsed_ms: Option<u64>,
) -> Result<(String, String), String> {
    let g: GameSnapshot =
        serde_json::from_str(&game_json).map_err(|e| format!("bad game snapshot: {e}"))?;
    let mut game = g.hydrate().map_err(|e| e.to_string())?;
    let report = match elapsed_ms {
        Some(ms) => game.apply_timed(&uci, ms).map_err(|e| e.to_string())?,
        None => game.apply(&uci).map_err(|e| e.to_string())?,
    };
    let out = GameSnapshot::from(&game);
    let game_json = serde_json::to_string(&out).map_err(|e| e.to_string())?;
    let report_json = serde_json::to_string(&report).map_err(|e| e.to_string())?;
    Ok((game_json, report_json))
}

/// List legal moves (UCI) from a serialised game.
pub fn game_legal_moves(game_json: String) -> Result<String, String> {
    let g: GameSnapshot =
        serde_json::from_str(&game_json).map_err(|e| format!("bad game snapshot: {e}"))?;
    let game = g.hydrate().map_err(|e| e.to_string())?;
    serde_json::to_string(&game.legal_moves()).map_err(|e| e.to_string())
}

fn snapshot(g: &Game) -> Result<String, String> {
    serde_json::to_string(&GameSnapshot::from(g)).map_err(|e| e.to_string())
}

/// Transport shape for the FFI boundary. Carries enough state to rehydrate a
/// [`Game`] without exposing `shakmaty` types through the FFI.
#[derive(serde::Serialize, serde::Deserialize)]
struct GameSnapshot {
    fen: String,
    history_uci: Vec<String>,
    white_player: PlayerKind,
    black_player: PlayerKind,
    time_control: Option<TimeControl>,
}

impl GameSnapshot {
    fn from(game: &Game) -> Self {
        // Start position is recovered from the first history entry's
        // fen_before, or the current FEN if no history. shakmaty doesn't
        // expose the start FEN separately, so we reconstruct from history.
        let fen = game
            .history()
            .first()
            .map(|e| e.fen_before.clone())
            .unwrap_or_else(|| game.fen());
        let history_uci = game.history().iter().map(|e| e.uci.clone()).collect();
        // Player kinds aren't currently exposed via accessors on Game;
        // add once play-vs-bot flows start exercising this.
        Self {
            fen,
            history_uci,
            white_player: PlayerKind::Human,
            black_player: PlayerKind::Human,
            time_control: game.time_control().copied(),
        }
    }

    fn hydrate(&self) -> chess_tutor_core::Result<Game> {
        let mut g = Game::from_fen(&self.fen, self.white_player, self.black_player)?;
        for uci in &self.history_uci {
            g.apply(uci)?;
        }
        if let Some(tc) = self.time_control {
            g = g.with_time_control(tc);
        }
        Ok(g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_roundtrips_startpos() {
        let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let json = analyze_fen(fen.into()).expect("valid fen");
        assert!(json.contains("\"schema_version\""));
    }

    #[test]
    fn game_apply_roundtrips() {
        let game = game_new_standard().unwrap();
        let (next, report) = game_apply(game, "e2e4".into()).unwrap();
        assert!(report.contains("\"san\":\"e4\""));
        let moves_json = game_legal_moves(next).unwrap();
        let moves: Vec<String> = serde_json::from_str(&moves_json).unwrap();
        assert_eq!(moves.len(), 20); // black's replies from after-e4
    }
}
