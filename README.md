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
  - `web/` — the front end (`index.html`, `app.js`, `analysis.js`, `overlays.js`,
    `variants.js`, `fieldThemes.js`, `presets.js`, `smoke_test.cjs`)
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
pause/play, restart, a **Freeze frame** ghost comparison (bodies, lanes and vision), a **PNG frame export** with legend, a 0.25×–8× speed
slider, ±5 minute seek, and a path box to load any XML by path (or use the **Load XML** dialog).

The viewer starts on the **Tactical** preset and overlays broadcast-style analysis: a light cumulative heatmap,
ball-carrier highlight, an attacking triangle, four attacking channels that
show where the ball can go, gold run landings, a role-shaped vision field
(keepers see wide, strikers see tight), a possession-window passing network
with outlet and full-mesh variants whose link weights decay when a pair stops combining, a translucent defensive hull when out of possession, a
pressing trigger, and an offside line that appears only while it is informative
(open play, an attacker near the line, and no parked back line). The overlays
follow possession and fade during set pieces. Each analytical layer has a
toggle in the effects bar; **Shuffle effects** cycles one enabled layer at a
time.

Click **Lab** above the pitch (or add `?lab=1` to the URL) to open the
Variant lab and compare alternate renders of the same analysis without editing
code. Vision cones have classic, soft-wedge,
spotlight and outline skins; pass lanes have classic, dot-free, corridor-target
and ghost-trail versions; run channels can be arrows, arrow-free lanes, landing
blobs, trails or thin traces; heatmaps switch between raster and hex bins; the
passing network switches between one outlet per player and the full match mesh;
the field has classic, broadcast and night palettes. Presets switch the whole
layer stack between Broadcast, Tactical and Clean. The recommended default is the
softened cone, the labeled pass corridor and the arrow-free run lane; the
original classic skins remain selectable. Shift-click an effect to solo it, or use the Broadcast / Tactical / Clean
presets; preset and variant choices both survive a reload. In the default pass-lane corridor, click a target chip to open that
receiver's player analysis. The chosen variant is stored in
`localStorage`, so a demo survives a reload. Pass lanes hold for ~2.4 s by
default and passing-network links decay over ~3 minutes; both are adjustable
under the pitch and fade as ghosts across a turnover
instead of disappearing on the first pass.

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
node --test web/*.test.mjs
node web/smoke_test.cjs          # requires the Node wasm bindings (see web/README.md)
```
