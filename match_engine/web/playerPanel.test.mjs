import test from 'node:test';
import assert from 'node:assert/strict';

import { createDribbleIndex } from './dribbles.js';
import { createPlayerPanel } from './playerPanel.js';

// ── Mini DOM ───────────────────────────────────────────────────
// The panel touches a narrow slice of the DOM: createElement/append,
// class + attribute reflection, listeners, querySelector by class /
// attribute / tag, and a 2-D canvas. This harness implements exactly
// that surface so the real markup path runs under Node.

function camelizeData(name) {
  return name.slice(5).replace(/-([a-z])/g, (_, c) => c.toUpperCase());
}

/** Single-compound selector match: `.cls`, `tag`, `[attr="v"]`, combos. */
function matchesCompound(node, selector) {
  const token = /(\.[\w-]+)|(\[[\w-]+(?:="([^"]*)")?\])|([a-zA-Z][\w-]*)|(#[\w-]+)/g;
  let consumed = 0;
  let m;
  while ((m = token.exec(selector))) {
    consumed += m[0].length;
    if (m[1]) {
      if (!node.className.split(/\s+/).includes(m[1].slice(1))) return false;
    } else if (m[2]) {
      const name = /^\[([\w-]+)/.exec(m[2])[1];
      const actual = node.attrs[name];
      if (m[3] === undefined ? actual === undefined : actual !== m[3]) return false;
    } else if (m[4]) {
      if (node.tagName !== m[4].toUpperCase()) return false;
    } else if (m[5]) {
      if (node.attrs.id !== m[5].slice(1)) return false;
    }
  }
  return consumed > 0 && consumed === selector.length;
}

class TestNode {
  constructor(tag, doc) {
    this.nodeType = 1;
    this.tagName = String(tag).toUpperCase();
    this.ownerDocument = doc;
    this.children = [];
    this.parent = null;
    this.attrs = Object.create(null);
    this.listeners = Object.create(null);
    this.dataset = {};
    this.style = {};
    this.className = '';
    this.tabIndex = -1;
    this.value = '';
    this._innerHTML = '';
    this._ctx = null;
  }

  get hidden() { return 'hidden' in this.attrs; }
  set hidden(v) { if (v) this.attrs.hidden = ''; else delete this.attrs.hidden; }

  get classList() {
    const node = this;
    const read = () => new Set(node.className.split(/\s+/).filter(Boolean));
    const write = (set) => { node.className = [...set].join(' '); };
    return {
      contains: (c) => read().has(c),
      add: (c) => { const s = read(); s.add(c); write(s); },
      remove: (c) => { const s = read(); s.delete(c); write(s); },
      toggle: (c, force) => {
        const s = read();
        const next = force === undefined ? !s.has(c) : Boolean(force);
        if (next) s.add(c); else s.delete(c);
        write(s);
        return next;
      },
    };
  }

  setAttribute(name, value) {
    this.attrs[name] = String(value);
    if (name === 'class') this.className = String(value);
    if (name === 'width' || name === 'height') this[name] = Number(value);
    if (name.startsWith('data-')) this.dataset[camelizeData(name)] = String(value);
  }

  getAttribute(name) {
    return name in this.attrs ? this.attrs[name] : null;
  }

  append(...nodes) {
    for (const raw of nodes) {
      if (raw == null) continue;
      const node = typeof raw === 'string'
        ? this.ownerDocument.createTextNode(raw)
        : raw;
      if (node.parent) node.parent.detach(node);
      node.parent = this;
      this.children.push(node);
    }
  }

  appendChild(node) { this.append(node); return node; }

  detach(node) {
    const i = this.children.indexOf(node);
    if (i >= 0) this.children.splice(i, 1);
    node.parent = null;
  }

  remove() { if (this.parent) this.parent.detach(this); }

  contains(node) {
    if (node === this) return true;
    return this.children.some((child) => child.contains && child.contains(node));
  }

  get textContent() {
    return this.children
      .map((child) => (child.nodeType === 3 ? child.data : child.textContent))
      .join('');
  }

  set textContent(value) {
    this.children.forEach((child) => { child.parent = null; });
    this.children = [];
    this._innerHTML = '';
    if (value !== '') this.append(this.ownerDocument.createTextNode(String(value)));
  }

  set innerHTML(value) {
    this.children.forEach((child) => { child.parent = null; });
    this.children = [];
    this._innerHTML = String(value);
  }

  get innerHTML() { return this._innerHTML; }

  querySelectorAll(selector) {
    const parts = selector.trim().split(/\s+/);
    // A node matches a descendant chain when its last part matches and each
    // preceding part is found among its ancestors, in order.
    const hasAncestors = (node, chain) => {
      let idx = chain.length - 2;
      let cur = node.parent;
      while (idx >= 0 && cur) {
        if (cur.nodeType === 1 && matchesCompound(cur, chain[idx])) idx--;
        cur = cur.parent;
      }
      return idx < 0;
    };
    const found = [];
    const walk = (node) => {
      for (const child of node.children) {
        if (child.nodeType !== 1) continue;
        if (matchesCompound(child, parts[parts.length - 1]) && hasAncestors(child, parts)) {
          found.push(child);
        }
        walk(child);
      }
    };
    walk(this);
    return found;
  }

  querySelector(selector) { return this.querySelectorAll(selector)[0] ?? null; }

  addEventListener(type, fn) {
    (this.listeners[type] ??= []).push(fn);
  }

  removeEventListener(type, fn) {
    this.listeners[type] = (this.listeners[type] ?? []).filter((f) => f !== fn);
  }

  /** Dispatch a listeners-only event (no bubbling needed here). */
  fire(type, extra = {}) {
    const ev = {
      type,
      target: this,
      currentTarget: this,
      preventDefault() {},
      stopPropagation() {},
      ...extra,
    };
    for (const fn of [...(this.listeners[type] ?? [])]) fn(ev);
    return ev;
  }

  click() { return this.fire('click'); }

  focus() { this.ownerDocument.activeElement = this; }

  getBoundingClientRect() {
    const width = this.width ?? 0;
    const height = this.height ?? 0;
    return { left: 0, top: 0, right: width, bottom: height, width, height };
  }

  get parentElement() { return this.parent; }

  getContext() {
    if (!this._ctx) this._ctx = makeRecordingContext(this);
    return this._ctx;
  }
}

function makeRecordingContext(canvas) {
  const calls = [];
  const ctx = new Proxy({}, {
    get(target, prop) {
      if (prop === 'canvas') return canvas;
      if (prop === 'calls') return calls;
      if (prop === 'measureText') return () => ({ width: 20 });
      if (prop === 'createRadialGradient' || prop === 'createLinearGradient') {
        return () => ({ addColorStop() {} });
      }
      if (typeof prop === 'string') return (...args) => { calls.push([prop, ...args]); };
      return target[prop];
    },
    set(target, prop, value) {
      target[prop] = value;
      return true;
    },
  });
  ctx.calls = calls;
  return ctx;
}

const doc = {
  activeElement: null,
  body: null,
  createElement(tag) { return new TestNode(tag, doc); },
  createTextNode(data) {
    return { nodeType: 3, data: String(data), parent: null, contains: () => false };
  },
};
doc.body = new TestNode('body', doc);

globalThis.document = doc;
globalThis.requestAnimationFrame = (fn) => { fn(0); return 1; };

// Export plumbing: capture the Blob instead of downloading it.
const exportedBlobs = [];
const RealBlob = globalThis.Blob;
globalThis.Blob = class CapturedBlob extends RealBlob {
  constructor(parts, options) {
    super(parts, options);
    exportedBlobs.push(this);
  }
};
URL.createObjectURL = () => `blob:export-${exportedBlobs.length}`;
URL.revokeObjectURL = () => {};

// ── Fixture ────────────────────────────────────────────────────

function row(ty, t, team, pid, name, x, y, opts = {}) {
  return [ty, opts.t ?? t, opts.mt ?? t, opts.period ?? 1, team, pid, name, x, y,
    opts.ex ?? null, opts.ey ?? null, opts.out ?? 1];
}

const HOME_ROSTER = [
  ['h7', 'Seven', 7, 3],
  ['h8', 'Eight', 8, 3],
  ['h9', 'Nine', 9, 4],
  ['h10', 'J. Smith', 10, 4],
  ['h11', 'Eleven', 11, 2],
];

function fixture() {
  const events = [
    row(3, 300, 0, 'h10', 'J. Smith', 40, 30),
    row(3, 320, 0, 'h10', 'J. Smith', 45, -20, { out: 0 }),
    row(3, 340, 0, 'h10', 'J. Smith', 30, 10, { out: 0 }),
    row(3, 360, 0, 'h10', 'J. Smith', 25, -30, { out: 0 }),
    row(3, 380, 0, 'h10', 'J. Smith', 50, 5),
    row(3, 400, 0, 'h10', 'J. Smith', 20, 25, { out: 0 }),
    row(16, 500, 1, 'a9', 'Away Nine', 10, 5),
    row(16, 505, 0, 'h9', 'Nine', 20, 0),
    row(16, 510, 0, 'h9', 'Nine', 30, 0),
    row(83, 600, 0, 'h10', 'J. Smith', 10, 0, { ex: 25, ey: 0 }),
    row(1, 605, 0, 'h10', 'J. Smith', 25, 0),
    row(3, 620, 0, 'h10', 'J. Smith', 0, 0),
    row(3, 640, 0, 'h10', 'J. Smith', -30, 0),
    row(13, 305, 0, 'h10', 'J. Smith', 45, 30),
    row(3, 700, 0, 'h9', 'Nine', -40, 0, { ex: -25, ey: 0 }),
    row(3, 720, 0, 'h9', 'Nine', -20, 10),
    row(3, 740, 0, 'h11', 'Eleven', -10, -10, { ex: 0, ey: -10 }),
    row(3, 760, 0, 'h11', 'Eleven', -25, 5, { out: 0 }),
    row(3, 780, 0, 'h8', 'Eight', 5, 20, { out: 0, ex: 15, ey: 20 }),
    row(3, 800, 0, 'h8', 'Eight', 40, 15, { out: 0 }),
    row(3, 3000, 0, 'h10', 'J. Smith', 30, 0, { period: 2, mt: 5000 }),
    row(34, 0, 0, '', '', 0, 0),
  ];
  return createDribbleIndex({ v: 1, events, roster: [HOME_ROSTER, [['a9', 'Away Nine', 9, 4]]] });
}

/** Fresh shell mirroring #player-panel's markup in index.html. */
function makeShell() {
  const h = (tag, props = {}, ...children) => {
    const node = doc.createElement(tag);
    for (const [key, value] of Object.entries(props)) {
      if (key === 'class') node.className = value;
      else if (key === 'text') node.textContent = value;
      else if (value !== false && value != null) {
        node.setAttribute(key, value === true ? '' : value);
      }
    }
    node.append(...children.filter((c) => c != null));
    return node;
  };
  const tabs = h('div', { class: 'pp-tabs', role: 'tablist' },
    h('button', { type: 'button', role: 'tab', 'data-tab': 'team', 'aria-selected': 'true' }),
    h('button', { type: 'button', role: 'tab', 'data-tab': 'player', 'aria-selected': 'false', tabindex: '-1' }),
  );
  const shell = h('aside', { id: 'player-panel', 'aria-label': 'Dribble analysis' },
    h('div', { class: 'pp-head' }, tabs, h('button', { class: 'pp-close', 'aria-label': 'Close analysis panel' })),
    h('div', { class: 'pp-context' }),
    h('div', { class: 'pp-body', role: 'tabpanel' }),
    h('div', { class: 'pp-actions' }),
  );
  return { shell, tabs };
}

function makePanel(nextIndex = fixture()) {
  const { shell, tabs } = makeShell();
  const log = { seeks: [], selections: [], zones: [], toggles: [], statuses: [], horizons: [] };
  const panel = createPlayerPanel(shell, {
    onSeek: (secs, dribble) => log.seeks.push({ secs, dribble }),
    onSelection: (sel) => log.selections.push(sel),
    onZoneSelection: (z) => log.zones.push(z),
    onToggleZones: () => log.toggles.push(true),
    onHorizonChange: (mode) => log.horizons.push(mode),
    status: (msg) => log.statuses.push(msg),
  });
  panel.setIndex(nextIndex, 0);
  return { panel, shell, tabs, log };
}

// ── Tests ──────────────────────────────────────────────────────

test('the team view renders filters, zone table, scale, insights and squad', () => {
  const { panel, shell } = makePanel();
  assert.equal(panel.isOpen(), false);
  panel.open('team');
  assert.equal(panel.isOpen(), true);
  assert.equal(doc.activeElement.dataset.tab, 'team', 'open() focuses the active tab');

  assert.match(shell.querySelector('.pp-context').textContent, /^Home · both halves$/);

  const body = shell.querySelector('.pp-body');
  // The stats window defaults to the live clock and is switchable.
  const segButtons = body.querySelectorAll('.seg-btn');
  assert.equal(segButtons.length, 2, 'so far / whole match');
  assert.equal(segButtons[0].getAttribute('aria-pressed'), 'true', 'live by default');
  assert.match(body.querySelector('.pp-scope-clock').textContent, /^up to /);

  assert.equal(body.querySelectorAll('select').length, 3, 'half, game state and player filters');
  assert.equal(body.querySelectorAll('.pp-scale-row').length, 7, 'seven effectiveness bands');
  assert.equal(body.querySelectorAll('.pp-player-btn').length, HOME_ROSTER.length);
  assert.equal(body.querySelectorAll('.pp-strip').length, 1, 'timeline scrubber');
  assert.equal(body.querySelectorAll('.pp-metric').length, 0, 'metrics belong to the player view');

  // Zone table: one row per zone that saw a dribble (8 in this fixture).
  const zoneRows = body.querySelectorAll('tbody tr');
  assert.equal(zoneRows.length, 8);
  assert.match(zoneRows[0].textContent, /^#6 /, 'busiest zone sorts first (3 attempts)');
  assert.match(zoneRows[0].textContent, /33%/);

  // Summary: 16 attempts, 9 wins (56%), one shot inside the window.
  const summary = shell.querySelector('.pp-summary').textContent;
  assert.match(summary, /16/);
  assert.match(summary, /56%/);
  assert.match(summary, /1/);

  // Rule-based insights are present for the team.
  assert.ok(body.querySelectorAll('.pp-insight').length >= 1);
});

test('a squad click opens the player view with metrics, zone map and facts', () => {
  const { panel, shell, log } = makePanel();
  panel.open('team');
  const body = shell.querySelector('.pp-body');
  const smith = body.querySelectorAll('.pp-player-btn')
    .find((b) => b.textContent.includes('J. Smith'));
  assert.ok(smith, 'the roster lists J. Smith');
  smith.click();

  assert.equal(panel.view, 'player');
  assert.deepEqual(log.selections.at(-1), { scope: 'player', pid: 'h10', team: 0 });
  assert.equal(shell.querySelector('.pp-context').textContent, 'J. Smith · #10 · Home');

  const playerBody = shell.querySelector('.pp-body');
  assert.equal(playerBody.querySelectorAll('.pp-metric').length, 4);
  assert.equal(playerBody.querySelectorAll('.pp-zonemap').length, 1, 'per-zone circle map');
  assert.equal(playerBody.querySelectorAll('.pp-strip').length, 1);
  assert.equal(playerBody.querySelectorAll('.pp-third').length, 3, 'split by third');
  assert.match(playerBody.textContent, /take-ons won/);

  // Percentile bars carry star ratings.
  const stars = playerBody.querySelectorAll('.pp-metric-star').map((s) => s.textContent);
  assert.ok(stars.some((s) => s.includes('★')), `expected stars, got ${JSON.stringify(stars)}`);

  // Zone map drew its circles (attempts labelled inside them).
  const map = playerBody.querySelector('.pp-zonemap');
  const texts = map.getContext('2d').calls.filter(([name]) => name === 'fillText');
  assert.ok(texts.some(([, text]) => text === '2'), 'busiest zone shows its volume');
  assert.ok(texts.some(([, text]) => text === '1'), 'quiet zones stay labelled too');

  // Player actions appear in the footer.
  const actions = shell.querySelector('.pp-actions').textContent;
  assert.match(actions, /Show dribble zones/);
  assert.match(actions, /Compare to average/);
  assert.match(actions, /Export data/);
});

test('the half filter narrows every stat it touches', () => {
  const { panel, shell } = makePanel();
  panel.open('team');
  const attempts = () => shell.querySelector('.pp-summary')
    .querySelectorAll('strong')[0].textContent;

  assert.equal(attempts(), '16');
  const [half] = shell.querySelector('.pp-body').querySelectorAll('select');
  half.value = '2';
  half.fire('change');
  assert.equal(attempts(), '1', 'only the second-half carry remains');
  assert.equal(shell.querySelector('.pp-body').querySelectorAll('tbody tr').length, 1);
  assert.match(shell.querySelector('.pp-context').textContent, /second half/);

  half.value = '1';
  half.fire('change');
  assert.equal(attempts(), '15');
});

test('the timeline strip seeks the sim to the clicked attempt', () => {
  const { panel, shell, log } = makePanel();
  panel.open('team');
  const strip = shell.querySelector('.pp-body').querySelector('.pp-strip');
  // First team dribble sits at t=300 s → X = 6 + (300/5400)·324 = 24 px.
  strip.fire('click', { clientX: 24, clientY: 28 });
  assert.equal(log.seeks.length, 1);
  assert.equal(log.seeks[0].secs, 300, 'match clock, not the display clock');
  assert.equal(log.seeks[0].dribble.kind, 'takeon');
  assert.equal(log.seeks[0].dribble.zone, 3);

  // Keyboard: Enter seeks to the first attempt in range.
  strip.fire('keydown', { key: 'Enter' });
  assert.equal(log.seeks.length, 2);
  assert.equal(log.seeks[1].secs, 300);
});

test('selecting a zone filters the view and shows a clearable chip', () => {
  const { panel, shell, log } = makePanel();
  panel.open('team');
  panel.setZone(3);
  assert.equal(panel.selectedZone, 3);
  assert.equal(log.zones.at(-1), 3);

  const chip = shell.querySelector('.pp-chip');
  assert.match(chip.textContent, /Zone 3: left flank · final third/);
  const selected = shell.querySelector('.pp-body').querySelectorAll('tbody tr')
    .filter((tr) => tr.classList.contains('is-selected'));
  assert.equal(selected.length, 1);
  assert.match(selected[0].textContent, /^#3 /);

  chip.click();
  assert.equal(panel.selectedZone, null);
  assert.equal(log.zones.at(-1), null);
  assert.equal(shell.querySelector('.pp-chip'), null);
});

test('tabs switch scope, arrow keys move between them, Escape closes', () => {
  const { panel, shell, tabs } = makePanel();
  panel.open('team');
  const [teamTab, playerTab] = tabs.querySelectorAll('[role="tab"]');

  playerTab.click();
  assert.equal(panel.view, 'player');
  // Falls back to the first squad player with dribbles (Eight, h8).
  assert.equal(panel.selection.pid, 'h8');
  assert.equal(playerTab.getAttribute('aria-selected'), 'true');
  assert.equal(teamTab.tabIndex, -1);

  teamTab.fire('keydown', { key: 'ArrowRight' });
  assert.equal(panel.view, 'player', 'wraps back to the player tab');
  playerTab.fire('keydown', { key: 'ArrowLeft' });
  assert.equal(panel.view, 'team');

  shell.fire('keydown', { key: 'Escape' });
  assert.equal(panel.isOpen(), false);
  assert.deepEqual(panel.selection, { scope: 'team', team: 0 });
});

test('the layer scope follows the panel selection for the pitch overlay', () => {
  const { panel } = makePanel();
  panel.open('team');
  assert.equal(panel.layerTeam(), 0);
  assert.equal(panel.layerStats().attempts, 16, 'team layer by default');

  panel.selectPlayer('h10');
  assert.equal(panel.layerStats().attempts, 10, 'player layer once a player is picked');

  panel.close();
  assert.equal(panel.layerStats().attempts, 16, 'closing falls back to the team lens');
  panel.setTeam(1);
  assert.equal(panel.layerTeam(), 1);
  assert.equal(panel.layerStats().attempts, 0, 'away side never dribbled in the fixture');
});

test('export serialises the current view, filters and every attempt', async () => {
  const { panel, shell, log } = makePanel();
  panel.open('player', { player: 'h10' });
  const exportBtn = shell.querySelector('.pp-actions').querySelectorAll('button')
    .find((b) => b.textContent === 'Export data');
  assert.ok(exportBtn, 'the player view offers an export');
  exportBtn.click();

  assert.equal(log.statuses.at(-1), 'Dribble data exported as JSON.');
  const payload = JSON.parse(await exportedBlobs.at(-1).text());
  assert.equal(payload.view, 'player');
  assert.equal(payload.player, 'h10');
  assert.equal(payload.attempts, 10);
  assert.equal(payload.zones.length, 12, 'all twelve zones are reported');
  assert.equal(payload.dribbles.length, 10);
  assert.equal(payload.dribbles[0].minute, '05:00');
  assert.equal(payload.dribbles[0].zone, 3);
  assert.equal(payload.filters.period, 'all');
  assert.equal(payload.horizon.mode, 'sofar', 'the export states its stats window');
  assert.equal(payload.horizon.untilSeconds, null, 'no clock reported yet → whole match');
});

test('the panel is inert before an index arrives', () => {
  const { shell } = makeShell();
  const panel = createPlayerPanel(shell, {});
  panel.open('team');
  assert.equal(panel.isOpen(), false, 'no index → no panel');
  assert.equal(panel.layerStats(), null);
  panel.setZone(3);
  assert.equal(panel.selectedZone, 3, 'selection is remembered for later');
  panel.setIndex(fixture(), 0);
  assert.equal(panel.layerStats().attempts, 16, 'stats arrive with the index');
  assert.equal(panel.selectedZone, null, 'a fresh index resets stale selections');
});

test('the panel grows live with the sim clock and can show the whole match', () => {
  const { panel, shell, log } = makePanel();
  panel.open('team');
  const attempts = () => shell.querySelector('.pp-summary').querySelectorAll('strong')[0].textContent;

  panel.setNow(0);
  assert.equal(attempts(), '0', 'kick-off shows nothing yet');
  assert.match(shell.querySelector('.pp-empty').textContent, /No dribbles yet/);
  panel.setNow(330);
  assert.equal(attempts(), '2', 'two take-ons by 05:30');
  panel.setNow(400);
  assert.equal(attempts(), '6');
  assert.match(shell.querySelector('.pp-scope-clock').textContent, /up to 06:40/);

  // The whole-match view is one click away, and back to the live clock.
  panel.setHorizonMode('whole');
  assert.equal(attempts(), '16');
  assert.equal(panel.horizonMode, 'whole');
  assert.deepEqual(log.horizons, ['whole']);
  const buttons = shell.querySelector('.pp-body').querySelectorAll('.seg-btn');
  assert.equal(buttons[1].getAttribute('aria-pressed'), 'true');
  panel.setHorizonMode('sofar');
  assert.equal(attempts(), '6');
  assert.deepEqual(log.horizons, ['whole', 'sofar']);

  // The sim clock keeps advancing underneath; stats only move when it must.
  panel.setNow(401);
  assert.equal(attempts(), '6', 'no dribble boundary crossed, same numbers');
  panel.setNow(600);
  assert.equal(attempts(), '7', 'the carry at 10:00 joins the live window');
});

test('a feed without dribble events explains itself instead of pretending', () => {
  const index = createDribbleIndex({
    v: 1,
    events: [row(1, 100, 0, 'h10', 'J. Smith', 0, 0, { ex: 10, ey: 0 })],
    roster: [HOME_ROSTER, [['a9', 'Away Nine', 9, 4]]],
  });
  const { panel, shell } = makePanel(index);
  panel.open('team');
  assert.equal(shell.querySelector('.pp-feed-title').textContent, 'No dribble events in this feed');
  assert.equal(shell.querySelectorAll('.pp-strip').length, 0, 'no dead scrubber');
  assert.equal(shell.querySelectorAll('.pp-scale-row').length, 0, 'no rating scale to misread');
  assert.equal(shell.querySelectorAll('.pp-player-btn').length, HOME_ROSTER.length, 'squad stays browseable');

  panel.selectPlayer('h10');
  assert.equal(panel.view, 'player');
  assert.equal(shell.querySelector('.pp-feed-title').textContent, 'No dribble events in this feed');
  assert.equal(shell.querySelectorAll('.pp-metric').length, 0, 'no fake zero metrics');
});

test('anonymous players are labelled by shirt in the squad and the header', () => {
  const index = createDribbleIndex({
    v: 1,
    events: [row(3, 300, 0, 'h49', '', 40, 30)],
    roster: [[['h49', '', 49, 5]], []],
  });
  const { panel, shell } = makePanel(index);
  panel.open('team');

  const squadRow = shell.querySelector('.pp-player-btn');
  assert.match(squadRow.textContent, /#49/);
  assert.match(squadRow.textContent, /Substitute/, 'role stands in for a missing name');
  assert.match(squadRow.getAttribute('title'), /Player 49/);
  const option = shell.querySelector('.pp-select[aria-label="Player"]').querySelectorAll('option')[1];
  assert.equal(option.textContent, '#49', 'the dropdown does not repeat the shirt');

  panel.selectPlayer('h49');
  assert.equal(shell.querySelector('.pp-context').textContent, 'Player 49 · Home');
  assert.match(shell.querySelector('.pp-identity').textContent, /Player 49/);
});

test('live updates wait for an open control instead of rebuilding under it', () => {
  const { panel, shell } = makePanel();
  panel.open('team');
  panel.setNow(0);
  const attempts = () => shell.querySelector('.pp-summary strong').textContent;
  assert.equal(attempts(), '0');

  // Focus a filter (as if its popup were open) — the clock moves on, but the
  // panel must not yank the control away mid-interaction.
  const select = shell.querySelector('.pp-select');
  select.focus();
  panel.setNow(400);
  assert.equal(attempts(), '0', 'the de-rendered view is held');
  assert.equal(panel.layerStats().attempts, 6, 'the model already knows the truth');

  // Leaving the body catches up; tabbing inside it would not.
  const body = shell.querySelector('.pp-body');
  const other = shell.querySelector('.pp-player-btn');
  body.fire('focusout', { relatedTarget: other });
  assert.equal(attempts(), '0', 'moving focus inside the body does not rebuild');
  body.fire('focusout', { relatedTarget: null });
  assert.equal(attempts(), '6');

  // The footer is rebuilt too: hold it the same way.
  const action = shell.querySelector('.pp-actions button');
  action.focus();
  panel.setNow(700);
  assert.equal(attempts(), '6', 'footer focus also holds the rebuild');
  shell.querySelector('.pp-actions').fire('focusout', { relatedTarget: null });
  assert.equal(attempts(), '10', 'catch-up after leaving the footer');
});
