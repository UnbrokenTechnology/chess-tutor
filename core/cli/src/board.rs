//! Unicode chess board rendering for the CLI.
//!
//! Pure string formatting from a FEN. Kept out of `chess-tutor-engine`
//! because rendering is presentation, not analysis — the engine stays
//! I/O-free.
//!
//! Supports:
//! - Unicode pieces (default) with an `--ascii` fallback for terminals that
//!   can't render the chess glyphs.
//! - Optional flip for Black's perspective.
//! - Chequered square shading via 256-colour ANSI backgrounds, with a warm
//!   amber highlight for last-move squares. Background colours are skipped
//!   in `--ascii` mode so the output stays pipe-friendly.

use std::fmt::Write;

// 256-colour palette indices chosen so black pieces stay readable, the
// chequered pattern is visible without being garish, and the highlight
// is obvious at a glance. Dark-mode values target a near-black terminal
// bg; light-mode values target a near-white terminal bg.
const DARK_MODE_DARK_SQUARE: u8 = 234;
const DARK_MODE_LIGHT_SQUARE: u8 = 237;
const DARK_MODE_HIGHLIGHT: u8 = 94;
const LIGHT_MODE_DARK_SQUARE: u8 = 250;
const LIGHT_MODE_LIGHT_SQUARE: u8 = 254;
const LIGHT_MODE_HIGHLIGHT: u8 = 222;

#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    pub ascii: bool,
    pub flip: bool,
    /// `(from_square, to_square)` as UCI squares (e.g. `("e2", "e4")`). Both
    /// squares are rendered with the highlight background colour.
    pub highlight: Option<(String, String)>,
    /// Glyphs are drawn in the terminal's foreground colour, so the *filled*
    /// Unicode chess pieces read as *bright* and the outlined ones as *dim*.
    /// On a dark terminal we therefore assign filled glyphs to white pieces
    /// and outlined glyphs to black — the opposite of what their Unicode
    /// names suggest. Flip this to `true` for a light-background terminal
    /// where the naming-matching assignment is correct. Also swaps the
    /// board's chequered-square palette to match.
    pub light_mode: bool,
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

            if !opts.ascii {
                let bg = cell_bg(r, f, is_hl, opts.light_mode);
                write!(out, "\x1b[48;5;{bg}m").unwrap();
            }
            out.push_str(&cell_glyph(ch, opts.ascii, opts.light_mode));
            out.push(' ');
            if !opts.ascii {
                out.push_str("\x1b[49m");
            }
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

/// Pick the 256-colour background index for a cell. a1 is a dark square
/// (standard chess convention: a1 file+rank = 0+0 => dark-coloured).
fn cell_bg(rank_idx: usize, file_idx: usize, highlighted: bool, light_mode: bool) -> u8 {
    if highlighted {
        return if light_mode {
            LIGHT_MODE_HIGHLIGHT
        } else {
            DARK_MODE_HIGHLIGHT
        };
    }
    let is_light_square = (rank_idx + file_idx) % 2 == 0;
    match (light_mode, is_light_square) {
        (false, true) => DARK_MODE_LIGHT_SQUARE,
        (false, false) => DARK_MODE_DARK_SQUARE,
        (true, true) => LIGHT_MODE_LIGHT_SQUARE,
        (true, false) => LIGHT_MODE_DARK_SQUARE,
    }
}

fn cell_glyph(ch: char, ascii: bool, light_mode: bool) -> String {
    if ch == ' ' {
        return if ascii { "." } else { " " }.to_string();
    }
    if ascii {
        return ch.to_string();
    }
    // Windows Terminal / Conhost ignores the VS15 text-presentation selector
    // on U+265F (BLACK CHESS PAWN ♟) and falls through to Segoe UI Emoji —
    // purple and off-grid. We can't use that glyph on Windows at all, so
    // both pawns substitute the outline ♙ and SGR distinguishes them.
    if cfg!(target_os = "windows") {
        match ch {
            'P' => return "\x1b[1;97m♙\u{FE0E}\x1b[22;39m".to_string(),
            'p' => return "\x1b[90m♙\u{FE0E}\x1b[39m".to_string(),
            _ => {}
        }
    }
    let is_white = ch.is_ascii_uppercase();
    let use_filled = if light_mode { !is_white } else { is_white };
    let glyph = match (ch.to_ascii_lowercase(), use_filled) {
        ('k', true) => "♚",
        ('k', false) => "♔",
        ('q', true) => "♛",
        ('q', false) => "♕",
        ('r', true) => "♜",
        ('r', false) => "♖",
        ('b', true) => "♝",
        ('b', false) => "♗",
        ('n', true) => "♞",
        ('n', false) => "♘",
        ('p', true) => "♟",
        ('p', false) => "♙",
        _ => return "?".to_string(),
    };
    if is_white {
        format!("{glyph}\u{FE0E}")
    } else {
        format!("\x1b[90m{glyph}\u{FE0E}\x1b[39m")
    }
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
    fn dark_mode_default_swaps_filled_and_outlined() {
        let out = render(STARTPOS, &RenderOptions::default());
        assert!(out.contains("♛\u{FE0E}"), "white queen should use filled ♛");
        assert!(out.contains("♜\u{FE0E}"), "white rook should use filled ♜");
        assert!(out.contains("♔"), "black king should use outlined ♔");
        assert!(out.contains("♕"), "black queen should use outlined ♕");
        assert!(out.contains("a b c d e f g h"));
    }

    #[test]
    fn light_mode_uses_naming_matching_glyphs() {
        let out = render(
            STARTPOS,
            &RenderOptions {
                light_mode: true,
                ..Default::default()
            },
        );
        assert!(
            out.contains("♔\u{FE0E}"),
            "white king should use outlined ♔"
        );
        assert!(out.contains("♛\u{FE0E}"), "black queen should use filled ♛");
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn dark_mode_uses_filled_white_pawn_without_emoji_tint() {
        let out = render(STARTPOS, &RenderOptions::default());
        assert!(
            out.contains("♟\u{FE0E}"),
            "white pawn should render as the filled U+265F in dark mode"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn pawns_use_windows_workaround_regardless_of_mode() {
        for light in [false, true] {
            let out = render(
                STARTPOS,
                &RenderOptions {
                    light_mode: light,
                    ..Default::default()
                },
            );
            assert!(
                !out.contains('\u{265F}'),
                "raw U+265F should not appear on Windows (light_mode={light})"
            );
            assert!(
                out.contains("\x1b[90m♙\u{FE0E}\x1b[39m"),
                "expected dim-grey ♙ for black pawn (light_mode={light})"
            );
            assert!(
                out.contains("\x1b[1;97m♙\u{FE0E}\x1b[22;39m"),
                "expected bold bright-white ♙ for white pawn (light_mode={light})"
            );
        }
    }

    #[test]
    fn unicode_mode_has_no_empty_square_marker() {
        let out = render(STARTPOS, &RenderOptions::default());
        assert!(
            !out.contains('•') && !out.contains('·'),
            "unicode mode should render empty squares as plain bg colour"
        );
    }

    #[test]
    fn black_pieces_use_dim_grey_fg() {
        let out = render(STARTPOS, &RenderOptions::default());
        assert!(
            out.contains("\x1b[90m♖\u{FE0E}\x1b[39m"),
            "black rook should be wrapped in dim-grey SGR"
        );
        assert!(
            !out.contains("\x1b[90m♜"),
            "white pieces must not be tinted dim grey"
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
        assert!(out.contains(". . . . . . . ."));
        assert!(
            !out.contains("\x1b["),
            "ASCII mode must not emit any ANSI escapes"
        );
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
        let first_line_after_header = out.lines().nth(1).unwrap();
        assert!(first_line_after_header.starts_with("1"));
        assert!(out.contains("h g f e d c b a"));
    }

    #[test]
    fn highlight_uses_amber_bg_not_reverse_video() {
        let out = render(
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1",
            &RenderOptions {
                highlight: Some(("e2".into(), "e4".into())),
                ..Default::default()
            },
        );
        assert!(
            out.contains(&format!("\x1b[48;5;{DARK_MODE_HIGHLIGHT}m")),
            "highlighted cells should use the amber bg, not reverse video"
        );
        assert!(
            !out.contains("\x1b[7m"),
            "reverse-video should no longer be in use"
        );
    }

    #[test]
    fn chequered_squares_alternate_backgrounds() {
        let out = render(STARTPOS, &RenderOptions::default());
        assert!(
            out.contains(&format!("\x1b[48;5;{DARK_MODE_LIGHT_SQUARE}m")),
            "light-square bg should appear in default (dark-mode) render"
        );
        assert!(
            out.contains(&format!("\x1b[48;5;{DARK_MODE_DARK_SQUARE}m")),
            "dark-square bg should appear in default (dark-mode) render"
        );
    }

    #[test]
    fn a1_is_a_dark_square() {
        let out = render("8/8/8/8/8/8/8/R7 w - - 0 1", &RenderOptions::default());
        let rank_1 = out.lines().find(|l| l.starts_with("1 ")).unwrap();
        assert!(
            rank_1.contains(&format!("\x1b[48;5;{DARK_MODE_DARK_SQUARE}m")),
            "rank 1 should include at least one dark-square bg, and a1 is dark"
        );
    }
}
