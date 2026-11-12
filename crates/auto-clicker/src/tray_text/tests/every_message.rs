//! Every kind of notice and problem has a Swedish form, and quoted text
//! reaches it as it is.

use std::collections::HashSet;
use std::mem::discriminant;

use super::super::{notice, problem};
use super::samples::*;

/// Asserts each of `english` and `swedish` differ, and that quoted text
/// reaches the Swedish as it is.
fn assert_swedish(english: &str, swedish: &str) {
    assert_ne!(swedish, english);
    assert!(!swedish.is_empty(), "{english}");
    assert_eq!(
        swedish.contains(QUOTED),
        english.contains(QUOTED),
        "{english}\n{swedish}"
    );
}

/// How many of `variants` the values cover, by kind.
fn kinds<T>(values: &[T]) -> usize {
    values
        .iter()
        .map(discriminant)
        .collect::<HashSet<_>>()
        .len()
}

#[test]
fn every_notice_has_a_swedish_form_that_keeps_quoted_text() {
    let all = notices();
    // A sample of each kind of notice and of each reason inside one.
    assert_eq!(kinds(&all), 8);
    assert_eq!(kinds(&hindrances()), 11);
    assert_eq!(kinds(&unplayables()), 6);
    assert_eq!(kinds(&stops()), 7);
    assert_eq!(kinds(&savings()), 3);
    for notice in all {
        assert_swedish(&notice.to_string(), &notice::swedish(&notice));
    }
}

#[test]
fn every_problem_has_a_swedish_form_that_keeps_quoted_text() {
    let all = problems();
    assert_eq!(kinds(&all), 7);
    assert_eq!(kinds(&blocked()), 4);
    assert_eq!(kinds(&inputs()), 9);
    assert_eq!(kinds(&unavailables()), 5);
    assert_eq!(kinds(&refusals()), 4);
    for problem in all {
        assert_swedish(&problem.to_string(), &problem::swedish(&problem));
    }
}
