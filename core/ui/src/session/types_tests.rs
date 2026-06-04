use super::*;
use chess_tutor_engine::book;
use chess_tutor_engine::opponent::BookSelection;

#[test]
fn any_commits_to_the_full_book() {
    let sel = OpeningSelection::any();
    match sel.to_book() {
        BookSelection::Allowed(ids) => assert_eq!(ids.len(), book::all_ids().len()),
        BookSelection::None => panic!("Any must map to the full Allowed book"),
    }
}

#[test]
fn full_allowed_book_round_trips_back_to_any() {
    let book = BookSelection::Allowed(book::all_ids());
    assert_eq!(OpeningSelection::from_book(&book).mode, OpeningMode::Any);
}

#[test]
fn none_round_trips() {
    let sel = OpeningSelection::from_book(&BookSelection::None);
    assert_eq!(sel.mode, OpeningMode::None);
    assert!(matches!(sel.to_book(), BookSelection::None));
}

#[test]
fn only_a_subset_round_trips_to_only_with_same_ids() {
    let subset: Vec<_> = book::all_ids().into_iter().take(5).collect();
    let sel = OpeningSelection::from_book(&BookSelection::Allowed(subset.clone()));
    assert_eq!(sel.mode, OpeningMode::Only);
    assert_eq!(sel.allowed.len(), 5);
    match sel.to_book() {
        BookSelection::Allowed(ids) => {
            assert_eq!(ids.len(), 5);
            let set: std::collections::HashSet<_> = ids.into_iter().collect();
            assert_eq!(set, subset.into_iter().collect());
        }
        BookSelection::None => panic!("Only with picks must map to Allowed"),
    }
}

#[test]
fn only_empty_maps_to_an_empty_allowed_set() {
    // The engine treats Allowed([]) as "no book" — same as None.
    let sel = OpeningSelection { mode: OpeningMode::Only, allowed: Default::default() };
    assert!(matches!(sel.to_book(), BookSelection::Allowed(ids) if ids.is_empty()));
}
