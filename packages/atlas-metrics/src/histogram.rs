//! Lock-free counters and fixed-bucket histograms.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// Upper bounds (inclusive) for latency histograms measured in milliseconds.
///
/// The final implicit bucket collects observations above the last bound.
pub const LATENCY_MS_BUCKETS: &[u64] = &[
    1, 2, 5, 10, 25, 50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 30_000, 60_000,
];

/// Upper bounds (inclusive) for histograms counting files per run.
pub const FILE_COUNT_BUCKETS: &[u64] = &[
    1, 2, 5, 10, 25, 50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 25_000, 50_000, 100_000,
];

/// Monotonic counter backed by a single atomic.
#[derive(Debug, Default)]
pub struct Counter {
    value: AtomicU64,
}

impl Counter {
    pub const fn new() -> Self {
        Self {
            value: AtomicU64::new(0),
        }
    }

    #[inline]
    pub fn increment(&self) {
        self.add(1);
    }

    #[inline]
    pub fn add(&self, n: u64) {
        self.value.fetch_add(n, Ordering::Relaxed);
    }

    #[inline]
    pub fn value(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }
}

/// One histogram bucket in a snapshot. `upper_bound` is `None` for the
/// overflow bucket that collects values above the last configured bound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistogramBucketSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upper_bound: Option<u64>,
    pub count: u64,
}

/// Point-in-time histogram summary. `p50`/`p90`/`p99` are bucket upper bounds
/// (nearest-rank estimation), which is exact for values on bucket edges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistogramSnapshot {
    pub count: u64,
    pub sum: u64,
    pub min: Option<u64>,
    pub max: Option<u64>,
    pub mean: f64,
    pub p50: u64,
    pub p90: u64,
    pub p99: u64,
    pub buckets: Vec<HistogramBucketSnapshot>,
}

/// Fixed-bucket histogram with lock-free recording.
#[derive(Debug)]
pub struct Histogram {
    bounds: &'static [u64],
    /// One slot per bound plus one overflow slot; length is `bounds.len() + 1`.
    counts: Box<[AtomicU64]>,
    count: AtomicU64,
    sum: AtomicU64,
    /// `u64::MAX` means "no observations yet".
    min: AtomicU64,
    max: AtomicU64,
}

impl Histogram {
    pub fn new(bounds: &'static [u64]) -> Self {
        let counts = (0..=bounds.len())
            .map(|_| AtomicU64::new(0))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            bounds,
            counts,
            count: AtomicU64::new(0),
            sum: AtomicU64::new(0),
            min: AtomicU64::new(u64::MAX),
            max: AtomicU64::new(0),
        }
    }

    /// Record one observation. Negative values are impossible by construction;
    /// callers pass durations and counts as `u64`.
    #[inline]
    pub fn observe(&self, value: u64) {
        let index = self.bucket_index(value);
        // Write derived statistics before incrementing the observation count so
        // a concurrent snapshot that sees `count > 0` also sees a populated
        // min/max (snapshots read `count` first).
        self.sum.fetch_add(value, Ordering::Relaxed);
        self.min.fetch_min(value, Ordering::Relaxed);
        self.max.fetch_max(value, Ordering::Relaxed);
        self.counts[index].fetch_add(1, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> HistogramSnapshot {
        let count = self.count.load(Ordering::Relaxed);
        let sum = self.sum.load(Ordering::Relaxed);
        let max = self.max.load(Ordering::Relaxed);
        let min = self.min.load(Ordering::Relaxed);
        let observed: Vec<u64> = self
            .counts
            .iter()
            .map(|bucket| bucket.load(Ordering::Relaxed))
            .collect();

        let mut buckets = Vec::new();
        for (index, bucket_count) in observed.iter().enumerate() {
            if *bucket_count == 0 {
                continue;
            }
            buckets.push(HistogramBucketSnapshot {
                upper_bound: self.bounds.get(index).copied(),
                count: *bucket_count,
            });
        }

        HistogramSnapshot {
            count,
            sum,
            // Guard the sentinel in case a snapshot races an in-flight first
            // observation that has not yet published `min`.
            min: (count > 0 && min != u64::MAX).then_some(min),
            max: (count > 0).then_some(max),
            mean: if count == 0 {
                0.0
            } else {
                sum as f64 / count as f64
            },
            p50: percentile(self.bounds, &observed, count, max, 50),
            p90: percentile(self.bounds, &observed, count, max, 90),
            p99: percentile(self.bounds, &observed, count, max, 99),
            buckets,
        }
    }

    #[inline]
    fn bucket_index(&self, value: u64) -> usize {
        self.bounds.partition_point(|bound| *bound < value)
    }
}

/// Nearest-rank percentile estimated from fixed buckets. Returns the upper
/// bound of the bucket that contains the requested rank, or the observed max
/// for the overflow bucket. Returns 0 when the histogram is empty.
fn percentile(bounds: &[u64], counts: &[u64], count: u64, max: u64, percentile: u64) -> u64 {
    if count == 0 {
        return 0;
    }
    // Nearest-rank: ceil(percentile / 100 * count), at least 1.
    let target = (count as u128 * percentile as u128).div_ceil(100) as u64;
    let mut cumulative = 0u64;
    for (index, bucket_count) in counts.iter().enumerate() {
        cumulative += *bucket_count;
        if cumulative >= target {
            return bounds.get(index).copied().unwrap_or(max);
        }
    }
    max
}
