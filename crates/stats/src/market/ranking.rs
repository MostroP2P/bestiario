//! Rankings and concentration — the fiat and payment-method rows of
//! `docs/SPEC.md` §6.3.
//!
//! A ranking is a list of `(key, weight)` in descending weight, ties by
//! key, with the two concentration figures read off it: the share of the
//! top three, and the Herfindahl–Hirschman index `∑ share²`, which is `1`
//! for a single key and `1/n` for `n` equal ones.

use std::collections::BTreeMap;

/// Keys by weight, heaviest first.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Ranking {
    pub entries: Vec<(String, i64)>,
    /// `∑` of the top three weights over the total; `0` with nothing ranked.
    pub top3_share: f64,
    /// `∑ (weight / total)²`; `0` with nothing ranked.
    pub hhi: f64,
}

impl Ranking {
    /// Ranks `weights`, dropping zero and negative ones: a key with no
    /// weight has no place in a ranking of weights.
    pub fn new(weights: BTreeMap<String, i64>) -> Self {
        let mut entries: Vec<(String, i64)> = weights
            .into_iter()
            .filter(|(_, weight)| *weight > 0)
            .collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        let total: i64 = entries.iter().map(|(_, weight)| weight).sum();
        let share = |weight: i64| weight as f64 / total as f64;
        let (top3_share, hhi) = if total > 0 {
            (
                entries.iter().take(3).map(|(_, w)| share(*w)).sum(),
                entries.iter().map(|(_, w)| share(*w).powi(2)).sum(),
            )
        } else {
            (0.0, 0.0)
        };

        Self {
            entries,
            top3_share,
            hhi,
        }
    }

    /// Whether nothing was ranked — no key carried any weight, so the
    /// concentration figures describe nothing and the cell is `—`.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The top three as `key weight[ unit], …`, for a text cell.
    pub fn top3(&self, unit: &str) -> String {
        self.entries
            .iter()
            .take(3)
            .map(|(key, weight)| format!("{key} {weight}{unit}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Sums `weight(order)` per key, where one order may name several keys
/// (its payment methods).
pub fn tally<'a>(
    orders: impl Iterator<Item = &'a super::Order>,
    keys: impl Fn(&super::Order) -> Vec<String>,
    weight: impl Fn(&super::Order) -> i64,
) -> Ranking {
    let mut weights: BTreeMap<String, i64> = BTreeMap::new();
    for order in orders {
        for key in keys(order) {
            *weights.entry(key).or_default() += weight(order);
        }
    }
    Ranking::new(weights)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ranking_is_heaviest_first_ties_by_key() {
        let ranking = Ranking::new(BTreeMap::from([
            ("b".to_string(), 2),
            ("a".to_string(), 2),
            ("c".to_string(), 5),
            ("zero".to_string(), 0),
        ]));

        assert_eq!(
            ranking.entries,
            vec![
                ("c".to_string(), 5),
                ("a".to_string(), 2),
                ("b".to_string(), 2)
            ]
        );
    }

    #[test]
    fn concentration_is_one_for_a_single_key_and_one_over_n_for_equal_ones() {
        let single = Ranking::new(BTreeMap::from([("a".to_string(), 7)]));
        assert_eq!(single.hhi, 1.0);
        assert_eq!(single.top3_share, 1.0);

        let equal = Ranking::new(BTreeMap::from([
            ("a".to_string(), 1),
            ("b".to_string(), 1),
            ("c".to_string(), 1),
            ("d".to_string(), 1),
        ]));
        assert!((equal.hhi - 0.25).abs() < 1e-12);
        assert!((equal.top3_share - 0.75).abs() < 1e-12);
    }

    #[test]
    fn nothing_ranked_is_empty_with_zero_concentration() {
        let ranking = Ranking::new(BTreeMap::new());

        assert!(ranking.is_empty());
        assert_eq!(ranking.hhi, 0.0);
        assert_eq!(ranking.top3(""), "");
    }

    #[test]
    fn the_text_form_is_the_top_three_with_the_unit() {
        let ranking = Ranking::new(BTreeMap::from([
            ("ARS".to_string(), 140_000),
            ("USD".to_string(), 20_000),
        ]));

        assert_eq!(ranking.top3(" sats"), "ARS 140000 sats, USD 20000 sats");
    }
}
