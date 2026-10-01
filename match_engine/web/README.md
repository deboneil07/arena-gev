# God's Eye View — WASM front end

Zero-copy HTML5 Canvas renderer driving the Rust match engine compiled to
WebAssembly.

```
web/
├── index.html        # pitch, HUD and controls
├── app.js            # fixed-step loop + Canvas drawing (Float32Array view)
├── smoke_test.cjs    # end-to-end test of the compiled .wasm (Node)
└── README.md
```

## Quick start (one command)

From the crate root:

```bash
./run_web.sh            # builds the wasm + bindings, then serves on :8000
./run_web.sh 9000       # …or a different port
```

Then open **<http://localhost:8000/web/>**.

The script adds the `wasm32` target and `wasm-bindgen-cli` if missing, compiles
the JS bindings into `web/pkg/`, copies the bundled fixtures into `web/matches/`,
generates `web/matches.json`, and starts a static server.

### Manual steps (if you prefer)

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli

cargo build --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir pkg \
  target/wasm32-unknown-unknown/release/match_engine.wasm

python -m http.server 8000     # then visit http://localhost:8000/web/
```

> Browsers refuse ES modules and `fetch()` over `file://`, so you must serve the
> crate root with a static server — opening `index.html` directly will not work.

## Browsing matches

The match panel lists every Opta F24 match bundled in `data/`. Click one to
load it and watch the simulation. The list is generated at build time into
`web/matches.json` by `cargo run --release --example gen_manifest`; the XML
files themselves are copied into `web/matches/` so the browser can fetch them.

## Loading a different XML file

Three ways, in order of convenience:

1. **`?xml=` query parameter** — put the file anywhere under the served root and
   address it:
   `http://localhost:8000/web/?xml=my_match.xml`
2. **”XML path” box** — type a path relative to `web/` and press **Load path**
   (e.g. `my_match.xml`, or `matches/OTHER.xml`).
3. **”Load XML” button** — pick any local `.xml` file with the OS file dialog
   (works without copying it into the served folder).

The controls also give you **Pause/Play**, **Restart**, and a **0.25×–8× speed**
slider. The HUD shows match time, FPS, entity count and ball altitude.

## Live event feed & splash

A right-hand **sidebar** shows the last 8 events (time · type, colour-coded by
team). **Goals, cards, fouls, corners, offsides and substitutions** also
trigger a large centre splash. The HUD carries a **scoreboard**, a
**possession bar** and ball altitude; a dashed **offside line** is drawn for
the team in possession; both **goal nets** are rendered. **⏪/⏩ 10 min** buttons
seek through the match for analysis.

All of it reads straight from the engine via the bridge:

```js
engine.get_event_seq();            // changes when a new event fires
engine.get_recent_event_count();   // newest is index count-1
engine.get_recent_event_label(i);  // "Goal", "Card", …
engine.get_recent_event_notable(i);
engine.get_score_home();           // scoreboard
engine.get_score_away();
engine.get_possession_home();      // 0..1 possession bar
engine.get_offside_line();         // NaN when nobody is in possession
engine.seek_to(600);               // jump to 10:00 (rebuilds + fast-forwards)
```

## Player individuality & zone discipline

Every player carries deterministic attributes (pace, aggression, workrate,
flair, positioning, determination, composure, stamina) hashed from their id,
plus a coarse role (GK/DF/MF/FW). These drive sprint speed, how eagerly each
individual presses/supports, how tightly they hold their zone (defenders stay
home, forwards push high) and how much they fidget. Set pieces reshape both
teams: corners flood the box, throw-ins/free kicks send only the taker to the
ball while everyone else holds shape.

## Buffer contract

`get_render_buffer_ptr()` returns a pointer to `23 × 6 = 138` `f32`s in WASM
linear memory:

| index | field |
|---|---|
| 0 | x (metres) |
| 1 | y (metres) |
| 2 | z / altitude (metres) |
| 3 | heading (radians) |
| 4 | team index (0 home, 1 away, 2 ball) |
| 5 | shirt number |

Slots `0..get_player_count()-1` are players; the ball is always pinned to the
final slot (`22`). `app.js` creates a new
`Float32Array(wasm.memory.buffer, ptr, 138)` **every frame** because the WASM
heap can grow during `tick()` and detach older views.

## Test the wasm without a browser

```bash
wasm-bindgen --target nodejs --out-dir pkg-node \
  target/wasm32-unknown-unknown/release/match_engine.wasm
node web/smoke_test.cjs
```
