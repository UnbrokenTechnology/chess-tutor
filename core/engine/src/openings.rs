//! Opening-name lookup and opening-line data.
//!
//! Two roles, sharing one TSV parse at first use:
//!
//! 1. **Recognition** (reverse index) — given a position, return the
//!    ECO code + opening name (e.g. `"B90 — Sicilian Defense: Najdorf"`)
//!    so the UI can label what the player is playing. Purely
//!    descriptive — does not feed move classification; moves are
//!    always judged on merit via the search + [`crate::eval::EvalTrace`].
//! 2. **Selection** (forward index) — given an [`OpeningId`], return
//!    the move sequence as `Vec<Move>` so the opponent's opening book
//!    ([`crate::book`]) can play through a chosen line. See
//!    [`entries`], [`entry`], [`find_id_exact`], and
//!    [`find_ids_matching`].
//!
//! # How it works
//!
//! The Lichess CC0 openings database ships as five TSV files (one per
//! ECO letter A–E), each row carrying `eco \t name \t pgn`. We bundle
//! the raw TSVs, replay each PGN once at first use through our own
//! [`Position`] / [`crate::san`] stack, and build:
//! - a `Vec<OpeningEntry>` of every replayable row (the forward index;
//!   row position in the vector is the row's stable [`OpeningId`]);
//! - a `HashMap<EPD, OpeningId>` keyed by the final position's EPD
//!   (FEN with halfmove and fullmove counters stripped — the reverse
//!   index). Recognition lookup is a single `HashMap::get` + vector
//!   indexing.
//!
//! EPD matching handles most transpositions naturally because the TSV
//! stores multiple move-order variants for the same named opening.
//! Rare cases where a transposition yields a different en-passant
//! square than any stored variant will miss — a limitation shared by
//! every EPD-based opening recogniser.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::position::Position;
use crate::san;
use crate::types::Move;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpeningIdentification {
    /// Encyclopaedia of Chess Openings code, e.g. `"B90"`.
    pub eco: String,
    /// Full name including any variation, e.g.
    /// `"Sicilian Defense: Najdorf, English Attack"`.
    pub name: String,
}

/// Stable handle to one row of the bundled opening TSVs. Assigned at
/// database build time (= row index in concatenated A→E order); stable
/// for the life of a process. Not stable across rebuilds of the bundled
/// TSV data — never persist these to disk; resolve names through
/// [`find_id_exact`] when reading user-saved selections.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct OpeningId(u32);

impl OpeningId {
    /// Underlying row index — exposed only so the opponent module can
    /// hash it into seeded RNG draws. Not for external persistence.
    pub fn raw(self) -> u32 {
        self.0
    }
}

/// One parsed opening line — both metadata and replayable moves.
#[derive(Debug, Clone)]
pub struct OpeningEntry {
    pub id: OpeningId,
    pub eco: String,
    pub name: String,
    /// Moves from `Position::startpos()` to the final position. Every
    /// move is legal at the time it is played; the sequence is what
    /// the book cursor walks through ply by ply.
    pub line: Vec<Move>,
}

impl From<&OpeningEntry> for OpeningIdentification {
    fn from(e: &OpeningEntry) -> Self {
        Self {
            eco: e.eco.clone(),
            name: e.name.clone(),
        }
    }
}

struct Database {
    entries: Vec<OpeningEntry>,
    by_epd: HashMap<String, OpeningId>,
}

const TSV_A: &str = include_str!("../data/openings/a.tsv");
const TSV_B: &str = include_str!("../data/openings/b.tsv");
const TSV_C: &str = include_str!("../data/openings/c.tsv");
const TSV_D: &str = include_str!("../data/openings/d.tsv");
const TSV_E: &str = include_str!("../data/openings/e.tsv");

static DB: OnceLock<Database> = OnceLock::new();

fn db() -> &'static Database {
    DB.get_or_init(build_db)
}

fn build_db() -> Database {
    let mut entries: Vec<OpeningEntry> = Vec::with_capacity(4096);
    let mut by_epd: HashMap<String, OpeningId> = HashMap::with_capacity(4096);
    for tsv in [TSV_A, TSV_B, TSV_C, TSV_D, TSV_E] {
        for line in tsv.lines().skip(1) {
            // Silently skip malformed rows and unparseable PGNs — one
            // bad line shouldn't kill the whole database.
            let Some((eco, name, pgn)) = parse_row(line) else {
                continue;
            };
            let Some((final_pos, moves)) = replay_pgn(pgn) else {
                continue;
            };
            let id = OpeningId(entries.len() as u32);
            entries.push(OpeningEntry {
                id,
                eco: eco.to_string(),
                name: name.to_string(),
                line: moves,
            });
            // First writer wins for the reverse index: if two TSV rows
            // transpose to the same EPD, we keep the one that appears
            // first in file order (A → E). Both rows still exist in
            // the forward `entries` vector — duplicates are only
            // collapsed in the EPD-keyed lookup.
            by_epd
                .entry(fen_to_epd(&final_pos.to_fen()))
                .or_insert(id);
        }
    }
    Database { entries, by_epd }
}

fn parse_row(line: &str) -> Option<(&str, &str, &str)> {
    let mut parts = line.splitn(3, '\t');
    let eco = parts.next()?;
    let name = parts.next()?;
    let pgn = parts.next()?;
    Some((eco, name, pgn))
}

/// Replay a PGN move sequence from the standard position. Returns
/// the final position and the moves played, or `None` if any token
/// fails to parse.
fn replay_pgn(pgn: &str) -> Option<(Position, Vec<Move>)> {
    let mut pos = Position::startpos();
    let mut moves = Vec::new();
    for token in pgn.split_whitespace() {
        if is_move_number(token) {
            continue;
        }
        let mv = san::parse(&mut pos, token).ok()?;
        let _ = pos.do_move(mv);
        moves.push(mv);
    }
    Some((pos, moves))
}

/// `"1."`, `"2."`, `"10..."`, `"3..."` and similar — move-number
/// indicators that should be skipped when iterating whitespace-split
/// PGN tokens.
fn is_move_number(token: &str) -> bool {
    let mut saw_digit = false;
    for c in token.chars() {
        match c {
            '0'..='9' => saw_digit = true,
            '.' => {
                if !saw_digit {
                    return false;
                }
            }
            _ => return false,
        }
    }
    saw_digit
}

/// Strip the halfmove and fullmove counters from a FEN to produce its
/// EPD form: board + side + castling rights + en-passant target.
/// Returns the input unchanged if it has fewer than 6 whitespace-
/// separated fields.
fn fen_to_epd(fen: &str) -> String {
    let fields: Vec<&str> = fen.split_whitespace().collect();
    if fields.len() >= 6 {
        fields[..4].join(" ")
    } else {
        fen.to_string()
    }
}

/// Look up an opening by the EPD of the current position.
pub fn identify_from_epd(epd: &str) -> Option<OpeningIdentification> {
    let id = db().by_epd.get(epd).copied()?;
    db().entries.get(id.0 as usize).map(OpeningIdentification::from)
}

/// Look up an opening by full FEN (halfmove/fullmove counters are
/// stripped before the lookup).
pub fn identify_from_fen(fen: &str) -> Option<OpeningIdentification> {
    let epd = fen_to_epd(fen);
    identify_from_epd(&epd)
}

/// Look up an opening for a live [`Position`]. Uses the position's
/// own FEN rendering so the EP convention matches the one used when
/// the database was built.
pub fn identify(position: &Position) -> Option<OpeningIdentification> {
    identify_from_fen(&position.to_fen())
}

// =========================================================================
// Forward index — for opening-book selection
// =========================================================================

/// All bundled opening lines, in the order they appear in the source
/// TSVs (A → E). Indexed by [`OpeningId`].
pub fn entries() -> &'static [OpeningEntry] {
    &db().entries
}

/// Look up one entry by its stable id.
pub fn entry(id: OpeningId) -> Option<&'static OpeningEntry> {
    db().entries.get(id.0 as usize)
}

/// Exact match on ECO code and full opening name. Returns the first
/// matching id, or `None` if no row matches. Used by the curated
/// default list to bind hand-picked entry references to ids — a
/// failed resolution means the bundled TSV no longer carries that
/// exact name (caller decides whether to log + skip or panic).
pub fn find_id_exact(eco: &str, name: &str) -> Option<OpeningId> {
    db()
        .entries
        .iter()
        .find(|e| e.eco == eco && e.name == name)
        .map(|e| e.id)
}

/// Case-insensitive substring match on the opening name. Returns
/// every matching id in the order they appear in the TSV. Used by
/// the CLI `openings allow / deny <pattern>` commands.
pub fn find_ids_matching(pattern: &str) -> Vec<OpeningId> {
    let needle = pattern.to_ascii_lowercase();
    db()
        .entries
        .iter()
        .filter(|e| e.name.to_ascii_lowercase().contains(&needle))
        .map(|e| e.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Play a sequence of SAN moves from the start position and return
    /// the resulting `Position`. Panics on any parse error so a broken
    /// test FEN surfaces loudly.
    fn play_sans(moves: &[&str]) -> Position {
        let mut pos = Position::startpos();
        for mv in moves {
            let m =
                san::parse(&mut pos, mv).unwrap_or_else(|e| panic!("illegal test move {mv}: {e}"));
            let _ = pos.do_move(m);
        }
        pos
    }

    #[test]
    fn startpos_is_not_an_opening() {
        // The TSVs start one ply in — "A00: Amar Opening" is 1.Nh3,
        // not the start position itself.
        assert!(identify(&Position::startpos()).is_none());
    }

    #[test]
    fn e4_identifies_as_kings_pawn_game() {
        let pos = play_sans(&["e4"]);
        let hit = identify(&pos).expect("1.e4 should be identifiable");
        let lower = hit.name.to_ascii_lowercase();
        assert!(
            lower.contains("pawn") || lower.contains("king"),
            "unexpected name for 1.e4: {}",
            hit.name,
        );
    }

    #[test]
    fn sicilian_najdorf_is_identifiable() {
        let pos = play_sans(&[
            "e4", "c5", "Nf3", "d6", "d4", "cxd4", "Nxd4", "Nf6", "Nc3", "a6",
        ]);
        let hit = identify(&pos).expect("Najdorf should be identifiable");
        let lower = hit.name.to_ascii_lowercase();
        assert!(
            lower.contains("najdorf") || lower.contains("sicilian"),
            "unexpected name for Najdorf: {}",
            hit.name,
        );
    }

    #[test]
    fn random_mid_game_position_returns_none() {
        // A made-up mid-game position highly unlikely to match any
        // stored opening EPD.
        assert!(identify_from_fen("8/8/3k4/8/8/3K4/8/8 w - - 0 1").is_none());
    }

    #[test]
    fn fen_to_epd_strips_clocks() {
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
        assert_eq!(
            fen_to_epd(fen),
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3",
        );
    }

    #[test]
    fn is_move_number_recognises_common_forms() {
        assert!(is_move_number("1."));
        assert!(is_move_number("10."));
        assert!(is_move_number("1..."));
        assert!(is_move_number("23..."));
        assert!(!is_move_number("Nf3"));
        assert!(!is_move_number("O-O"));
        assert!(!is_move_number("..."));
        assert!(!is_move_number(""));
    }

    #[test]
    fn database_loads_a_plausible_number_of_openings() {
        // Sanity check that PGN replay is actually hitting most rows.
        // The source TSVs have ~3,690 entries combined (A–E minus the
        // five headers). Collisions collapse duplicates, and a handful
        // of rows may fail to parse — but the live DB should still be
        // in the thousands.
        let size = db().by_epd.len();
        assert!(
            size > 2_500,
            "opening DB shrunk unexpectedly: {size} entries",
        );
    }
}
