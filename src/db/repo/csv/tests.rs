//! Round-tripping the values instances actually publish.

use super::*;

/// What `join` writes and `split` reads has to be the same list.
fn round_trip(values: &[&str]) {
    let owned: Vec<String> = values.iter().map(|value| value.to_string()).collect();

    assert_eq!(split(&join(&owned)), owned, "{values:?}");
}

#[test]
fn ordinary_methods_stay_plain_csv() {
    // Arrange
    let values = vec!["revolut".to_string(), "sepa".to_string()];

    // Act
    let encoded = join(&values);

    // Assert — no quoting where none is needed, so the column stays readable.
    assert_eq!(encoded, "revolut,sepa");
    assert_eq!(split(&encoded), values);
}

#[test]
fn a_method_containing_a_comma_survives() {
    // `pm` carries methods as separate tag values, so a comma inside one is
    // legal on the wire. Joined unquoted it would read back as two methods and
    // corrupt the payment-method statistics of SPEC 6.4.
    let values = vec!["Banco, en efectivo".to_string(), "sepa".to_string()];

    let encoded = join(&values);

    assert_eq!(encoded, "\"Banco, en efectivo\",sepa");
    assert_eq!(split(&encoded), values);
}

#[test]
fn a_method_containing_a_quote_survives() {
    round_trip(&["pago \"en mano\"", "revolut"]);
}

#[test]
fn a_method_that_is_only_a_quote_survives() {
    round_trip(&["\""]);
}

#[test]
fn a_method_with_both_a_comma_and_a_quote_survives() {
    round_trip(&["Zelle, \"solo\" titular"]);
}

#[test]
fn padding_is_part_of_what_was_published() {
    // The parser rejects a blank method but keeps a padded one verbatim, so
    // unquoted csv would silently trim what an instance actually published.
    round_trip(&["  face to face  "]);
}

#[test]
fn a_real_captured_method_survives() {
    round_trip(&[
        "QR Santander",
        "BBVA Efectivo Móvil",
        "Sabadell Instant Money. Códigos al teléfono del comprador.",
    ]);
}

#[test]
fn a_single_method_is_its_own_field() {
    round_trip(&["face to face"]);
}

#[test]
fn no_methods_encode_to_nothing_and_back() {
    assert_eq!(join(&[]), "");
    assert_eq!(split(""), Vec::<String>::new());
}

#[test]
fn a_column_of_only_separators_yields_no_methods() {
    // Not four methods named after nothing.
    assert_eq!(split(",,,"), Vec::<String>::new());
}

#[test]
fn unquoted_fields_are_trimmed_of_the_padding_csv_adds() {
    // A human editing the database by hand writes `a, b`; both readings agree
    // that the second method is `b`.
    assert_eq!(split("revolut, sepa"), vec!["revolut", "sepa"]);
}

#[test]
fn a_quote_inside_an_unquoted_field_is_just_a_character() {
    assert_eq!(split("5\" nail"), vec!["5\" nail"]);
}
