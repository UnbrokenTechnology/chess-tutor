//! Opening-name lookup.
//!
//! Given a position, return the ECO code and opening name
//! (e.g. `"B90 — Sicilian Defense: Najdorf"`). Purely descriptive —
//! does **not** feed move classification. Moves are always judged on
//! merit (via the search + [`crate::eval::EvalTrace`]), not on whether
//! they match a database entry. Opening names are vocabulary the
//! student can use to talk about the positions they're playing.
//!
//! # How it works
//!
//! The Lichess CC0 openings database ships as five TSV files (one per
//! ECO letter A–E), each row carrying `eco \t name \t pgn`. We bundle
//! the raw TSVs, replay each PGN once at first use through our own
//! [`Position`] / [`crate::san`] stack, and build a `HashMap` keyed by
//! the resulting position's **EPD** (FEN with the halfmove and
//! fullmove counters stripped). Lookup at runtime is a single
//! `HashMap::get`.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpeningIdentification {
    /// Encyclopaedia of Chess Openings code, e.g. `"B90"`.
    pub eco: String,
    /// Full name including any variation, e.g.
    /// `"Sicilian Defense: Najdorf, English Attack"`.
    pub name: String,
}

const TSV_A: &str = include_str!("../data/openings/a.tsv");
const TSV_B: &str = include_str!("../data/openings/b.tsv");
const TSV_C: &str = include_str!("../data/openings/c.tsv");
const TSV_D: &str = include_str!("../data/openings/d.tsv");
const TSV_E: &str = include_str!("../data/openings/e.tsv");

static DB: OnceLock<HashMap<String, OpeningIdentification>> = OnceLock::new();

fn db() -> &'static HashMap<String, OpeningIdentification> {
    DB.get_or_init(build_db)
}

fn build_db() -> HashMap<String, OpeningIdentification> {
    let mut db = HashMap::with_capacity(4096);
    for tsv in [TSV_A, TSV_B, TSV_C, TSV_D, TSV_E] {
        for line in tsv.lines().skip(1) {
            // Silently skip malformed rows and unparseable PGNs — one
            // bad line shouldn't kill the whole database.
            let Some((eco, name, pgn)) = parse_row(line) else {
                continue;
            };
            let Some(epd) = play_pgn_to_epd(pgn) else {
                continue;
            };
            // First writer wins, matching the prior-repo behaviour:
            // if two TSV rows transpose to the same EPD, we keep the
            // one that appears first in the file order (A → E).
            db.entry(epd).or_insert_with(|| OpeningIdentification {
                eco: eco.to_string(),
                name: name.to_string(),
            });
        }
    }
    db
}

fn parse_row(line: &str) -> Option<(&str, &str, &str)> {
    let mut parts = line.splitn(3, '\t');
    let eco = parts.next()?;
    let name = parts.next()?;
    let pgn = parts.next()?;
    Some((eco, name, pgn))
}

/// Play a PGN sequence starting from the standard position. Returns
/// the resulting position's EPD, or `None` if any token fails to parse
/// or no legal move matches.
fn play_pgn_to_epd(pgn: &str) -> Option<String> {
    let mut pos = Position::startpos();
    for token in pgn.split_whitespace() {
        if is_move_number(token) {
            continue;
        }
        let mv = san::parse(&mut pos, token).ok()?;
        let _ = pos.do_move(mv);
    }
    Some(fen_to_epd(&pos.to_fen()))
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
    db().get(epd).cloned()
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
        let size = db().len();
        assert!(
            size > 2_500,
            "opening DB shrunk unexpectedly: {size} entries",
        );
    }
}
