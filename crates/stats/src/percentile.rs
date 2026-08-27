//! Percentiles by nearest rank.
//!
//! The p50/p90 figures of `docs/SPEC.md` §6 are read off sorted samples
//! without interpolation: the p90 of nine latencies is one of those nine
//! latencies, not a number between two of them that nobody waited. Nearest
//! rank is also the definition under which the p100 is the maximum and the
//! p0 the minimum, which is what a reader expects of the words.

/// The `p`-th percentile of `samples` (`0.0..=1.0`), or `None` when there
/// are no samples to take it from — or when the samples do not order: a
/// `NaN` among them has no rank, and a percentile over it would name a
/// value nobody can place, so none is named.
///
/// Sorts a copy: the caller's order is not touched, and the sample sizes
/// here are a network's worth of orders, not a stream.
pub fn percentile<T: Copy + PartialOrd>(samples: &[T], p: f64) -> Option<T> {
    if samples.is_empty() {
        return None;
    }

    // A sample that does not compare to itself (a NaN) has no rank.
    if samples
        .iter()
        .any(|sample| sample.partial_cmp(sample).is_none())
    {
        return None;
    }

    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("every sample compares"));

    // Nearest rank: the smallest index k such that at least p of the
    // samples are at or below sorted[k]. `ceil(p × n)`, one-based, clamped
    // so that p = 0 still names the minimum.
    let rank = (p.clamp(0.0, 1.0) * sorted.len() as f64).ceil() as usize;
    Some(sorted[rank.max(1) - 1])
}

#[cfg(test)]
mod tests;
