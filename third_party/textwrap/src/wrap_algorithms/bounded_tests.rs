use super::{wrap_first_fit, wrap_first_fit_with};
use crate::core::{Fragment, Word};
use std::cell::Cell;
use std::ops::ControlFlow;

#[test]
fn callback_and_full_collection_keep_known_first_fit_boundaries() {
    let words = [Word::from("aa "), Word::from("bb "), Word::from("cccc "), Word::from("d")];
    for (widths, lengths) in [
        (vec![5.0], vec![2, 1, 1]),
        (vec![2.0, 6.0], vec![1, 1, 2]),
        (vec![0.0], vec![1, 1, 1, 1]),
        (vec![], vec![1, 1, 1, 1]),
    ] {
        let mut offset = 0;
        let expected: Vec<_> = lengths.iter().map(|length| {
            let line = &words[offset..offset + length];
            offset += length;
            line
        }).collect();
        assert_eq!(wrap_first_fit(&words, &widths), expected);
        let mut emitted = Vec::new();
        wrap_first_fit_with(&words, &widths, |line| {
            emitted.push(line);
            ControlFlow::Continue(())
        });
        assert_eq!(emitted, expected);
        for limit in 1..=5 {
            let mut prefix = Vec::new();
            wrap_first_fit_with(&words, &widths, |line| {
                prefix.push(line);
                if prefix.len() == limit {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            });
            assert_eq!(prefix, expected.iter().copied().take(limit).collect::<Vec<_>>());
        }
    }
}

#[test]
fn penalty_is_included_when_deciding_the_boundary() {
    let words = [
        Word::from("aa "),
        Word { penalty: "-", ..Word::from("bb ") },
        Word::from("c"),
    ];
    let expected = vec![&words[..1], &words[1..]];
    assert_eq!(wrap_first_fit(&words, &[5.0]), expected);
    let mut prefix = Vec::new();
    wrap_first_fit_with(&words, &[5.0], |line| {
        prefix.push(line);
        ControlFlow::Break(())
    });
    assert_eq!(prefix, vec![&words[..1]]);
}

#[derive(Debug)]
struct ObservedFragment {
    calls: Cell<usize>,
}

impl Fragment for ObservedFragment {
    fn width(&self) -> f64 {
        self.calls.set(self.calls.get() + 1);
        1.0
    }

    fn whitespace_width(&self) -> f64 {
        self.calls.set(self.calls.get() + 1);
        1.0
    }

    fn penalty_width(&self) -> f64 {
        self.calls.set(self.calls.get() + 1);
        0.0
    }
}

#[test]
fn break_stops_emission_and_fragment_work_at_one_lookahead() {
    let fragments: Vec<_> = (0..10_000).map(|_| ObservedFragment {
        calls: Cell::new(0),
    }).collect();
    let mut emitted = 0;
    wrap_first_fit_with(&fragments, &[1.0], |line| {
        assert!(std::ptr::eq(line, &fragments[emitted..emitted + 1]));
        emitted += 1;
        assert!(emitted <= 7);
        if emitted == 7 {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    assert_eq!(emitted, 7);
    assert!(fragments[..7].iter().all(|fragment| fragment.calls.get() == 4));
    assert_eq!(fragments[7].calls.get(), 2);
    assert!(fragments[8..].iter().all(|fragment| fragment.calls.get() == 0));
}

#[test]
fn empty_input_emits_one_empty_line() {
    let words: [Word<'_>; 0] = [];
    let mut calls = 0;
    wrap_first_fit_with(&words, &[], |line| {
        assert!(line.is_empty());
        calls += 1;
        ControlFlow::Break(())
    });
    assert_eq!(calls, 1);
    assert_eq!(wrap_first_fit(&words, &[]), vec![&words[..]]);
}
