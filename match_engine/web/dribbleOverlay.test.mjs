import test from 'node:test';
import assert from 'node:assert/strict';

import { createDribbleIndex } from './dribbles.js';
import { DRIBBLE_LEGEND, drawDribbleLayer, zoneFillStyle } from './dribbleOverlay.js';

function makeCtx() {
  const calls = [];
  const canvas = { width: 1050, height: 680 };
  const ctx = new Proxy(
    {},
    {
      get(target, prop) {
        if (prop === 'canvas') return canvas;
        if (prop === 'measureText') return () => ({ width: 20 });
        if (typeof prop === 'string') return (...args) => calls.push([prop, ...args]);
        return target[prop];
      },
      set(target, prop, value) {
        target[prop] = value;
        return true;
      },
    },
  );
  return { ctx, calls };
}

function row(ty, t, team, pid, name, x, y, opts = {}) {
  return [ty, t, opts.mt ?? t, opts.period ?? 1, team, pid, name, x, y,
    opts.ex ?? null, opts.ey ?? null, opts.out ?? 1];
}

function fixture() {
  return createDribbleIndex({
    v: 1,
    events: [
      row(3, 300, 0, 'h10', 'J. Smith', 40, 30),
      row(3, 320, 0, 'h10', 'J. Smith', 45, -20, { out: 0 }),
      row(83, 340, 0, 'h10', 'J. Smith', 10, 0, { ex: 25, ey: 0 }),
      row(3, 360, 1, 'a9', 'Away Nine', -40, -30),
    ],
    roster: [[['h10', 'J. Smith', 10, 4]], [['a9', 'Away Nine', 9, 4]]],
  });
}

const toScreen = (x, y) => [x * 8 + 525, 340 - y * 8];

test('the zone wash encodes activity by hue and outcome by opacity', () => {
  const hot = zoneFillStyle(10, 10, 1.0);
  const quiet = zoneFillStyle(1, 10, 1.0);
  const failing = zoneFillStyle(10, 10, 0.0);
  const hue = (s) => Number(/hsla\((\d+),/.exec(s)[1]);
  const alpha = (s) => Number(/, ([\d.]+)\)$/.exec(s)[1]);
  // Hot = red, moderate = yellow, quiet = green.
  assert.equal(hue(hot), 0);
  assert.equal(hue(zoneFillStyle(5, 10, null)), 60);
  assert.ok(hue(quiet) > 90, 'a rarely used zone still reads green');
  // Same activity, different outcome → different opacity.
  assert.ok(alpha(hot) > alpha(failing), 'success must read solid, failure faint');
  assert.ok(alpha(quiet) > 0 && alpha(quiet) < 0.6);
});

test('the layer draws zone fills, the classification grid and zone ids', () => {
  const { ctx, calls } = makeCtx();
  const index = fixture();
  const stats = index.teamStats(0);
  drawDribbleLayer(ctx, toScreen, stats, 0, {});

  const fills = calls.filter(([name]) => name === 'fillRect');
  assert.equal(fills.length, 3, 'one wash per used zone (three home zones)');
  const strokes = calls.filter(([name]) => name === 'strokeRect');
  assert.ok(strokes.length >= 12, 'all twelve zones are outlined');
  const labels = calls.filter(([name]) => name === 'fillText');
  assert.ok(labels.length >= 12, 'zone ids are readable on the pitch');
  // The away dribble sits in the away frame: only home zones are washed.
  assert.equal(fills.length, 3, 'the away attempt never paints a home zone');
});

test('markers distinguish won take-ons, lost dribbles and carries', () => {
  const { ctx, calls } = makeCtx();
  const index = fixture();
  const stats = index.teamStats(0);
  drawDribbleLayer(ctx, toScreen, stats, 0, { numbers: false });

  const arcs = calls.filter(([name]) => name === 'arc');
  assert.equal(arcs.length, 1, 'the successful carry is a single circle');
  // Two crosses (failed take-on) + won take-on arrow/lines: several strokes.
  const lines = calls.filter(([name]) => name === 'lineTo');
  assert.ok(lines.length >= 4, 'arrows and crosses draw through lineTo');
  const strokes = calls.filter(([name]) => name === 'stroke');
  assert.ok(strokes.length >= 3, 'check, cross and carry circle each stroke');
  // Sequence numbers only when the list is short.
  const texts = calls.filter(([name]) => name === 'fillText');
  assert.ok(texts.some(([, t]) => t === '1'), 'short lists are numbered');
});

test('markers and numbers can be switched off, empty stats draw nothing', () => {
  const { ctx, calls } = makeCtx();
  drawDribbleLayer(ctx, toScreen, { zones: Array.from({ length: 12 }, () => ({ attempts: 0 })), dribbles: [] }, 0, {});
  assert.ok(calls.filter(([n]) => n === 'fillRect').length === 0, 'unused zones stay unwashed');

  const index = fixture();
  const stats = index.teamStats(0);
  const before = calls.length;
  drawDribbleLayer(ctx, toScreen, stats, 0, { markers: false, numbers: false });
  const after = calls.slice(before);
  assert.equal(after.filter(([n]) => n === 'arc').length, 0, 'no markers when disabled');
  const fills = after.filter(([n]) => n === 'fillText');
  assert.equal(fills.length, 12, 'zone ids remain even without sequence numbers');

  const { ctx: c2, calls: calls2 } = makeCtx();
  drawDribbleLayer(c2, toScreen, stats, 0, { alpha: 0 });
  assert.equal(calls2.length, 0, 'fully transparent layer costs nothing');
});

test('hover and selection outline exactly one zone each', () => {
  const { ctx, calls } = makeCtx();
  const index = fixture();
  const stats = index.teamStats(0);
  drawDribbleLayer(ctx, toScreen, stats, 0, { hoverZone: 3, selectedZone: 8, markers: false, numbers: false });
  const grid = calls.filter(([n]) => n === 'strokeRect');
  // 12 grid + 1 hover + 1 selection.
  assert.equal(grid.length, 14);
});

test('the legend is exported for the canvas key', () => {
  assert.equal(DRIBBLE_LEGEND.length, 3);
  for (const item of DRIBBLE_LEGEND) {
    assert.match(item.color, /^#[0-9a-f]{6}$/);
    assert.ok(item.label.length > 3);
  }
});
