//! One-line agent-friendly descriptions for every [`TermId`].
//!
//! The engine ships two label flavours: `TermId::label()` is the
//! kebab-case identifier the eval table prints (`king.danger`), and
//! `TermId::pretty_label()` is the plain-English version used by
//! retrospective prose (`king safety`). Neither carries the
//! single most-important piece of context for an agent reading the
//! eval trace: **does a positive number mean WE are doing something,
//! or THEY are?**
//!
//! Every `pub fn description` line here resolves that. The convention:
//! every gloss reads as if `+net` were positive — telling the reader
//! what a positive number indicates and whose direction it favours.
//! For terms like `king.danger` where the engine's convention is
//! "white's `danger` field holds the pressure WE are applying to the
//! enemy king", the gloss makes that explicit so an agent doesn't
//! reverse-engineer it from the row sign.
//!
//! Stays close to PLAN-cli.md's worked example: term names in the
//! eval table get annotated with these one-liners; `chess-tutor eval
//! --glossary` dumps the table standalone for browsing.

use chess_tutor_engine::analysis::TermId;

/// One-line gloss for an evaluation term. See module `//!` for the
/// directional convention.
pub fn description(id: TermId) -> &'static str {
    match id {
        // -- material / imbalance / initiative ------------------------
        TermId::MaterialPieceValue => {
            "Raw piece-count material. Positive = WE have more material (queens, rooks, minors, pawns)."
        }
        TermId::MaterialPsqPositional => {
            "Piece-square-table bonus: are our pieces on their textbook-good squares? Positive = WE are."
        }
        TermId::Imbalance => {
            "Bonus for owning a pair (two bishops) or having favourable piece-vs-piece trade-offs. Positive = WE benefit."
        }
        TermId::Initiative => {
            "Late-game tempo bonus that tilts who-moves-first into the score (SF11 keeps the eval honest at depth)."
        }

        // -- space ---------------------------------------------------
        TermId::Space => {
            "Squares we control on our half of the board, weighted by how many minor pieces are still on. Positive = WE control more."
        }

        // -- king safety ---------------------------------------------
        TermId::KingPawnShield => {
            "Friendly pawns directly in front of our castled king. Positive = OUR shield is stronger than theirs."
        }
        TermId::KingPawnStorm => {
            "Enemy pawn advances aimed at our king's shelter. Positive = THEIR storm against us is less severe than ours against them."
        }
        TermId::KingPawnDistance => {
            "How far our king is from its nearest pawn (matters in the endgame). Positive = OUR king is closer."
        }
        TermId::KingDanger => {
            "Pressure WE apply to the enemy king (attacker count near king, weak shield squares, no defenders). Positive = WE are attacking more than they are."
        }
        TermId::KingPawnlessFlank => {
            "Penalty when our king's flank has no pawns at all. Positive = THEIR king is more exposed than ours."
        }
        TermId::KingFlankAttacks => {
            "Number of OUR attacks on squares near the enemy king's flank. Positive = WE are attacking near their king."
        }

        // -- passed pawns --------------------------------------------
        TermId::PassedRankBonus => {
            "Bonus per OUR passed pawn, scaling with how far advanced it is. Positive = OUR passers are more dangerous."
        }
        TermId::PassedKingProximity => {
            "Endgame term: how close each king is to OUR passers vs theirs. Positive = WE win the king race."
        }
        TermId::PassedFreeAdvance => {
            "Bonus when nothing blocks or attacks the square in front of OUR passer. Positive = OUR passers have clearer paths."
        }
        TermId::PassedStopperPenalty => {
            "Penalty when an enemy piece sits on or attacks OUR passer's path. Positive = THEIR passers are blockaded more than ours."
        }

        // -- pawn structure ------------------------------------------
        TermId::PawnsConnected => {
            "Bonus for OUR pawns defending each other in chains. Positive = OUR chains are stronger."
        }
        TermId::PawnsIsolated => {
            "Penalty for OUR pawns with no friendly pawn on adjacent files. Positive = THEY have more isolated pawns than we do."
        }
        TermId::PawnsBackward => {
            "Penalty for OUR pawns that can't safely advance and aren't defended by another pawn. Positive = THEY have more backward pawns."
        }
        TermId::PawnsDoubled => {
            "Penalty for OUR pawns on the same file. Positive = THEY have more doubled pawns."
        }
        TermId::PawnsWeakUnopposed => {
            "Extra penalty for OUR weak pawns on half-open files (no enemy pawn opposing them). Positive = THEIRS are weaker."
        }
        TermId::PawnsWeakLever => {
            "Penalty for OUR pawns attacked by enemy pawns. Positive = THEIR pawns are under more pawn pressure than ours."
        }

        // -- per-piece positional ------------------------------------
        TermId::PiecesOutposts => {
            "Bonus for OUR minor pieces sitting on a defended outpost (safe square no enemy pawn can chase). Positive = WE have more outposted minors."
        }
        TermId::PiecesReachableOutposts => {
            "Bonus for OUR minor pieces that could reach an outpost next move. Positive = WE have better outpost potential."
        }
        TermId::PiecesMinorBehindPawn => {
            "Bonus for OUR minors sheltering behind a friendly pawn. Positive = WE have safer minors."
        }
        TermId::PiecesKingProtector => {
            "Per-piece bonus scaling with how close it sits to OUR king. Positive = OUR king has more defenders nearby."
        }
        TermId::PiecesBishopPawns => {
            "Penalty when OUR bishop is hemmed in by friendly pawns of its own color. Positive = THEIR bishop is more hemmed in than ours."
        }
        TermId::PiecesLongDiagonalBishop => {
            "Bonus when OUR bishop commands the long diagonal uncontested. Positive = WE control more long diagonals."
        }
        TermId::PiecesRookOnQueenFile => {
            "Bonus for OUR rook on the enemy queen's file. Positive = WE have rook-vs-queen file pressure."
        }
        TermId::PiecesRookOnOpenFile => {
            "Bonus for OUR rooks on fully open files. Positive = WE control more open files."
        }
        TermId::PiecesRookOnSemiopenFile => {
            "Bonus for OUR rooks on semi-open files (own pawn gone, theirs still there). Positive = WE pressure more weak pawns."
        }
        TermId::PiecesTrappedRook => {
            "Penalty when OUR rook can only move along 1–2 squares because OUR king blocks it after losing castling. Positive = THEIR rook is trapped more than ours."
        }
        TermId::PiecesWeakQueen => {
            "Penalty when OUR queen is attacked through a discovered-attack ray. Positive = THEIR queen is under more X-ray pressure than ours."
        }

        // -- mobility (net) ------------------------------------------
        TermId::MobilityKnight => {
            "Net knight mobility advantage, weighted by SF11's per-square table. Positive = OUR knights have more good squares."
        }
        TermId::MobilityBishop => {
            "Net bishop mobility advantage. Positive = OUR bishops are more active."
        }
        TermId::MobilityRook => {
            "Net rook mobility advantage. Positive = OUR rooks are more active."
        }
        TermId::MobilityQueen => {
            "Net queen mobility advantage. Positive = OUR queen is more active."
        }

        // -- threats -------------------------------------------------
        TermId::ThreatsByMinor => {
            "Material at risk to OUR minor-piece attacks on enemy pieces (net of defenders). Positive = WE threaten more material."
        }
        TermId::ThreatsByRook => {
            "Material at risk to OUR rook attacks on enemy pieces. Positive = WE threaten more."
        }
        TermId::ThreatsByKing => {
            "OUR king joining the attack (endgame). Positive = OUR king is more active in the assault."
        }
        TermId::ThreatsHanging => {
            "Penalty for OUR pieces attacked more times than defended. Positive = THEIR pieces are hanging more than ours."
        }
        TermId::ThreatsRestricted => {
            "Bonus when OUR pieces restrict the enemy's piece mobility. Positive = WE cramp them more than they cramp us."
        }
        TermId::ThreatsBySafePawn => {
            "Bonus when OUR pawns attack enemy pieces from a square the enemy can't safely capture on. Positive = WE have more safe-pawn threats."
        }
        TermId::ThreatsByPawnPush => {
            "Bonus when pushing one of OUR pawns one square would attack an enemy piece. Positive = WE have more pawn-push threats."
        }
        TermId::ThreatsKnightOnQueen => {
            "Bonus when OUR knight is one move from attacking the enemy queen. Positive = WE pressure their queen more."
        }
        TermId::ThreatsSliderOnQueen => {
            "Bonus when OUR rook/bishop sees the enemy queen through one blocker (a discovered-attack / pin pre-cursor). Positive = WE have more X-ray pressure on their queen."
        }
    }
}

/// Render the full glossary as a multi-line table — used by
/// `chess-tutor eval --glossary` and visible via the JSON
/// `description` field on every term.
pub fn render_glossary_table() -> String {
    let mut out = String::new();
    use std::fmt::Write;
    writeln!(
        out,
        "Term-id glossary. Every gloss describes what a POSITIVE net value means."
    )
    .unwrap();
    writeln!(
        out,
        "Eval-trace rows are net-of-colour (white − black) unless explicitly per-side."
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(out, "{:<28}  description", "term").unwrap();
    writeln!(out, "{}", "-".repeat(28 + 2 + 80)).unwrap();
    for &id in &TermId::ALL {
        writeln!(out, "{:<28}  {}", id.label(), description(id)).unwrap();
    }
    out
}

#[cfg(test)]
#[path = "glossary_tests.rs"]
mod tests;
