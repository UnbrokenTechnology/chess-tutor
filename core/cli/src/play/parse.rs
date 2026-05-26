//! REPL input parsing (search/analyze/toggle/move commands).

use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::types::Move;
use crate::uci;

pub(super) fn parse_search_command(input: &str) -> Result<usize, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(1);
    }
    let n: usize = trimmed
        .parse()
        .map_err(|_| format!("bad count {:?}; expected a positive integer", trimmed))?;
    if n == 0 {
        return Err("count must be at least 1".to_string());
    }
    Ok(n)
}

#[derive(Debug, PartialEq)]
pub(super) struct AnalyzeArgs {
    pub(super) multi_pv: usize,
    pub(super) top_percent: f32,
}

pub(super) fn parse_analyze_command(input: &str) -> Result<AnalyzeArgs, String> {
    let mut tokens = input.split_whitespace();
    let first = tokens.next();
    let second = tokens.next();
    if tokens.next().is_some() {
        return Err("too many arguments; usage: analyze [N] [PERCENT]".to_string());
    }
    let multi_pv = match first {
        None => 3,
        Some(tok) => {
            let n: usize = tok
                .parse()
                .map_err(|_| format!("bad count {:?}; expected a positive integer", tok))?;
            if n == 0 {
                return Err("count must be at least 1".to_string());
            }
            n
        }
    };
    let top_percent = match second {
        None => 75.0,
        Some(tok) => {
            let p: f32 = tok
                .parse()
                .map_err(|_| format!("bad percent {:?}; expected a number", tok))?;
            if !(p > 0.0 && p <= 100.0) {
                return Err("percent must be in (0, 100]".to_string());
            }
            p
        }
    };
    Ok(AnalyzeArgs { multi_pv, top_percent })
}

pub(super) fn parse_toggle(input: &str) -> Result<Option<bool>, String> {
    match input.trim() {
        "" => Ok(None),
        "on" | "true" | "1" => Ok(Some(true)),
        "off" | "false" | "0" => Ok(Some(false)),
        other => Err(format!("expected 'on' or 'off', got {:?}", other)),
    }
}

pub(super) fn parse_user_move(pos: &mut Position, input: &str) -> Result<Move, String> {
    match san::parse(pos, input) {
        Ok(mv) => Ok(mv),
        Err(san_err) => match uci::parse(pos, input) {
            Ok(mv) => Ok(mv),
            Err(uci_err) => Err(format!("not SAN ({san_err}); not UCI ({uci_err})")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_search_default_to_one() {
        assert_eq!(parse_search_command(""), Ok(1));
        assert_eq!(parse_search_command("   "), Ok(1));
    }

    #[test]
    fn parse_search_accepts_n() {
        assert_eq!(parse_search_command("3"), Ok(3));
        assert_eq!(parse_search_command("  5 "), Ok(5));
    }

    #[test]
    fn parse_search_rejects_zero() {
        assert!(parse_search_command("0").is_err());
    }

    #[test]
    fn parse_analyze_default() {
        assert_eq!(
            parse_analyze_command(""),
            Ok(AnalyzeArgs {
                multi_pv: 3,
                top_percent: 75.0,
            })
        );
    }

    #[test]
    fn parse_analyze_n_p() {
        assert_eq!(
            parse_analyze_command("4 80"),
            Ok(AnalyzeArgs {
                multi_pv: 4,
                top_percent: 80.0,
            })
        );
    }

    #[test]
    fn parse_analyze_rejects_too_many() {
        assert!(parse_analyze_command("1 2 3").is_err());
    }
}
