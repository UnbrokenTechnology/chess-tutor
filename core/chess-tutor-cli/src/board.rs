//! Unicode chess board rendering for the CLI.
//!
//! Pure string formatting from a FEN. Kept out of `chess-tutor-core` because
//! rendering is presentation, not analysis — the core stays I/O-free.
//!
//! Supports:
//! - Unicode pieces (default) with an `--ascii` fallback for terminals that
//!   can't render the chess glyphs.
//! - Optional flip for Black's perspective.
//! - Optional last-move highlight via ANSI reverse video — degrades silently
//!   to unhighlighted output in terminals that ignore escapes.

use std::fmt::Write;

#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    pub ascii: bool,
    pub flip: bool,
    /// `(from_square, to_square)` as UCI squares (e.g. `("e2", "e4")`). Both
    /// squares are rendered with reverse-video highlight.
    pub highlight: Option<(String, String)>,
}

/// Render a FEN's board portion into a multi-line display string.
pub fn render(fen: &str, opts: &RenderOptions) -> String {
    let board_fen = fen.split_whitespace().next().unwrap_or(fen);

    // grid[0] is rank 8, grid[7] is rank 1 (FEN natural order).
    let mut grid = [[' '; 8]; 8];
    let mut rank_idx = 0usize;
    let mut file_idx = 0usize;
    for ch in board_fen.chars() {
        match ch {
            '/' => {
                rank_idx += 1;
                file_idx = 0;
            }
            '1'..='8' => {
                file_idx += ch.to_digit(10).unwrap() as usize;
            }
            _ => {
                if rank_idx < 8 && file_idx < 8 {
                    grid[rank_idx][file_idx] = ch;
                    file_idx += 1;
                }
            }
        }
    }

    let highlight = opts
        .highlight
        .as_ref()
        .and_then(|(from, to)| Some((square_to_idx(from)?, square_to_idx(to)?)));

    let ranks: Vec<usize> = if opts.flip {
        (0..8).rev().collect()
    } else {
        (0..8).collect()
    };
    let files: Vec<usize> = if opts.flip {
        (0..8).rev().collect()
    } else {
        (0..8).collect()
    };

    let mut out = String::new();
    write_file_labels(&mut out, opts.flip);

    for &r in &ranks {
        let rank_num = 8 - r;
        write!(out, "{} ", rank_num).unwrap();
        for &f in &files {
            let ch = grid[r][f];
            let is_hl = highlight
                .map(|(from, to)| (r, f) == from || (r, f) == to)
                .unwrap_or(false);

            if is_hl {
                out.push_str("\x1b[7m"); // reverse video
            }
            out.push_str(&cell_glyph(ch, opts.ascii));
            if is_hl {
                out.push_str("\x1b[0m");
            }
            out.push(' ');
        }
        writeln!(out, "{}", rank_num).unwrap();
    }

    write_file_labels(&mut out, opts.flip);
    out
}

fn write_file_labels(out: &mut String, flip: bool) {
    let files = if flip { "hgfedcba" } else { "abcdefgh" };
    out.push_str("  ");
    for f in files.chars() {
        out.push(f);
        out.push(' ');
    }
    out.push('\n');
}

fn cell_glyph(ch: char, ascii: bool) -> String {
    if ch == ' ' {
        return if ascii { "." } else { "·" }.to_string();
    }
    if ascii {
        return ch.to_string();
    }
    // U+FE0E (VARIATION SELECTOR-15) forces text presentation. Without it,
    // U+265F (BLACK CHESS PAWN ♟) renders as a double-width colour emoji on
    // modern terminals — it's the only chess glyph with Emoji_Presentation
    // set by default. Appended universally for consistency; it's a no-op
    // for characters that already default to text.
    let piece = match ch {
        'K' => "♔",
        'Q' => "♕",
        'R' => "♖",
        'B' => "♗",
        'N' => "♘",
        'P' => "♙",
        'k' => "♚",
        'q' => "♛",
        'r' => "♜",
        'b' => "♝",
        'n' => "♞",
        'p' => "♟",
        _ => return "?".to_string(),
    };
    format!("{piece}\u{FE0E}")
}

fn square_to_idx(sq: &str) -> Option<(usize, usize)> {
    let mut chars = sq.chars();
    let file_ch = chars.next()?;
    let rank_ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let file = match file_ch {
        'a'..='h' => (file_ch as u8 - b'a') as usize,
        _ => return None,
    };
    let rank = match rank_ch {
        '1'..='8' => (b'8' - rank_ch as u8) as usize,
        _ => return None,
    };
    Some((rank, file))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STARTPOS: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    #[test]
    fn renders_startpos_unicode() {
        let out = render(STARTPOS, &RenderOptions::default());
        // Every Unicode piece is followed by U+FE0E (text presentation
        // selector) to suppress the emoji rendering of the black pawn.
        assert!(out.contains("♜\u{FE0E}"));
        assert!(out.contains("♟\u{FE0E}"));
        assert!(out.contains("♔\u{FE0E}"));
        assert!(out.contains("♙\u{FE0E}"));
        assert!(out.contains("a b c d e f g h"));
    }

    #[test]
    fn black_pawn_has_text_presentation_selector() {
        let out = render(STARTPOS, &RenderOptions::default());
        assert!(
            out.contains("♟\u{FE0E}"),
            "black pawn should be rendered with VS15 to avoid emoji presentation"
        );
    }

    #[test]
    fn renders_startpos_ascii() {
        let out = render(
            STARTPOS,
            &RenderOptions {
                ascii: true,
                ..Default::default()
            },
        );
        assert!(out.contains("r n b q k b n r"));
        assert!(out.contains("R N B Q K B N R"));
        // Empty squares should be dots in ASCII mode.
        assert!(out.contains(". . . . . . . ."));
    }

    #[test]
    fn flip_swaps_perspective() {
        let out = render(
            STARTPOS,
            &RenderOptions {
                flip: true,
                ..Default::default()
            },
        );
        // First rank in a flipped view is rank 1, so bottom row becomes top.
        let first_line_after_header = out.lines().nth(1).unwrap();
        assert!(first_line_after_header.starts_with("1"));
        // File labels reverse.
        assert!(out.contains("h g f e d c b a"));
    }

    #[test]
    fn highlight_wraps_from_and_to_in_escape() {
        let out = render(
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1",
            &RenderOptions {
                highlight: Some(("e2".into(), "e4".into())),
                ..Default::default()
            },
        );
        assert!(out.contains("\x1b[7m"));
        assert!(out.contains("\x1b[0m"));
    }
}
