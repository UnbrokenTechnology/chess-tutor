use super::*;
use crate::san;

/// Parse a space-separated SAN line into a Vec<Move>, advancing a scratch
/// position move by move (so SAN disambiguation resolves correctly).
fn pv_from_san(start_fen: &str, line: &str) -> Vec<Move> {
    let mut pos = Position::from_fen(start_fen).unwrap();
    let mut moves = Vec::new();
    for tok in line.split_whitespace() {
        let mv = san::parse(&mut pos, tok)
            .unwrap_or_else(|e| panic!("parsing SAN {tok:?}: {e}"));
        pos.do_move(mv);
        moves.push(mv);
    }
    moves
}

/// The regression target: the `e5` push. After `e5`, Black chases the
/// light-squared bishop off the board (`…d5`, `…Bd7`, `…a6`) while
/// developing — White reacts every move and the static-eval edge (space /
/// king attack) is an illusion. Material stays even, so this is a
/// loss-of-*initiative*, not a material loss.
const E5_FEN: &str = "rnbqkb1r/pp2npp1/3pp2p/8/2BQP3/5N2/PPP2PPP/RNB2RK1 w - - 0 1";

#[test]
fn detects_initiative_loss_on_e5_push() {
    // Engine line for `e5` (from `critique` / `search --force-include`):
    // Black's immediate reply `…d5` attacks the `Bc4`, White must retreat
    // `Bb5+`, and the harassment continues. The opponent's *first* reply
    // already chases a piece — that's the signal.
    let pv = pv_from_san(E5_FEN, "e5 d5 Bb5+ Bd7 c4 a6 Bxd7+ Nxd7 cxd5 Nxd5 Nbd2");
    let pre = Position::from_fen(E5_FEN).unwrap();
    let loss = detect_initiative_loss(&pre, &pv, Color::White)
        .expect("e5's immediate reply …d5 chases the bishop — initiative loss");
    assert_eq!(loss.hits[0].ply, 1, "the chase must start on the opponent's first reply");
    assert_eq!(loss.hits[0].target, Square::C4, "the immediately-chased piece is the c4 bishop");
}

#[test]
fn quiet_developing_line_is_not_initiative_loss() {
    // A calm symmetric opening: nobody is chased, no forcing run.
    let pv = pv_from_san(
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        "e4 e5 Nf3 Nc6 Bc4 Bc5 c3 Nf6 d3 d6",
    );
    let pre = Position::startpos();
    assert!(
        detect_initiative_loss(&pre, &pv, Color::White).is_none(),
        "a quiet developing line has no forcing run — must not fire"
    );
}

#[test]
fn harassment_must_start_on_the_immediate_reply() {
    // If the opponent's FIRST reply is quiet and the piece-attack only
    // appears later, the loss isn't reliably the user move's fault — the
    // detector must stay silent (that's silent-sequencing / depth-honesty
    // territory). Here `…a6` (ply 1) is a quiet pawn move; `…Nbc6` (ply 3)
    // attacks the queen, but it's not the immediate reply.
    let pv = pv_from_san(E5_FEN, "Nc3 a6 a4 Nbc6 Qd3 Bd7");
    let pre = Position::from_fen(E5_FEN).unwrap();
    let result = detect_initiative_loss(&pre, &pv, Color::White);
    assert!(
        result.is_none(),
        "harassment that doesn't start on the opponent's first reply must not fire: {result:?}"
    );
}

#[test]
fn empty_pv_returns_none() {
    let pre = Position::startpos();
    assert!(detect_initiative_loss(&pre, &[], Color::White).is_none());
}

// NOTE on the `…Qc8` silent-sequencing bookend: at the engine level the
// detector *does* report `…Qc8`'s immediate `Bd5`/`Be5` pressure (it's a
// factual "your piece is pressed" report). The discrimination from the
// genuine-lesson `e5` case happens at the teaching layer via
// `is_silent_sequencing` — `…Qc8`'s gap only resolves past human depth, so
// the UI routes it to the depth-honesty note instead of an initiative card.
// That bookend is therefore tested in
// `core/ui/src/retrospective_view/initiative.rs`
// (`stays_silent_on_qc8_silent_sequencing`).
