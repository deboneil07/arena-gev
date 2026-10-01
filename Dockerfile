# ---- Build the WASM engine + web assets ----
FROM rust:latest AS builder
RUN rustup target add wasm32-unknown-unknown

WORKDIR /src
# Dependency manifests first, for better Docker layer caching.
COPY match_engine/Cargo.toml match_engine/Cargo.lock ./
COPY match_engine/src ./src
COPY match_engine/examples ./examples
COPY match_engine/data ./data
COPY match_engine/tests ./tests
COPY match_engine/web ./web

# Install the wasm-bindgen CLI version pinned by Cargo.lock so the
# generated bindings match the wasm-bindgen crate used to build the
# wasm. Prefer the pre-built musl binary (fast); fall back to
# compiling from source if the asset is unavailable.
RUN WBG_VER=$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock \
        | grep -oE 'version = "[0-9.]+"' | grep -oE '[0-9.]+') \
    && ( curl -sSL "https://github.com/rustwasm/wasm-bindgen/releases/download/${WBG_VER}/wasm-bindgen-${WBG_VER}-x86_64-unknown-linux-musl.tar.gz" \
         -o /tmp/wbg.tar.gz \
         && tar xzf /tmp/wbg.tar.gz -C /tmp \
         && install -m755 "/tmp/wasm-bindgen-${WBG_VER}-x86_64-unknown-linux-musl/wasm-bindgen" /usr/local/bin/wasm-bindgen \
       || cargo install wasm-bindgen-cli --version "$WBG_VER" )

RUN cargo build --target wasm32-unknown-unknown --release
RUN wasm-bindgen --target web --out-dir web/pkg \
    target/wasm32-unknown-unknown/release/match_engine.wasm
RUN mkdir -p web/matches && cp data/*.xml web/matches/
RUN cargo run --release --example gen_manifest

# ---- Serve the static web app ----
FROM caddy:alpine
# Render's container runtime refuses to exec a binary that carries
# file capabilities, failing with "exec /usr/bin/caddy: operation not
# permitted". The official caddy image sets cap_net_bind_service on the
# binary so it can bind privileged ports as non-root. We only serve on
# the high port Render injects via $PORT, so strip the capability.
RUN apk add --no-cache libcap-setcap && setcap -r /usr/bin/caddy
COPY --from=builder /src/web /usr/share/caddy
COPY Caddyfile /etc/caddy/Caddyfile
# Render sets PORT at runtime; Caddy listens on it via the Caddyfile.
ENV PORT=80
# Invoke caddy directly. Clearing the base image's ENTRYPOINT and
# spelling out the full command guarantees the process is started
# exactly as written, which is what Render's runtime expects.
ENTRYPOINT []
CMD ["caddy", "run", "--config", "/etc/caddy/Caddyfile", "--adapter", "caddyfile"]
