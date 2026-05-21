//! Threats narration — hanging pieces, SEE-losing exchanges, and
//! Stockfish-pattern pressure. The engine returns structured data
//! (`ThreatsOutcome`); this module turns it into prose.

use std::io;

use chess_tutor_engine::analysis::{HangingPiece, PressureKind, PressuredPiece, ThreatsOutcome};
use chess_tutor_engine::types::Square;

use crate::util::{format_attackers, piece_name};

/// Render threats narration lines given the structured
/// [`ThreatsOutcome`]. Writes up to four possible lines — hanging
/// pieces on ours/theirs, SEE-losing pieces on ours/theirs —
/// whichever have positive deltas. Returns `true` if any line was
/// written, so the caller can suppress the generic `threats` entry
/// in the secondary-terms list.
pub(crate) fn render_threats(
    out: &mut dyn io::Write,
    outcome: &ThreatsOutcome,
) -> io::Result<bool> {
    let mut wrote = false;

    // Strictly-hanging threats first — they're the most visceral.
    if outcome.ours_hanging_delta > 0 && !outcome.ours_hanging.is_empty() {
        writeln!(
            out,
            "                {}",
            phrase_our_hanging(&outcome.ours_hanging),
        )?;
        wrote = true;
    }
    // Only narrate "you expose the opponent's …" when the threat is
    // *guaranteed* — survives every legal opponent response. The
    // raw theirs_hanging list misfires on pieces the opponent can
    // defend on their next turn (1.Nf3 attacking e5 is the canonical
    // case: ...Nc6 defends, so we should not tell the student they
    // can win the pawn).
    if outcome.theirs_hanging_delta > 0 && !outcome.theirs_hanging_guaranteed.is_empty() {
        writeln!(
            out,
            "                {}",
            phrase_their_hanging(&outcome.theirs_hanging_guaranteed),
        )?;
        wrote = true;
    }

    // SEE-losing (defended but unequal exchange).
    if outcome.ours_see_losing_delta > 0 && !outcome.ours_see_losing.is_empty() {
        writeln!(
            out,
            "                {}",
            phrase_our_see_losing(&outcome.ours_see_losing),
        )?;
        wrote = true;
    }
    // Same guarantee rule for "their piece loses to a trade".
    if outcome.theirs_see_losing_delta > 0 && !outcome.theirs_see_losing_guaranteed.is_empty() {
        writeln!(
            out,
            "                {}",
            phrase_their_see_losing(&outcome.theirs_see_losing_guaranteed),
        )?;
        wrote = true;
    }

    // Pressured (Stockfish positional threat patterns). Lives
    // below hanging/SEE-losing because those are the dominant
    // stories; a pressured piece is less urgent but still worth
    // calling out.
    //
    // De-dup against the lists already rendered above: if a target
    // appears as hanging or SEE-losing, suppress its pressure
    // entry so we don't say the same thing twice in different
    // words.
    let already_rendered_ours: Vec<Square> = outcome
        .ours_hanging
        .iter()
        .chain(outcome.ours_see_losing.iter())
        .map(|h| h.location.square)
        .collect();
    let already_rendered_theirs: Vec<Square> = outcome
        .theirs_hanging
        .iter()
        .chain(outcome.theirs_see_losing.iter())
        .map(|h| h.location.square)
        .collect();
    let ours_pressured_filtered: Vec<PressuredPiece> = outcome
        .ours_pressured
        .iter()
        .filter(|p| !already_rendered_ours.contains(&p.location.square))
        .cloned()
        .collect();
    let theirs_pressured_filtered: Vec<PressuredPiece> = outcome
        .theirs_pressured
        .iter()
        .filter(|p| !already_rendered_theirs.contains(&p.location.square))
        .cloned()
        .collect();
    if outcome.ours_pressured_delta > 0 && !ours_pressured_filtered.is_empty() {
        writeln!(
            out,
            "                {}",
            phrase_our_pressured(&ours_pressured_filtered),
        )?;
        wrote = true;
    }
    if outcome.theirs_pressured_delta > 0 && !theirs_pressured_filtered.is_empty() {
        writeln!(
            out,
            "                {}",
            phrase_their_pressured(&theirs_pressured_filtered),
        )?;
        wrote = true;
    }

    Ok(wrote)
}

/// One-word passive verb describing how the attacker(s) pressure
/// the target — chosen per pattern kind so the student learns to
/// tell a pawn kick from a minor-on-major jab from a
/// rook-on-queen stare.
fn pressure_verb_passive(kind: PressureKind) -> &'static str {
    match kind {
        PressureKind::MinorOnMajor => "harried",
        PressureKind::RookOnQueen => "pressured",
        PressureKind::SafePawnThreat => "kicked",
    }
}

fn phrase_our_pressured(pressured: &[PressuredPiece]) -> String {
    match pressured {
        [] => String::new(),
        [single] => format!(
            "Your {} on {} is {} ({}).",
            piece_name(single.location.piece),
            single.location.square.to_algebraic(),
            pressure_verb_passive(single.kind),
            format_attackers(&single.attackers),
        ),
        many => {
            let list = many
                .iter()
                .map(|p| {
                    format!(
                        "{} on {} {} ({})",
                        piece_name(p.location.piece),
                        p.location.square.to_algebraic(),
                        pressure_verb_passive(p.kind),
                        format_attackers(&p.attackers),
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            format!("Your pieces are under pressure — {list}.")
        }
    }
}

fn phrase_their_pressured(pressured: &[PressuredPiece]) -> String {
    match pressured {
        [] => String::new(),
        [single] => format!(
            "The opponent's {} on {} is {} ({}).",
            piece_name(single.location.piece),
            single.location.square.to_algebraic(),
            pressure_verb_passive(single.kind),
            format_attackers(&single.attackers),
        ),
        many => {
            let list = many
                .iter()
                .map(|p| {
                    format!(
                        "{} on {} {} ({})",
                        piece_name(p.location.piece),
                        p.location.square.to_algebraic(),
                        pressure_verb_passive(p.kind),
                        format_attackers(&p.attackers),
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            format!("Opponent pieces are under pressure — {list}.")
        }
    }
}

fn phrase_our_see_losing(pieces: &[HangingPiece]) -> String {
    match pieces {
        [] => String::new(),
        [single] => format!(
            "Your {} on {} is defended but loses material to the exchange ({}).",
            piece_name(single.location.piece),
            single.location.square.to_algebraic(),
            format_attackers(&single.attackers),
        ),
        many => {
            let list = many
                .iter()
                .map(|p| {
                    format!(
                        "{} on {} ({})",
                        piece_name(p.location.piece),
                        p.location.square.to_algebraic(),
                        format_attackers(&p.attackers),
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            format!("Your pieces lose material to exchanges — {list}.")
        }
    }
}

fn phrase_their_see_losing(pieces: &[HangingPiece]) -> String {
    match pieces {
        [] => String::new(),
        [single] => format!(
            "The opponent's {} on {} loses material to the exchange ({}).",
            piece_name(single.location.piece),
            single.location.square.to_algebraic(),
            format_attackers(&single.attackers),
        ),
        many => {
            let list = many
                .iter()
                .map(|p| {
                    format!(
                        "{} on {} ({})",
                        piece_name(p.location.piece),
                        p.location.square.to_algebraic(),
                        format_attackers(&p.attackers),
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            format!("Opponent pieces lose material to exchanges — {list}.")
        }
    }
}

fn phrase_our_hanging(hanging: &[HangingPiece]) -> String {
    match hanging {
        [] => String::new(),
        [single] => format!(
            "You leave a hanging {} on {} ({}).",
            piece_name(single.location.piece),
            single.location.square.to_algebraic(),
            format_attackers(&single.attackers),
        ),
        many => {
            let list = many
                .iter()
                .map(|p| {
                    format!(
                        "{} on {} ({})",
                        piece_name(p.location.piece),
                        p.location.square.to_algebraic(),
                        format_attackers(&p.attackers),
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            format!("You leave hanging pieces — {list}.")
        }
    }
}

fn phrase_their_hanging(hanging: &[HangingPiece]) -> String {
    match hanging {
        [] => String::new(),
        [single] => format!(
            "You expose the opponent's {} on {} ({}).",
            piece_name(single.location.piece),
            single.location.square.to_algebraic(),
            format_attackers(&single.attackers),
        ),
        many => {
            let list = many
                .iter()
                .map(|p| {
                    format!(
                        "{} on {} ({})",
                        piece_name(p.location.piece),
                        p.location.square.to_algebraic(),
                        format_attackers(&p.attackers),
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            format!("You expose the opponent's pieces — {list}.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::analysis::PieceLocation;
    use chess_tutor_engine::types::PieceType;

    fn pl(square: Square, piece: PieceType) -> PieceLocation {
        PieceLocation { square, piece }
    }

    // ---- phrase_our_hanging / phrase_their_hanging -----------------

    #[test]
    fn phrase_our_hanging_single_with_attacker() {
        let entry = HangingPiece {
            location: pl(Square::D2, PieceType::Knight),
            attackers: vec![pl(Square::E3, PieceType::Pawn)],
        };
        assert_eq!(
            phrase_our_hanging(&[entry]),
            "You leave a hanging knight on d2 (attacked by the e3 pawn)."
        );
    }

    #[test]
    fn phrase_our_hanging_multiple() {
        let entries = vec![
            HangingPiece {
                location: pl(Square::D2, PieceType::Knight),
                attackers: vec![pl(Square::E3, PieceType::Pawn)],
            },
            HangingPiece {
                location: pl(Square::F5, PieceType::Bishop),
                attackers: vec![pl(Square::G6, PieceType::Pawn)],
            },
        ];
        let out = phrase_our_hanging(&entries);
        assert!(out.contains("knight on d2 (attacked by the e3 pawn)"));
        assert!(out.contains("bishop on f5 (attacked by the g6 pawn)"));
        assert!(out.starts_with("You leave hanging pieces —"));
    }

    #[test]
    fn phrase_their_hanging_single_with_attacker() {
        let entry = HangingPiece {
            location: pl(Square::D7, PieceType::Knight),
            attackers: vec![pl(Square::E6, PieceType::Pawn)],
        };
        assert_eq!(
            phrase_their_hanging(&[entry]),
            "You expose the opponent's knight on d7 (attacked by the e6 pawn)."
        );
    }

    // ---- phrase_our_see_losing / phrase_their_see_losing -----------

    #[test]
    fn phrase_our_see_losing_single() {
        let entry = HangingPiece {
            location: pl(Square::E5, PieceType::Knight),
            attackers: vec![
                pl(Square::D6, PieceType::Pawn),
                pl(Square::G4, PieceType::Knight),
            ],
        };
        let out = phrase_our_see_losing(&[entry]);
        assert!(out.contains("knight on e5"));
        assert!(out.contains("defended but loses material"));
        assert!(out.contains("d6 pawn and g4 knight"));
    }

    #[test]
    fn phrase_their_see_losing_single() {
        let entry = HangingPiece {
            location: pl(Square::D5, PieceType::Knight),
            attackers: vec![pl(Square::E4, PieceType::Pawn)],
        };
        let out = phrase_their_see_losing(&[entry]);
        assert!(out.contains("opponent's knight on d5"));
        assert!(out.contains("loses material"));
        assert!(out.contains("e4 pawn"));
    }

    #[test]
    fn phrase_our_see_losing_multiple_uses_semicolon_list() {
        let entries = vec![
            HangingPiece {
                location: pl(Square::E5, PieceType::Knight),
                attackers: vec![pl(Square::D6, PieceType::Pawn)],
            },
            HangingPiece {
                location: pl(Square::B3, PieceType::Bishop),
                attackers: vec![pl(Square::C4, PieceType::Pawn)],
            },
        ];
        let out = phrase_our_see_losing(&entries);
        assert!(out.starts_with("Your pieces lose material"));
        assert!(out.contains("knight on e5"));
        assert!(out.contains("bishop on b3"));
    }

    // ---- pressure phrasing -----------------------------------------

    #[test]
    fn pressure_verb_passive_covers_every_kind() {
        assert_eq!(pressure_verb_passive(PressureKind::MinorOnMajor), "harried");
        assert_eq!(
            pressure_verb_passive(PressureKind::RookOnQueen),
            "pressured"
        );
        assert_eq!(
            pressure_verb_passive(PressureKind::SafePawnThreat),
            "kicked"
        );
    }

    #[test]
    fn phrase_our_pressured_single_minor_on_major() {
        let entry = PressuredPiece {
            location: pl(Square::A1, PieceType::Rook),
            attackers: vec![pl(Square::C2, PieceType::Knight)],
            kind: PressureKind::MinorOnMajor,
        };
        assert_eq!(
            phrase_our_pressured(&[entry]),
            "Your rook on a1 is harried (attacked by the c2 knight)."
        );
    }

    #[test]
    fn phrase_our_pressured_single_safe_pawn() {
        let entry = PressuredPiece {
            location: pl(Square::F6, PieceType::Knight),
            attackers: vec![pl(Square::E5, PieceType::Pawn)],
            kind: PressureKind::SafePawnThreat,
        };
        assert_eq!(
            phrase_our_pressured(&[entry]),
            "Your knight on f6 is kicked (attacked by the e5 pawn)."
        );
    }

    #[test]
    fn phrase_their_pressured_single_rook_on_queen() {
        let entry = PressuredPiece {
            location: pl(Square::D8, PieceType::Queen),
            attackers: vec![pl(Square::D1, PieceType::Rook)],
            kind: PressureKind::RookOnQueen,
        };
        assert_eq!(
            phrase_their_pressured(&[entry]),
            "The opponent's queen on d8 is pressured (attacked by the d1 rook)."
        );
    }

    #[test]
    fn phrase_our_pressured_multiple_uses_semicolon_list() {
        let entries = vec![
            PressuredPiece {
                location: pl(Square::A1, PieceType::Rook),
                attackers: vec![pl(Square::C2, PieceType::Knight)],
                kind: PressureKind::MinorOnMajor,
            },
            PressuredPiece {
                location: pl(Square::F6, PieceType::Bishop),
                attackers: vec![pl(Square::E5, PieceType::Pawn)],
                kind: PressureKind::SafePawnThreat,
            },
        ];
        let out = phrase_our_pressured(&entries);
        assert!(
            out.starts_with("Your pieces are under pressure —"),
            "got: {out}"
        );
        assert!(out.contains("rook on a1 harried (attacked by the c2 knight)"));
        assert!(out.contains("bishop on f6 kicked (attacked by the e5 pawn)"));
    }

    #[test]
    fn phrase_their_pressured_multiple_uses_semicolon_list() {
        let entries = vec![
            PressuredPiece {
                location: pl(Square::D8, PieceType::Queen),
                attackers: vec![pl(Square::D1, PieceType::Rook)],
                kind: PressureKind::RookOnQueen,
            },
            PressuredPiece {
                location: pl(Square::A8, PieceType::Rook),
                attackers: vec![pl(Square::C7, PieceType::Knight)],
                kind: PressureKind::MinorOnMajor,
            },
        ];
        let out = phrase_their_pressured(&entries);
        assert!(
            out.starts_with("Opponent pieces are under pressure —"),
            "got: {out}"
        );
        assert!(out.contains("queen on d8 pressured (attacked by the d1 rook)"));
        assert!(out.contains("rook on a8 harried (attacked by the c7 knight)"));
    }
}
