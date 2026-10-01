// End-to-end smoke test for the compiled WASM package.
//
// Run after building:
//   cargo build --target wasm32-unknown-unknown --release
//   wasm-bindgen --target nodejs --out-dir pkg-node \
//       target/wasm32-unknown-unknown/release/match_engine.wasm
//   node web/smoke_test.cjs
//
// Verifies the real .wasm binary parses, simulates, and produces a valid flat
// render buffer — the same code path the browser uses.

const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert');
const { WasmMatchEngine } = require('../pkg-node/match_engine.js');

const XML = path.join(__dirname, '..', 'data', '2647319.xml');

function fail(msg) {
  console.error(`✗ ${msg}`);
  process.exit(1);
}

if (!fs.existsSync(XML)) fail(`fixture not found at ${XML}`);
const xml = fs.readFileSync(XML, 'utf8');

// ── Construct ──────────────────────────────────────────────────
const engine = new WasmMatchEngine(xml);
assert.strictEqual(engine.get_entity_stride(), 6);
assert.strictEqual(engine.get_total_entities(), 23);
assert.strictEqual(engine.get_render_buffer_copy().length, 23 * 6);
assert.ok(engine.get_duration_secs() > 5000, 'expected a full match length');
assert.strictEqual(engine.get_is_finished(), false);
assert.ok(engine.get_render_buffer_ptr() !== 0, 'render buffer pointer is null');
console.log(`✓ constructed engine (duration ${engine.get_duration_secs().toFixed(0)} s)`);

// ── Tick a few seconds and validate the buffer ─────────────────
for (let i = 0; i < 300; i++) engine.tick(1 / 60);
const buf = engine.get_render_buffer_copy();
assert.strictEqual(buf.length, 138);
assert.ok(buf.every(Number.isFinite), 'non-finite value in render buffer');

const teamOf = (i) => buf[i * 6 + 4];
const home = Array.from({ length: 23 }, (_, i) => teamOf(i)).filter((t) => t === 0).length;
const away = Array.from({ length: 23 }, (_, i) => teamOf(i)).filter((t) => t === 1).length;
const balls = Array.from({ length: 23 }, (_, i) => teamOf(i)).filter((t) => t === 2).length;
assert.deepStrictEqual([home, away, balls], [11, 11, 1], 'entity team layout wrong');

for (let i = 0; i < 23; i++) assert.ok(buf[i * 6 + 2] >= 0, 'negative altitude');
assert.ok(engine.get_sim_time() >= 4.9, 'sim clock did not advance');
console.log(`✓ ticked 5 s (clock ${engine.get_sim_time().toFixed(2)} s, ${home}+${away}+1 entities)`);

// ── Full match: performance + consistency ──────────────────────
const t0 = Date.now();
let maxAlt = 0;
let steps = 0;
while (!engine.get_is_finished() && steps < 60 * 60 * 120) {
  engine.tick(1 / 60);
  steps++;
  if (steps % 30 === 0) {
    const b = engine.get_render_buffer_copy();
    maxAlt = Math.max(maxAlt, b[22 * 6 + 2]);
    if (!b.every(Number.isFinite)) fail('non-finite value during full match');
  }
}
const ms = Date.now() - t0;
assert.ok(engine.get_is_finished(), 'match did not finish');
assert.ok(maxAlt > 0.5, `ball never left the ground (max altitude ${maxAlt})`);
console.log(`✓ full match in ${(ms / 1000).toFixed(2)} s wall clock, max ball altitude ${maxAlt.toFixed(2)} m`);

// ── Invalid XML should throw, not crash ────────────────────────
let threw = false;
try {
  new WasmMatchEngine('<not valid xml');
} catch (e) {
  threw = true;
}
assert.ok(threw, 'invalid XML should throw');
console.log('✓ invalid XML rejected cleanly');

console.log('\nAll WASM smoke tests passed.');
