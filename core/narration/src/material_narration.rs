//! Material-capture narration — turn a [`MaterialOutcome`] into
//! *"Best line: Nxd5 exd5 — you lose a pawn (knight for bishop +
//! pawn)."* prose.
//!
//! Labeled "Best line" — **not** "Forced sequence" — because the
//! PV is the engine's principal variation under optimal play from
//! both sides, not a line where every move is compelled by checks
//! or only-legal-recaptures. The student often has real
//! alternatives at each ply.

use std::io;

use chess_tutor_engine::analysis::{MaterialOutcome, MoveAnalysis};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Color, PieceType};

use crate::util::{piece_name, pv_to_san_through};

pub(crate) fn render_material_sequence(
    out: &mut dyn io::Write,
    pre_move_pos: &Position,
    user: &MoveAnalysis,
    outcome: &MaterialOutcome,
    root_stm: Color,
) -> io::Result<()> {
    // Render the PV as SAN up through last_ply. The engine already
    // settled its trace at that ply, so captures past it are search
    // noise rather than the teaching story.
    let sequence = pv_to_san_through(pre_move_pos, &user.pv, outcome.last_ply);
    let joined = sequence.join(" ");
    let story = phrase_material_outcome(outcome, root_stm);
    writeln!(
        out,
        "                Best line: {joined}{}",
        if story.is_empty() {
            ".".to_string()
        } else {
            format!(" — {story}.")
        },
    )?;
    Ok(())
}

/// Turn a [`MaterialOutcome`] into a short English phrase
/// describing what happened, from `root_stm`'s POV. Returns an
/// empty string when the captures balance to roughly nothing — in
/// that case the SAN sequence itself is the story.
fn phrase_material_outcome(outcome: &MaterialOutcome, root_stm: Color) -> String {
    // Use classical piece values (pawn=1, minor=3, rook=5,
    // queen=9) for the net count — this matches how a human player
    // thinks about material balance, not the engine's internal
    // weighting.
    let mut by_us: Vec<PieceType> = Vec::new();
    let mut by_them: Vec<PieceType> = Vec::new();
    for ev in &outcome.events {
        if ev.captor == root_stm {
            by_us.push(ev.captured_piece);
        } else {
            by_them.push(ev.captured_piece);
        }
    }
    let us_points: i32 = by_us.iter().map(|p| classical_value(*p)).sum();
    let them_points: i32 = by_them.iter().map(|p| classical_value(*p)).sum();
    let net = us_points - them_points;

    match net.signum() {
        0 => {
            if outcome.events.is_empty() {
                String::new()
            } else {
                "even trade".to_string()
            }
        }
        1 => format!("you win {}", format_classical_gain(net, &by_us, &by_them)),
        _ => format!("you lose {}", format_classical_gain(-net, &by_them, &by_us)),
    }
}

/// Classical point value of a piece — what a human reads when
/// assessing a material imbalance. Pawn=1, minor=3, rook=5,
/// queen=9. The king is uncapturable and returns 0.
fn classical_value(pt: PieceType) -> i32 {
    match pt {
        PieceType::Pawn => 1,
        PieceType::Knight | PieceType::Bishop => 3,
        PieceType::Rook => 5,
        PieceType::Queen => 9,
        PieceType::King => 0,
    }
}

/// Format the gain/loss side's summary: `"a pawn"`, `"two points
/// (knight for bishop+pawn)"`, etc. Inputs are already in
/// "magnitude" form — a positive `net` where `winner_captured` is
/// the bigger pile.
fn format_classical_gain(
    net: i32,
    winner_captured: &[PieceType],
    loser_captured: &[PieceType],
) -> String {
    let headline = match net {
        1 => "a pawn".to_string(),
        n => format!("{n} points"),
    };
    if loser_captured.is_empty() {
        headline
    } else {
        let win_list = list_pieces(winner_captured);
        let lose_list = list_pieces(loser_captured);
        format!("{headline} ({win_list} for {lose_list})")
    }
}

/// Turn a list of captured piece types into a comma-separated
/// phrase: `[Knight, Pawn]` → `"knight + pawn"`. Empty list →
/// `"(nothing)"`.
fn list_pieces(pieces: &[PieceType]) -> String {
    if pieces.is_empty() {
        return "(nothing)".to_string();
    }
    let names: Vec<&'static str> = pieces.iter().map(|p| piece_name(*p)).collect();
    names.join(" + ")
}
