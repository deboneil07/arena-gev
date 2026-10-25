import test from 'node:test';
import assert from 'node:assert/strict';

import { ROLE } from './analysis.js';
import { createPassingNetwork } from './graphs.js';
import { createHeatmap } from './heatmap.js';
import {
  drawAnalysis,
  drawHeatmap,
  drawOffsideLine,
} from './overlays.js';

function makeCtx() {
  const calls = [];
  const canvas = { width: 1050, height: 680 };
  const ctx = new Proxy(
    {},
    {
      get(target, prop) {
        if (prop === 'canvas') return canvas;
        if (prop === 'createImageData') {
          return (w, h) => ({ width: w, height: h, data: new Uint8ClampedArray(w * h * 4) });
        }
        if (prop === 'measureText') return () => ({ width: 20 });
        if (typeof prop === 'string') {
          return (...args) => calls.push([prop, ...args]);
        }
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

globalThis.document = {
  createElement(tag) {
    assert.equal(tag, 'canvas');
    const off = { width: 0, height: 0 };
    off.getContext = () => makeCtx().ctx;
    return off;
  },
};

const toScreen = (x, y) => [x * 8 + 525, 340 - y * 8];

test('drawHeatmap composites the density image inside the pitch rect', () => {
  const { ctx, calls } = makeCtx();
  const heat = createHeatmap({ cols: 21, rows: 14 });
  heat.sample([{ x: 0, y: 0 }]);
  drawHeatmap(ctx, { x: 60, y: 40, w: 900, h: 580 }, heat, 0, 1);
  const drawn = calls.find(([name]) => name === 'drawImage');
  assert.ok(drawn);
  assert.deepEqual(drawn.slice(2), [60, 40, 900, 580]);
});

test('drawAnalysis draws a carrier ring, thread and attack triangle', () => {
  const { ctx, calls } = makeCtx();
  const players = [
    { x: 0, y: 0, team: 0, role: ROLE.FWD, shirt: 9 },
    { x: 4, y: 3, team: 0, role: ROLE.FWD, shirt: 11 },
    { x: -3, y: 4, team: 0, role: ROLE.MID, shirt: 8 },
    { x: 30, y: 0, team: 1, role: ROLE.DEF, shirt: 4 },
  ];
  const ball = { x: 0.5, y: 0.2 };
  drawAnalysis(ctx, toScreen, players, 0, 1, ball, 0);
  // Carrier ring (arc), thread (lineTo), triangle (stroke).
  assert.ok(calls.some(([name]) => name === 'arc'));
  assert.ok(calls.filter(([name]) => name === 'lineTo').length >= 2);
  assert.ok(calls.filter(([name]) => name === 'stroke').length >= 2);
});

test('drawAnalysis draws a defensive hull for the non-possessing team', () => {
  const { ctx, calls } = makeCtx();
  const players = [
    { x: -40, y: -10, team: 1, role: 2, shirt: 2 },
    { x: -40, y: 10, team: 1, role: 2, shirt: 3 },
    { x: -30, y: 0, team: 1, role: 2, shirt: 4 },
    { x: 0, y: 0, team: 0, role: ROLE.FWD, shirt: 9 },
  ];
  const ball = { x: 0, y: 0 };
  drawAnalysis(ctx, toScreen, players, 1, 1, ball, 0);
  assert.ok(calls.some(([name]) => name === 'fill'));
});

test('drawOffsideLine is a dashed line only for a finite x and visible alpha', () => {
  const { ctx, calls } = makeCtx();
  drawOffsideLine(ctx, toScreen, -12, 1, 40, 620);
  assert.ok(calls.some(([name]) => name === 'setLineDash'));
  assert.ok(calls.some(([name]) => name === 'lineTo'));
  const before = calls.length;
  drawOffsideLine(ctx, toScreen, NaN, 1, 40, 620);
  drawOffsideLine(ctx, toScreen, -12, 0, 40, 620);
  assert.equal(calls.length, before);
});
