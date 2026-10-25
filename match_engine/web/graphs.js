// Match graphs: momentum/xT timeline, passing network, shot map (all Canvas,
// all renderer-side, all fed by engine getters + analysis maths).
//
// Nothing here touches the sim: graphs accumulate from the same snapshots
// the overlays use, so halftime/fulltime views are free at any seek point.

import { shotXg, xtAt, xtGain } from './analysis.js';

/**
 * Normalize the engine's u64 event cursor. wasm-bindgen exposes u64 as a JS
 * BigInt, and mixing BigInt with Number throws; unknown cursors map to -1.
 */
export function seqNumber(v) {
  return typeof v === 'bigint' ? Number(v) : (v ?? -1);
}

/**
 * Cumulative threat timeline. Feed rows drive it: completed passes add
 * xT gain, shots add xG, goals/cards mark the line. One series per team.
 */
export function createMomentumTracker() {
  let home = [];
  let away = [];
  let lastKey = -1;

  function observe(key, rows) {
    if (key === lastKey) return;
    lastKey = key;
    for (const r of rows) {
      const series = r.team === 0 ? home : r.team === 1 ? away : null;
      if (!series) continue;
      const prev = series.length ? series[series.length - 1].v : 0;
      series.push({ t: r.t, v: prev + (r.dv || 0), label: r.label, type: r.type });
    }
  }

  function reset() {
    home = [];
    away = [];
    lastKey = -1;
  }

  return { observe, reset, get home() { return home; }, get away() { return away; } };
}

/** Value of a feed row for the momentum strip (model estimates). */
export function rowValue(type, label) {
  if (type === 16) return { dv: 1.0, big: true };
  if (type === 15 || type === 13 || type === 14) return { dv: 0.12, big: true };
  if (type === 17) return { dv: 0.0, big: true };
  if (type === 1 || type === 2) return { dv: 0.015, big: false };
  return { dv: 0.0, big: false };
}

/**
 * Passing network: average positions + top links from completed passes.
 * The renderer feeds `{ from: {x,y,team,shirt}, to: {x,y} }` sightings; the
 * network aggregates them into nodes (avg pos per shirt) and weighted links.
 */
export function createPassingNetwork() {
  const touches = new Map(); // key team:shirt -> {x,y,n}
  const links = new Map(); // key from>to -> n

  function addTouch(p) {
    const k = `${p.team}:${p.shirt}`;
    const t = touches.get(k) ?? { x: 0, y: 0, n: 0, team: p.team, shirt: p.shirt };
    t.x += p.x;
    t.y += p.y;
    t.n += 1;
    touches.set(k, t);
  }

  function observePass(from, to) {
    if (!from || !to) return;
    // Both ends of a pass are touches: the passer and the receiver both need
    // a node, otherwise links to a player who has not passed yet disappear.
    addTouch(from);
    addTouch(to);
    const lk = `${from.team}:${from.shirt}>${to.team}:${to.shirt}`;
    links.set(lk, (links.get(lk) ?? 0) + 1);
  }

  function snapshot(topLinks = 5, team = null) {
    const teamFilter = team === 0 || team === 1 ? team : null;
    const nodes = [...touches.values()]
      .filter((t) => teamFilter === null || t.team === teamFilter)
      .map((t) => ({
        x: t.x / Math.max(1, t.n),
        y: t.y / Math.max(1, t.n),
        n: t.n,
        team: t.team,
        shirt: t.shirt,
      }));
    const teamOf = (key) => Number(key.split(':')[0]);
    const top = [...links.entries()]
      .filter(([k]) => {
        if (teamFilter === null) return true;
        const [a, b] = k.split('>');
        return teamOf(a) === teamFilter && teamOf(b) === teamFilter;
      })
      .sort((a, b) => b[1] - a[1])
      .slice(0, topLinks)
      .map(([k, n]) => {
        const [a, b] = k.split('>');
        return { a, b, n };
      });
    return {
      nodes,
      links: top,
      maxTouches: nodes.reduce((m, n) => Math.max(m, n.n), 0),
      maxLink: top.reduce((m, l) => Math.max(m, l.n), 0),
    };
  }

  function reset() {
    touches.clear();
    links.clear();
  }

  return { observePass, snapshot, reset };
}

export { shotXg, xtAt, xtGain };
