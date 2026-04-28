//! Initiative narrator — picks one of three templates from an
//! [`InitiativeOutcome`] and writes a single teaching line about the
//! forcing-hierarchy relationship between the user's move and the
//! opponent's best reply.
//!
//! The three templates correspond to the three teaching cases the
//! user wants explicit narration for:
//!
//! 1. **Reinforcement** — user's threat lands and the opponent has
//!    no check / capture available to outrank it. *Teaches the rule
//!    by showing it works.*
//! 2. **Refutation** — opponent's reply is a check or capture and
//!    the eval at settled-ply has tilted against the user.
//!    *Teaches the rule by showing what the user missed.*
//! 3. **Held despite** — opponent's reply is a check or capture but
//!    the user is still winning. *Teaches the limit of the rule:
//!    the hierarchy is a processing order, not a winning rule.*
//!
//! Per the user's terminology guidance (memory: chess-accurate
//! teaching strings), this narrator names the rule explicitly
//! (*"checks must be answered before any other threat"*) so the
//! student picks up the procedural skill, not just a pattern.
//!
//! Phrasing precedence: when the opponent's reply is *both* a check
//! and a capture (e.g., `Bxh7+`), narrate it as a check — checks
//! dominate captures in the forcing hierarchy.

use std::io;

use chess_tutor_engine::analysis::InitiativeOutcome;

/// Minimum |swing| (ply 1 → settled, user POV, in engine cp) for the
/// refutation template to fire. Below this, the opponent's
/// check/capture didn't materially change the line — no point
/// pretending the user's threat got "refuted."
const REFUTATION_SWING_GATE: i32 = 50;

/// Render the initiative narration to `out`. Returns `Ok(true)` when
/// a line was written, `Ok(false)` when no template matched (caller
/// uses the bool to decide whether to suppress the redundant
/// fallback "Also helped / hurt" line for terms this narrator would
/// have consumed — currently none, since initiative is about
/// move-relationship, not eval terms).
pub fn render_initiative(
    out: &mut dyn io::Write,
    outcome: &InitiativeOutcome,
) -> io::Result<bool> {
    let Some(line) = build_line(outcome) else {
        return Ok(false);
    };
    writeln!(out, "                {line}")?;
    Ok(true)
}

/// Pure template-selection + phrase-assembly. Split out as a
/// `pub(crate)` helper so the unit tests can exercise template
/// branches without going through the output writer.
pub(crate) fn build_line(outcome: &InitiativeOutcome) -> Option<String> {
    if !outcome.user_move_was_threat {
        return None;
    }
    // Without a named opponent reply we can't form any of the three
    // templates — they all reference `pv[1]` either explicitly (2/3)
    // or implicitly ("no check or capture available," 1).
    let san = outcome.opponent_reply_san.as_deref()?;

    let opponent_forces =
        outcome.opponent_reply_is_check || outcome.opponent_reply_is_capture;

    if !opponent_forces {
        // Template 1: reinforcement.
        return Some(template_reinforcement());
    }

    if outcome.user_still_favored {
        // Template 3: held despite.
        return Some(template_held_despite(san, outcome.opponent_reply_is_check));
    }

    // Template 2: refutation. Suppress when the swing is too small
    // to claim anything got refuted — that's the
    // "mutual-forcing-equal exchange" case the brief flagged.
    if outcome.eval_swing_cp > -REFUTATION_SWING_GATE {
        return None;
    }
    Some(template_refutation(san, outcome.opponent_reply_is_check))
}

fn template_reinforcement() -> String {
    "Your move creates a threat the opponent has to address — there's no \
     check or capture available to play first."
        .to_string()
}

fn template_refutation(reply_san: &str, is_check: bool) -> String {
    // Check dominates capture in the forcing hierarchy — phrase it
    // as a check when both flags are set (e.g., `Bxh7+`).
    if is_check {
        format!(
            "Your move creates a threat — but the opponent has {reply_san}, a check \
             that takes priority. Checks must be answered before any other threat, \
             so yours doesn't get a chance to land."
        )
    } else {
        format!(
            "Your move creates a threat — but the opponent has {reply_san}, a \
             capture that takes priority. Captures change the material picture \
             immediately, so your threat sits behind it."
        )
    }
}

fn template_held_despite(reply_san: &str, is_check: bool) -> String {
    if is_check {
        format!(
            "Your move creates a threat. The opponent's {reply_san} has to be \
             answered first — checks come before threats — but after the dust \
             settles, your threat still lands and you're better."
        )
    } else {
        format!(
            "Your move creates a threat. The opponent's {reply_san} addresses \
             the material first, but it doesn't actually save them — your \
             threat still lands and you're better."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(
        threat: bool,
        check: bool,
        capture: bool,
        san: Option<&str>,
        swing: i32,
        favored: bool,
    ) -> InitiativeOutcome {
        InitiativeOutcome {
            user_move_was_threat: threat,
            opponent_reply_is_check: check,
            opponent_reply_is_capture: capture,
            opponent_reply_san: san.map(|s| s.to_string()),
            eval_swing_cp: swing,
            user_still_favored: favored,
        }
    }

    // ---- gating: no narration when user didn't make a threat -------

    #[test]
    fn no_narration_when_user_did_not_make_a_threat() {
        let o = outcome(false, false, false, Some("Nf6"), 0, true);
        assert_eq!(build_line(&o), None);
    }

    #[test]
    fn no_narration_when_no_opponent_reply_san() {
        // Mate / stalemate after user's move — we have no SAN to
        // name, so don't fire any template.
        let o = outcome(true, false, false, None, 0, true);
        assert_eq!(build_line(&o), None);
    }

    // ---- template 1: reinforcement ---------------------------------

    #[test]
    fn template_reinforcement_when_threat_with_quiet_opponent_reply() {
        let o = outcome(true, false, false, Some("Nf6"), 0, true);
        let line = build_line(&o).unwrap();
        assert!(
            line.contains("creates a threat"),
            "missing rule statement, got: {line}",
        );
        assert!(
            line.contains("no check or capture available"),
            "missing hierarchy mention, got: {line}",
        );
    }

    // ---- template 2: refutation ------------------------------------

    #[test]
    fn template_refutation_check_phrasing_when_opponent_replies_with_check() {
        // Significant negative swing -> refutation fires.
        let o = outcome(true, true, false, Some("Qa3+"), -300, false);
        let line = build_line(&o).unwrap();
        assert!(line.contains("Qa3+"));
        assert!(
            line.contains("a check that takes priority"),
            "expected check phrasing, got: {line}",
        );
        assert!(
            line.contains("Checks must be answered"),
            "missing rule statement, got: {line}",
        );
    }

    #[test]
    fn template_refutation_capture_phrasing_when_opponent_replies_with_pure_capture() {
        let o = outcome(true, false, true, Some("Bxc4"), -200, false);
        let line = build_line(&o).unwrap();
        assert!(line.contains("Bxc4"));
        assert!(
            line.contains("a capture that takes priority"),
            "expected capture phrasing, got: {line}",
        );
    }

    #[test]
    fn template_refutation_check_phrasing_dominates_when_reply_is_both() {
        // Bxh7+ is a check AND a capture. Per the forcing
        // hierarchy, narrate it as a check.
        let o = outcome(true, true, true, Some("Bxh7+"), -400, false);
        let line = build_line(&o).unwrap();
        assert!(line.contains("Bxh7+"));
        assert!(
            line.contains("a check that takes priority"),
            "checks should dominate captures in phrasing, got: {line}",
        );
        assert!(
            !line.contains("a capture that takes priority"),
            "should not also use capture phrasing, got: {line}",
        );
    }

    #[test]
    fn refutation_suppressed_when_swing_too_small() {
        // Opponent has a check but the line settles at roughly the
        // same eval — don't claim a refutation.
        let o = outcome(true, true, false, Some("Kh1"), -10, false);
        assert_eq!(build_line(&o), None);
    }

    #[test]
    fn refutation_fires_at_gate_threshold() {
        let o = outcome(true, true, false, Some("Qa3+"), -REFUTATION_SWING_GATE - 1, false);
        let line = build_line(&o).unwrap();
        assert!(line.contains("a check that takes priority"));
    }

    // ---- template 3: held despite ----------------------------------

    #[test]
    fn template_held_despite_when_opponent_checks_but_user_still_favored() {
        let o = outcome(true, true, false, Some("Qa3+"), -100, true);
        let line = build_line(&o).unwrap();
        assert!(line.contains("Qa3+"));
        assert!(
            line.contains("checks come before threats"),
            "missing rule statement, got: {line}",
        );
        assert!(
            line.contains("your threat still lands"),
            "missing 'still works' framing, got: {line}",
        );
    }

    #[test]
    fn template_held_despite_capture_phrasing_when_pure_capture() {
        let o = outcome(true, false, true, Some("Rxd4"), -50, true);
        let line = build_line(&o).unwrap();
        assert!(line.contains("Rxd4"));
        assert!(
            line.contains("addresses the material first"),
            "expected capture-flavoured held-despite phrasing, got: {line}",
        );
    }

    #[test]
    fn held_despite_takes_precedence_over_refutation_when_user_favored() {
        // user_still_favored = true → template 3 even if swing
        // would otherwise gate refutation.
        let o = outcome(true, true, false, Some("Qa3+"), -500, true);
        let line = build_line(&o).unwrap();
        assert!(
            line.contains("your threat still lands"),
            "favored flag should pick template 3, got: {line}",
        );
    }
}
