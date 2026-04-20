//! Detect checks, captures, and material threats for both sides.
//!
//! Built on top of `shakmaty::Position::legal_moves` plus [`super::see`].
//! A [`ThreatScan`] enumerates, from one side's perspective:
//!
//! - **Checks** — legal moves that give check.
//! - **Captures** — legal moves that take a piece. The SEE is recorded so
//!   callers can distinguish free gains from losing captures.
//! - **Threats** — quiet, non-checking moves that, if left unanswered, would
//!   win material on the next turn. A move qualifies when the post-move
//!   position has at least one enemy square where the mover's SEE is
//!   positive.
//!
//! "Both sides" is handled by [`ThreatScans::from_position`]: the mover's
//! scan uses the current position, and the opponent's scan uses the
//! null-move-flipped position. Flipping fails when the mover is in check
//! (the non-mover's king would be in check, which is not a legal chess
//! position) — in that case the opponent scan is `None`.
//!
//! Square-valued fields are stored as names (`"e4"`) and roles as lowercase
//! letters (`"p"`, `"n"`, `"b"`, `"r"`, `"q"`, `"k"`) so the whole struct
//! round-trips through serde without depending on `shakmaty`'s `serde`
//! feature. This matches the [`super::CandidateMove`] convention.

use serde::{Deserialize, Serialize};
use shakmaty::{
    fen::Fen, san::SanPlus, CastlingMode, Chess, Color, EnPassantMode, Move, Position, Role,
    Square,
};

use super::see;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThreatMove {
    pub uci: String,
    pub san: String,
    pub from: String,
    pub to: String,
    pub role: String,
    pub captured: Option<String>,
    /// SEE (centipawns) for this move. Zero for non-captures.
    pub see: i32,
    pub gives_check: bool,
    /// Enemy squares where the mover has a winning follow-up capture
    /// (`see_on_square > 0`) in the resulting position. Empty on moves that
    /// don't create any new material threat.
    pub threatens: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThreatScan {
    /// Whose moves this scan describes — `"white"` or `"black"`.
    pub side: String,
    /// Moves that deliver check (may also appear in `captures`).
    pub checks: Vec<ThreatMove>,
    /// Moves that capture a piece (may also appear in `checks`).
    pub captures: Vec<ThreatMove>,
    /// Quiet, non-checking moves that threaten to win material next turn.
    pub threats: Vec<ThreatMove>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThreatScans {
    pub mover: ThreatScan,
    /// `None` when the null-move flip is illegal — only happens when the
    /// mover is currently in check.
    pub opponent: Option<ThreatScan>,
}

impl ThreatScans {
    pub fn from_position(position: &Chess) -> Self {
        let mover = ThreatScan::from_position(position);
        let opponent = flipped(position).map(|p| ThreatScan::from_position(&p));
        Self { mover, opponent }
    }
}

impl ThreatScan {
    /// Scan from the perspective of whoever is to move in `position`.
    pub fn from_position(position: &Chess) -> Self {
        let side = color_name(position.turn()).to_string();
        let mut checks = Vec::new();
        let mut captures = Vec::new();
        let mut threats = Vec::new();

        for mv in position.legal_moves() {
            let tm = annotate(position, &mv);

            let mut classified = false;
            if tm.captured.is_some() {
                captures.push(tm.clone());
                classified = true;
            }
            if tm.gives_check {
                checks.push(tm.clone());
                classified = true;
            }
            if !classified && !tm.threatens.is_empty() {
                threats.push(tm);
            }
        }

        Self {
            side,
            checks,
            captures,
            threats,
        }
    }
}

fn annotate(position: &Chess, mv: &Move) -> ThreatMove {
    let role = mv.role();
    let from = mv.from().expect("standard chess move has a from square");
    let to = mv.to();
    let captured = mv.capture();
    let see_value = see::see(position, from, to);

    let san = SanPlus::from_move(position.clone(), mv).to_string();
    let uci = mv.to_uci(CastlingMode::Standard).to_string();

    let mut next = position.clone();
    next.play_unchecked(mv);
    let gives_check = next.is_check();

    let threatens: Vec<String> = winning_captures_for(&next, position.turn())
        .into_iter()
        .map(square_name)
        .collect();

    ThreatMove {
        uci,
        san,
        from: square_name(from),
        to: square_name(to),
        role: role_letter(role).to_string(),
        captured: captured.map(|r| role_letter(r).to_string()),
        see: see_value,
        gives_check,
        threatens,
    }
}

/// Enemy squares where `side` has an SEE-positive capture in `position`.
/// The king is excluded — checks are reported via `gives_check`, not here.
fn winning_captures_for(position: &Chess, side: Color) -> Vec<Square> {
    let board = position.board();
    let mut out = Vec::new();
    for sq in Square::ALL {
        let Some(piece) = board.piece_at(sq) else {
            continue;
        };
        if piece.color == side || piece.role == Role::King {
            continue;
        }
        if let Some(v) = see::see_on_square(position, sq, side) {
            if v > 0 {
                out.push(sq);
            }
        }
    }
    out
}

/// Null-move the position: swap the side to move, clear the en passant
/// square (it belongs to the side about to move, not the opponent), and
/// re-parse. Returns `None` when the resulting position is illegal — which
/// happens when the current mover is in check (flipping would leave the
/// non-mover's king in check).
fn flipped(position: &Chess) -> Option<Chess> {
    let fen = Fen::from_position(position.clone(), EnPassantMode::Legal).to_string();
    let parts: Vec<&str> = fen.split_whitespace().collect();
    if parts.len() < 6 {
        return None;
    }
    let new_side = if parts[1] == "w" { "b" } else { "w" };
    let new_fen = format!(
        "{} {} {} - {} {}",
        parts[0], new_side, parts[2], parts[4], parts[5]
    );
    let parsed: Fen = new_fen.parse().ok()?;
    parsed.into_position(CastlingMode::Standard).ok()
}

fn square_name(sq: Square) -> String {
    let file = (u32::from(sq) % 8) as u8 + b'a';
    let rank = (u32::from(sq) / 8) as u8 + b'1';
    format!("{}{}", file as char, rank as char)
}

fn role_letter(r: Role) -> char {
    match r {
        Role::Pawn => 'p',
        Role::Knight => 'n',
        Role::Bishop => 'b',
        Role::Rook => 'r',
        Role::Queen => 'q',
        Role::King => 'k',
    }
}

fn color_name(c: Color) -> &'static str {
    match c {
        Color::White => "white",
        Color::Black => "black",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(fen: &str) -> Chess {
        fen.parse::<Fen>()
            .unwrap()
            .into_position(CastlingMode::Standard)
            .unwrap()
    }

    #[test]
    fn startpos_has_no_checks_captures_or_threats() {
        let p = pos("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
        let scan = ThreatScan::from_position(&p);
        assert!(scan.checks.is_empty());
        assert!(scan.captures.is_empty());
        assert!(scan.threats.is_empty());
    }

    #[test]
    fn fools_mate_check_detected() {
        // After 1. f3 e5 2. g4, Black plays Qh4# — the only legal check.
        let p = pos("rnbqkbnr/pppp1ppp/8/4p3/6P1/5P2/PPPPP2P/RNBQKBNR b KQkq g3 0 2");
        let scan = ThreatScan::from_position(&p);
        let qh4 = scan
            .checks
            .iter()
            .find(|m| m.san.starts_with("Qh4"))
            .expect("Qh4 should be detected as a check");
        assert_eq!(qh4.uci, "d8h4");
        assert_eq!(qh4.role, "q");
    }

    #[test]
    fn simple_pawn_capture_listed() {
        let p = pos("4k3/8/8/3p4/2P5/8/8/4K3 w - - 0 1");
        let scan = ThreatScan::from_position(&p);
        assert_eq!(scan.captures.len(), 1);
        let cap = &scan.captures[0];
        assert_eq!(cap.uci, "c4d5");
        assert_eq!(cap.captured.as_deref(), Some("p"));
        assert_eq!(cap.see, 100);
        assert!(!cap.gives_check);
    }

    #[test]
    fn pawn_fork_is_a_threat() {
        // Black Rc5 + Nb5 on ranks 5, White pawn on d3. d3-d4 attacks both.
        // After d4: SEE on c5 and e5 is positive for White.
        let p = pos("4k3/8/8/2r1n3/8/3P4/8/4K3 w - - 0 1");
        let scan = ThreatScan::from_position(&p);
        let fork = scan
            .threats
            .iter()
            .find(|m| m.uci == "d3d4")
            .expect("d3-d4 should register as a threat");
        assert_eq!(fork.role, "p");
        assert!(fork.captured.is_none());
        assert!(!fork.gives_check);
        assert!(fork.threatens.iter().any(|s| s == "c5"));
        assert!(fork.threatens.iter().any(|s| s == "e5"));
    }

    #[test]
    fn checking_capture_appears_in_both_lists() {
        // Rxh8+ captures the rook and checks the a8 king along the 8th rank.
        let p = pos("k6r/8/8/8/8/8/8/4K2R w K - 0 1");
        let scan = ThreatScan::from_position(&p);
        assert!(scan.captures.iter().any(|m| m.uci == "h1h8"));
        assert!(scan.checks.iter().any(|m| m.uci == "h1h8"));
    }

    #[test]
    fn both_sides_scans_the_opponent_too() {
        // 4-Knights shape: White Nxe5 and Black Nxe4 are both available.
        let p = pos("r1bqkb1r/pppp1ppp/2n2n2/4p3/4P3/2N2N2/PPPP1PPP/R1BQKB1R w KQkq - 4 4");
        let scans = ThreatScans::from_position(&p);
        assert_eq!(scans.mover.side, "white");
        let opponent = scans
            .opponent
            .expect("flip is legal when neither side is in check");
        assert_eq!(opponent.side, "black");
        assert!(scans.mover.captures.iter().any(|m| m.to == "e5"));
        assert!(opponent.captures.iter().any(|m| m.to == "e4"));
    }

    #[test]
    fn flipped_fails_when_mover_in_check() {
        // White king on e1 in check from the e8 rook.
        let p = pos("4r3/8/8/8/7k/8/8/4K3 w - - 0 1");
        let scans = ThreatScans::from_position(&p);
        assert!(scans.opponent.is_none());
    }
}
