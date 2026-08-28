# syntax=docker/dockerfile:1

# Container image for the App Platform deployment of docs/DEPLOY.md.
#
# Two stages, because the toolchain is roughly a gigabyte and none of it is
# needed to run the daemon. The runtime image carries the binary, the CA
# roots and nothing else.
#
# `migrations/` is deliberately not copied into the runtime stage:
# `sqlx::migrate!` embeds every migration in the binary at compile time
# (src/db/mod.rs), so the directory is a build input, not a runtime one.

# ---------------------------------------------------------------------------
# Builder
# ---------------------------------------------------------------------------
FROM rust:1-bookworm AS builder

WORKDIR /src

# Dependencies first, against stub sources, so that a change under src/ does
# not rebuild the whole dependency graph. Without this, every deploy pays
# several minutes to recompile crates that did not change.
COPY Cargo.toml Cargo.lock ./
COPY crates/stats/Cargo.toml crates/stats/Cargo.toml
RUN mkdir -p src crates/stats/src \
 && echo 'fn main() {}' > src/main.rs \
 && : > src/lib.rs \
 && : > crates/stats/src/lib.rs \
 && cargo build --release --locked \
 && rm -rf src crates

# The real sources. `--locked` so the image is built from the versions
# resolved in Cargo.lock and not from whatever resolves today.
#
# `cargo clean -p` first, and it is not optional: COPY preserves the build
# context's modification times, so every real source file lands *older* than
# the stub artifacts just built from it. Cargo's fingerprint is mtime-based,
# reads that as "already fresh", and ships the stub — a binary that starts,
# prints nothing and exits 0. Dropping our own two packages forces the
# rebuild while leaving every compiled dependency in place, which is the
# whole point of the stage above.
COPY . .
RUN cargo clean --release -p bestiario -p bestiario-stats \
 && cargo build --release --locked --bin bestiario \
 && strip target/release/bestiario

# ---------------------------------------------------------------------------
# Runtime
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim

# reqwest is built against rustls, which carries its own roots, but the relay
# websocket stack may consult the system store. Installing it costs a few
# hundred kilobytes and removes a class of TLS failure that only appears in
# production.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# Unprivileged: the daemon reads relays and writes one SQLite file, and needs
# nothing that root would give it.
RUN useradd --system --create-home --uid 10001 bestiario

COPY --from=builder /src/target/release/bestiario /usr/local/bin/bestiario

# The working directory doubles as the database directory. On App Platform it
# is ephemeral — see docs/DEPLOY.md for what that costs and how to avoid it.
RUN mkdir -p /data && chown bestiario:bestiario /data
WORKDIR /data
USER bestiario

# No settings.toml is shipped. The image is configured entirely through
# BESTIARIO__* (src/config/mod.rs), which is why a missing file at the default
# path is not an error.
ENV BESTIARIO__DATABASE__URL="sqlite:///data/bestiario.db"

ENTRYPOINT ["/usr/local/bin/bestiario"]
CMD ["sync"]
