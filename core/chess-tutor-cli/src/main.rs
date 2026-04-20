//! Phase 1 CLI harness.
//!
//!     chess-tutor analyze "<fen>"          — JSON analysis report
//!     chess-tutor explain "<fen>"          — prose (stubbed until explainer lands)
//!     chess-tutor board "<fen>"            — render a FEN as a Unicode board
//!     chess-tutor play                     — interactive loop with live board
//!     chess-tutor review <pgn-file>        — walk a PGN, annotating every move
//!                                            (stubbed until PGN import lands)

mod board;

use std::io::{self, BufRead, Write};
use std::time::Instant;

use anyhow::{Context, Result};
use chess_tutor_core::{
    analysis::{see_on_square, AttackMap},
    analyze,
    explain::Explainer,
    game::{Game, GameStatus, PlayerKind, Side, TimeControl},
};
use clap::{Parser, Subcommand};
use shakmaty::{Board, Color, Piece, Role, Square};

use crate::board::{render as render_board, RenderOptions};

#[derive(Parser)]
#[command(name = "chess-tutor", version, about = "Deterministic chess analysis + explanation.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the analysis pipeline on a FEN and print the JSON report.
    Analyze { fen: String },
    /// Run the explainer on a FEN and print user-facing prose.
    Explain { fen: String },
    /// Play an interactive game over stdin. Type UCI moves (e.g. e2e4); type
    /// `undo`, `resign`, `fen`, or `quit`. Pass `--time` (seconds) and
    /// optionally `--increment` (seconds) for Fischer-clock play.
    Play {
        /// Initial clock per side, in seconds. Omit for untimed play.
        #[arg(long)]
        time: Option<u64>,
        /// Fischer increment in seconds. Requires --time. Defaults to 0.
        #[arg(long, default_value_t = 0)]
        increment: u64,
        /// Use plain ASCII pieces (K Q R B N P) instead of Unicode.
        #[arg(long)]
        ascii: bool,
        /// Flip the board to always show the current mover at the bottom.
        #[arg(long)]
        auto_flip: bool,
        /// Hide the live board and fall back to text-only status updates.
        #[arg(long)]
        no_board: bool,
    },
    /// Render a position's board in the terminal.
    Board {
        /// FEN. Omit to render the standard start position.
        #[arg(default_value = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1")]
        fen: String,
        /// Use plain ASCII pieces.
        #[arg(long)]
        ascii: bool,
        /// Render from Black's perspective.
        #[arg(long)]
        flip: bool,
    },
    /// Walk a PGN file and annotate every move. Stubbed until PGN import lands.
    Review { pgn: String },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Analyze { fen } => {
            let report = analyze(&fen).context("analysis failed")?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Explain { fen } => {
            let report = analyze(&fen).context("analysis failed")?;
            let phrases = Explainer::new().explain(&report);
            if phrases.is_empty() {
                println!("(no phrases yet — explainer templates land in Phase 1)");
            } else {
                for p in phrases {
                    println!("{}", p.text);
                }
            }
        }
        Command::Play {
            time,
            increment,
            ascii,
            auto_flip,
            no_board,
        } => play_loop(time, increment, ascii, auto_flip, !no_board)?,
        Command::Board { fen, ascii, flip } => {
            let out = render_board(
                &fen,
                &RenderOptions {
                    ascii,
                    flip,
                    highlight: None,
                },
            );
            print!("{out}");
        }
        Command::Review { pgn: _ } => {
            println!("(review is stubbed — PGN import lands in Phase 1)");
        }
    }
    Ok(())
}

fn play_loop(
    time_sec: Option<u64>,
    increment_sec: u64,
    ascii: bool,
    auto_flip: bool,
    show_board: bool,
) -> Result<()> {
    let mut game = Game::new_standard(PlayerKind::Human, PlayerKind::Human);
    if let Some(sec) = time_sec {
        game = game.with_time_control(TimeControl::fischer(sec * 1_000, increment_sec * 1_000));
    }

    let stdin = io::stdin();
    let mut out = io::stdout().lock();

    writeln!(
        out,
        "chess-tutor play — type moves as SAN (e4, Nf3, O-O) or UCI (e2e4, g1f3)."
    )?;
    writeln!(
        out,
        "commands: moves / hanging / attackers <sq|piece> / see <sq> / undo / resign / fen / flip / help / quit"
    )?;
    if game.has_time_control() {
        writeln!(
            out,
            "clock: {}s + {}s increment",
            time_sec.unwrap(),
            increment_sec
        )?;
    }

    let mut manual_flip = false;
    let mut turn_started = Instant::now();

    loop {
        let mover = game.side_to_move();

        if show_board {
            let flip = manual_flip || (auto_flip && mover == Side::Black);
            let highlight = last_move_squares(&game);
            writeln!(out)?;
            write!(
                out,
                "{}",
                render_board(
                    &game.fen(),
                    &RenderOptions {
                        ascii,
                        flip,
                        highlight,
                    },
                )
            )?;
        }

        if game.has_time_control() {
            writeln!(
                out,
                "{:?} to move. W {:.1}s / B {:.1}s",
                mover,
                game.remaining_ms(Side::White).unwrap() as f64 / 1000.0,
                game.remaining_ms(Side::Black).unwrap() as f64 / 1000.0,
            )?;
        } else {
            writeln!(out, "{:?} to move.", mover)?;
        }
        write!(out, "> ")?;
        out.flush()?;

        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break; // EOF
        }
        let elapsed_ms = turn_started.elapsed().as_millis() as u64;
        let cmd = line.trim();
        match cmd {
            "" => continue,
            "quit" | "exit" => break,
            "help" | "?" => {
                writeln!(
                    out,
                    "moves: SAN (e4, Nf3, O-O, Qxf7#) or UCI (e2e4, g1f3)."
                )?;
                writeln!(out, "commands:")?;
                writeln!(out, "  moves               list every legal move as SAN")?;
                writeln!(out, "  hanging             list pieces attacked more than defended")?;
                writeln!(out, "  attackers <sq>      show who attacks a square (e.g. `attackers e4`)")?;
                writeln!(out, "  attackers <piece>   show attackers on every piece of that letter")?;
                writeln!(out, "                      (K/Q/R/B/N/P = white, k/q/r/b/n/p = black)")?;
                writeln!(out, "  see <sq>            SEE (centipawns) for each side capturing on <sq>")?;
                writeln!(out, "  undo / resign / fen / flip / help / quit")?;
            }
            "moves" => {
                let sans = game.legal_moves_san();
                writeln!(out, "{} legal moves: {}", sans.len(), sans.join(" "))?;
            }
            "hanging" => {
                report_hanging(game.position(), &mut out)?;
            }
            "attackers" => {
                writeln!(out, "usage: attackers <square> | attackers <piece-letter>")?;
            }
            cmd if cmd.starts_with("attackers ") => {
                let arg = cmd.trim_start_matches("attackers ").trim();
                report_attackers(game.position(), arg, &mut out)?;
            }
            "see" => {
                writeln!(out, "usage: see <square>   e.g. `see e5`")?;
            }
            cmd if cmd.starts_with("see ") => {
                let arg = cmd.trim_start_matches("see ").trim();
                report_see(game.position(), arg, &mut out)?;
            }
            "flip" => {
                manual_flip = !manual_flip;
            }
            "fen" => {
                writeln!(out, "{}", game.fen())?;
            }
            "undo" => match game.undo() {
                Some(entry) => writeln!(out, "undid {}", entry.san)?,
                None => writeln!(out, "nothing to undo")?,
            },
            "resign" => {
                game.resign(mover);
                writeln!(out, "status: {:?}", game.status())?;
            }
            input => {
                let uci = match game.parse_move(input) {
                    Ok(uci) => uci,
                    Err(e) => {
                        writeln!(out, "rejected: {e}")?;
                        continue;
                    }
                };
                let result = if game.has_time_control() {
                    game.apply_timed(&uci, elapsed_ms)
                } else {
                    game.apply(&uci)
                };
                match result {
                    Ok(report) => {
                        writeln!(out, "played {} ({:?})", report.entry.san, report.class)?;
                        if !matches!(game.status(), GameStatus::Ongoing) {
                            // Show the final position before breaking.
                            if show_board {
                                let flip = manual_flip || (auto_flip && mover == Side::Black);
                                let highlight = last_move_squares(&game);
                                writeln!(out)?;
                                write!(
                                    out,
                                    "{}",
                                    render_board(
                                        &game.fen(),
                                        &RenderOptions {
                                            ascii,
                                            flip,
                                            highlight,
                                        },
                                    )
                                )?;
                            }
                            writeln!(out, "game over: {:?}", game.status())?;
                            break;
                        }
                        turn_started = Instant::now();
                    }
                    Err(e) => writeln!(out, "rejected: {e}")?,
                }
            }
        }
    }
    Ok(())
}

fn last_move_squares(game: &Game) -> Option<(String, String)> {
    let last = game.history().last()?;
    // UCI moves are 4 chars (e2e4) or 5 with promotion (e7e8q).
    let uci = &last.uci;
    if uci.len() < 4 {
        return None;
    }
    Some((uci[0..2].to_string(), uci[2..4].to_string()))
}

// ---------------------------------------------------------------------------
// Inspection helpers (`hanging`, `attackers`) — thin CLI wrappers over
// `chess_tutor_core::analysis::AttackMap`. The core does the actual work;
// these just format the output.
// ---------------------------------------------------------------------------

fn report_hanging(position: &shakmaty::Chess, out: &mut impl Write) -> io::Result<()> {
    use shakmaty::Position;
    let board = position.board();
    let am = AttackMap::from_position(position);

    let mut hits: Vec<Square> = Vec::new();
    for sq in Square::ALL {
        if am.is_hanging(board, sq) {
            hits.push(sq);
        }
    }

    if hits.is_empty() {
        writeln!(out, "no hanging pieces.")?;
        return Ok(());
    }

    writeln!(out, "hanging pieces ({}):", hits.len())?;
    for sq in hits {
        let piece = board.piece_at(sq).expect("is_hanging requires a piece");
        let own = am.count(sq, piece.color);
        let foe = am.count(sq, piece.color.other());
        writeln!(
            out,
            "  {} {} — {} attacker{} vs {} defender{}",
            square_name(sq),
            piece_letter(piece),
            foe,
            pluralise(foe),
            own,
            pluralise(own),
        )?;
    }
    Ok(())
}

fn report_see(
    position: &shakmaty::Chess,
    arg: &str,
    out: &mut impl Write,
) -> io::Result<()> {
    use shakmaty::Position;
    let Some(sq) = parse_square(arg) else {
        writeln!(out, "unrecognised square '{arg}'. Try e.g. `see e5`.")?;
        return Ok(());
    };

    let board = position.board();
    let occupant = board
        .piece_at(sq)
        .map(|p| format!(" ({})", piece_letter(p)))
        .unwrap_or_default();
    writeln!(out, "{}{}:", square_name(sq), occupant)?;

    if board.piece_at(sq).is_none() {
        writeln!(out, "  (empty — SEE is only meaningful on occupied squares)")?;
        return Ok(());
    }

    for side in [Color::White, Color::Black] {
        match see_on_square(position, sq, side) {
            Some(v) => writeln!(out, "  {:?}: {:+} cp", side, v)?,
            None => writeln!(out, "  {:?}: no attacker", side)?,
        }
    }
    Ok(())
}

fn report_attackers(
    position: &shakmaty::Chess,
    arg: &str,
    out: &mut impl Write,
) -> io::Result<()> {
    use shakmaty::Position;
    let board = position.board();
    let am = AttackMap::from_position(position);

    let targets = match parse_inspect_target(arg, board) {
        Some(sqs) if !sqs.is_empty() => sqs,
        Some(_) => {
            writeln!(out, "no pieces of that type on the board.")?;
            return Ok(());
        }
        None => {
            writeln!(
                out,
                "unrecognised target '{arg}'. Use a square (e.g. e4) or a piece letter (K Q R B N P, lowercase for black)."
            )?;
            return Ok(());
        }
    };

    for sq in targets {
        describe_square(&am, board, sq, out)?;
    }
    Ok(())
}

fn parse_inspect_target(arg: &str, board: &Board) -> Option<Vec<Square>> {
    let arg = arg.trim();

    // Try as a square name.
    if let Some(sq) = parse_square(arg) {
        return Some(vec![sq]);
    }

    // Try as a single piece letter (FEN convention).
    if arg.chars().count() == 1 {
        let ch = arg.chars().next().unwrap();
        let (color, role) = match ch {
            'K' => (Color::White, Role::King),
            'Q' => (Color::White, Role::Queen),
            'R' => (Color::White, Role::Rook),
            'B' => (Color::White, Role::Bishop),
            'N' => (Color::White, Role::Knight),
            'P' => (Color::White, Role::Pawn),
            'k' => (Color::Black, Role::King),
            'q' => (Color::Black, Role::Queen),
            'r' => (Color::Black, Role::Rook),
            'b' => (Color::Black, Role::Bishop),
            'n' => (Color::Black, Role::Knight),
            'p' => (Color::Black, Role::Pawn),
            _ => return None,
        };
        let needle = Piece { color, role };
        let squares = Square::ALL
            .into_iter()
            .filter(|sq| board.piece_at(*sq) == Some(needle))
            .collect();
        return Some(squares);
    }

    None
}

fn parse_square(s: &str) -> Option<Square> {
    if s.len() != 2 {
        return None;
    }
    let mut chars = s.chars();
    let file_ch = chars.next()?;
    let rank_ch = chars.next()?;
    if !('a'..='h').contains(&file_ch) || !('1'..='8').contains(&rank_ch) {
        return None;
    }
    let file = (file_ch as u8 - b'a') as u32;
    let rank = (rank_ch as u8 - b'1') as u32;
    Some(Square::new(rank * 8 + file))
}

fn describe_square(
    am: &AttackMap,
    board: &Board,
    sq: Square,
    out: &mut impl Write,
) -> io::Result<()> {
    let occupant = board
        .piece_at(sq)
        .map(|p| format!(" ({})", piece_letter(p)))
        .unwrap_or_default();

    let white = am.attackers(sq, Color::White);
    let black = am.attackers(sq, Color::Black);

    writeln!(out, "{}{}:", square_name(sq), occupant)?;
    writeln!(out, "  White: {}", describe_sources(board, white))?;
    writeln!(out, "  Black: {}", describe_sources(board, black))?;
    Ok(())
}

fn describe_sources(board: &Board, bb: shakmaty::Bitboard) -> String {
    if bb.is_empty() {
        return "—".to_string();
    }
    bb.into_iter()
        .map(|sq| match board.piece_at(sq) {
            Some(p) if p.role == Role::Pawn => square_name(sq),
            Some(p) => format!("{}{}", role_letter(p.role), square_name(sq)),
            None => square_name(sq),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn square_name(sq: Square) -> String {
    let file = (u32::from(sq) % 8) as u8 + b'a';
    let rank = (u32::from(sq) / 8) as u8 + b'1';
    format!("{}{}", file as char, rank as char)
}

fn piece_letter(p: Piece) -> char {
    let ch = role_letter(p.role);
    if p.color == Color::White {
        ch
    } else {
        ch.to_ascii_lowercase()
    }
}

fn role_letter(r: Role) -> char {
    match r {
        Role::Pawn => 'P',
        Role::Knight => 'N',
        Role::Bishop => 'B',
        Role::Rook => 'R',
        Role::Queen => 'Q',
        Role::King => 'K',
    }
}

fn pluralise(n: u8) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}
