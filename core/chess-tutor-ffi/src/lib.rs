//! FFI surface for Swift (via uniffi), Kotlin (via uniffi), and Web (via
//! wasm-bindgen, phase 2).
//!
//! Intentionally narrow: a single `analyze_fen` entry point that returns a
//! JSON-encoded [`chess_tutor_core::PositionAnalysis`]. Shells decode the
//! JSON into their native types. Keeping the surface tiny means bindings
//! stay trivial and version drift across platforms is cheap to resolve.

use chess_tutor_core::{analyze, Error};

/// Analyse a FEN and return the resulting [`PositionAnalysis`] as JSON.
///
/// Returns an error string on failure; callers should surface this in the UI
/// as an invalid-position message rather than crashing.
pub fn analyze_fen(fen: String) -> Result<String, String> {
    let report = analyze(&fen).map_err(|e: Error| e.to_string())?;
    serde_json::to_string(&report).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_startpos() {
        let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let json = analyze_fen(fen.into()).expect("valid fen");
        assert!(json.contains("\"schema_version\""));
    }
}
