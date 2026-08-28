//! What the file may say, and what a secret prints as.

use super::*;

fn parse(toml: &str) -> Result<SecretRef, toml::de::Error> {
    #[derive(Deserialize)]
    struct Holder {
        nsec: SecretRef,
    }
    toml::from_str::<Holder>(toml).map(|holder| holder.nsec)
}

#[test]
fn a_reference_carries_the_variables_name_and_not_its_value() {
    let reference = parse(r#"nsec = "env:BESTIARIO_PUBLISH_NSEC""#).expect("a reference");

    assert_eq!(
        reference,
        SecretRef::Env("BESTIARIO_PUBLISH_NSEC".to_string())
    );
    assert_eq!(reference.describe(), "env:BESTIARIO_PUBLISH_NSEC");
}

#[test]
fn a_key_written_into_the_file_is_refused_rather_than_quietly_working() {
    // The whole point: a setup that works with the key in the file is a
    // setup nobody revisits, and the file is in every backup.
    let error =
        parse(r#"nsec = "nsec1vl029mgpspedva04g90vltkh6fvh240zqtv9k0t9af8935ke9laqsnlfe5""#)
            .expect_err("a literal key");

    assert!(
        error.to_string().contains("env:NAME") && error.to_string().contains("file:PATH"),
        "the message has to say what to write instead, both forms: {error}"
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

    assert_eq!(reference.describe(), "env:BESTIARIO_PUBLISH_NSEC");
}

fn unreadable(_: &Path) -> std::io::Result<String> {
    Err(std::io::Error::other("no test reads a file through this"))
}

#[test]
fn a_variable_that_is_not_set_reads_as_nothing() {
    let reference = parse(r#"nsec = "env:NEVER_SET""#).expect("a reference");

    assert_eq!(
        reference.read(|_| None, unreadable),
        Err(Unresolved::NotSet)
    );
}

#[test]
fn a_variable_that_is_set_reads_as_its_value() {
    let reference = parse(r#"nsec = "env:SET""#).expect("a reference");

    let secret = reference
        .read(
            |name| (name == "SET").then(|| "the key".to_string()),
            unreadable,
        )
        .expect("set");

    assert_eq!(secret.expose(), "the key");
}

// ---- the file a secret is mounted as (§12)

#[test]
fn a_path_is_a_reference_too_and_prints_as_one() {
    let reference = parse(r#"nsec = "file:/run/secrets/bestiario-nsec""#).expect("a reference");

    assert_eq!(
        reference,
        SecretRef::File(PathBuf::from("/run/secrets/bestiario-nsec"))
    );
    // A path is not a secret, so it is safe to say which one failed.
    assert_eq!(reference.describe(), "file:/run/secrets/bestiario-nsec");
}

#[test]
fn a_file_reads_as_its_contents_without_the_newline_echo_leaves() {
    // A key one invisible byte off from the operator's is the least
    // debuggable failure this could have.
    let reference = parse(r#"nsec = "file:/run/secrets/nsec""#).expect("a reference");

    let secret = reference
        .read(
            |_| panic!("a file reference reads no variable"),
            |path| {
                assert_eq!(path, Path::new("/run/secrets/nsec"));
                Ok("the key\n".to_string())
            },
        )
        .expect("readable");

    assert_eq!(secret.expose(), "the key");
}

#[test]
fn a_file_that_cannot_be_read_says_why_and_never_quotes_it() {
    let reference = parse(r#"nsec = "file:/run/secrets/absent""#).expect("a reference");

    let error = reference
        .read(
            |_| None,
            |_| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no such file",
                ))
            },
        )
        .expect_err("absent");

    assert!(
        error.to_string().contains("no such file"),
        "the operator needs the reason: {error}"
    );
}

#[test]
fn a_prefix_naming_no_file_is_refused() {
    parse(r#"nsec = "file:""#).expect_err("names nothing");
    parse(r#"nsec = "file:   ""#).expect_err("names nothing but spaces");
}

#[test]
fn a_secret_never_prints_itself() {
    let secret = Secret("nsec1supersecret".to_string());

    assert_eq!(format!("{secret:?}"), REDACTED);
}
