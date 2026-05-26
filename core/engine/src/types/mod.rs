//! Core value types used throughout the engine: colors, piece types, pieces,
//! squares, files, ranks, directions, values/scores, bitboards, and moves.
//!
//! Design notes:
//!
//! - `Square` is a newtype over `u8` with the discriminant 64 reserved as a
//!   sentinel (`Square::NONE`). A valid square is always in `0..=63`. This
//!   matches the reference's packed move encoding, where a "none" square
//!   still needs to fit in the 6 bits used for from/to.
//!
//! - `Piece` is a strict enum with discriminants `1..=6` (white) and `9..=14`
//!   (black). The gap at 7-8 isn't arbitrary: `piece >> 3` gives the color
//!   and `piece & 7` gives the piece type, both zero-allocated. Empty squares
//!   are represented by `Option<Piece>` — the niche at 0 lets `Option<Piece>`
//!   still fit in one byte.
//!
//! - `Score` packs two `i16` values (middle-game + end-game) into a single
//!   `i32`. Carrying the pair around as one value is how the reference
//!   amortises the cost of tapered evaluation across search nodes.

mod color;
mod piece;
mod square;
mod direction;
mod value;
mod misc;
mod moves;

pub use color::*;
pub use piece::*;
pub use square::*;
pub use direction::*;
pub use value::*;
pub use misc::*;
pub use moves::*;

#[cfg(test)]
mod tests;
