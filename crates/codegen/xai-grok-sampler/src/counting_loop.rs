//! Dialect-shared detector for counter-only / identical-tail generation loops.
//! Copied into the sampler so Chat Completions and Messages abort the same
//! way the gateway does. Keep in lockstep with
//! `sigma-crates/infra/gateway/src/routing/counting_loop.rs` core.

/// Consecutive identical counter-only or identical-tail tokens before abort.
pub const COUNTING_LOOP_BOUND: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountingLoopObservation {
    Continue,
    Abort,
}

#[derive(Debug, Clone, Default)]
pub struct CountingLoopDetector {
    last_token: Option<LoopToken>,
    streak: u32,
    saw_non_loop: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LoopToken {
    Counter { k: u32, n: u32 },
    Identical { text: String },
}

impl CountingLoopDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed visible output text (one SSE delta, one buffered chunk, or a
    /// concatenation of consecutive identical fragments).
    pub fn observe(&mut self, text: &str) -> CountingLoopObservation {
        for token in tokenize_visible(text) {
            match self.ingest(token) {
                CountingLoopObservation::Abort => return CountingLoopObservation::Abort,
                CountingLoopObservation::Continue => {}
            }
        }
        CountingLoopObservation::Continue
    }

    fn ingest(&mut self, token: LoopToken) -> CountingLoopObservation {
        match &token {
            LoopToken::Identical { text } if !is_identical_tail(text) => {
                self.saw_non_loop = true;
                self.last_token = None;
                self.streak = 0;
                return CountingLoopObservation::Continue;
            }
            LoopToken::Counter { .. } | LoopToken::Identical { .. } => {}
        }
        match &self.last_token {
            Some(prev) if prev.same_loop_as(&token) => {
                self.streak = self.streak.saturating_add(1);
                self.last_token = Some(token);
            }
            _ => {
                self.last_token = Some(token);
                self.streak = 1;
                return CountingLoopObservation::Continue;
            }
        }
        if self.streak >= COUNTING_LOOP_BOUND {
            CountingLoopObservation::Abort
        } else {
            CountingLoopObservation::Continue
        }
    }
}

impl LoopToken {
    fn same_loop_as(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Counter { k: k0, n: n0 },
                Self::Counter { k: k1, n: n1 },
            ) => {
                // Same fraction repeated (`197/197. 197/197.`) or a strictly
                // incrementing counter (`1/6. 2/6.` / `1/1. 2/2.`).
                (*k0 == *k1 && *n0 == *n1)
                    || (*k1 == k0.saturating_add(1)
                        && (*n1 == *n0 || *n1 == n0.saturating_add(1)))
            }
            (Self::Identical { text: a }, Self::Identical { text: b }) => a == b,
            _ => false,
        }
    }
}

fn tokenize_visible(text: &str) -> Vec<LoopToken> {
    let mut words: Vec<String> = Vec::new();
    let mut counters: Vec<LoopToken> = Vec::new();
    for raw in text.split_whitespace() {
        let trimmed = raw.trim_matches(|c: char| {
            matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | '"' | '\'' | '`')
        });
        if trimmed.is_empty() {
            continue;
        }
        if let Some(counter) = parse_counter(trimmed) {
            counters.push(counter);
            continue;
        }
        words.push(trimmed.to_ascii_lowercase());
    }
    if counters.is_empty()
        && !words.is_empty()
        && words.iter().all(|word| is_identical_tail(word))
    {
        return vec![LoopToken::Identical {
            text: words.join(" "),
        }];
    }
    let mut out = counters;
    for word in words {
        out.push(LoopToken::Identical { text: word });
    }
    out
}

fn parse_counter(token: &str) -> Option<LoopToken> {
    let (left, right) = token.split_once('/')?;
    let k: u32 = left.parse().ok()?;
    let n: u32 = right.parse().ok()?;
    (k > 0 && n > 0).then_some(LoopToken::Counter { k, n })
}

fn is_identical_tail(token: &str) -> bool {
    let mut any = false;
    token.split_whitespace().all(|word| {
        any = true;
        matches!(word, "owned" | "0" | "complete" | "remaining" | "sites")
    }) && any
}


#[cfg(test)]
mod tests {
    use super::{COUNTING_LOOP_BOUND, CountingLoopDetector, CountingLoopObservation};

    fn feed(text: &str) -> CountingLoopObservation {
        let mut detector = CountingLoopDetector::new();
        detector.observe(text)
    }

    #[test]
    fn incrementing_counter_aborts_at_bound() {
        let mut detector = CountingLoopDetector::new();
        let mut last = CountingLoopObservation::Continue;
        for k in 1..=COUNTING_LOOP_BOUND + 2 {
            last = detector.observe(&format!("{k}/{k}. "));
        }
        assert_eq!(last, CountingLoopObservation::Abort);
    }

    #[test]
    fn repeated_identical_owned_aborts() {
        let mut detector = CountingLoopDetector::new();
        let mut last = CountingLoopObservation::Continue;
        for _ in 0..COUNTING_LOOP_BOUND {
            last = detector.observe("0 owned. ");
        }
        assert_eq!(last, CountingLoopObservation::Abort);
    }

    #[test]
    fn distinct_numbered_list_does_not_abort() {
        assert_eq!(
            feed("1. foo 2. bar 3. baz 4. quux 5. xyzzy 6. thud 7. plugh 8. waldo 9. fred"),
            CountingLoopObservation::Continue
        );
    }
}
