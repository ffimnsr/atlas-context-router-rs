//! Labeled metric families with lock-protected label registration.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::histogram::{Counter, Histogram, HistogramSnapshot};

/// Recover a poisoned lock instead of propagating panics from metric code.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Counter family keyed by a label such as build kind or query mode.
#[derive(Debug, Default)]
pub struct LabeledCounter {
    inner: Mutex<HashMap<String, Arc<Counter>>>,
}

impl LabeledCounter {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    #[inline]
    pub fn increment(&self, label: &str) {
        self.counter(label).increment();
    }

    #[inline]
    pub fn add(&self, label: &str, n: u64) {
        self.counter(label).add(n);
    }

    pub fn snapshot(&self) -> BTreeMap<String, u64> {
        lock(&self.inner)
            .iter()
            .map(|(label, counter)| (label.clone(), counter.value()))
            .collect()
    }

    fn counter(&self, label: &str) -> Arc<Counter> {
        let mut map = lock(&self.inner);
        Arc::clone(
            map.entry(label.to_owned())
                .or_insert_with(|| Arc::new(Counter::new())),
        )
    }
}

/// Histogram family keyed by a label such as tool name or query mode.
#[derive(Debug)]
pub struct LabeledHistogram {
    bounds: &'static [u64],
    inner: Mutex<HashMap<String, Arc<Histogram>>>,
}

impl LabeledHistogram {
    pub fn new(bounds: &'static [u64]) -> Self {
        Self {
            bounds,
            inner: Mutex::new(HashMap::new()),
        }
    }

    #[inline]
    pub fn observe(&self, label: &str, value: u64) {
        self.histogram(label).observe(value);
    }

    pub fn snapshot(&self) -> BTreeMap<String, HistogramSnapshot> {
        lock(&self.inner)
            .iter()
            .map(|(label, histogram)| (label.clone(), histogram.snapshot()))
            .collect()
    }

    fn histogram(&self, label: &str) -> Arc<Histogram> {
        let mut map = lock(&self.inner);
        Arc::clone(
            map.entry(label.to_owned())
                .or_insert_with(|| Arc::new(Histogram::new(self.bounds))),
        )
    }
}
