//! Opening book — selects a curated PGN line at game start and feeds
//! the bot moves from it until the human deviates or the line ends.
//!
//! Designed as a thin runtime layer over the existing
//! [`crate::openings`] data: that module already parses the Lichess
//! CC0 TSV bundle at first use, so the book just adds a [`BookCursor`]
//! that walks the [`OpeningEntry::line`] of one chosen row.
//!
//! Strict invariant: only the **play** engine consults the book. The
//! analytical engine (retrospective, hint, `analyze`) must still see
//! the unbiased top-of-search result so its verdicts judge the
//! student's move against true best play. See
//! [`crate::opponent`] for the matching invariant on
//! [`crate::opponent::OpponentProfile`].
//!
//! # Curated default
//!
//! [`curated_default_ids`] returns the 8 (approx.) hand-picked
//! openings every new game starts with unless the user opts out via
//! [`crate::opponent::BookSelection::None`] or removes entries via
//! the CLI `openings deny` command. The list is small on purpose —
//! Phase B is "varied games out of the box," not "exhaustive opening
//! coverage." Future commits expand both the curated default and
//! pull entries from the full ~3,900-row TSV via
//! [`crate::openings::find_ids_matching`].

use std::sync::OnceLock;

use crate::openings::{self, OpeningEntry, OpeningId};
use crate::opponent::{BookSelection, OpponentProfile};
use crate::position::Position;
use crate::types::Move;

/// Runtime state for an opening line the bot is currently playing
/// through. Owned by the play loop (CLI / desktop), not the engine.
#[derive(Clone, Debug)]
pub struct BookCursor {
    id: OpeningId,
    /// Owned copy of the move list so the cursor doesn't borrow from
    /// the openings DB (lets the play loop hold it across mutable
    /// engine calls).
    line: Vec<Move>,
    /// Index of the next ply to play / verify.
    next_ply: usize,
}

impl BookCursor {
    /// Pick a line at random from the profile's allowed set, seeded by
    /// `profile.seed`. Returns `None` when there's no usable book:
    ///
    /// - the profile has [`BookSelection::None`] or an empty Allowed
    ///   set,
    /// - the game starts from a non-startpos FEN (custom positions
    ///   skip the book — we have no idea where in any line they sit),
    /// - the chosen entry's line is empty (defensive — every TSV row
    ///   should carry at least one move).
    pub fn pick(profile: &OpponentProfile, start_pos: &Position) -> Option<Self> {
        // Mid-game start positions can't safely be matched to any book
        // line — even if the position happens to appear in some opening
        // we'd be guessing which line the user meant.
        if start_pos.to_fen() != Position::startpos().to_fen() {
            return None;
        }
        let allowed = match &profile.book {
            BookSelection::None => return None,
            BookSelection::Allowed(ids) if ids.is_empty() => return None,
            BookSelection::Allowed(ids) => ids,
        };
        let idx = (pick_u64(profile.seed) as usize) % allowed.len();
        let id = allowed[idx];
        let entry = openings::entry(id)?;
        if entry.line.is_empty() {
            return None;
        }
        Some(Self {
            id,
            line: entry.line.clone(),
            next_ply: 0,
        })
    }

    /// The next move the bot would play if it's the bot's turn. `None`
    /// when the line is exhausted — caller should drop the cursor and
    /// fall through to engine search.
    pub fn peek(&self) -> Option<Move> {
        self.line.get(self.next_ply).copied()
    }

    /// Inform the cursor that `played` was just made on the board
    /// (either side). Returns `true` when the move matched the next
    /// expected book ply and the cursor advanced; `false` when the
    /// move diverged from the line — the caller should drop the
    /// cursor and stop consulting the book for the rest of the game.
    #[must_use]
    pub fn observe(&mut self, played: Move) -> bool {
        match self.line.get(self.next_ply) {
            Some(&expected) if expected == played => {
                self.next_ply += 1;
                true
            }
            _ => false,
        }
    }

    /// The opening this cursor is walking through, for prompt /
    /// status display.
    pub fn opening(&self) -> &'static OpeningEntry {
        openings::entry(self.id)
            .expect("cursor holds an OpeningId resolved at pick time")
    }
}

// =========================================================================
// Curated default list
// =========================================================================

/// Hand-picked default subset of openings. Each entry must match an
/// existing TSV row exactly; mismatches surface at first call to
/// [`curated_default_ids`] (and are caught at test time by
/// `every_curated_entry_resolves` below). The names below are taken
/// verbatim from the Lichess CC0 bundle in `data/openings/*.tsv`.
///
/// Roughly balanced across White's two main first moves (1.e4 and
/// 1.d4) plus a Black defence representing each side; depth chosen to
/// take the bot 4–8 plies into a recognisable theoretical position
/// without committing to deep main-line theory the student probably
/// hasn't studied.
const CURATED: &[(&str, &str)] = &[
    ("C50", "Italian Game"),
    ("C60", "Ruy Lopez"),
    ("D06", "Queen's Gambit"),
    ("D10", "Slav Defense"),
    ("B10", "Caro-Kann Defense"),
    ("C00", "French Defense"),
    ("B20", "Sicilian Defense"),
    ("E61", "King's Indian Defense"),
];

static CURATED_IDS: OnceLock<Vec<OpeningId>> = OnceLock::new();

/// Resolve the curated default list to ids on first call. Skips any
/// (eco, name) pair that no longer exists in the bundled TSV (would
/// only happen if the TSV is updated and a curated entry's name
/// shifts). Test `every_curated_entry_resolves` keeps this honest.
pub fn curated_default_ids() -> Vec<OpeningId> {
    CURATED_IDS
        .get_or_init(|| {
            CURATED
                .iter()
                .filter_map(|(eco, name)| openings::find_id_exact(eco, name))
                .collect()
        })
        .clone()
}

// =========================================================================
// Internal helpers
// =========================================================================

/// SplitMix64 step: mixes a 64-bit seed into a draw. Pure, depends only
/// on its input, and is good enough for picking one entry out of a few
/// dozen — not a cryptographic RNG. Re-seeding per call (rather than
/// holding a stream) keeps the helper stateless, which fits Phase B's
/// "one pick per game" usage; future per-move noise will want a real
/// stream.
fn pick_u64(seed: u64) -> u64 {
    let mut x = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::san;

    fn play(pos: &mut Position, san_str: &str) -> Move {
        let mv = san::parse(pos, san_str).expect("legal SAN");
        let _ = pos.do_move(mv);
        mv
    }

    #[test]
    fn curated_default_resolves_to_nonempty_list() {
        let ids = curated_default_ids();
        assert!(!ids.is_empty(), "curated default list resolved to empty");
    }

    #[test]
    fn every_curated_entry_resolves() {
        // Catches TSV drift: if a hand-picked (eco, name) stops
        // matching a row, the list silently shrinks — this test
        // fails loudly instead.
        for (eco, name) in CURATED {
            assert!(
                openings::find_id_exact(eco, name).is_some(),
                "curated entry ({eco}, {name:?}) no longer resolves in the TSV",
            );
        }
    }

    #[test]
    fn pick_returns_none_when_book_is_off() {
        let profile = OpponentProfile {
            book: BookSelection::None,
            ..OpponentProfile::with_seed(1)
        };
        assert!(BookCursor::pick(&profile, &Position::startpos()).is_none());
    }

    #[test]
    fn pick_returns_none_for_non_startpos() {
        let profile = OpponentProfile::with_seed(1);
        // Some random midgame FEN — book is only valid from startpos.
        let pos = Position::from_fen("r3k2r/pppq1ppp/2n2n2/3pp3/3PP3/2N2N2/PPPQ1PPP/R3K2R w KQkq - 0 1")
            .expect("test FEN");
        assert!(BookCursor::pick(&profile, &pos).is_none());
    }

    #[test]
    fn pick_returns_some_with_curated_default_on_startpos() {
        let profile = OpponentProfile {
            book: BookSelection::Allowed(curated_default_ids()),
            ..OpponentProfile::with_seed(42)
        };
        let cursor = BookCursor::pick(&profile, &Position::startpos())
            .expect("curated default + startpos should yield a cursor");
        assert!(cursor.peek().is_some(), "cursor's line is empty");
    }

    #[test]
    fn observe_advances_on_matching_move_drops_on_divergence() {
        // Force a specific entry so we know what moves to expect.
        let id = openings::find_id_exact("C50", "Italian Game")
            .expect("Italian Game must be in TSV");
        let entry = openings::entry(id).unwrap();
        let mut cursor = BookCursor {
            id,
            line: entry.line.clone(),
            next_ply: 0,
        };
        let first = cursor.peek().expect("Italian Game has at least one move");
        assert!(cursor.observe(first), "matching move should advance");
        // Now fabricate a non-matching move: any legal move from
        // the post-first-move position that isn't the line's ply 1.
        let mut pos = Position::startpos();
        let _ = pos.do_move(first);
        let expected_next = cursor.peek().expect("line has > 1 move");
        let divergent = san::parse(&mut pos, "a6")
            .or_else(|_| san::parse(&mut pos, "h6"))
            .expect("at least one quiet pawn move is legal");
        assert_ne!(
            divergent, expected_next,
            "test setup picked a move that happens to match the line",
        );
        assert!(!cursor.observe(divergent), "diverging move should drop cursor");
    }

    #[test]
    fn pick_is_deterministic_for_same_seed() {
        let profile_a = OpponentProfile {
            book: BookSelection::Allowed(curated_default_ids()),
            ..OpponentProfile::with_seed(0xCAFE)
        };
        let profile_b = profile_a.clone();
        let a = BookCursor::pick(&profile_a, &Position::startpos()).expect("a");
        let b = BookCursor::pick(&profile_b, &Position::startpos()).expect("b");
        assert_eq!(a.opening().id, b.opening().id);
    }

    #[test]
    fn full_line_walk_advances_until_exhausted_then_drops() {
        // End-to-end: pick the Italian Game line, replay every move
        // through the cursor, then play one more move from the post-
        // line position to confirm the cursor drops on exhaustion.
        let id = openings::find_id_exact("C50", "Italian Game")
            .expect("Italian Game in TSV");
        let entry = openings::entry(id).unwrap();
        let mut cursor = BookCursor {
            id,
            line: entry.line.clone(),
            next_ply: 0,
        };
        let mut pos = Position::startpos();
        for &mv in &entry.line {
            assert!(cursor.observe(mv), "every line move should advance the cursor");
            let _ = pos.do_move(mv);
        }
        assert!(cursor.peek().is_none(), "exhausted line has no next move");
        // One more move from the live position — observe should now
        // see no expected ply and return false, signalling drop.
        let extra = play(&mut pos, "Nf6")
            ; // any legal continuation; if Nf6 isn't legal here the test
            // setup is wrong for the chosen line
        assert!(
            !cursor.observe(extra),
            "observing past line end must drop the cursor",
        );
    }
}
