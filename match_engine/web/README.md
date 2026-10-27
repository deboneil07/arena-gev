# God's Eye View — WASM front end

Zero-copy HTML5 Canvas renderer driving the Rust match engine compiled to
WebAssembly.

```
web/
├── index.html         # pitch, HUD and controls
├── app.js             # fixed-step loop + Canvas drawing (Float32Array view)
├── analysis.js        # pure tactical geometry (roles, shape units, triangles)
├── analysis.test.mjs  # Node unit tests for the geometry maths
├── heatmap.js         # pure cumulative team heatmap (density grid)
├── heatmap.test.mjs
├── overlays.js        # Canvas drawing for the team overlays
├── overlays.test.mjs  # headless Canvas-harness tests for the draw routines
├── graphs.js          # momentum strip + passing network accumulators
├── graphs.test.mjs
├── effects.js         # analytical layer toggles + shuffle state
├── effects.test.mjs
├── dribbles.js        # pure whole-match dribble analysis (zones, stats, insights)
├── dribbles.test.mjs
├── dribbleOverlay.js  # Canvas drawing for the 12-zone dribble heat layer
├── dribbleOverlay.test.mjs
├── playerPanel.js     # slide-in analysis panel (team lens + per-player lens)
├── playerPanel.test.mjs
├── transitions.js     # renderer-only substitution choreography
├── transitions.test.mjs
├── smoke.test.mjs     # boots app.js under a stub DOM (link/id/render checks)
├── smoke_test.cjs     # end-to-end test of the compiled .wasm (Node)
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
slider. The HUD shows match time, FPS, entity count and ball altitude, plus a
team pill naming the side currently being analysed and its formation.

## Live event feed & splash

A right-hand **sidebar** shows the last 8 events (time · type, colour-coded by
team). **Goals, cards, fouls, corners, offsides and substitutions** also
trigger a large centre splash. The HUD carries a **scoreboard**, a
**possession bar** and ball altitude; both **goal nets** are rendered.
**⏪/⏩ 5 min** buttons seek through the match for analysis.

All of it reads straight from the engine via the bridge:

```js
engine.get_event_seq();            // changes when a new event fires
engine.get_recent_event_count();   // newest is index count-1
engine.get_recent_event_label(i);  // "Goal", "Card", …
engine.get_recent_event_notable(i);
engine.get_score_home();           // scoreboard
engine.get_score_away();
engine.get_possession_home();      // 0..1 possession bar
engine.get_player_role(i);         // 0 unknown, 1 GK, 2 DEF, 3 MID, 4 FWD
engine.get_possession_team();      // 0 home, 1 away, -1 neutral
engine.seek_to(300);               // jump to 05:00 (rebuilds + fast-forwards)
```

## Team analysis overlays

The overlays **follow possession**: exactly one team is shown at a time (the
side in possession). The layer waits for 0.35 s of stable possession and then
crossfades over 0.18 s, so turnovers never cause the whole analysis to
flicker. Every analytical layer is on by default and can be switched off
independently from the effects bar above the pitch.

- **Shape (triangle)** — three unit segments traced through the current
  defenders, midfielders and forwards (the GK is excluded), plus a translucent
  triangle joining the three unit centroids. DEF / MID / ATT are colour-coded
  and labelled at the end of each segment. A unit with one player shows a dot
  instead of a line; the triangle only appears when all three units exist.
- **Passing network** — nodes at each player's average touch position, sized
  by passes made; the eight strongest links are drawn with width and opacity
  scaled by pass count. Nodes carry shirt numbers only. Only the followed
  team's network is drawn.
- **Live heatmap** — a cumulative density map of the followed team's
  positions from load/seek onward, sampled every 0.25 s of match time. Home
  uses a warm ramp, away a cool ramp; the soft wash is confined to the pitch
  and resets on load, restart and seek so it always describes the current
  position.
- **Momentum strip** — sidebar canvas: cumulative model-estimate threat per
  team with goal/shot/card ticks.
- **Passing lanes** — live arrows from the ball to the best receivers. The
  geometry is re-scored at 10 Hz and the endpoints resolve every frame, so
  the arrow stays attached to the moving ball and receiver. Each lane grows
  from the ball, with opacity/width driven by a logistic completion
  probability (corridor clearance, distance, receiver pressure, xT gain).
- **Run channels** — live 6–12 m forward runs the attackers should make
  during a progression, scored by xT gain and the time gained on the nearest
  defender. The arrow is anchored to the runner, its weight scales with the
  score, bows away from the nearest defender, and grows from a point with an
  eased extension.
- **Vision cone** — a translucent sector for the carrier. Every player has a
  stable individual profile: radius 16–28 m and half-angle 48–72° (deterministic
  from team + shirt), so narrow and wide-vision players read differently. The
  cone faces the best available pass (not the ball at the carrier's feet).
  Teammates in the cone with a clear sightline are joined in teal; the rest
  of the near circle is the blind sector with dotted red links. Occlusion by
  an opponent downgrades a teammate to blind.
- **Player heat** — the analysis minimap sometimes shows a randomly chosen
  player's cumulative heat (rotates every 45 s, resets with the followed team).
- **Scripted sequences** — goal mob, save puff, woodwork ping, card/penalty
  banners; all renderer ghosts, all seek-safe.

The passing network is anchored to the live player positions and shows while
the followed team still has the ball, after at least three consecutive passes.
A sustained interception breaks the retained sequence immediately (the layer
hides as soon as possession is lost). A new pair's link is almost invisible and
darkens only as those two players keep exchanging the ball (absolute pass
count, not relative to the strongest link).

### Effect toggles & shuffle

The effects bar above the pitch controls each analytical layer independently:
**Heatmap**, **Passing network**, **Offside line**, **Analysis** (carrier ring,
THREAT arrow/text, press ring and defensive hull), **Pass lanes**, **Run
channels**, **Vision cone** and **Dribbles**. All are on by default, so the
initial view is
unchanged. Hiding a layer only skips its draw call: the samplers and trackers
keep running, so switching it back on resumes the live state instead of
resetting it. Toggle choices persist across seeks, restarts and match loads.

**🔀 Shuffle effects** shows exactly one enabled layer at a time and advances
every 4 seconds, wrapping through the enabled set. The current layer's button
is highlighted. Turning Shuffle off restores the normal view for every enabled
toggle. Disabling the layer currently on screen moves Shuffle to the next
enabled layer; if no layers are enabled, Shuffle shows nothing.

Set-piece robustness: a corner holds the ball at the flag while the box fills.
If the feed includes a delivery pass, that cross is used; if the synthetic feed
omits it, the engine stages a plausible delivery itself. Goalkeepers are never
selected as corner/free-kick/throw-in takers and are leashed to their own goal.
Duplicate "player on" rows in synthetic feeds are ignored so two bodies can
never share one identity (which used to tear the goalkeeper's anchor apart).

The pure geometry and accumulators (`analysis.js`, `heatmap.js`, `graphs.js`)
are DOM-free and covered by Node tests:

```bash
node --test web/*.test.mjs
```

## Dribble analysis

The **Dribbles** effect draws a whole-match take-on/carry layer over the pitch,
and clicking into it opens a slide-in analysis panel. Everything is derived
once per match from `engine.get_analysis_json()` (a static schema-v1 log of
every event plus both rosters), so results never depend on playback position —
seeking, pausing and switching matches all stay consistent.

### Pitch layer

- The pitch is divided into **12 zones** (4 rows × 3 columns) defined in the
  **attacker's own frame**: home attacks +x, away's view is mirrored on both
  axes, so zone 1 is always "own third, defender's left". Zone ids 1–12 run
  row-major and are printed in the corner of every cell.
- Each used zone is washed with a heat colour scaled to its attempt volume and
  tinted by success rate; empty zones stay unwashed. The **Zones** slider in
  the view controls sets the layer opacity (default 55 %).
- Attempt markers sit at the dribble's start: **✓** green = won take-on
  (arrow to where the ball was carried/received within 5 s), **✗** red = lost
  take-on, **○** blue = progressive carry. Sequence numbers appear while the
  team has 20 attempts or fewer.
- Clicking a zone selects it: the zone is outlined on the pitch, a clearable
  chip appears in the panel, the zone table and zone-map ring highlight that
  row, and the timeline dims attempts from other zones (its click-to-seek
  then only lands on this zone's attempts). Zone ids live in the attacker's
  frame, so a selection follows the team lens through possession flips.
  Hovering pre-highlights the cell.

### Analysis panel

Open it from a zone click, from a player's context menu (**Open player
analysis**), or from **Show team dribble zones**; `Escape` closes it. The team
lens follows the pitch's followed side (and flips with it live) unless the
per-player lens is open.

- **Filters** — half (1st/2nd/both), game state (ahead/level/behind) and a
  player picker apply to every stat in the view.
- **Team lens** — attempt/success/shot-after totals, the busiest-zone table
  (click a row to select that zone), the ✓/✗/○ effectiveness scale, rule-based
  insights and the squad list (click a player to switch lenses).
- **Player lens** — four percentile metrics vs team-mates with ≥2 dribbles
  (attempts, success rate, gain, shot-after rate) with star ratings; a
  **per-zone circle map** where circle *size* = attempts, *fill* = success
  rate and *ring* = confidence (few attempts → fainter ring); a thirds split;
  and a **minute strip** of every attempt across the 90 minutes. Clicking the
  strip seeks the sim to that attempt (read-only scrubber — it jumps, it does
  not scrub). The view's **Export** button downloads the current view,
  filters and every attempt as JSON.

### View controls

**Zoom** (slider; ctrl/cmd + wheel also zooms), drag-to-pan on the pitch and
**Reset view**. The transform is a single canvas matrix — identity at rest,
so hit tests and export coordinates stay honest. Plain wheel still scrolls
the page; touch keeps vertical page scrolling (`touch-action: pan-y`).

`window.__gev` exposes read-only handles (`engine`, `players`, `panel`,
`index`, `view`, `toScreen`, `seekTo`) for console debugging and automated
checks.

## Off-ball movement, phases & substitutions

The macro formation engine owns each player's anchor. The utility brain adds
the details that make the other ten players look like footballers rather than
escorts:

- **Duties** — every outfielder is classified from the lineup and template as
  a centre-back, full-back, holder, central midfielder, No.10, winger or
  striker. The duty travels with the agent through substitutions and drives
  press bias, marking fit and shape. Bench players arrive as category 5 in the
  feed, so a substitute inherits the outgoing player's role and duty; he fills
  the slot's line instead of becoming an unknown forward.
- **Phases** — a hysteretic team state machine (build-up, progression, final
  third; high press, mid-block, low block; attacking/defensive transition)
  scales tempo, zone size and line height. It needs a short dwell before it
  changes, so the block does not flicker.
- **One-to-one marks** — the defending team solves a role-weighted assignment
  each tick. Each opponent is picked up by at most one marker; the presser and
  cover are excluded because the ball already occupies them; unassigned
  players hold shape instead of converging on the nearest shirt.
- **Shared event lookahead** — every player, tracked or not, reads the same
  block: this team's next ball event, the opponent's next event, and the
  landing point when that next beat is a pass. A later cross can never shadow
  an earlier shot or tackle. A player starts moving when his own travel time
  says he must in order to arrive with a settling cushion. Strikers attack the
  box, the No.10 and midfield shift across into the support pocket, the holder
  screens for the second ball, wingers attack the far post, and defenders
  shuffle laterally. The macro block owns every defender's and midfielder's
  line height, so a cue never dissolves the back line or the visible midfield
  banks; only forwards may attack a new depth. Event-bound actors stay exempt
  so the feed's logged spot remains authoritative.
- **Substitutions** — the engine swaps identity in place on the `On` event.
  `transitions.js` watches the `(team, shirt)` buffer for that swap and draws a
  renderer-only walk-off to the pavilion and a walk-on from it. The live body
  is hidden only while its incoming ghost is moving; overlays filter it out.
  Engine positions, event ownership and the 23×6 buffer are untouched.

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

## Test the wasm and overlays without a browser

```bash
node --test web/*.test.mjs

wasm-bindgen --target nodejs --out-dir pkg-node \
  target/wasm32-unknown-unknown/release/match_engine.wasm
node web/smoke_test.cjs
```
