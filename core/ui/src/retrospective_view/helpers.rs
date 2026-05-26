//! Shared formatting / classification helpers for the retrospective
//! card builders: piece names, articles, score/delta formatting, verdict
//! labels, surprise notes, and the post-user-move scratch position.

use chess_tutor_engine::analysis::{
    MoveAnalysis, MoveVerdict, SurpriseKind,
};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Color, PieceType, Value};

use crate::view::Sentiment;

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

pub(super) fn post_user_move_position(pre: &Position, user: &MoveAnalysis) -> Position {
    let mut p = pre.clone();
    if let Some(&mv) = user.pv.first() {
        p.do_move(mv);
    }
    p
}

/// Classical point-value net across realized captures from
/// `root_stm`'s POV. Positive = our side captured more by points
/// (P:1, N:3, B:3, R:5, Q:9); negative = opponent did; zero = even
/// (the B↔N case the cp-based net would call "lost material").
pub(super) fn realized_point_net(
    events: &[&chess_tutor_engine::analysis::CaptureEvent],
    root_stm: Color,
) -> i32 {
    let mut net = 0i32;
    for ev in events {
        let pts = ev.captured_piece.classical_points() as i32;
        if ev.captor == root_stm {
            net += pts;
        } else {
            net -= pts;
        }
    }
    net
}

/// Build a teaching note for a point-even trade whose engine cp net
/// leans meaningfully in one direction. Returns `None` when both
/// mg and eg leans are tight (≤ 30 cp), in which case the trade is
/// genuinely equal and there's nothing to teach.
///
/// The note's job is to give the student a concrete fact about the
/// position — "the engine values your bishop higher than their
/// knight in this position" — without framing it as a verdict on
/// the move. A 50 cp lean is small enough that the move can still
/// be best for other reasons; the student should understand it as
/// information, not a critique.
pub(super) fn phase_dependent_trade_note(
    events: &[&chess_tutor_engine::analysis::CaptureEvent],
    root_stm: Color,
) -> Option<String> {
    const PHASE_NOTE_THRESHOLD_CP: i32 = 30;
    let (net_mg, net_eg) = events.iter().fold((0i32, 0i32), |(mg, eg), ev| {
        let sign = if ev.captor == root_stm { 1 } else { -1 };
        (mg + sign * ev.value_mg, eg + sign * ev.value_eg)
    });
    let mg_abs = net_mg.abs();
    let eg_abs = net_eg.abs();
    if mg_abs < PHASE_NOTE_THRESHOLD_CP && eg_abs < PHASE_NOTE_THRESHOLD_CP {
        return None;
    }
    // Identify which side the lean favors. The trade is "even by
    // points" but cp may favor either side; positive cp = us, negative
    // = opponent. We pick the dominant phase as the headline and call
    // out the other for contrast if it disagrees in direction.
    let dominant_cp = if eg_abs > mg_abs { net_eg } else { net_mg };
    let dominant_phase = if eg_abs > mg_abs { "endgame" } else { "middlegame" };
    let other_phase = if eg_abs > mg_abs { "middlegame" } else { "endgame" };
    let other_cp = if eg_abs > mg_abs { net_mg } else { net_eg };

    let lean_text = format!(
        "{:+.2} pawns at {} phase",
        dominant_cp as f32 / 100.0,
        dominant_phase
    );
    let favor_us = dominant_cp > 0;
    let lead = if favor_us {
        format!(
            "Even by points, but the engine reads this slightly in your favor — {}.",
            lean_text
        )
    } else {
        format!(
            "Even by points, but the engine reads this slightly in your opponent's favor — {}.",
            lean_text
        )
    };
    // If mg and eg agree in direction, the lean is consistent across
    // the game. If they disagree, the trade is phase-dependent and
    // that's the more interesting story.
    let phase_clause = if dominant_cp.signum() == other_cp.signum() || other_cp == 0 {
        format!(
            " The {} valuation is similar ({:+.2} pawns), so the imbalance \
             holds across the game.",
            other_phase,
            other_cp as f32 / 100.0
        )
    } else {
        format!(
            " In the {} the trade reads {:+.2} pawns — phase-dependent: the \
             engine values these pieces differently depending on how much \
             material remains on the board.",
            other_phase,
            other_cp as f32 / 100.0
        )
    };
    Some(format!("{}{}", lead, phase_clause))
}

pub(super) fn piece_name(pt: PieceType) -> &'static str {
    match pt {
        PieceType::Pawn => "pawn",
        PieceType::Knight => "knight",
        PieceType::Bishop => "bishop",
        PieceType::Rook => "rook",
        PieceType::Queen => "queen",
        PieceType::King => "king",
    }
}

pub(super) fn article(name: &str) -> String {
    let c = name.chars().next().unwrap_or('x').to_ascii_lowercase();
    if matches!(c, 'a' | 'e' | 'i' | 'o' | 'u') {
        format!("an {}", name)
    } else {
        format!("a {}", name)
    }
}

pub(super) fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

pub(super) fn join_with_and(parts: &[String]) -> String {
    match parts.len() {
        0 => String::new(),
        1 => parts[0].clone(),
        2 => format!("{} and {}", parts[0], parts[1]),
        _ => {
            let (last, lead) = parts.split_last().unwrap();
            format!("{}, and {}", lead.join(", "), last)
        }
    }
}

pub(super) fn verdict_label(v: MoveVerdict) -> &'static str {
    match v {
        MoveVerdict::Best => "Best",
        MoveVerdict::Good => "Good",
        MoveVerdict::Inaccuracy => "Inaccuracy",
        MoveVerdict::Mistake => "Mistake",
        MoveVerdict::Blunder => "Blunder",
        MoveVerdict::BestAvailable => "Best available",
    }
}

pub(super) fn verdict_sentiment(v: MoveVerdict) -> Sentiment {
    match v {
        MoveVerdict::Best | MoveVerdict::Good => Sentiment::Positive,
        MoveVerdict::Inaccuracy => Sentiment::Mixed,
        MoveVerdict::Mistake | MoveVerdict::Blunder => Sentiment::Negative,
        MoveVerdict::BestAvailable => Sentiment::Neutral,
    }
}

pub(super) fn sharp_or_verdict_annotation(v: MoveVerdict, is_sharp: bool) -> &'static str {
    if is_sharp {
        return "!";
    }
    match v {
        MoveVerdict::Blunder => "??",
        MoveVerdict::Mistake => "?",
        _ => "",
    }
}

pub(super) fn format_score_pawns(score: Value) -> String {
    let abs = score.0.abs();
    let mate_threshold = Value::MATE.0 - Value::MAX_PLY;
    if abs >= mate_threshold {
        let plies = Value::MATE.0 - abs;
        let moves = (plies + 1) / 2;
        if score.0 >= 0 {
            format!("#{}", moves)
        } else {
            format!("-#{}", moves)
        }
    } else {
        format!("{:+.2}", score.0 as f32 / 100.0)
    }
}

pub(super) fn format_delta_pawns(delta_cp: i32) -> String {
    format!("{:+.2}", delta_cp as f32 / 100.0)
}

pub(super) fn surprise_note(verdict: MoveVerdict, surprise: Option<SurpriseKind>) -> Option<String> {
    match (verdict, surprise) {
        (MoveVerdict::Mistake | MoveVerdict::Blunder, Some(SurpriseKind::LooksGoodButBad)) => {
            Some("This looked natural but the deeper line gives back material.".to_string())
        }
        (MoveVerdict::Best | MoveVerdict::Good, Some(SurpriseKind::LooksBadButGood)) => {
            Some("This looked risky on the surface — the longer line pays off.".to_string())
        }
        _ => None,
    }
}

