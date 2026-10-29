import test from 'node:test';
import assert from 'node:assert/strict';

import { ROLE, raycastVision, visionCone } from './analysis.js';
import { createPassingNetwork } from './graphs.js';
import { createHeatmap } from './heatmap.js';
import {
  drawAnalysis,
  drawAttackChannels,
  drawAttackLanes,
  drawHeatmap,
  drawOffsideLine,
  drawPassingNetwork,
  drawVisionCone,
  drawFrozenFrame,
  drawVisionField,
  laneTargetHits,
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
        if (prop === 'createRadialGradient' || prop === 'createLinearGradient') {
          return (...args) => {
            calls.push([prop, ...args]);
            return { addColorStop: (...stops) => calls.push(['addColorStop', ...stops]) };
          };
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

test('drawPassingNetwork anchors links and nodes to live player positions', () => {
  const { ctx, calls } = makeCtx();
  const net = createPassingNetwork();
  for (let i = 0; i < 3; i++) {
    net.observePass({ x: 0, y: 0, team: 0, shirt: 9 }, { x: 10, y: 0, team: 0, shirt: 8 });
  }
  const snap = net.snapshot(8, 0);
  const live = new Map([
    ['0:9', { x: 20, y: 5 }],
    ['0:8', { x: 30, y: 5 }],
  ]);
  drawPassingNetwork(ctx, toScreen, snap, 1, 0, live);
  const a = toScreen(20, 5);
  const b = toScreen(30, 5);
  assert.ok(calls.some(([n, x, y]) => n === 'moveTo' && Math.abs(x - a[0]) < 1e-6 && Math.abs(y - a[1]) < 1e-6));
  assert.ok(calls.some(([n, x, y]) => n === 'lineTo' && Math.abs(x - b[0]) < 1e-6 && Math.abs(y - b[1]) < 1e-6));
});

test('drawPassingNetwork draws the whole-team mesh, zero-pass pairs included', () => {
  const { ctx, calls } = makeCtx();
  const net = createPassingNetwork();
  for (let i = 0; i < 3; i++) {
    net.observePass({ x: 0, y: 0, team: 0, shirt: 9 }, { x: 10, y: 0, team: 0, shirt: 8 });
  }
  const roster = [
    { team: 0, shirt: 9, x: 20, y: 5 },
    { team: 0, shirt: 8, x: 30, y: 5 },
    { team: 0, shirt: 1, x: -40, y: 0 }, // keeper has not touched the ball yet
  ];
  const snap = net.snapshot(8, 0, roster);
  assert.equal(snap.nodes.length, 3);
  assert.equal(snap.links.length, 3);
  const live = new Map(roster.map((r) => [`0:${r.shirt}`, { x: r.x, y: r.y }]));
  drawPassingNetwork(ctx, toScreen, snap, 1, 0, live);
  // The zero-pass keeper pair is still drawn, faintly, into the mesh.
  const [kx, ky] = toScreen(-40, 0);
  assert.ok(calls.some(([n, x, y]) =>
    n === 'lineTo' && Math.abs(x - kx) < 1e-6 && Math.abs(y - ky) < 1e-6));
});

test('drawPassingNetwork pulses a pass that is not a retained outlet wire', () => {
  const { ctx, calls } = makeCtx();
  const net = createPassingNetwork();
  net.observePass({ x: 0, y: 0, team: 0, shirt: 9 }, { x: 10, y: 0, team: 0, shirt: 8 });
  net.observePass({ x: 0, y: 0, team: 0, shirt: 9 }, { x: 10, y: 0, team: 0, shirt: 8 });
  net.observePass({ x: 10, y: 0, team: 0, shirt: 8 }, { x: 20, y: 0, team: 0, shirt: 6 });
  const roster = [
    { team: 0, shirt: 9, x: 20, y: 5 },
    { team: 0, shirt: 8, x: 30, y: 5 },
    { team: 0, shirt: 6, x: 40, y: 5 },
  ];
  const snap = net.snapshot(8, 0, roster, 'outlet');
  assert.ok(snap.links.every((l) => !((l.a === '0:9' && l.b === '0:6') || (l.a === '0:6' && l.b === '0:9'))));
  drawPassingNetwork(ctx, toScreen, snap, 1, 0, null, { a: '0:9', b: '0:6', progress: 0.5 });
  // The transient ghost thread and its bead keep the pass visible even though
  // 9→6 is nobody's top outlet in this window.
  assert.ok(calls.some(([n, , , r]) => n === 'arc' && Math.abs(r - 3) < 1e-9));
});

test('drawVisionField fills the lit polygon and casts blocker shadows', () => {
  const { ctx, calls } = makeCtx();
  const field = raycastVision(
    { x: 0, y: 0, heading: 0, role: ROLE.MID },
    [{ x: 10, y: 0, radius: 1 }],
    1,
  );
  const fills = () => calls.filter(([n]) => n === 'fill').length;
  const before = fills();
  drawVisionField(ctx, toScreen, field, 1, 8);
  // Sector wash + penumbra + umbra + the angular-falloff wedge buckets.
  const painted = fills() - before;
  assert.ok(painted >= 4, `expected the lit field and shadows, painted ${painted}`);
  assert.ok(painted <= 36, `falloff buckets should stay cheap, painted ${painted}`);
  assert.ok(calls.some(([n]) => n === 'stroke'));
  assert.ok(calls.some(([n]) => n === 'createRadialGradient'));
  // Without shadows only the sector wash and the lit wedges are painted.
  const noShadow = { ...field, shadows: [] };
  const mid = fills();
  drawVisionField(ctx, toScreen, noShadow, 1, 8);
  assert.ok(fills() - mid >= 2);
  assert.ok(fills() - mid < painted);
});

test('drawVisionCone shades the near cone with the role falloff', () => {
  const { ctx, calls } = makeCtx();
  const cone = visionCone(
    { x: 0, y: 0, heading: 0, role: ROLE.FWD },
    [{ x: 10, y: 0, shirt: 9 }],
    [],
    1,
  );
  drawVisionCone(ctx, toScreen, cone, 1);
  // The visible cone is split into brightness buckets, then the blind
  // sector is filled and the sightlines are stroked.
  assert.ok(calls.filter(([n]) => n === 'fill').length >= 3);
  assert.ok(calls.some(([n]) => n === 'lineTo'));
  assert.ok(calls.some(([n]) => n === 'stroke'));
});

test('drawAttackLanes and drawAttackChannels draw live channels', () => {
  const { ctx, calls } = makeCtx();
  drawAttackLanes(ctx, toScreen, [{
    index: 2,
    y0: 0,
    y1: 17,
    centerY: 8.5,
    x0: 0,
    x1: 52.5,
    score: 0.8,
    growth: 1,
    alpha: 1,
    blur: 0,
    pulses: [{ fromX: 10, toX: 30, progress: 0.5 }],
  }], 1);
  assert.ok(calls.some(([n]) => n === 'fillRect'));
  assert.ok(calls.some(([n]) => n === 'createRadialGradient'));
  assert.ok(calls.filter(([n]) => n === 'stroke').length >= 2);
  drawAttackChannels(ctx, toScreen, [{
    from: { x: 20, y: 0 },
    to: { x: 34, y: -4 },
    growth: 1,
    alpha: 1,
    blur: 0,
    score: 0.9,
    curve: 2,
    landing: {
      lane: 1, y0: -17, y1: 0, centerY: -8.5, x0: 0, x1: 52.5, x: 34, y: -4,
    },
  }], 1);
  assert.ok(calls.some(([n]) => n === 'quadraticCurveTo'));
  assert.ok(calls.some(([n]) => n === 'translate'));
  // No-op for empty or invisible input.
  const before = calls.length;
  drawAttackLanes(ctx, toScreen, [], 1);
  drawAttackChannels(ctx, toScreen, [], 1);
  drawAttackLanes(ctx, toScreen, [{
    index: 0, y0: -34, y1: -17, centerY: -25.5, x0: 0, x1: 52.5,
    score: 1, growth: 1, alpha: 0, blur: 0,
  }], 1);
  assert.equal(calls.length, before);
});

test('every vision-cone variant paints without throwing', () => {
  const cone = visionCone(
    { x: 0, y: 0, heading: 0, role: ROLE.MID },
    [{ x: 8, y: 1, shirt: 9 }, { x: -8, y: 0, shirt: 6 }],
    [{ x: 12, y: 0, shirt: 4 }],
    1,
  );
  for (const variant of ['classic', 'soft-wedge', 'spotlight', 'minimal']) {
    const { ctx, calls } = makeCtx();
    drawVisionCone(ctx, toScreen, cone, 1, variant);
    assert.ok(calls.some(([n]) => n === 'fill' || n === 'stroke'), variant);
  }
});

test('every pass-lane variant paints without throwing and corridor names the target', () => {
  const lane = {
    index: 2,
    y0: 0,
    y1: 17,
    centerY: 8.5,
    x0: 0,
    x1: 52.5,
    score: 0.8,
    growth: 1,
    alpha: 1,
    blur: 0,
    target: { x: 30, y: 8.5, shirt: 9, team: 0, p: 0.8 },
    pulses: [{ fromX: 10, toX: 30, progress: 0.5 }],
  };
  for (const variant of ['classic', 'pulse-free', 'corridor', 'ghost-trail']) {
    const { ctx, calls } = makeCtx();
    drawAttackLanes(ctx, toScreen, [lane], 1, variant);
    assert.ok(calls.some(([n]) => n === 'fill'), variant);
  }
  const { ctx, calls } = makeCtx();
  drawAttackLanes(ctx, toScreen, [lane], 1, 'corridor');
  assert.ok(calls.some(([n, text]) => n === 'fillText' && text === '9'));
  assert.equal(laneTargetHits.length, 1);
  assert.equal(laneTargetHits[0].shirt, 9);
  assert.equal(laneTargetHits[0].team, 0);
});

test('every run-channel variant paints without throwing', () => {
  const channel = {
    from: { x: 20, y: 0 },
    to: { x: 34, y: -4 },
    score: 0.7,
    growth: 1,
    alpha: 1,
    blur: 0,
    curve: 2,
    landing: { x0: 30, x1: 36, y0: -6, y1: -2 },
  };
  for (const variant of ['classic', 'lanes', 'heat-landing', 'ghosts', 'minimal']) {
    const { ctx, calls } = makeCtx();
    drawAttackChannels(ctx, toScreen, [channel], 1, variant);
    assert.ok(calls.some(([n]) => n === 'stroke'), variant);
  }
});

test('hexbin heatmap paints and caches its offscreen canvas', () => {
  const heat = createHeatmap({ cols: 21, rows: 14 });
  heat.sample([{ x: 0, y: 0 }, { x: 10, y: 5 }]);
  const { ctx, calls } = makeCtx();
  drawHeatmap(ctx, { x: 60, y: 40, w: 900, h: 580 }, heat, 0, 1, 'hexbin');
  assert.ok(calls.some(([n]) => n === 'drawImage'));
});

test('minimal analysis drops the threat label but keeps the carrier ring', () => {
  const players = [
    { x: 0, y: 0, team: 0, role: ROLE.FWD, shirt: 9 },
    { x: 4, y: 3, team: 0, role: ROLE.FWD, shirt: 11 },
    { x: -3, y: 4, team: 0, role: ROLE.MID, shirt: 8 },
    { x: 30, y: 0, team: 1, role: ROLE.DEF, shirt: 4 },
  ];
  const ball = { x: 0.5, y: 0.2 };
  const { ctx, calls } = makeCtx();
  drawAnalysis(ctx, toScreen, players, 0, 1, ball, 0, { links: false }, 'minimal');
  assert.ok(calls.some(([name]) => name === 'arc'));
  assert.ok(!calls.some(([name, text]) => name === 'fillText' && text === 'THREAT'));
});

test('frozen-frame ghosts draw bodies, shirts and a dashed ball', () => {
  const { ctx, calls } = makeCtx();
  drawFrozenFrame(ctx, toScreen, {
    players: [
      { x: 0, y: 0, team: 0, shirt: 9 },
      { x: 10, y: 5, team: 1, shirt: 4 },
    ],
    ball: { x: 2, y: 1 },
    lanes: [{ x0: 0, x1: 52.5, y0: 0, y1: 17, centerY: 8.5 }],
    vision: { apex: { x: 0, y: 0 }, conePoints: [
      { x: 0, y: 0 }, { x: 10, y: -5 }, { x: 10, y: 5 }, { x: 0, y: 0 },
    ] },
  }, 1);
  assert.ok(calls.some(([n]) => n === 'arc'));
  assert.ok(calls.some(([n, text]) => n === 'fillText' && text === '9'));
  assert.ok(calls.some(([n]) => n === 'setLineDash'));
  assert.ok(calls.filter(([n]) => n === 'fill').length >= 2);
  assert.ok(calls.some(([n]) => n === 'stroke'));
});

test('frozen frame draws previous-possession ghosts underneath', () => {
  const { ctx, calls } = makeCtx();
  drawFrozenFrame(ctx, toScreen, {
    players: [{ x: 0, y: 0, team: 0, shirt: 9 }],
    ball: { x: 0, y: 0 },
    previous: {
      players: [{ x: 10, y: 0, team: 1, shirt: 4 }, { x: -10, y: 0, team: 1, shirt: 5 }],
      ball: { x: 10, y: 0 },
    },
  }, 1);
  // Two previous bodies + one current body, each an arc.
  assert.ok(calls.filter(([n]) => n === 'arc').length >= 3);
});
