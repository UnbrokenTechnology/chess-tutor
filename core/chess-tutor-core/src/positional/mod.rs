//! Positional feature extraction: pawn structure, king safety, piece activity.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PositionalReport {
    pub pawn_structure: PawnStructure,
    pub files: FileControl,
    pub minor_pieces: MinorPieces,
    pub king_safety: KingSafety,
    pub mobility: Mobility,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PawnStructure {
    pub passed: SideCounts,
    pub isolated: SideCounts,
    pub doubled: SideCounts,
    pub backward: SideCounts,
    pub hanging: SideCounts,
    pub islands: SideCounts,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileControl {
    pub open_files: Vec<char>,
    pub white_semi_open: Vec<char>,
    pub black_semi_open: Vec<char>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MinorPieces {
    pub white_bishop_pair: bool,
    pub black_bishop_pair: bool,
    pub white_outposts: Vec<String>,
    pub black_outposts: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KingSafety {
    pub white: KingSafetySide,
    pub black: KingSafetySide,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KingSafetySide {
    pub ring_attackers: u32,
    pub ring_defenders: u32,
    pub shelter_score: i32,
    pub open_lines_to_king: Vec<char>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Mobility {
    pub white: u32,
    pub black: u32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct SideCounts {
    pub white: u32,
    pub black: u32,
}
