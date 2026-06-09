use super::*;

use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Color, PieceType};

#[test]
fn startpos_has_no_captures_and_even_material() {
    let pos = Position::startpos();
    let (captured, adv) = captured_diff(&pos, Color::White);
    assert!(captured.is_empty());
    assert_eq!(adv, 0);
}

#[test]
fn bot_up_a_knight_lists_the_users_missing_knight() {
    // White to move, Black is missing exactly one knight (g8 empty).
    let fen = "rnbqkb1r/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    // Bot is White; user is Black. White captured Black's knight.
    let (captured, adv) = captured_diff(&pos, Color::White);
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].kind(), PieceType::Knight);
    assert_eq!(captured[0].color(), Color::Black);
    // Bot is up a knight: +3 from the bot's POV.
    assert_eq!(adv, 3);
}

#[test]
fn captured_list_is_heaviest_first() {
    // Black is missing a queen and a pawn (queen off d8, one pawn off a7).
    let fen = "rnb1kbnr/1ppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let (captured, adv) = captured_diff(&pos, Color::White);
    assert_eq!(captured.len(), 2);
    // Queen (9) before pawn (1).
    assert_eq!(captured[0].kind(), PieceType::Queen);
    assert_eq!(captured[1].kind(), PieceType::Pawn);
    assert_eq!(adv, 10);
}

#[test]
fn point_advantage_is_signed_from_bot_pov() {
    // White missing a rook -> if the bot is White, it's down 5.
    let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBN1 w Qkq - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let (_captured, adv) = captured_diff(&pos, Color::White);
    assert_eq!(adv, -5);
    // Same position framed from Black-as-bot: Black is up 5.
    let (_captured2, adv2) = captured_diff(&pos, Color::Black);
    assert_eq!(adv2, 5);
}

#[test]
fn opening_recognition_is_sticky_after_leaving_book() {
    // 1.e4 e5 2.Nf3 Nc6 3.Bb5 — the Ruy Lopez, a position the catalog knows.
    let ruy = Position::from_fen(
        "r1bqkbnr/pppp1ppp/2n5/1B2p3/4P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3",
    )
    .unwrap();
    let direct = recognized_opening(std::iter::once(&ruy));
    assert!(
        direct.is_some(),
        "the Ruy Lopez position should be a recognized opening"
    );

    // A bare K+P endgame the recognizer doesn't know.
    let endgame = Position::from_fen("4k3/8/8/8/4P3/8/8/4K3 w - - 0 1").unwrap();
    assert!(recognized_opening(std::iter::once(&endgame)).is_none());

    // Sticky: walking [opening, off-book] keeps the opening name even
    // though the final position is no longer in the book.
    let walked = recognized_opening([&ruy, &endgame].into_iter());
    assert_eq!(walked, direct);
}

#[test]
fn opening_is_none_at_the_start_position() {
    // The bare start position isn't itself a named opening.
    let start = Position::startpos();
    assert!(recognized_opening(std::iter::once(&start)).is_none());
}
