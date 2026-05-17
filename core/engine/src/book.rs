//! Opening book — for each bot move, picks from the curated openings
//! whose move-prefix still matches the moves played so far.
//!
//! Designed as a thin runtime layer over the existing
//! [`crate::openings`] data: that module already parses the Lichess
//! CC0 TSV bundle at first use; the book just walks the allowed-id
//! list and finds those whose `OpeningEntry::line` starts with the
//! current game's move list.
//!
//! # Per-ply matching (not pre-commit)
//!
//! The bot doesn't pre-commit to a single opening at game start.
//! Every time it needs a move, [`BookCursor::peek`] walks the allowed
//! openings, keeps those whose stored move sequence starts with the
//! game's moves so far, and picks one deterministically from the
//! survivors (using `seed` + ply count).
//!
//! Why: an engine playing Black can't know what opening it's in
//! until White plays — `1.e4` rules out Slav / Queen's Gambit / KID
//! immediately; `1.d4` rules out Sicilian / French / Caro-Kann. And
//! even an engine playing White might want to follow into different
//! continuations depending on Black's response (Italian vs Ruy Lopez
//! after `1.e4 e5 2.Nf3 Nc6`). The pre-commit design we had before
//! made the bot drop out of book the instant the human entered a
//! different curated line — which was the visible failure mode that
//! prompted this rewrite.
//!
//! # Determinism
//!
//! For a fixed `seed`, the same history of moves always produces the
//! same pick. The seed is combined with the current ply count via
//! [`pick_u64`] so different ply counts in the same game can pick
//! different candidates — but the candidate *set* is monotonically
//! constrained by the moves already played (a move only narrows the
//! set, never widens it), so coherent lines emerge naturally: the
//! bot can't "switch character" mid-game because its own prior
//! moves have already disqualified the alternatives.
//!
//! # Strict invariant
//!
//! Only the **play** engine consults the book. The analytical engine
//! (retrospective, hint, `analyze`) must still see the unbiased
//! top-of-search result so its verdicts judge the student's move
//! against true best play. See [`crate::opponent`] for the matching
//! invariant on [`crate::opponent::OpponentProfile`].
//!
//! # Curated default
//!
//! [`curated_default_ids`] returns the (currently 8) hand-picked
//! openings every new game starts with unless the user opts out via
//! [`crate::opponent::BookSelection::None`] or removes entries via
//! the CLI `openings deny` command.

use std::sync::OnceLock;

use crate::openings::{self, OpeningId};
use crate::opponent::{BookSelection, OpponentProfile};
use crate::position::Position;
use crate::types::Move;

/// Per-game book state. Holds the allowed-opening list and the
/// per-game seed; the "where in the line am I" question is answered
/// fresh per call by [`peek`](Self::peek) against the move history,
/// so there's no cursor position to advance and no state to restore
/// on takeback.
#[derive(Clone, Debug)]
pub struct BookCursor {
    /// Allowed opening ids, sorted ascending so candidate ordering at
    /// pick time is stable regardless of how the user constructed the
    /// list.
    allowed: Vec<OpeningId>,
    /// Per-game seed — combined with ply count to pick deterministically
    /// from the matching candidates at each bot move.
    seed: u64,
}

/// One book pick: the move to play, plus which opening it came from
/// (for display / annotation).
#[derive(Clone, Copy, Debug)]
pub struct BookPick {
    pub mv: Move,
    pub opening_id: OpeningId,
}

impl BookCursor {
    /// Initialise the book state for a game. Returns `None` (= "no
    /// book this game") when:
    ///
    /// - The profile has [`BookSelection::None`] or an empty Allowed
    ///   list.
    /// - The game starts from a non-startpos FEN. Custom positions
    ///   skip the book — we have no way to know which line the user
    ///   meant.
    pub fn new(profile: &OpponentProfile, start_pos: &Position) -> Option<Self> {
        if start_pos.to_fen() != Position::startpos().to_fen() {
            return None;
        }
        let mut allowed = match &profile.book {
            BookSelection::None => return None,
            BookSelection::Allowed(ids) if ids.is_empty() => return None,
            BookSelection::Allowed(ids) => ids.clone(),
        };
        allowed.sort_by_key(|id| id.raw());
        allowed.dedup();
        Some(Self {
            allowed,
            seed: profile.seed,
        })
    }

    /// Find the next book move given the game's move history so far.
    /// Walks the allowed openings, keeps those whose stored line
    /// starts with `history` and has at least one more move beyond it,
    /// and picks one deterministically from `(seed, history.len())`.
    /// Returns `None` if no curated opening still matches — caller
    /// should fall through to engine search.
    pub fn peek(&self, history: &[Move]) -> Option<BookPick> {
        let mut candidates: Vec<(OpeningId, Move)> = Vec::with_capacity(self.allowed.len());
        for &id in &self.allowed {
            let Some(entry) = openings::entry(id) else {
                continue;
            };
            if entry.line.len() <= history.len() {
                // Line is exhausted at this point — bot would have to
                // search anyway.
                continue;
            }
            // history must be a prefix of entry.line.
            if entry
                .line
                .iter()
                .zip(history.iter())
                .all(|(line_mv, played_mv)| line_mv == played_mv)
            {
                let next_mv = entry.line[history.len()];
                candidates.push((id, next_mv));
            }
        }
        if candidates.is_empty() {
            return None;
        }
        // Stable order: candidates inherit `self.allowed`'s OpeningId
        // ordering, so `idx` is reproducible across runs.
        let salt = self.seed.wrapping_add(history.len() as u64);
        let idx = (pick_u64(salt) as usize) % candidates.len();
        let (opening_id, mv) = candidates[idx];
        Some(BookPick { mv, opening_id })
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
/// dozen — not a cryptographic RNG.
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

    fn replay(moves: &[Move]) -> Position {
        let mut pos = Position::startpos();
        for &mv in moves {
            pos.do_move(mv);
        }
        pos
    }

    fn san_to_moves(sans: &[&str]) -> Vec<Move> {
        let mut pos = Position::startpos();
        let mut moves = Vec::new();
        for s in sans {
            let mv = san::parse(&mut pos, s).expect("legal SAN");
            pos.do_move(mv);
            moves.push(mv);
        }
        moves
    }

    fn cursor_with(ids: Vec<OpeningId>, seed: u64) -> BookCursor {
        let profile = OpponentProfile {
            book: BookSelection::Allowed(ids),
            seed,
            ..OpponentProfile::with_seed(seed)
        };
        BookCursor::new(&profile, &Position::startpos()).expect("cursor")
    }

    // ---- curated default sanity ------------------------------------

    #[test]
    fn curated_default_resolves_to_nonempty_list() {
        assert!(!curated_default_ids().is_empty());
    }

    #[test]
    fn every_curated_entry_resolves() {
        for (eco, name) in CURATED {
            assert!(
                openings::find_id_exact(eco, name).is_some(),
                "curated entry ({eco}, {name:?}) no longer resolves in the TSV",
            );
        }
    }

    // ---- BookCursor::new gating ------------------------------------

    #[test]
    fn new_returns_none_when_book_is_off() {
        let profile = OpponentProfile {
            book: BookSelection::None,
            ..OpponentProfile::with_seed(1)
        };
        assert!(BookCursor::new(&profile, &Position::startpos()).is_none());
    }

    #[test]
    fn new_returns_none_for_non_startpos() {
        let profile = OpponentProfile::with_seed(1);
        let pos = Position::from_fen(
            "r3k2r/pppq1ppp/2n2n2/3pp3/3PP3/2N2N2/PPPQ1PPP/R3K2R w KQkq - 0 1",
        )
        .expect("test FEN");
        assert!(BookCursor::new(&profile, &pos).is_none());
    }

    #[test]
    fn new_returns_some_with_curated_default() {
        let profile = OpponentProfile {
            book: BookSelection::Allowed(curated_default_ids()),
            ..OpponentProfile::with_seed(42)
        };
        let cursor =
            BookCursor::new(&profile, &Position::startpos()).expect("curated + startpos");
        assert!(
            cursor.peek(&[]).is_some(),
            "peek with empty history must pick something"
        );
    }

    // ---- peek correctness ------------------------------------------

    #[test]
    fn peek_returns_none_when_history_diverges_from_every_opening() {
        // 1.a4 isn't the first move of any curated opening, so peek
        // after this history must fall through to search.
        let history = san_to_moves(&["a4"]);
        let cursor = cursor_with(curated_default_ids(), 0xCAFE);
        assert!(cursor.peek(&history).is_none());
    }

    #[test]
    fn peek_recovers_when_user_enters_a_different_curated_opening() {
        // The pre-commit bug we're fixing: at game start the cursor
        // committed to a single opening (say Ruy Lopez); if the human
        // then played a move consistent with a *different* curated
        // opening (say Italian — 3.Bc4 instead of 3.Bb5), the cursor
        // dropped out of book even though Italian was still in the
        // allowed list. With per-ply lookup the cursor should follow
        // whichever curated opening still matches the history.
        let ids = vec![
            openings::find_id_exact("C50", "Italian Game").expect("Italian Game"),
            openings::find_id_exact("C60", "Ruy Lopez").expect("Ruy Lopez"),
        ];
        let cursor = cursor_with(ids, 0xCAFE);
        // After "1.e4 e5 2.Nf3 Nc6" both openings still match — the
        // shared prefix is the entire Ruy Lopez line minus its last
        // ply and the entire Italian line minus its last ply. peek
        // must return Bc4 or Bb5; either way the bot's *next move*
        // is a book pick rather than a search result.
        let history = san_to_moves(&["e4", "e5", "Nf3", "Nc6"]);
        let pick = cursor.peek(&history).expect("both openings still match");
        let mut pos = replay(&history);
        let san_str = san::format(&pos, pick.mv);
        assert!(
            san_str == "Bc4" || san_str == "Bb5",
            "peek must pick Bc4 (Italian) or Bb5 (Ruy Lopez), got {san_str}",
        );
        pos.do_move(pick.mv);
    }

    #[test]
    fn peek_follows_longer_curated_line_after_shorter_one_is_exhausted() {
        // Two curated openings share a prefix but have different
        // lengths. After replaying the shorter one's full line, the
        // longer one is still alive — peek must surface its next move.
        let short_id = openings::find_id_exact("C50", "Italian Game").expect("short");
        // Pick *any* deeper Italian variation that starts with the
        // same shared prefix; first matching row.
        let long_id = openings::find_ids_matching("Italian Game: Giuoco Pianissimo")
            .into_iter()
            .next()
            .expect("at least one deeper Italian variant");
        let cursor = cursor_with(vec![short_id, long_id], 0xBEEF);
        // History matches both at the shared root, but C50's line is
        // length 5; the deeper variant continues past it. After 5
        // plies the cursor should find the deeper one.
        let short_line = openings::entry(short_id).expect("short entry").line.clone();
        let pick = cursor
            .peek(&short_line)
            .expect("deeper curated entry must still have a next move");
        let long_line = &openings::entry(long_id).expect("long entry").line;
        assert_eq!(
            pick.mv,
            long_line[short_line.len()],
            "peek's next move must come from the longer line at the right ply",
        );
    }

    #[test]
    fn peek_with_empty_history_can_pick_any_first_move() {
        // The list ordering shouldn't affect which one gets picked
        // (sort + seed pin it).
        let ids = curated_default_ids();
        let cursor_a = cursor_with(ids.clone(), 0xCAFE);
        let mut shuffled = ids.clone();
        shuffled.reverse();
        let cursor_b = cursor_with(shuffled, 0xCAFE);
        let a = cursor_a.peek(&[]).expect("a");
        let b = cursor_b.peek(&[]).expect("b");
        assert_eq!(a.opening_id, b.opening_id);
    }

    // ---- determinism -----------------------------------------------

    #[test]
    fn peek_is_deterministic_for_same_seed_and_history() {
        let cursor = cursor_with(curated_default_ids(), 0xBEEF);
        let history = san_to_moves(&["e4"]);
        let a = cursor.peek(&history).expect("a");
        let b = cursor.peek(&history).expect("b");
        assert_eq!(a.opening_id, b.opening_id);
        assert_eq!(a.mv, b.mv);
    }

    #[test]
    fn different_seeds_can_pick_different_first_moves() {
        // With a heterogeneous curated list (1.e4 and 1.d4 lines both
        // represented), two seeds should be reasonably likely to land
        // on different first moves. Try a small spread; this is
        // statistical but the seed range is large.
        let ids = curated_default_ids();
        let mut seen = std::collections::HashSet::new();
        for seed in 0u64..16 {
            let cursor = cursor_with(ids.clone(), seed);
            let pick = cursor.peek(&[]).expect("pick");
            seen.insert(pick.mv);
        }
        assert!(
            seen.len() >= 2,
            "expected ≥2 distinct first-move picks across 16 seeds; got {}",
            seen.len(),
        );
    }

    // ---- end-of-line behaviour -------------------------------------

    #[test]
    fn peek_returns_none_at_end_of_line() {
        // Use a list of one opening so we know exactly what line we
        // expect to follow; replay every move; the next peek should
        // come back None.
        let id = openings::find_id_exact("C50", "Italian Game").expect("Italian Game");
        let entry = openings::entry(id).expect("entry");
        let cursor = cursor_with(vec![id], 0);
        let history = entry.line.clone();
        // Sanity: the history we replay must reach a valid position
        // (otherwise the openings db is broken; not really our test).
        let _ = replay(&history);
        assert!(
            cursor.peek(&history).is_none(),
            "with history exhausting the line there should be no next book move",
        );
    }
}
