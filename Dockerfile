# Build stage. Debian rather than Alpine on purpose: rusqlite's bundled SQLite
# and the rustls crypto provider both compile C, and glibc keeps that boring.
FROM rust:1.98-bookworm AS builder

WORKDIR /build

# Cache dependency compilation separately from source changes.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
RUN mkdir src && \
    echo 'fn main() {}' > src/main.rs && \
    echo '' > src/lib.rs && \
    cargo build --release --locked && \
    rm -rf src

COPY src ./src
# Touch so cargo does not reuse the dummy build artifacts.
RUN touch src/main.rs src/lib.rs && \
    cargo build --release --locked && \
    strip target/release/chess-puzzle-api

# Runtime stage. distroless/cc carries glibc and libgcc and nothing else:
# no shell, no package manager, minimal attack surface.
FROM gcr.io/distroless/cc-debian12

COPY --from=builder /build/target/release/chess-puzzle-api /usr/local/bin/chess-puzzle-api

# The puzzle database is immutable; mount it read-only at runtime.
VOLUME ["/data"]
ENV PUZZLES_DB=/data/puzzles.db \
    API_DB=/data/api.db \
    BIND_ADDR=0.0.0.0:8080

EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/chess-puzzle-api"]
CMD ["serve"]
