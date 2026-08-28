//! What the file may say, and what a secret prints as.

use super::*;

fn parse(toml: &str) -> Result<EnvRef, toml::de::Error> {
    #[derive(Deserialize)]
    struct Holder {
        nsec: EnvRef,
    }
    toml::from_str::<Holder>(toml).map(|holder| holder.nsec)
}

#[test]
fn a_reference_carries_the_variables_name_and_not_its_value() {
    let reference = parse(r#"nsec = "env:BESTIARIO_PUBLISH_NSEC""#).expect("a reference");

    assert_eq!(reference.name(), "BESTIARIO_PUBLISH_NSEC");
}

#[test]
fn a_key_written_into_the_file_is_refused_rather_than_quietly_working() {
    // The whole point: a setup that works with the key in the file is a
    // setup nobody revisits, and the file is in every backup.
    let error =
        parse(r#"nsec = "nsec1vl029mgpspedva04g90vltkh6fvh240zqtv9k0t9af8935ke9laqsnlfe5""#)
            .expect_err("a literal key");

    assert!(
        error.to_string().contains("env:NAME"),
        "the message has to say what to write instead: {error}"
    );
}

#[test]
fn a_prefix_naming_no_variable_is_refused() {
    parse(r#"nsec = "env:""#).expect_err("names nothing");
    parse(r#"nsec = "env:   ""#).expect_err("names nothing but spaces");
}

#[test]
fn surrounding_whitespace_is_not_part_of_a_variables_name() {
    let reference = parse("nsec = \"env: BESTIARIO_PUBLISH_NSEC \"").expect("a reference");

    assert_eq!(reference.name(), "BESTIARIO_PUBLISH_NSEC");
}

#[test]
fn a_variable_that_is_not_set_reads_as_nothing() {
    let reference = parse(r#"nsec = "env:NEVER_SET""#).expect("a reference");

    assert_eq!(reference.read(|_| None), None);
}

#[test]
fn a_variable_that_is_set_reads_as_its_value() {
    let reference = parse(r#"nsec = "env:SET""#).expect("a reference");

    let secret = reference
        .read(|name| (name == "SET").then(|| "the key".to_string()))
        .expect("set");

    assert_eq!(secret.expose(), "the key");
}

#[test]
fn a_secret_never_prints_itself() {
    let secret = Secret("nsec1supersecret".to_string());

    assert_eq!(format!("{secret:?}"), REDACTED);
}
