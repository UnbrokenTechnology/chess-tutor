//! Shallow-vs-deep surprise-tag selection for the retrospective
//! footer. Filters out combinations where the tag would contradict
//! the verdict or pile onto one that's already clear.

use chess_tutor_engine::analysis::{MoveVerdict, SurpriseKind};

/// Decide whether (and how) to render the shallow-vs-deep surprise
/// tag given the verdict and the raw surprise kind.
///
/// Rules:
/// - `Best / Good` + `LooksBadButGood` → positive surprise — "you
///   played a move that looks risky but actually pays off." Good
///   teaching content.
/// - `Inaccuracy / Mistake` + `LooksGoodButBad` → the main
///   teaching case. The move looked reasonable at first glance but
///   the engine sees deeper tactics the student didn't.
/// - **Everything else suppressed**, because the tag would
///   contradict or over-narrate the verdict:
///   - `Best / Good` + `LooksGoodButBad` — move is fine, don't
///     tell the student they fell for a trap they didn't fall for.
///   - `Inaccuracy / Mistake` + `LooksBadButGood` — contradicts
///     the verdict (rare and confusing).
///   - `Blunder` — verdict alone is clear, extra tag is noise.
///   - `BestAvailable` — position was already lost, don't pile
///     on.
///
/// Phrasing intentionally avoids strong chess-terminology
/// ("refutes") for the LooksGoodButBad case — the shallow-vs-deep
/// delta threshold is low enough that a move firing this tag isn't
/// necessarily being refuted in any formal sense; it's just
/// "deeper analysis doesn't like it as much."
pub(crate) fn select_surprise_phrase(
    verdict: MoveVerdict,
    surprise: Option<SurpriseKind>,
) -> Option<&'static str> {
    let kind = surprise?;
    match (verdict, kind) {
        (MoveVerdict::Best | MoveVerdict::Good, SurpriseKind::LooksBadButGood) => {
            Some("looks risky at first glance — the longer line pays off")
        }
        (MoveVerdict::Inaccuracy | MoveVerdict::Mistake, SurpriseKind::LooksGoodButBad) => {
            Some("looks reasonable short-term — the follow-up favors the opponent")
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surprise_phrase_none_when_no_surprise() {
        assert_eq!(select_surprise_phrase(MoveVerdict::Inaccuracy, None), None);
    }

    #[test]
    fn surprise_phrase_fires_on_inaccuracy_looks_good_but_bad() {
        let phrase =
            select_surprise_phrase(MoveVerdict::Inaccuracy, Some(SurpriseKind::LooksGoodButBad))
                .expect("should fire");
        assert!(phrase.contains("reasonable"));
        assert!(!phrase.contains("refute"));
    }

    #[test]
    fn surprise_phrase_fires_on_mistake_looks_good_but_bad() {
        assert!(
            select_surprise_phrase(MoveVerdict::Mistake, Some(SurpriseKind::LooksGoodButBad),)
                .is_some()
        );
    }

    #[test]
    fn surprise_phrase_fires_on_best_looks_bad_but_good() {
        let phrase = select_surprise_phrase(MoveVerdict::Best, Some(SurpriseKind::LooksBadButGood))
            .expect("should fire");
        assert!(phrase.contains("risky"));
    }

    #[test]
    fn surprise_phrase_fires_on_good_looks_bad_but_good() {
        assert!(
            select_surprise_phrase(MoveVerdict::Good, Some(SurpriseKind::LooksBadButGood))
                .is_some()
        );
    }

    #[test]
    fn surprise_phrase_suppressed_on_good_looks_good_but_bad() {
        assert_eq!(
            select_surprise_phrase(MoveVerdict::Good, Some(SurpriseKind::LooksGoodButBad)),
            None,
        );
    }

    #[test]
    fn surprise_phrase_suppressed_on_best_looks_good_but_bad() {
        assert_eq!(
            select_surprise_phrase(MoveVerdict::Best, Some(SurpriseKind::LooksGoodButBad)),
            None,
        );
    }

    #[test]
    fn surprise_phrase_suppressed_on_blunder() {
        assert_eq!(
            select_surprise_phrase(MoveVerdict::Blunder, Some(SurpriseKind::LooksGoodButBad)),
            None,
        );
        assert_eq!(
            select_surprise_phrase(MoveVerdict::Blunder, Some(SurpriseKind::LooksBadButGood)),
            None,
        );
    }

    #[test]
    fn surprise_phrase_suppressed_on_best_available() {
        assert_eq!(
            select_surprise_phrase(
                MoveVerdict::BestAvailable,
                Some(SurpriseKind::LooksGoodButBad),
            ),
            None,
        );
        assert_eq!(
            select_surprise_phrase(
                MoveVerdict::BestAvailable,
                Some(SurpriseKind::LooksBadButGood),
            ),
            None,
        );
    }

    #[test]
    fn surprise_phrase_suppressed_when_kind_contradicts_verdict() {
        assert_eq!(
            select_surprise_phrase(MoveVerdict::Inaccuracy, Some(SurpriseKind::LooksBadButGood)),
            None,
        );
        assert_eq!(
            select_surprise_phrase(MoveVerdict::Mistake, Some(SurpriseKind::LooksBadButGood)),
            None,
        );
    }
}
