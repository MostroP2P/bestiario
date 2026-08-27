use super::*;

#[test]
fn the_median_of_an_odd_count_is_the_middle_sample() {
    assert_eq!(percentile(&[30, 10, 20], 0.5), Some(20));
}

#[test]
fn the_median_of_an_even_count_is_the_lower_middle_not_an_average() {
    // Nearest rank: no value is invented between two samples.
    assert_eq!(percentile(&[10, 20, 30, 40], 0.5), Some(20));
}

#[test]
fn the_p90_of_ten_samples_is_the_ninth() {
    let samples: Vec<i64> = (1..=10).collect();

    assert_eq!(percentile(&samples, 0.9), Some(9));
}

#[test]
fn the_extremes_are_the_minimum_and_the_maximum() {
    assert_eq!(percentile(&[5, 1, 9], 0.0), Some(1));
    assert_eq!(percentile(&[5, 1, 9], 1.0), Some(9));
}

#[test]
fn a_single_sample_is_every_percentile() {
    assert_eq!(percentile(&[7], 0.5), Some(7));
    assert_eq!(percentile(&[7], 0.9), Some(7));
}

#[test]
fn no_samples_is_no_percentile() {
    assert_eq!(percentile::<i64>(&[], 0.5), None);
}

#[test]
fn the_input_is_left_in_its_own_order() {
    let samples = [3, 1, 2];

    percentile(&samples, 0.5);

    assert_eq!(samples, [3, 1, 2]);
}

#[test]
fn fiat_amounts_take_percentiles_too() {
    assert_eq!(percentile(&[10.5, 2.0, 7.25], 0.5), Some(7.25));
}

#[test]
fn samples_that_do_not_order_have_no_percentile() {
    // A NaN has no rank; rather than a panic or an arbitrary neighbour,
    // there is no percentile.
    assert_eq!(percentile(&[f64::NAN, 1.0], 0.5), None);
    assert_eq!(percentile(&[1.0, f64::NAN, 2.0], 0.9), None);
    assert_eq!(percentile(&[f64::NAN], 0.5), None);
}
