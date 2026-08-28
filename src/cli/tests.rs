//! Parsing tests for the CLI surface.
//!
//! These drive `clap` directly rather than spawning the binary: the thing
//! under test is whether an invocation is accepted and what it parses to, and
//! a subprocess would only add startup cost to the same answer.

use clap::CommandFactory;

use super::*;

fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(std::iter::once("bestiario").chain(args.iter().copied()))
}

fn expect_parse(args: &[&str]) -> Cli {
    parse(args).unwrap_or_else(|e| panic!("`bestiario {}` should parse:\n{e}", args.join(" ")))
}

#[test]
fn the_command_tree_is_internally_consistent() {
    // Catches duplicated short flags, conflicting names and malformed
    // defaults, which otherwise only surface at runtime.
    Cli::command().debug_assert();
}

#[test]
fn every_invocation_documented_in_the_spec_parses() {
    // The command list of docs/SPEC.md §10, verbatim. If the spec and the CLI
    // ever disagree, this is where it shows.
    let documented: &[&[&str]] = &[
        &["backfill"],
        &["backfill", "--from", "1735689600"],
        &["backfill", "--until", "1767225600"],
        &["backfill", "--kind", "38383"],
        &["sync"],
        &["summary"],
        &["instances"],
        &["instance", "lnp2pbot"],
        &["compare"],
        &["series", "orders.completed"],
        &["series", "volume.sats", "--by", "month"],
        &[
            "series",
            "volume.sats",
            "--by",
            "week",
            "--split",
            "instance",
        ],
        &["market", "ARS"],
        &["stats", "orders"],
        &["stats", "orders", "--by", "status"],
        &["stats", "volume", "--by", "fiat"],
        &["stats", "volume", "--in", "USD"],
        &["stats", "market", "--by", "fiat"],
        &["stats", "timing", "--by", "method"],
        &["stats", "dev-fees", "--by", "instance"],
        &["stats", "disputes", "--by", "initiator"],
        &["stats", "rates", "--fiat", "ARS"],
        &["orders", "308e1b34-3d4e-4b2f-8b1e-0f6d5a2c9e77"],
        &["rebuild"],
    ];

    for args in documented {
        expect_parse(args);
    }
}

#[test]
fn help_renders_for_every_subcommand() {
    // A subcommand whose help panics is a subcommand nobody can discover.
    let mut command = Cli::command();
    command.build();

    for subcommand in command.get_subcommands_mut() {
        let rendered = subcommand.render_long_help().to_string();
        assert!(
            !rendered.is_empty(),
            "`{}` renders empty help",
            subcommand.get_name()
        );
    }
}

#[test]
fn global_flags_are_accepted_after_the_subcommand() {
    // `bestiario summary --json` is how anyone would actually type it.
    let cli = expect_parse(&["summary", "--json", "--instance", "lnp2pbot"]);

    assert!(cli.json);
    assert_eq!(cli.instance.as_deref(), Some("lnp2pbot"));
}

#[test]
fn a_window_bound_accepts_a_unix_timestamp() {
    let cli = expect_parse(&["summary", "--from", "1735689600"]);

    assert_eq!(cli.from, Some(1_735_689_600));
}

#[test]
fn a_window_bound_accepts_a_date_and_reads_it_as_midnight_utc() {
    // 2025-01-01T00:00:00Z, the value docs/SPEC.md §9 uses for backfill_from.
    let cli = expect_parse(&["summary", "--from", "2025-01-01"]);

    assert_eq!(cli.from, Some(1_735_689_600));
}

#[test]
fn both_window_bounds_parse_the_same_way() {
    let cli = expect_parse(&["summary", "--from", "2025-01-01", "--until", "2026-01-01"]);

    assert_eq!(cli.from, Some(1_735_689_600));
    assert_eq!(cli.until, Some(1_767_225_600));
}

#[test]
fn a_window_bound_that_is_neither_a_timestamp_nor_a_date_is_rejected() {
    for value in ["yesterday", "2025-13-01", "01-01-2025", "2025/01/01"] {
        let error = parse(&["summary", "--from", value]).expect_err(value);

        assert!(
            error.to_string().contains(value),
            "the error for `{value}` should name it: {error}"
        );
    }
}

#[test]
fn a_window_bound_before_the_epoch_is_rejected() {
    // Negative timestamps parse as integers but mean nothing here, and would
    // silently widen the window rather than fail.
    // Written as `--from=-1`, since a bare `-1` is a flag as far as clap is
    // concerned and never reaches the value parser.
    let error = parse(&["summary", "--from=-1"]).expect_err("negative timestamp");

    assert!(error.to_string().contains("epoch"), "{error}");
}

#[test]
fn an_unknown_dimension_is_rejected_with_the_accepted_values() {
    // The point of typed dimensions: `--by fiatt` must fail here rather than
    // produce an empty report further down.
    let error = parse(&["stats", "orders", "--by", "fiatt"]).expect_err("bad dimension");
    let message = error.to_string();

    assert!(message.contains("fiatt"), "{message}");
    assert!(
        message.contains("status"),
        "should list the alternatives: {message}"
    );
}

#[test]
fn a_dimension_that_does_not_apply_to_a_family_is_rejected() {
    // Volume has no `status` slice — every order counted is already a
    // completed one, so the dimension would be a constant.
    parse(&["stats", "volume", "--by", "status"]).expect_err("status is not a volume dimension");
}

#[test]
fn stats_families_have_the_documented_defaults() {
    // No `--by` is the global report, not a slice by some default dimension.
    let cli = expect_parse(&["stats", "orders"]);
    assert!(matches!(
        cli.command,
        Command::Stats(StatsCommand::Orders { by: None })
    ));

    let cli = expect_parse(&["stats", "orders", "--by", "weekday"]);
    assert!(matches!(
        cli.command,
        Command::Stats(StatsCommand::Orders {
            by: Some(OrderDimension::Weekday)
        })
    ));

    let cli = expect_parse(&["stats", "volume"]);
    assert!(matches!(
        cli.command,
        Command::Stats(StatsCommand::Volume {
            by: None,
            convert_to: None
        })
    ));
}

#[test]
fn series_defaults_to_monthly_buckets_with_no_split() {
    let cli = expect_parse(&["series", "orders.completed"]);

    match cli.command {
        Command::Series { metric, by, split } => {
            assert_eq!(metric, "orders.completed");
            assert_eq!(by, Period::Month);
            assert_eq!(split, None);
        }
        other => panic!("expected a series command, got {other:?}"),
    }
}

#[test]
fn an_omitted_config_flag_is_distinguishable_from_an_explicit_one() {
    // The flag is an Option rather than a defaulted path so that
    // `--config settings.toml` stays distinguishable from no flag at all:
    // only the omission may tolerate a missing file.
    assert_eq!(expect_parse(&["summary"]).config, None);
    assert_eq!(
        expect_parse(&["summary", "--config", "/etc/bestiario.toml"]).config,
        Some(std::path::PathBuf::from("/etc/bestiario.toml"))
    );
    assert_eq!(
        expect_parse(&["summary", "--config", "settings.toml"]).config,
        Some(std::path::PathBuf::from("settings.toml")),
        "naming the default path explicitly is still an explicit path"
    );
}

#[test]
fn verbosity_counts_repeats() {
    assert_eq!(expect_parse(&["summary"]).verbose, 0);
    assert_eq!(expect_parse(&["summary", "-v"]).verbose, 1);
    assert_eq!(expect_parse(&["summary", "-vv"]).verbose, 2);
}

#[test]
fn a_subcommand_is_required() {
    // Running the bare binary should print help, not do something arbitrary.
    parse(&[]).expect_err("a bare invocation should not parse");
}

#[test]
fn an_unknown_subcommand_is_rejected() {
    parse(&["summarise"]).expect_err("unknown subcommand");
}
