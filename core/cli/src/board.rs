//! Unicode chess board rendering for the CLI.
//!
//! Pure string formatting from a [`BoardView`]. The same descriptor
//! the egui renderer paints; the only CLI-flavoured concerns are the
//! ANSI escape sequences and the file/rank labels.
//!
//! Supports:
//! - Unicode pieces (default) with an `--ascii` fallback for terminals
//!   that can't render the chess glyphs.
//! - Chequered square shading via 256-colour ANSI backgrounds, with a
//!   warm amber highlight for last-move squares. Background colours are
//!   skipped in `--ascii` mode so the output stays pipe-friendly.
//! - Orientation derived from the view (caller picks `flipped` when
//!   composing).

use std::fmt::Write;

use chess_tutor_engine::types::{Color, Piece, PieceType};
use chess_tutor_ui::view::BoardView;

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
    /// Glyphs are drawn in the terminal's foreground colour, so the *filled*
    /// Unicode chess pieces read as *bright* and the outlined ones as *dim*.
    /// On a dark terminal we therefore assign filled glyphs to white pieces
    /// and outlined glyphs to black — the opposite of what their Unicode
    /// names suggest. Flip this to `true` for a light-background terminal
    /// where the naming-matching assignment is correct. Also swaps the
    /// board's chequered-square palette to match.
    pub light_mode: bool,
}

/// Render a [`BoardView`] into a multi-line display string. The view
/// is already pre-oriented; file/rank labels are derived from the
/// per-cell `square` fields.
pub fn render(view: &BoardView, opts: &RenderOptions) -> String {
    let mut out = String::new();
    write_file_labels(&mut out, view);

    for row in &view.rows {
        let rank_num = row[0].square.rank().index() + 1;
        write!(out, "{} ", rank_num).unwrap();
        for cell in row {
            if !opts.ascii {
                let bg = cell_bg(cell.is_light, cell.last_move, opts.light_mode);
                write!(out, "\x1b[48;5;{bg}m").unwrap();
            }
            out.push_str(&cell_glyph(cell.piece, opts.ascii, opts.light_mode));
            out.push(' ');
            if !opts.ascii {
                out.push_str("\x1b[49m");
            }
        }
        writeln!(out, "{}", rank_num).unwrap();
    }

    write_file_labels(&mut out, view);
    out
}

/// File labels above and below the board. Files come straight from
/// the bottom row's cell squares, so flipped boards print `h..a`.
fn write_file_labels(out: &mut String, view: &BoardView) {
    out.push_str("  ");
    for cell in &view.rows[7] {
        out.push((b'a' + cell.square.file().index() as u8) as char);
        out.push(' ');
    }
    out.push('\n');
}

/// Pick the 256-colour background index for a cell.
fn cell_bg(is_light_square: bool, highlighted: bool, light_mode: bool) -> u8 {
    if highlighted {
        return if light_mode {
            LIGHT_MODE_HIGHLIGHT
        } else {
            DARK_MODE_HIGHLIGHT
        };
    }
    match (light_mode, is_light_square) {
        (false, true) => DARK_MODE_LIGHT_SQUARE,
        (false, false) => DARK_MODE_DARK_SQUARE,
        (true, true) => LIGHT_MODE_LIGHT_SQUARE,
        (true, false) => LIGHT_MODE_DARK_SQUARE,
    }
}

fn cell_glyph(piece: Option<Piece>, ascii: bool, light_mode: bool) -> String {
    let Some(piece) = piece else {
        return if ascii { "." } else { " " }.to_string();
    };
    let is_white = piece.color() == Color::White;
    let pt = piece.kind();
    if ascii {
        let letter = piece_letter(pt);
        return if is_white {
            letter.to_string()
        } else {
            letter.to_ascii_lowercase().to_string()
        };
    }
    // Windows Terminal / Conhost ignores the VS15 text-presentation
    // selector on U+265F (BLACK CHESS PAWN ♟) and falls through to
    // Segoe UI Emoji — purple and off-grid. Both pawns substitute the
    // outline ♙ on Windows and SGR distinguishes their colours.
    if cfg!(target_os = "windows") && pt == PieceType::Pawn {
        return if is_white {
            "\x1b[1;97m♙\u{FE0E}\x1b[22;39m".to_string()
        } else {
            "\x1b[90m♙\u{FE0E}\x1b[39m".to_string()
        };
    }
    let use_filled = if light_mode { !is_white } else { is_white };
    let glyph = match (pt, use_filled) {
        (PieceType::King, true) => "♚",
        (PieceType::King, false) => "♔",
        (PieceType::Queen, true) => "♛",
        (PieceType::Queen, false) => "♕",
        (PieceType::Rook, true) => "♜",
        (PieceType::Rook, false) => "♖",
        (PieceType::Bishop, true) => "♝",
        (PieceType::Bishop, false) => "♗",
        (PieceType::Knight, true) => "♞",
        (PieceType::Knight, false) => "♘",
        (PieceType::Pawn, true) => "♟",
        (PieceType::Pawn, false) => "♙",
    };
    if is_white {
        format!("{glyph}\u{FE0E}")
    } else {
        format!("\x1b[90m{glyph}\u{FE0E}\x1b[39m")
    }
}

fn piece_letter(pt: PieceType) -> char {
    match pt {
        PieceType::King => 'K',
        PieceType::Queen => 'Q',
        PieceType::Rook => 'R',
        PieceType::Bishop => 'B',
        PieceType::Knight => 'N',
        PieceType::Pawn => 'P',
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::position::Position;
    use chess_tutor_engine::san;
    use chess_tutor_engine::types::Move;
    use chess_tutor_ui::view::BoardView;

    const STARTPOS_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    fn startpos_view(flipped: bool) -> BoardView {
        let pos = Position::startpos();
        BoardView::compose(&pos, flipped, None, None, &[], None, Vec::new())
    }

    #[test]
    fn dark_mode_default_swaps_filled_and_outlined() {
        let out = render(&startpos_view(false), &RenderOptions::default());
        assert!(out.contains("♛\u{FE0E}"), "white queen should use filled ♛");
        assert!(out.contains("♜\u{FE0E}"), "white rook should use filled ♜");
        assert!(out.contains("♔"), "black king should use outlined ♔");
        assert!(out.contains("♕"), "black queen should use outlined ♕");
        assert!(out.contains("a b c d e f g h"));
    }

    #[test]
    fn light_mode_uses_naming_matching_glyphs() {
        let out = render(
            &startpos_view(false),
            &RenderOptions {
                light_mode: true,
                ascii: false,
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
        let out = render(&startpos_view(false), &RenderOptions::default());
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
                &startpos_view(false),
                &RenderOptions {
                    light_mode: light,
                    ascii: false,
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
        let out = render(&startpos_view(false), &RenderOptions::default());
        assert!(
            !out.contains('•') && !out.contains('·'),
            "unicode mode should render empty squares as plain bg colour"
        );
    }

    #[test]
    fn black_pieces_use_dim_grey_fg() {
        let out = render(&startpos_view(false), &RenderOptions::default());
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
            &startpos_view(false),
            &RenderOptions {
                ascii: true,
                light_mode: false,
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
        let out = render(&startpos_view(true), &RenderOptions::default());
        let first_line_after_header = out.lines().nth(1).unwrap();
        assert!(first_line_after_header.starts_with("1"));
        assert!(out.contains("h g f e d c b a"));
    }

    #[test]
    fn highlight_uses_amber_bg_not_reverse_video() {
        let mut pos = Position::from_fen(STARTPOS_FEN).unwrap();
        let e2e4: Move = san::parse(&mut pos, "e4").unwrap();
        let view = BoardView::compose(&pos, false, Some(e2e4), None, &[], None, Vec::new());
        let out = render(&view, &RenderOptions::default());
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
        let out = render(&startpos_view(false), &RenderOptions::default());
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
        // Start position puts white pieces on rank 1; a1/c1/e1/g1 are
        // dark squares, so the dark-square bg escape must appear in
        // rank 1's rendered line.
        let out = render(&startpos_view(false), &RenderOptions::default());
        let rank_1 = out.lines().find(|l| l.starts_with("1 ")).unwrap();
        assert!(
            rank_1.contains(&format!("\x1b[48;5;{DARK_MODE_DARK_SQUARE}m")),
            "rank 1 should include at least one dark-square bg, and a1 is dark"
        );
    }
}
