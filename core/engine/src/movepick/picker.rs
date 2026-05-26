//! The [`MovePicker`](super::MovePicker) staged-generation state machine:
//! the constructors (`new_main`, `new_qs`), the `next_move` FSM, and the
//! per-stage generation / scoring routines. The struct, buffer pool, and
//! `Stage` enum live in [`super`]; the scoring helpers in [`super::helpers`].

use super::helpers::{
    captured_piece_value, is_pseudo_legal, mvv_lva, partial_insertion_sort, pick_best_index,
};
use super::{
    checkout_move_bufs, split_bufs, ButterflyHistory, CaptureHistory, ContHistKeys, ContHistStore,
    MovePicker, PieceToHistory, ScoredMove, Stage, EVASION_QUIET_PENALTY, QUIET_SORT_BASE,
};
use crate::movegen::{generate_pseudo_legal_moves, MoveList};
use crate::position::Position;
use crate::types::{Depth, Move, Square, Value};

impl MovePicker {
    /// Construct a picker for the main search. `depth` must be positive.
    /// `killers` are the two killer moves for the current ply (either may
    /// be `Move::NONE`). The position is read once here (to validate the
    /// TT move and decide whether we're in check); subsequent calls to
    /// [`MovePicker::next_move`] must pass the same position back in
    /// alongside the history table used for quiet-move scoring.
    pub fn new_main(
        pos: &Position,
        tt_move: Move,
        depth: Depth,
        killers: [Move; 2],
        counter_move: Move,
        cont_keys: ContHistKeys,
    ) -> Self {
        debug_assert!(depth.0 > 0);

        let in_check = pos.in_check();
        let tt_ok = tt_move.is_valid() && is_pseudo_legal(pos, tt_move);
        let tt_move = if tt_ok { tt_move } else { Move::NONE };

        let stage = if in_check {
            if tt_move == Move::NONE {
                Stage::EvasionInit
            } else {
                Stage::EvasionTt
            }
        } else if tt_move == Move::NONE {
            Stage::CaptureInit
        } else {
            Stage::MainTt
        };

        let mut bufs = checkout_move_bufs();
        let (captures, bad_captures, quiets, evasions) = split_bufs(&mut bufs);
        Self {
            tt_move,
            killers,
            counter_move,
            cont_keys,
            depth,
            recapture_square: None,
            stage,
            cur: 0,
            bufs: Some(bufs),
            captures,
            bad_captures,
            quiets,
            evasions,
        }
    }

    /// Construct a picker for quiescence search. `depth` must be
    /// non-positive (the qs ladder: `QS_CHECKS = 0`, `QS_NO_CHECKS = -1`,
    /// … `QS_RECAPTURES = -5`). At the deepest qs ply we only accept
    /// captures that land on `recapture_square`.
    pub fn new_qs(
        pos: &Position,
        tt_move: Move,
        depth: Depth,
        recapture_square: Option<Square>,
        cont_keys: ContHistKeys,
    ) -> Self {
        debug_assert!(depth.0 <= 0);

        let in_check = pos.in_check();
        let tt_ok = tt_move.is_valid()
            && (depth > Depth::QS_RECAPTURES || Some(tt_move.to()) == recapture_square)
            && is_pseudo_legal(pos, tt_move);
        let tt_move = if tt_ok { tt_move } else { Move::NONE };

        let stage = if in_check {
            if tt_move == Move::NONE {
                Stage::EvasionInit
            } else {
                Stage::EvasionTt
            }
        } else if tt_move == Move::NONE {
            Stage::QCaptureInit
        } else {
            Stage::QSearchTt
        };

        let mut bufs = checkout_move_bufs();
        let (captures, bad_captures, quiets, evasions) = split_bufs(&mut bufs);
        Self {
            tt_move,
            killers: [Move::NONE; 2],
            counter_move: Move::NONE,
            cont_keys,
            depth,
            recapture_square,
            stage,
            cur: 0,
            bufs: Some(bufs),
            captures,
            bad_captures,
            quiets,
            evasions,
        }
    }

    /// Return the next pseudo-legal move for the search to try. Returns
    /// `Move::NONE` once the pipeline is exhausted. Setting `skip_quiets`
    /// causes the picker to stop after good captures + killers + bad
    /// captures (used by search when aggressive pruning has already
    /// rejected quiet moves at this node). `pos` must be the same
    /// position (by value) that was passed to the constructor — the
    /// picker's staged generation only makes sense against that one
    /// state.
    pub fn next_move(
        &mut self,
        pos: &Position,
        history: Option<&ButterflyHistory>,
        cont_history: Option<&ContHistStore>,
        capture_history: Option<&CaptureHistory>,
        skip_quiets: bool,
    ) -> Move {
        loop {
            match self.stage {
                // ---- TT stages: return ttMove once, then advance ----
                Stage::MainTt => {
                    self.stage = Stage::CaptureInit;
                    return self.tt_move;
                }
                Stage::EvasionTt => {
                    self.stage = Stage::EvasionInit;
                    return self.tt_move;
                }
                Stage::QSearchTt => {
                    self.stage = Stage::QCaptureInit;
                    return self.tt_move;
                }

                // ---- Main pipeline: captures, killers, quiets, bad ----
                Stage::CaptureInit => {
                    self.generate_captures(pos, capture_history);
                    self.cur = 0;
                    self.stage = Stage::GoodCapture;
                    continue;
                }
                Stage::GoodCapture => {
                    if let Some(mv) = self.next_good_capture(pos) {
                        return mv;
                    }
                    self.stage = Stage::Killer0;
                    continue;
                }
                Stage::Killer0 => {
                    self.stage = Stage::Killer1;
                    let k = self.killers[0];
                    if self.is_valid_killer(pos, k) {
                        return k;
                    }
                    continue;
                }
                Stage::Killer1 => {
                    self.stage = Stage::CounterMove;
                    let k = self.killers[1];
                    if self.is_valid_killer(pos, k) && k != self.killers[0] {
                        return k;
                    }
                    continue;
                }
                Stage::CounterMove => {
                    self.stage = Stage::QuietInit;
                    let cm = self.counter_move;
                    if self.is_valid_counter_move(pos, cm) {
                        return cm;
                    }
                    continue;
                }
                Stage::QuietInit => {
                    if skip_quiets {
                        self.stage = Stage::BadCapture;
                        self.cur = 0;
                        continue;
                    }
                    self.generate_quiets(pos, history, cont_history);
                    let limit = QUIET_SORT_BASE * self.depth.0;
                    partial_insertion_sort(&mut self.quiets, limit);
                    self.cur = 0;
                    self.stage = Stage::Quiet;
                    continue;
                }
                Stage::Quiet => {
                    if skip_quiets {
                        self.stage = Stage::BadCapture;
                        self.cur = 0;
                        continue;
                    }
                    while self.cur < self.quiets.len() {
                        let mv = self.quiets[self.cur].mv;
                        self.cur += 1;
                        if mv == self.tt_move
                            || mv == self.killers[0]
                            || mv == self.killers[1]
                            || mv == self.counter_move
                        {
                            continue;
                        }
                        return mv;
                    }
                    self.stage = Stage::BadCapture;
                    self.cur = 0;
                    continue;
                }
                Stage::BadCapture => {
                    while self.cur < self.bad_captures.len() {
                        let mv = self.bad_captures[self.cur].mv;
                        self.cur += 1;
                        if mv == self.tt_move {
                            continue;
                        }
                        return mv;
                    }
                    self.stage = Stage::Done;
                    return Move::NONE;
                }

                // ---- Evasion pipeline: unified captures + quiets ----
                Stage::EvasionInit => {
                    self.generate_evasions(pos, history, cont_history);
                    self.cur = 0;
                    self.stage = Stage::Evasion;
                    continue;
                }
                Stage::Evasion => {
                    while self.cur < self.evasions.len() {
                        let best_idx = pick_best_index(&self.evasions, self.cur);
                        if best_idx != self.cur {
                            self.evasions.swap(self.cur, best_idx);
                        }
                        let mv = self.evasions[self.cur].mv;
                        self.cur += 1;
                        if mv == self.tt_move {
                            continue;
                        }
                        return mv;
                    }
                    self.stage = Stage::Done;
                    return Move::NONE;
                }

                // ---- Qsearch pipeline: captures only (recapture-restricted at deep qs) ----
                Stage::QCaptureInit => {
                    self.generate_captures(pos, capture_history);
                    self.cur = 0;
                    self.stage = Stage::QCapture;
                    continue;
                }
                Stage::QCapture => {
                    while self.cur < self.captures.len() {
                        let best_idx = pick_best_index(&self.captures, self.cur);
                        if best_idx != self.cur {
                            self.captures.swap(self.cur, best_idx);
                        }
                        let mv = self.captures[self.cur].mv;
                        self.cur += 1;
                        if mv == self.tt_move {
                            continue;
                        }
                        // At the deepest qs ply, only accept moves to
                        // `recapture_square`.
                        if self.depth <= Depth::QS_RECAPTURES
                            && Some(mv.to()) != self.recapture_square
                        {
                            continue;
                        }
                        return mv;
                    }
                    self.stage = Stage::Done;
                    return Move::NONE;
                }

                Stage::Done => return Move::NONE,
            }
        }
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    fn is_valid_killer(&self, pos: &Position, mv: Move) -> bool {
        mv.is_valid() && mv != self.tt_move && !pos.is_capture(mv) && is_pseudo_legal(pos, mv)
    }

    /// Counter-move validation: same constraints as a killer, plus
    /// dedupe against the killers themselves so the picker doesn't
    /// return the same move twice.
    fn is_valid_counter_move(&self, pos: &Position, mv: Move) -> bool {
        mv.is_valid()
            && mv != self.tt_move
            && mv != self.killers[0]
            && mv != self.killers[1]
            && !pos.is_capture(mv)
            && is_pseudo_legal(pos, mv)
    }

    /// Generate every pseudo-legal capture and score it with MVV-LVA
    /// plus Stockfish 11's capture-history tiebreaker (`captureHistory
    /// [moved_piece][to_sq][captured_piece_type]`). The capture-hist
    /// borrow is used only inside this call so β-cutoff updates can
    /// take `&mut` afterwards.
    fn generate_captures(&mut self, pos: &Position, capture_history: Option<&CaptureHistory>) {
        self.captures.clear();
        self.bad_captures.clear();
        let mut all = MoveList::new();
        generate_pseudo_legal_moves(pos, &mut all);
        for &mv in &all {
            if !pos.is_capture(mv) {
                continue;
            }
            let mut score = mvv_lva(pos, mv);
            if let Some(ch) = capture_history {
                let moved_piece_idx = pos.moved_piece(mv).index() as u8;
                let to_idx = mv.to().index() as u8;
                // Mirror Stockfish: read `piece_on(to)` directly (en
                // passant resolves to slot 0 because the to-square is
                // empty, which matches Stockfish's behaviour).
                let captured_pt_idx = pos
                    .piece_on(mv.to())
                    .map(|p| p.kind().index() as u8)
                    .unwrap_or(0);
                score += ch.get(moved_piece_idx, to_idx, captured_pt_idx) as i32;
            }
            self.captures.push(ScoredMove { mv, score });
        }
    }

    /// Generate every pseudo-legal quiet (non-capture) and score by
    /// butterfly history plus Stockfish 11's continuation-history sum
    /// (1-ply, 2-ply, 4-ply, 6-ply with weights 2/2/2/1). The
    /// `cont_history` borrow is used only inside this call so the
    /// caller's mutable borrow on the same store can resume after
    /// `next_move` returns the next quiet.
    fn generate_quiets(
        &mut self,
        pos: &Position,
        history: Option<&ButterflyHistory>,
        cont_history: Option<&ContHistStore>,
    ) {
        self.quiets.clear();
        let history =
            history.expect("generate_quiets: main picker must be called with a history reference");
        let us = pos.side_to_move();
        // Resolve the four parent sub-tables once per call. If
        // `cont_history` is absent (test harness without engine state),
        // skip the cont-hist contribution entirely.
        let cont_subs: Option<[&PieceToHistory; 4]> = cont_history.map(|store| {
            [
                store.sub_for_key(self.cont_keys[0]),
                store.sub_for_key(self.cont_keys[1]),
                store.sub_for_key(self.cont_keys[2]),
                store.sub_for_key(self.cont_keys[3]),
            ]
        });
        let mut all = MoveList::new();
        generate_pseudo_legal_moves(pos, &mut all);
        for &mv in &all {
            if pos.is_capture(mv) {
                continue;
            }
            let mut score = history.get(us, mv.from(), mv.to()) as i32;
            if let Some(subs) = &cont_subs {
                let pi = pos.moved_piece(mv).index();
                let ti = mv.to().index();
                score += 2 * subs[0][pi][ti] as i32;
                score += 2 * subs[1][pi][ti] as i32;
                score += 2 * subs[2][pi][ti] as i32;
                score += subs[3][pi][ti] as i32;
            }
            self.quiets.push(ScoredMove { mv, score });
        }
    }

    /// Generate every pseudo-legal move when in check. Evasions are
    /// scored so captures come out ahead of quiets — the search relies on
    /// pick-best order for the typical "there's only one way out of
    /// check" case to be tried first.
    fn generate_evasions(
        &mut self,
        pos: &Position,
        history: Option<&ButterflyHistory>,
        cont_history: Option<&ContHistStore>,
    ) {
        self.evasions.clear();
        let us = pos.side_to_move();
        // Stockfish 11 evasion-quiet scoring uses the 1-ply-ago
        // cont-hist sub-table only.
        let cont_sub: Option<&PieceToHistory> =
            cont_history.map(|store| store.sub_for_key(self.cont_keys[0]));
        let mut all = MoveList::new();
        generate_pseudo_legal_moves(pos, &mut all);
        for &mv in &all {
            let score = if pos.is_capture(mv) {
                // Captures: MVV ordering, with the attacker's type as a
                // small tiebreak (prefer capturing with the least valuable
                // piece when two captures land on the same target).
                let victim_mg = captured_piece_value(pos, mv).0;
                let attacker_pt = pos.moved_piece(mv).kind();
                victim_mg - attacker_pt as i32
            } else {
                // Quiets: history + 1-ply cont-hist, pushed below every
                // capture by a large constant so the picker returns
                // captures first.
                let h = history
                    .map(|h| h.get(us, mv.from(), mv.to()) as i32)
                    .unwrap_or(0);
                let c = cont_sub
                    .map(|sub| sub[pos.moved_piece(mv).index()][mv.to().index()] as i32)
                    .unwrap_or(0);
                h + c - EVASION_QUIET_PENALTY
            };
            self.evasions.push(ScoredMove { mv, score });
        }
    }

    /// Iterate captures with pick-best ordering, returning the next
    /// winning capture and shunting losing captures to `bad_captures`.
    fn next_good_capture(&mut self, pos: &Position) -> Option<Move> {
        while self.cur < self.captures.len() {
            let best_idx = pick_best_index(&self.captures, self.cur);
            if best_idx != self.cur {
                self.captures.swap(self.cur, best_idx);
            }
            let entry = self.captures[self.cur];
            self.cur += 1;
            if entry.mv == self.tt_move {
                continue;
            }
            if pos.see_ge(entry.mv, Value::ZERO) {
                return Some(entry.mv);
            }
            self.bad_captures.push(entry);
        }
        None
    }
}
