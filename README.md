# arena-gev — God's Eye View

A Rust football match simulation engine fed by Opta F24 XML, with a
WASM-powered web UI that renders all 22 players and the ball on an
HTML5 canvas from a zero-copy render buffer.

## Project layout

- `match_engine/` — the Rust crate (parser, simulation engine, WASM bridge)
  - `src/` — parser, engine, ball, agent, brain, formation, dispatcher, timeline
  - `examples/` — diagnostic CLIs plus `gen_manifest` (builds the match list)
  - `tests/` — unit, integration, accuracy and shape-regression suites
  - `data/` — bundled Opta F24 fixtures (`*.xml`) and `mappings/`
  - `web/` — the front end (`index.html`, `app.js`, `smoke_test.cjs`)
- `Dockerfile`, `Caddyfile`, `render.yaml` — Render deployment

## Run locally

```
cd match_engine
./run_web.sh
```

`run_web.sh` builds the WASM engine, copies the fixtures into
`web/matches/`, generates `web/matches.json`, and serves the crate root
on `:8000`. Open <http://localhost:8000/web/>.

The left-hand **Matches** panel lists every bundled fixture — scroll the
list and click a match to load and watch it. The controls give you
pause/play, restart, a 0.25×–8× speed slider, ±10 minute seek, and a
path box to load any XML by path (or use the **Load XML** dialog).

## Deploy to Render

The repo ships a multi-stage `Dockerfile` (Rust WASM build → Caddy
static server) and a `render.yaml`. Push to GitHub, then:

1. In Render, create a **Web Service** and connect this repository.
2. Set the environment to **Docker** (Render auto-detects the
   `Dockerfile` at the repo root).
3. Deploy. The container compiles the WASM engine, copies the fixtures
   into `web/matches/`, generates `matches.json`, and serves the app on
   the port Render injects through `$PORT`.

Or with the Render CLI:

```
render apply --file render.yaml
```

## Tests

```
cd match_engine
cargo test
```
