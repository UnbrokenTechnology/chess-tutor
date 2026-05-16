//! `chess-tutor-engine` — a classical chess engine in Rust whose evaluation is
//! fully decomposable so a teaching UI can surface *why* a move is good or bad.
//!
//! Architecture and term decomposition are derived from Stockfish 11 (the last
//! version before NNUE). Numerical weight tables are factual data carried over
//! from the reference; all code is independently authored in Rust idiom — no
//! copied identifiers, comments, or structural ordering.

pub mod analysis;
pub mod attacks;
pub mod bitbases;
pub mod bitboard;
pub mod book;
pub mod endgame;
pub mod engine;
pub mod eval;
pub mod magics;
pub mod material;
pub mod movegen;
pub mod movepick;
pub mod openings;
pub mod opponent;
pub mod pawns;
pub mod position;
pub mod prefetch;
pub mod psqt;
pub mod san;
pub mod search;
pub mod traps;
pub mod tt;
pub mod types;
pub mod zobrist;
