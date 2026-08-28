# Releasing

A release is one command. `cargo release` cuts the version locally and pushes
a tag; the tag is what GitHub Actions turns into a published release.

```sh
cargo install cargo-release    # once
cargo release minor            # a rehearsal: prints, changes nothing
cargo release minor --execute  # the real thing
```

`patch`, `minor` and `major` are the levels; an explicit version
(`cargo release 0.2.0 --execute`) works too.

Without a level, cargo-release ships the version `Cargo.toml` already carries
instead of bumping it. That is how the first release is cut: `docs/ROADMAP.md`
names `v0.1.0` as the first useful one and the manifest is already at `0.1.0`,
so a level here would tag `v0.1.1` and the release the roadmap promised would
never exist.

```sh
cargo release             # a rehearsal of the version in Cargo.toml
cargo release --execute   # tags it as it stands
```

## What the command does

`release.toml` is the configuration. Running from `main`, cargo-release:

1. sets `[workspace.package] version` in `Cargo.toml` to the version being
   released, which both crates inherit — one version for the workspace, never
   two — leaving it as it stands when no level was given;
2. runs `scripts/changelog.sh update <version>`, which prepends the version's
   section to `CHANGELOG.md`, built from the conventional commits since the
   previous tag;
3. commits that as `chore(release): <version>`;
4. tags it `v<version>` and pushes the commit and the tag.

Nothing is published to crates.io: bestiario is a binary, and `publish` is
`false` for both crates.

## What the tag does

Pushing `v<version>` starts `.github/workflows/release.yml`, which:

1. refuses the tag if it does not match the version in `Cargo.toml`;
2. runs `cargo test --workspace --locked` on the tagged commit — a release
   binary is the one artefact nobody re-checks afterwards;
3. builds `bestiario` for `x86_64-unknown-linux-gnu` in release mode;
4. packs the binary with `README.md` and `settings.toml.example` into
   `bestiario-v<version>-x86_64-unknown-linux-gnu.tar.gz`, alongside its
   SHA-256;
5. creates the GitHub release for the tag, its body being the `CHANGELOG.md`
   section for that version — so the release page says exactly what changed —
   with the tarball and the checksum attached.

A version with a pre-release suffix (`0.2.0-rc.1`) is marked as a pre-release
on GitHub. Linux x86_64 is the only target for now; another one is another
entry in the build matrix and nothing else.

## The notes

`scripts/changelog.sh` is the single source of the notes. It reads the commits
since the previous `v*` tag and groups them by conventional-commit type —
`feat:` under Features, `fix:` under Fixes, and so on, with a subject marked
`!` or a body carrying `BREAKING CHANGE` lifted to the top and the
`chore(release)` commit itself left out. The commit log is therefore the thing
to get right; `CHANGELOG.md` is generated from it and should not be edited by
hand.

```sh
scripts/changelog.sh notes 0.2.0            # what the release would say
scripts/changelog.sh notes 0.2.0 v0.1.0     # from an explicit starting point
```

`scripts/check-changelog.sh` tests the derivation against a fixture history
and runs in CI.

## If the release job fails

The tag is already pushed, so fix the cause and re-run the failed job from the
Actions tab. If the tag itself was wrong, delete it locally and on the remote,
drop the release commit, and cut the version again — no release exists until
the workflow creates one.

Do not delete the tag and then re-run the job: `gh release create` is called
with `--verify-tag`, so it stops rather than inventing the tag again at the tip
of `main`, which would attach the binary this run built to a different commit.
