use super::*;
use chess_tutor_engine::book;
use chess_tutor_engine::opponent::BookSelection;

#[test]
fn any_is_the_full_book_and_commits_to_it() {
    let sel = OpeningSelection::any();
    assert_eq!(sel.allowed.len(), book::all_ids().len());
    match sel.to_book() {
        BookSelection::Allowed(ids) => assert_eq!(ids.len(), book::all_ids().len()),
        BookSelection::None => panic!("the full book must commit to Allowed"),
    }
}

#[test]
fn full_allowed_book_round_trips() {
    let sel = OpeningSelection::from_book(&BookSelection::Allowed(book::all_ids()));
    assert_eq!(sel.allowed.len(), book::all_ids().len());
}

#[test]
fn none_round_trips() {
    let sel = OpeningSelection::from_book(&BookSelection::None);
    assert!(sel.allowed.is_empty());
    assert!(matches!(sel.to_book(), BookSelection::None));
}

#[test]
fn a_subset_round_trips() {
    let subset: Vec<_> = book::all_ids().into_iter().take(5).collect();
    let sel = OpeningSelection::from_book(&BookSelection::Allowed(subset.clone()));
    assert_eq!(sel.allowed.len(), 5);
    match sel.to_book() {
        BookSelection::Allowed(ids) => {
            let set: std::collections::HashSet<_> = ids.into_iter().collect();
            assert_eq!(set, subset.into_iter().collect());
        }
        BookSelection::None => panic!("a non-empty subset must commit to Allowed"),
    }
}

#[test]
fn empty_selection_commits_to_none() {
    let sel = OpeningSelection::default();
    assert!(matches!(sel.to_book(), BookSelection::None));
}
