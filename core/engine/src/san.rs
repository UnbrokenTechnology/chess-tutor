//! Standard Algebraic Notation — parse and format.
//!
//! Parsing is **lenient**: annotations (`!`, `?`, `+`, `#`, trailing
//! `e.p.`) are stripped, the capture marker (`x`) is optional, and
//! castling accepts both `O-O`/`O-O-O` and `0-0`/`0-0-0`. So `Qc6`
//! parses to the same move as `Qxc6+` when c6 is a capture delivering
//! check — a pragmatic convenience for terminal play where the user
//! shouldn't have to spell the move out fully.
//!
//! Formatting is **canonical**: [`format`] always returns the shortest
//! unambiguous SAN for a `(Position, Move)` pair — proper file/rank
//! disambiguation, `x` on captures, `=Q` on promotions, and `+`/`#`
//! based on the opponent's reply state after the move.

use crate::movegen::legal_moves_vec;
use crate::position::Position;
use crate::types::{File, Move, MoveKind, PieceType, Rank, Square};

// =========================================================================
// Parsing
// =========================================================================

#[derive(Debug)]
struct SanTokens {
    piece: PieceType,
    from_file: Option<File>,
    from_rank: Option<Rank>,
    is_capture: Option<bool>, // None = user didn't say either way
    to: Square,
    promotion: Option<PieceType>,
}

/// Parse a SAN string and return the matching legal move for `pos`.
///
/// Lenient about embedded / missing `x`, trailing check/mate markers,
/// and NAG-ish annotations (`!`, `?`, `!!`, `?!`, …). Returns `Err` if
/// the parse fails, no legal move matches, or more than one matches.
pub fn parse(pos: &mut Position, input: &str) -> Result<Move, String> {
    let cleaned = strip_annotations(input.trim());
    if cleaned.is_empty() {
        return Err("empty SAN input".to_string());
    }

    // Castling — check before the piece-letter logic.
    if cleaned == "O-O" || cleaned == "0-0" {
        return find_castle(pos, true).ok_or_else(|| "O-O is not legal here".to_string());
    }
    if cleaned == "O-O-O" || cleaned == "0-0-0" {
        return find_castle(pos, false).ok_or_else(|| "O-O-O is not legal here".to_string());
    }

    let tokens = parse_tokens(&cleaned).ok_or_else(|| format!("cannot parse SAN {:?}", input))?;
    let legal = legal_moves_vec(pos);

    let mut matches: Vec<Move> = Vec::new();
    for mv in &legal {
        if move_matches_tokens(pos, *mv, &tokens) {
            matches.push(*mv);
        }
    }

    match matches.len() {
        0 => Err(format!("no legal move matches {:?}", input)),
        1 => Ok(matches[0]),
        _ => {
            // More than one legal move matches the tokens — report the
            // candidates so the user can narrow down.
            let options: Vec<String> = matches.iter().map(|m| format(pos, *m)).collect();
            Err(format!(
                "ambiguous SAN {:?}: matches {}",
                input,
                options.join(", ")
            ))
        }
    }
}

fn find_castle(pos: &mut Position, king_side: bool) -> Option<Move> {
    for mv in legal_moves_vec(pos) {
        if mv.kind() != MoveKind::Castling {
            continue;
        }
        let to_file = mv.to().file();
        let is_ks = to_file == File::G;
        if is_ks == king_side {
            return Some(mv);
        }
    }
    None
}

fn strip_annotations(s: &str) -> String {
    let mut out = s.to_string();
    // Trailing decorators can repeat and mix: remove any run of
    // `!`, `?`, `+`, `#` from the end.
    while let Some(last) = out.chars().last() {
        if matches!(last, '!' | '?' | '+' | '#') {
            out.pop();
        } else {
            break;
        }
    }
    // Trailing `e.p.` for en-passant captures.
    for suffix in [" e.p.", "e.p.", " ep", "ep"] {
        if out.ends_with(suffix) {
            out.truncate(out.len() - suffix.len());
            break;
        }
    }
    out.trim().to_string()
}

fn parse_tokens(input: &str) -> Option<SanTokens> {
    let bytes = input.as_bytes();
    let mut i = 0usize;

    // Piece letter (optional; pawn if absent).
    let piece = match bytes.first()? {
        b'K' => {
            i += 1;
            PieceType::King
        }
        b'Q' => {
            i += 1;
            PieceType::Queen
        }
        b'R' => {
            i += 1;
            PieceType::Rook
        }
        b'B' => {
            i += 1;
            PieceType::Bishop
        }
        b'N' => {
            i += 1;
            PieceType::Knight
        }
        _ => PieceType::Pawn,
    };

    // Find and strip promotion suffix from the tail: `=Q`, `=N`, etc., or
    // bare trailing piece letter for lenient styles.
    let body = &input[i..];
    let (body, promotion) = split_promotion(body);
    if body.is_empty() {
        return None;
    }

    // Strip optional `x` capture marker.
    let mut core: Vec<u8> = body.bytes().collect();
    let mut is_capture: Option<bool> = None;
    if let Some(pos_x) = core.iter().position(|&b| b == b'x' || b == b'X') {
        core.remove(pos_x);
        is_capture = Some(true);
    }
    // At this point `core` is: [disambig...] [dest-file] [dest-rank].
    if core.len() < 2 {
        return None;
    }
    let rank_byte = core[core.len() - 1];
    let file_byte = core[core.len() - 2];
    let to_file = match file_byte {
        b'a'..=b'h' => File::from_index(file_byte - b'a')?,
        _ => return None,
    };
    let to_rank = match rank_byte {
        b'1'..=b'8' => Rank::from_index(rank_byte - b'1')?,
        _ => return None,
    };
    let to = Square::new(to_file, to_rank);

    // Remaining bytes are disambig: 0, 1, or 2 of them.
    let disambig = &core[..core.len() - 2];
    let mut from_file: Option<File> = None;
    let mut from_rank: Option<Rank> = None;
    for &b in disambig {
        match b {
            b'a'..=b'h' => from_file = File::from_index(b - b'a'),
            b'1'..=b'8' => from_rank = Rank::from_index(b - b'1'),
            _ => return None,
        }
    }

    Some(SanTokens {
        piece,
        from_file,
        from_rank,
        is_capture,
        to,
        promotion,
    })
}

/// Split a SAN body into `(remaining, Option<promotion>)`. Accepts
/// `=Q`, `Q` at the end for promotion; leaves non-promotion tails alone.
fn split_promotion(body: &str) -> (&str, Option<PieceType>) {
    if body.is_empty() {
        return (body, None);
    }
    let bytes = body.as_bytes();
    let last = bytes[bytes.len() - 1];
    let promo = match last {
        b'Q' | b'q' => Some(PieceType::Queen),
        b'R' | b'r' => Some(PieceType::Rook),
        b'B' => Some(PieceType::Bishop), // keep uppercase only for bishop to
        // avoid eating a destination file
        // letter like `b` in `Nb3`
        b'N' | b'n' => Some(PieceType::Knight),
        _ => None,
    };
    // Guard: a bare `q`/`r`/`n` at the end of e.g. `Nb3` would be misread.
    // But `Nb3` ends in `3`, not a letter; the only non-promotion ends
    // that could collide are real destination-file letters like `b`, so
    // we only accept `B` (uppercase) as a bare promotion marker.
    if let Some(p) = promo {
        // Strip the promotion letter …
        let mut cut = bytes.len() - 1;
        // … and an optional `=` before it.
        if cut > 0 && bytes[cut - 1] == b'=' {
            cut -= 1;
        }
        // Sanity: need at least a 2-char destination before the promo.
        if cut >= 2 {
            return (&body[..cut], Some(p));
        }
    }
    (body, None)
}

fn move_matches_tokens(pos: &Position, mv: Move, t: &SanTokens) -> bool {
    if mv.to() != t.to {
        return false;
    }
    let moved = pos.piece_on(mv.from()).map(piece_type_of);
    if moved != Some(t.piece) {
        return false;
    }
    if let Some(f) = t.from_file {
        if mv.from().file() != f {
            return false;
        }
    }
    if let Some(r) = t.from_rank {
        if mv.from().rank() != r {
            return false;
        }
    }
    match (mv.kind(), t.promotion) {
        (MoveKind::Promotion, Some(p)) if mv.promoted_to() != p => return false,
        (MoveKind::Promotion, None) => return false, // promo required
        (k, Some(_)) if k != MoveKind::Promotion => return false,
        _ => {}
    }
    // Pawn captures: the from-file disambiguation is mandatory in real SAN
    // (e.g. `exd5`). If the user typed just `d5` for a pawn capture we'll
    // still match it here — the capture marker is optional by design.
    if let Some(cap) = t.is_capture {
        let was_cap = is_capture_move(pos, mv);
        if cap != was_cap {
            return false;
        }
    }
    true
}

fn is_capture_move(pos: &Position, mv: Move) -> bool {
    if mv.kind() == MoveKind::EnPassant {
        return true;
    }
    pos.piece_on(mv.to()).is_some()
}

fn piece_type_of(piece: crate::types::Piece) -> PieceType {
    use crate::types::Piece::*;
    match piece {
        WhitePawn | BlackPawn => PieceType::Pawn,
        WhiteKnight | BlackKnight => PieceType::Knight,
        WhiteBishop | BlackBishop => PieceType::Bishop,
        WhiteRook | BlackRook => PieceType::Rook,
        WhiteQueen | BlackQueen => PieceType::Queen,
        WhiteKing | BlackKing => PieceType::King,
    }
}

// =========================================================================
// Formatting
// =========================================================================

/// Format a move as canonical SAN for `pos` (the position *before* the
/// move is played). Always includes minimum-needed disambiguation, the
/// `x` capture marker, promotion suffix, and a `+` or `#` if the move
/// gives check or mate.
///
/// `pos` is not permanently modified: check/mate detection uses do/undo.
pub fn format(pos: &Position, mv: Move) -> String {
    // Clone so the caller's position is untouched by the do/undo we
    // need for the check/mate suffix.
    let mut scratch = pos.clone();
    format_on(&mut scratch, mv)
}

/// Same as [`format`], but operates on `pos` directly. `pos` is
/// restored to its original state before returning.
pub fn format_on(pos: &mut Position, mv: Move) -> String {
    // Castling first.
    if mv.kind() == MoveKind::Castling {
        let base = if mv.to().file() == File::G {
            "O-O"
        } else {
            "O-O-O"
        };
        return with_check_suffix(pos, mv, base);
    }

    let moved = piece_type_of(pos.piece_on(mv.from()).expect("from must be occupied"));
    let is_cap = is_capture_move(pos, mv);

    let mut out = String::new();

    if moved == PieceType::Pawn {
        if is_cap {
            out.push(file_char(mv.from().file()));
            out.push('x');
        }
        out.push_str(&mv.to().to_algebraic());
        if mv.kind() == MoveKind::Promotion {
            out.push('=');
            out.push(piece_letter(mv.promoted_to()));
        }
    } else {
        out.push(piece_letter(moved));
        out.push_str(&disambig_for(pos, mv, moved));
        if is_cap {
            out.push('x');
        }
        out.push_str(&mv.to().to_algebraic());
    }

    with_check_suffix(pos, mv, &out)
}

fn with_check_suffix(pos: &mut Position, mv: Move, base: &str) -> String {
    let state = pos.do_move(mv);
    let opponent_in_check = pos.in_check();
    let opponent_has_moves = !legal_moves_vec(pos).is_empty();
    pos.undo_move(mv, state);

    let suffix = match (opponent_in_check, opponent_has_moves) {
        (true, false) => "#",
        (true, true) => "+",
        _ => "",
    };
    format!("{base}{suffix}")
}

/// Work out the minimum disambiguation needed to make a piece's SAN
/// unambiguous. Returns `""`, `"<file>"`, `"<rank>"`, or `"<file><rank>"`.
fn disambig_for(pos: &Position, mv: Move, piece: PieceType) -> String {
    // Find all other legal moves for the same piece type landing on the
    // same destination square.
    let mut peers: Vec<Square> = Vec::new();
    // legal_moves_vec needs &mut Position; we clone to keep `pos` free.
    let mut scratch = pos.clone();
    for other in legal_moves_vec(&mut scratch) {
        if other.from() == mv.from() {
            continue;
        }
        if other.to() != mv.to() {
            continue;
        }
        let other_pt = piece_type_of(match pos.piece_on(other.from()) {
            Some(p) => p,
            None => continue,
        });
        if other_pt == piece {
            peers.push(other.from());
        }
    }
    if peers.is_empty() {
        return String::new();
    }

    let from_file = mv.from().file();
    let from_rank = mv.from().rank();
    let unique_file = peers.iter().all(|p| p.file() != from_file);
    let unique_rank = peers.iter().all(|p| p.rank() != from_rank);

    if unique_file {
        return file_char(from_file).to_string();
    }
    if unique_rank {
        return rank_char(from_rank).to_string();
    }
    format!("{}{}", file_char(from_file), rank_char(from_rank))
}

fn piece_letter(pt: PieceType) -> char {
    match pt {
        PieceType::King => 'K',
        PieceType::Queen => 'Q',
        PieceType::Rook => 'R',
        PieceType::Bishop => 'B',
        PieceType::Knight => 'N',
        PieceType::Pawn => 'P', // only used in disambig paths; pawns usually omit
    }
}

fn file_char(f: File) -> char {
    (b'a' + f as u8) as char
}

fn rank_char(r: Rank) -> char {
    (b'1' + r as u8) as char
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
#[path = "san_tests.rs"]
mod tests;
