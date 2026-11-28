// SPDX-License-Identifier: GPL-3.0-only
//! Cost samples.

/// A category of events: the count, the total time, and the samples.
#[derive(Clone, Debug, Default)]
pub struct Cat {
    /// Events recorded.
    pub count: u64,
    /// Sum of the times in ns.
    pub total_ns: u64,
    samples: Vec<u32>,
}

impl Cat {
    /// Records one event of `ns` nanoseconds.
    pub fn rec(&mut self, ns: u64) {
        self.count += 1;
        self.total_ns += ns;
        self.samples.push(ns.min(u64::from(u32::MAX)) as u32);
    }

    /// The mean in ns.
    pub fn mean(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.total_ns as f64 / self.count as f64
        }
    }

    /// The `q` quantile (0 to 1) in ns.
    pub fn quantile(&self, q: f64) -> u32 {
        if self.samples.is_empty() {
            return 0;
        }
        let mut s = self.samples.clone();
        s.sort_unstable();
        s[((s.len() - 1) as f64 * q) as usize]
    }

    /// One line: count, mean, p50, p99, max.
    pub fn line(&self) -> String {
        format!(
            "{:>9} events, mean {:>7.0} ns, p50 {:>6} ns, p99 {:>7} ns, max {:>8} ns",
            self.count,
            self.mean(),
            self.quantile(0.5),
            self.quantile(0.99),
            self.quantile(1.0)
        )
    }
}

/// The mean cost of reading the clock, in ns, so a reader can judge the
/// share of a per event sample that is the clock itself.
pub fn clock_overhead_ns() -> f64 {
    let n = 1_000_000u32;
    let t0 = std::time::Instant::now();
    let mut acc = 0u128;
    for _ in 0..n {
        acc += std::time::Instant::now().elapsed().as_nanos();
    }
    std::hint::black_box(acc);
    t0.elapsed().as_nanos() as f64 / f64::from(n) / 2.0
}

/// Resident set size of this process in bytes, from `/proc/self/statm`.
pub fn rss_bytes() -> u64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1).map(str::to_owned))
        .and_then(|p| p.parse::<u64>().ok())
        .map_or(0, |pages| pages * 4096)
}
