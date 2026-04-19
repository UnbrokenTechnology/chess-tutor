//! Polyglot opening book reader + ECO identification.
//!
//! Phase 1 stub. Book file lives in `assets/book.bin` (built by
//! `scripts/build-book.sh`) and is loaded on demand by the platform shells.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpeningHit {
    pub eco: String,
    pub name: String,
    pub moves: Vec<String>,
    pub common_continuations: Vec<String>,
}

/// Read-only handle over a loaded Polyglot book. Phase 1: empty shell.
#[derive(Debug, Default)]
pub struct Book;

impl Book {
    pub fn new() -> Self {
        Self
    }

    /// Look up the current position in the book.
    pub fn lookup(&self, _fen: &str) -> Option<OpeningHit> {
        None
    }
}
