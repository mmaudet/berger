# Berger — multi-stage container image.
#
# Stage 1 (builder) compiles a release binary with the full crate.
# Stage 2 (runtime) is a slim Debian image carrying only the binary plus
# the TLS root certificates reqwest needs for HTTPS.

# ---- Stage 1: builder ------------------------------------------------------
FROM rust:1.92-bookworm AS builder

WORKDIR /build

# Copy the manifests first so the dependency graph is cached independently
# of the source: editing src/ alone does not re-resolve or re-fetch crates.
COPY Cargo.toml Cargo.lock ./

# Then the sources, the embedded SQL migrations, and the WebUI templates.
# Askama compiles templates/ at build time, so it must be present.
COPY src ./src
COPY migrations ./migrations
COPY templates ./templates

# Build the release binary. The whole crate is built and tested in CI; the
# image build only needs the binary itself.
RUN cargo build --release --bin berger

# ---- Stage 2: runtime ------------------------------------------------------
FROM debian:bookworm-slim AS runtime

# reqwest with rustls still needs the system trust store to verify the
# Bichon, LLM and webhook endpoints. ca-certificates is the only runtime
# dependency — rusqlite is statically linked (the `bundled` feature).
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Run as an unprivileged user; /data holds the SQLite sidecar.
RUN useradd --create-home --uid 10001 berger \
    && mkdir /data \
    && chown berger:berger /data

COPY --from=builder /build/target/release/berger /usr/local/bin/berger

USER berger
WORKDIR /data

# The WebUI listens on 7000 (PRD §5.7).
EXPOSE 7000

# Default to the daemon; `docker run … berger explain <id>` still works.
ENTRYPOINT ["berger"]
CMD ["run", "--config", "/etc/berger/berger.yaml"]
