// Slide-in analysis panel: team dribble summary and per-player detail.
//
// Owns its own DOM subtree (built from the shell in index.html), renders
// from the pure model in dribbles.js, and reports user intent outward
// through callbacks — seeking the sim, driving the pitch layer's selection
// and exporting data. It never touches the engine directly.

import {
  POST_SHOT_WINDOW,
  effectivenessLabel,
  formatClock,
  zoneLabel,
  zoneOf,
  zoneWorldRect,
} from './dribbles.js';

const POSITIONS = {
  1: 'Goalkeeper', 2: 'Defender', 3: 'Midfielder', 4: 'Forward', 5: 'Substitute',
};

const PITCH_L = 52.5;
const PITCH_H = 34;

function el(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (key === 'class') node.className = value;
    else if (key === 'text') node.textContent = value;
    else if (key === 'html') node.innerHTML = value;
    else if (key.startsWith('on') && typeof value === 'function') {
      node.addEventListener(key.slice(2).toLowerCase(), value);
    } else if (value !== false && value != null) node.setAttribute(key, value === true ? '' : value);
  }
  for (const child of children.flat()) {
    if (child == null) continue;
    if (typeof child === 'string') node.append(document.createTextNode(child));
    // Structural markers (e.g. `{ redraw }`) are not DOM: the caller
    // returns them alongside the node, never as children.
    else if (child.nodeType) node.append(child);
  }
  return node;
}

function pctText(rate) {
  return rate == null ? '—' : `${Math.round(rate * 100)}%`;
}

function starsText(n) {
  if (!n) return '';
  return `${'★'.repeat(n)}${'☆'.repeat(5 - n)}`;
}

/**
 * @param shell the `#player-panel` element from index.html
 * @param callbacks `{ onSeek(secs), onSelection(selection), onZoneSelection(zone),
 *   onToggleZones(), onHorizonChange(mode), status(msg) }`
 */
export function createPlayerPanel(shell, callbacks = {}) {
  if (!shell) return null;

  const ui = {
    tabs: shell.querySelector('.pp-tabs'),
    close: shell.querySelector('.pp-close'),
    context: shell.querySelector('.pp-context'),
    body: shell.querySelector('.pp-body'),
    actions: shell.querySelector('.pp-actions'),
  };

  let index = null;
  let team = 0;
  let pid = null;
  let view = 'team';
  let compare = false;
  let selectedZone = null;
  let filters = { period: 'all', state: 'all' };
  let horizonMode = 'sofar'; // stats window: live clock, or the full match
  let nowSecs = Infinity;    // sim clock while live (∞ until the app reports one)
  let lastHorizon;           // snapped stat boundary of the last render
  let clockEl = null;        // live "up to mm:ss" caption
  let clockText = '';
  let pendingLive = false;   // a stat boundary was crossed while focus was inside the body
  let tooltipTimer = 0;
  let liveVision = null;      // latest live vision snapshot from the app
  let liveVisionBody = null;  // reused DOM node inside the player view
  let liveVisionSig = '';

  /** The panel is open when it carries `is-open` (CSS drives the slide). */
  const isOpen = () => shell.classList.contains('is-open');

  const tooltip = el('div', { class: 'pp-tooltip', role: 'status', hidden: true });

  function hideTooltip() {
    if (tooltipTimer) clearTimeout(tooltipTimer);
    tooltipTimer = setTimeout(() => { tooltip.hidden = true; }, 60);
  }

  function showTooltip(html, x, y) {
    if (tooltipTimer) clearTimeout(tooltipTimer);
    tooltip.innerHTML = html;
    tooltip.hidden = false;
    // Keep the nowrap box inside the map: hugging the right/bottom edge
    // would otherwise overflow the panel body (which clips horizontally).
    const host = tooltip.parentElement;
    const hw = Number.isFinite(host?.clientWidth) ? host.clientWidth : Infinity;
    const hh = Number.isFinite(host?.clientHeight) ? host.clientHeight : Infinity;
    const tw = Number.isFinite(tooltip.offsetWidth) ? tooltip.offsetWidth : 0;
    const th = Number.isFinite(tooltip.offsetHeight) ? tooltip.offsetHeight : 0;
    tooltip.style.left = `${Math.max(2, Math.min(x, hw - tw - 4))}px`;
    tooltip.style.top = `${Math.max(2, Math.min(y, hh - th - 4))}px`;
  }

  // ── Scope: filters + live time window ─────────────────────────
  /** Stat filters for the current view, including the live time horizon. */
  function scopeFilters() {
    return {
      period: filters.period,
      state: filters.state,
      until: horizonMode === 'sofar' ? nowSecs : null,
    };
  }

  /** `mm:ss` of the live horizon, or null while no clock is known yet. */
  function horizonClock() {
    return Number.isFinite(nowSecs) ? formatClock(nowSecs) : null;
  }

  function clockCaption() {
    const clock = horizonClock();
    return clock ? `up to ${clock}` : 'up to now';
  }

  /**
   * The app reports the sim clock every frame. Re-render only when the stat
   * boundary moves — the model snaps horizons, so most frames are a no-op.
   */
  function setNow(matchSecs) {
    if (!Number.isFinite(matchSecs)) return;
    nowSecs = matchSecs;
    if (horizonMode !== 'sofar') return;
    const caption = clockCaption();
    if (clockEl && caption !== clockText) {
      clockText = caption;
      clockEl.textContent = caption;
    }
    if (!index || !isOpen()) return;
    const horizon = index.horizonAt(nowSecs);
    if (horizon === lastHorizon) return;
    // Never rebuild the panel under a keyboard user mid-interaction — an open
    // <select> would snap shut, and a focused button would vanish. Catch up
    // when focus leaves the rebuilt sections.
    if (ui.body.contains(document.activeElement) || ui.actions.contains(document.activeElement)) {
      pendingLive = true;
      return;
    }
    lastHorizon = horizon;
    render();
  }

  function setHorizonMode(mode) {
    if (mode !== 'sofar' && mode !== 'whole') return;
    if (horizonMode === mode) return;
    horizonMode = mode;
    if (isOpen()) render();
    callbacks.onHorizonChange?.(mode);
  }

  function scopeRow() {
    const option = (mode, text) => el('button', {
      type: 'button',
      class: 'seg-btn',
      'aria-pressed': String(horizonMode === mode),
      text,
      onclick: () => setHorizonMode(mode),
    });
    clockText = clockCaption();
    clockEl = horizonMode === 'sofar'
      ? el('span', { class: 'pp-scope-clock', text: clockText })
      : null;
    return el('div', { class: 'pp-scope' },
      el('div', { class: 'seg', role: 'group', 'aria-label': 'Stats time window' },
        option('sofar', 'So far'), option('whole', 'Whole match')),
      clockEl,
    );
  }

  /** Honest empty state for a feed that carries no take-on/carry events. */
  function feedNote() {
    return el('div', { class: 'pp-block pp-feed-empty' },
      el('div', { class: 'pp-feed-title', text: 'No dribble events in this feed' }),
      el('div', {
        class: 'pp-feed-body',
        text: 'This match file records no take-ons or carries, so there are no zones or dribble stats to show. The pitch, events and other overlays still work.',
      }),
    );
  }

  // ── Shared blocks ─────────────────────────────────────────────

  function insightList(notes) {
    if (!notes.length) return null;
    return el('div', { class: 'pp-insights' },
      notes.map((n) => el('div', { class: `pp-insight pp-insight-${n.kind}` },
        el('div', { class: 'pp-insight-title', text: n.title }),
        el('div', { class: 'pp-insight-body', text: n.body }),
      )),
    );
  }

  function metricRow(label, value, pct, stars, hint) {
    const bar = el('div', { class: 'pp-pct', title: pct == null ? 'Not enough team-mates with sample' : `${pct}th percentile` },
      el('i', { style: `width:${pct == null ? 0 : pct}%` }),
    );
    return el('div', { class: 'pp-metric' },
      el('div', { class: 'pp-metric-label', text: label, title: hint || label }),
      el('div', { class: 'pp-metric-value', text: value }),
      el('div', { class: 'pp-metric-pct' }, bar,
        el('span', { class: 'pp-metric-star', text: starsText(stars) })),
    );
  }

  function filterRow() {
    const half = el('select', {
      class: 'pp-select',
      'aria-label': 'Time period',
      onchange: (ev) => { filters.period = ev.target.value; render(); },
    },
      el('option', { value: 'all', selected: filters.period === 'all', text: 'Both halves' }),
      el('option', { value: '1', selected: filters.period === '1', text: 'First half' }),
      el('option', { value: '2', selected: filters.period === '2', text: 'Second half' }),
    );
    const state = el('select', {
      class: 'pp-select',
      'aria-label': 'Game state',
      onchange: (ev) => { filters.state = ev.target.value; render(); },
    },
      el('option', { value: 'all', selected: filters.state === 'all', text: 'Any game state' }),
      el('option', { value: 'level', selected: filters.state === 'level', text: 'Scores level' }),
      el('option', { value: 'ahead', selected: filters.state === 'ahead', text: 'Team ahead' }),
      el('option', { value: 'behind', selected: filters.state === 'behind', text: 'Team behind' }),
    );
    // Player scope: picking a name opens that player's analysis, "All
    // players" returns to the team view — one selection, two lenses.
    const inPlayerView = view === 'player' && pid;
    const player = el('select', {
      class: 'pp-select',
      'aria-label': 'Player',
      onchange: (ev) => {
        if (ev.target.value === 'all') {
          pid = null;
          view = 'team';
          render();
          callbacks.onSelection?.({ scope: 'team', team });
        } else {
          selectPlayer(ev.target.value);
        }
      },
    },
      el('option', { value: 'all', selected: !inPlayerView, text: 'All players' }),
      index.rosterFor(team).map((p) => el('option', {
        value: p.pid,
        selected: Boolean(inPlayerView && pid === p.pid),
        // The shirt is already the prefix; an unnamed player needs no
        // "Player 12" echo next to it.
        text: `#${p.shirt}${p.name ? ` ${p.name}` : ''}`,
      })),
    );
    return el('div', { class: 'pp-filters' }, half, state, player);
  }

  function zoneChip() {
    if (!selectedZone) return null;
    return el('button', {
      class: 'pp-chip',
      type: 'button',
      title: 'Clear the zone filter',
      text: `Zone ${selectedZone}: ${zoneLabel(selectedZone)} ✕`,
      onclick: () => setZone(null),
    });
  }

  // ── Zone map (player view): circles encode volume/outcome/confidence ──
  function drawZoneMap(canvas, stats, teamSide) {
    const c = canvas.getContext('2d');
    const W = canvas.width;
    const H = canvas.height;
    c.clearRect(0, 0, W, H);
    // Mini pitch, same dressing as the sidebar minimap.
    c.fillStyle = '#1e3822';
    c.fillRect(0, 0, W, H);
    c.strokeStyle = 'rgba(255,255,255,0.35)';
    c.lineWidth = 1;
    c.strokeRect(1, 1, W - 2, H - 2);
    c.beginPath();
    c.moveTo(W / 2, 1);
    c.lineTo(W / 2, H - 1);
    c.stroke();

    const pad = 4;
    const mx = (x) => pad + ((x + PITCH_L) / (PITCH_L * 2)) * (W - pad * 2);
    const my = (y) => pad + ((PITCH_H - y) / (PITCH_H * 2)) * (H - pad * 2);

    // Zone grid.
    c.strokeStyle = 'rgba(233,242,255,0.18)';
    for (let z = 1; z <= 12; z++) {
      const r = zoneWorldRect(z, teamSide);
      const x0 = mx(r.x0);
      const x1 = mx(r.x1);
      const yTop = my(r.y1);
      const yBottom = my(r.y0);
      c.strokeRect(x0, yTop, x1 - x0, yBottom - yTop);
    }

    const maxAttempts = stats.zones.reduce((m, z) => Math.max(m, z.attempts), 0) || 1;
    stats.zones.forEach((zone, i) => {
      if (!zone.attempts) return;
      const r = zoneWorldRect(i + 1, teamSide);
      const cx = mx((r.x0 + r.x1) / 2);
      const cy = my((r.y0 + r.y1) / 2);
      const radius = 6 + 16 * Math.sqrt(zone.attempts / maxAttempts);
      const success = zone.success ?? 0;
      const hue = 120 * Math.min(1, Math.max(0, success));
      c.beginPath();
      c.arc(cx, cy, radius, 0, Math.PI * 2);
      c.fillStyle = `hsla(${hue.toFixed(0)}, 75%, 50%, 0.55)`;
      c.fill();
      // Ring weight = confidence (sample size).
      c.lineWidth = zone.attempts >= 6 ? 3 : zone.attempts >= 3 ? 2 : 1;
      c.strokeStyle = `hsla(${hue.toFixed(0)}, 85%, 72%, 0.95)`;
      c.stroke();
      if (selectedZone === i + 1) {
        c.lineWidth = 2;
        c.strokeStyle = '#ffbe5a';
        c.beginPath();
        c.arc(cx, cy, radius + 3, 0, Math.PI * 2);
        c.stroke();
      }
      c.fillStyle = 'rgba(13,17,23,0.9)';
      c.font = 'bold 9px ui-sans-serif, sans-serif';
      c.textAlign = 'center';
      c.textBaseline = 'middle';
      c.fillText(String(zone.attempts), cx, cy);
    });
    return { mx, my };
  }

  function zoneMapBlock(stats, teamSide) {
    const canvas = el('canvas', { class: 'pp-zonemap', width: 336, height: 217, tabindex: 0,
      'aria-label': 'Dribble zone map: circle size is attempts, colour is success rate, ring weight is sample size' });
    let geom = null;
    const redraw = () => { geom = drawZoneMap(canvas, stats, teamSide); };
    redraw();

    const zoneAt = (ev) => {
      const rect = canvas.getBoundingClientRect();
      const px = ((ev.clientX - rect.left) / rect.width) * canvas.width;
      const py = ((ev.clientY - rect.top) / rect.height) * canvas.height;
      const wx = ((px - 4) / (canvas.width - 8)) * (PITCH_L * 2) - PITCH_L;
      const wy = PITCH_H - ((py - 4) / (canvas.height - 8)) * (PITCH_H * 2);
      return zoneOf(wx, wy, teamSide);
    };
    canvas.addEventListener('mousemove', (ev) => {
      const z = zoneAt(ev);
      const zone = stats.zones[z - 1];
      if (!zone || !zone.attempts) { hideTooltip(); return; }
      const box = canvas.parentElement.getBoundingClientRect();
      showTooltip(
        `<strong>${zoneLabel(z)}</strong><br>` +
        `${zone.attempts} attempt${zone.attempts === 1 ? '' : 's'} · ${pctText(zone.success)} success<br>` +
        `${zone.avgGain == null ? 'no measured gain' : `${zone.avgGain.toFixed(1)} m gained`}<br>` +
        `${zone.shots} shot${zone.shots === 1 ? '' : 's'} within ${POST_SHOT_WINDOW}s`,
        ev.clientX - box.left + 10,
        ev.clientY - box.top + 10,
      );
    });
    canvas.addEventListener('mouseleave', hideTooltip);
    canvas.addEventListener('click', (ev) => setZone(zoneAt(ev)));
    canvas.addEventListener('keydown', (ev) => {
      if (ev.key !== 'Enter' && ev.key !== ' ') return;
      ev.preventDefault();
      if (selectedZone) { setZone(null); return; }
      // The keyboard has no cursor to click with: pick the busiest zone.
      let best = -1;
      stats.zones.forEach((z, i) => { if (z.attempts && (best < 0 || z.attempts > stats.zones[best].attempts)) best = i; });
      if (best >= 0) setZone(best + 1);
    });
    const legend = el('div', { class: 'pp-canvas-legend' },
      el('span', { text: 'circle = attempts' }),
      el('span', { text: 'colour = success' }),
      el('span', { text: 'ring = sample' }),
    );
    return {
      node: el('div', { class: 'pp-block' },
        el('div', { class: 'pp-block-title', text: 'Dribble zones' }),
        el('div', { class: 'pp-canvas-wrap' }, canvas, tooltip),
        legend,
      ),
      redraw,
    };
  }

  // ── Minute strip: click a dot to jump there ───────────────────
  function drawStrip(canvas, list, opts = {}) {
    const c = canvas.getContext('2d');
    const W = canvas.width;
    const H = canvas.height;
    c.clearRect(0, 0, W, H);
    const maxT = Math.max(90 * 60, ...list.map((d) => d.t), 1);
    const X = (t) => 6 + (t / maxT) * (W - 12);

    // Half-time break marker.
    c.strokeStyle = 'rgba(48,54,61,0.9)';
    c.lineWidth = 1;
    c.beginPath();
    c.moveTo(0, H / 2);
    c.lineTo(W, H / 2);
    c.stroke();
    c.setLineDash([3, 4]);
    c.beginPath();
    c.moveTo(X(45 * 60), 4);
    c.lineTo(X(45 * 60), H - 4);
    c.stroke();
    c.setLineDash([]);

    c.fillStyle = 'rgba(139,148,158,0.9)';
    c.font = '9px ui-sans-serif, sans-serif';
    c.textAlign = 'center';
    c.textBaseline = 'top';
    for (let m = 0; m <= 90; m += 15) {
      const x = X(m * 60);
      c.fillText(String(m), x, H - 11);
      c.strokeStyle = 'rgba(48,54,61,0.9)';
      c.beginPath();
      c.moveTo(x, H / 2 - 3);
      c.lineTo(x, H / 2 + 3);
      c.stroke();
    }

    const filtered = opts.zone ? list.filter((d) => d.zone === opts.zone) : list;
    const dimmed = new Set(filtered);
    for (const d of list) {
      const isMatch = dimmed.has(d);
      const x = X(d.t);
      const col = (d.zone - 1) % 3;
      const r = col === 2 ? 4.2 : col === 1 ? 3.4 : 2.8;
      c.beginPath();
      c.arc(x, H / 2, r, 0, Math.PI * 2);
      if (!isMatch) {
        c.fillStyle = 'rgba(84,91,100,0.45)';
        c.fill();
        continue;
      }
      c.fillStyle = d.ok ? 'rgba(80,220,140,0.95)' : 'rgba(255,92,92,0.95)';
      c.fill();
    }
    return { X, maxT };
  }

  function stripBlock(list, title) {
    const canvas = el('canvas', { class: 'pp-strip', width: 336, height: 56, tabindex: 0,
      'aria-label': `${title}. Click an attempt to seek the simulation to that moment.` });
    let geom = null;
    const redraw = () => { geom = drawStrip(canvas, list, { zone: selectedZone }); };
    redraw();
    canvas.addEventListener('click', (ev) => {
      const rect = canvas.getBoundingClientRect();
      const px = ((ev.clientX - rect.left) / rect.width) * canvas.width;
      let best = null;
      let bestD = 8 * 8; // pixel hit radius
      for (const d of list) {
        if (selectedZone && d.zone !== selectedZone) continue;
        const dx = geom.X(d.t) - px;
        if (dx * dx < bestD) { bestD = dx * dx; best = d; }
      }
      if (best) callbacks.onSeek?.(best.mt, best);
    });
    canvas.addEventListener('keydown', (ev) => {
      if (ev.key === 'Enter' || ev.key === ' ') {
        ev.preventDefault();
        const first = list.find((d) => !selectedZone || d.zone === selectedZone);
        if (first) callbacks.onSeek?.(first.mt, first);
      }
    });
    return {
      node: el('div', { class: 'pp-block' },
        el('div', { class: 'pp-block-title' }, `${title} `, el('span', { class: 'hint', text: 'click to seek' })),
        canvas,
      ),
      redraw,
    };
  }

  // ── Team view ─────────────────────────────────────────────────
  function teamView() {
    const scope = scopeFilters();
    const stats = index.teamStats(team, scope);
    const dribbles = stats.dribbles;
    // Panel-owned empty copy: "no data in the feed" and "filtered to zero"
    // are different stories, and the model's generic note can't tell them.
    const notes = index.insights(stats, 'this team').filter((n) => n.kind !== 'empty');

    const zonesUsed = stats.zones
      .map((z, i) => ({ ...z, id: i + 1 }))
      .filter((z) => z.attempts > 0)
      .sort((a, b) => b.attempts - a.attempts);

    const noDribbleFeed = !index.hasDribbleData;
    const teamHasNone = !noDribbleFeed && index.teamStats(team).attempts === 0;
    const clock = horizonClock();
    const filtered = filters.period !== 'all' || filters.state !== 'all';
    const emptyText = teamHasNone
      ? 'No dribbles recorded for this team in this match.'
      : horizonMode === 'sofar' && !filtered && clock
        ? `No dribbles yet up to ${clock}. Play on or switch to Whole match.`
        : 'No dribbles match these filters. Try Whole match, another half or game state.';

    const zoneTable = zonesUsed.length
      ? el('table', { class: 'pp-table' },
        el('thead', {}, el('tr', {},
          el('th', { text: 'Zone' }), el('th', { text: 'Att' }),
          el('th', { text: 'Success' }), el('th', { text: 'Rating' }),
        )),
        el('tbody', {}, zonesUsed.map((z) => el('tr', {
          class: selectedZone === z.id ? 'is-selected' : '',
          tabindex: 0,
          onclick: () => setZone(selectedZone === z.id ? null : z.id),
          onkeydown: (ev) => {
            if (ev.key === 'Enter') { ev.preventDefault(); setZone(selectedZone === z.id ? null : z.id); }
          },
        },
          el('td', {}, `#${z.id} `, el('span', { class: 'pp-zone-name', text: zoneLabel(z.id) })),
          el('td', { text: String(z.attempts) }),
          el('td', { text: pctText(z.success) }),
          el('td', { text: effectivenessLabel(Math.round((z.success ?? 0) * 100)) }),
        ))),
      )
      : el('div', { class: 'pp-empty', text: emptyText });

    const scale = el('div', { class: 'pp-scale' },
      [['90–100', 'Elite'], ['75–89', 'Very good'], ['60–74', 'Good'], ['45–59', 'Average'],
       ['30–44', 'Below average'], ['15–29', 'Poor'], ['0–14', 'Critical']].map(([range, label]) =>
        el('div', { class: 'pp-scale-row' },
          el('span', { class: 'pp-scale-range', text: range }),
          el('span', { class: 'pp-scale-bar' }),
          el('span', { class: 'pp-scale-label', text: label }),
        )),
    );

    const roster = el('div', { class: 'pp-roster' },
      index.rosterFor(team).map((p) => {
        const s = index.playerStats(p.pid, scope);
        return el('button', {
          type: 'button',
          class: 'pp-player-btn',
          title: `${p.label} · ${s.attempts} dribbles`,
          onclick: () => selectPlayer(p.pid),
        },
          el('span', { class: 'pp-shirt', text: `#${p.shirt}` }),
          el('span', { class: 'pp-player-name', text: p.name || POSITIONS[p.pos] || 'Player' }),
          el('span', { class: 'pp-player-att', text: s.attempts ? `${s.attempts} att` : '—' }),
        );
      }),
    );

    const summary = el('div', { class: 'pp-summary' },
      el('span', {}, el('strong', { text: String(stats.attempts) }), ' dribbles'),
      el('span', {}, el('strong', { text: pctText(stats.success) }), ' success'),
      el('span', {}, el('strong', { text: String(stats.shotAfter) }), ` shots within ${POST_SHOT_WINDOW}s`),
    );

    const squad = el('div', { class: 'pp-block' },
      el('div', { class: 'pp-block-title', text: 'Squad' }),
      roster,
    );

    // No dribble events anywhere in this feed: explain, don't pretend.
    if (noDribbleFeed) return [filterRow(), feedNote(), summary, squad];

    return [
      filterRow(),
      zoneChip(),
      summary,
      zoneTable,
      el('div', { class: 'pp-block' },
        el('div', { class: 'pp-block-title', text: 'Zone effectiveness' }),
        scale,
      ),
      insightList(notes),
      stripBlock(dribbles, 'Match timeline'),
      squad,
    ];
  }

  // ── Player view ───────────────────────────────────────────────
  function playerView() {
    const entry = index.playerById(pid);
    if (!entry) return [el('div', { class: 'pp-empty', text: 'Pick a player from the squad list.' })];
    const scope = scopeFilters();
    const stats = index.playerStats(pid, scope);
    const pct = index.playerPercentiles(pid, scope);
    const dribbles = stats.dribbles;
    const notes = index.insights(stats, entry.label).filter((n) => n.kind !== 'empty');

    const identity = el('div', { class: 'pp-identity' },
      el('div', { class: 'pp-name' },
        el('strong', { text: entry.label }),
        // The label already carries the shirt when the feed never named him.
        entry.name ? el('span', { class: 'pp-shirt-big', text: `#${entry.shirt}` }) : null,
      ),
      el('div', { class: 'pp-meta', text: `${POSITIONS[entry.pos] ?? 'Player'} · ${team === 0 ? 'Home' : 'Away'}` }),
    );

    if (!index.hasDribbleData) return [identity, filterRow(), feedNote()];

    if (!stats.attempts) {
      const whole = index.playerStats(pid).attempts;
      const clock = horizonClock();
      const body = !whole
        ? 'This player did not carry the ball at any point in this match.'
        : horizonMode === 'sofar' && clock
          ? `Nothing yet up to ${clock} — play on or switch to Whole match.`
          : 'Switch the half or game-state filter, or pick a player who carried the ball.';
      return [
        identity,
        filterRow(),
        el('div', { class: 'pp-empty' },
          el('div', { class: 'pp-empty-title', text: `No dribbles recorded for ${entry.label}` }),
          el('div', { class: 'pp-empty-body', text: body }),
        ),
      ];
    }

    const [def, mid, fin] = stats.thirds;
    const thirdValue = (third, label) => {
      const rating = third.attempts
        ? effectivenessLabel(Math.round((third.success ?? 0) * 100))
        : '—';
      return el('div', { class: 'pp-third' },
        el('div', { class: 'pp-third-name', text: label }),
        el('div', { class: 'pp-third-stats', text: `${third.attempts} att · ${pctText(third.success)} success` }),
        el('div', { class: 'pp-third-rating', text: rating }),
      );
    };

    const compareBlock = !compare ? null : (() => {
      const mates = index.rosterFor(team)
        .map((p) => index.playerStats(p.pid, scope))
        .filter((s) => s.attempts >= 2);
      const avg = (get) => (mates.length ? mates.reduce((a, s) => a + get(s), 0) / mates.length : 0);
      const row = (label, mine, theirs, fmt) => el('tr', {},
        el('td', { text: label }),
        el('td', { text: fmt(mine) }),
        el('td', { text: fmt(theirs) }),
      );
      return el('div', { class: 'pp-block' },
        el('div', { class: 'pp-block-title', text: 'Compared with team-mates' }),
        el('table', { class: 'pp-table pp-compare' },
          el('thead', {}, el('tr', {},
            el('th', { text: 'Metric' }), el('th', { text: entry.label }), el('th', { text: 'Team average' }))),
          el('tbody', {},
            row('Attempts', stats.attempts, avg((s) => s.attempts), (v) => v.toFixed(1)),
            row('Success', stats.success ?? 0, avg((s) => s.success ?? 0), (v) => `${Math.round(v * 100)}%`),
            row('Progressive m', stats.gainTotal, avg((s) => s.gainTotal), (v) => v.toFixed(1)),
            row(`Shot within ${POST_SHOT_WINDOW}s`, stats.shotRate ?? 0, avg((s) => s.shotRate ?? 0), (v) => `${Math.round(v * 100)}%`),
          ),
        ),
      );
    })();

    const zoneMap = zoneMapBlock(stats, team);
    const strip = stripBlock(dribbles, 'Match timeline');

    return [
      identity,
      filterRow(),
      zoneChip(),
      el('div', { class: 'pp-metrics' },
        metricRow('Success rate', pctText(stats.success), pct.success, index.stars(pct.success),
          'Share of dribbles that beat the defender or kept the ball'),
        metricRow('Dribbles', String(stats.attempts), pct.attempts, index.stars(pct.attempts),
          'Take-ons plus progressive carries'),
        metricRow('Progressive metres', stats.gainSamples ? stats.gainTotal.toFixed(0) : '—', pct.gain, index.stars(pct.gain),
          'Metres gained forward on dribbles with a measured end point'),
        metricRow(`Shot within ${POST_SHOT_WINDOW}s`, pctText(stats.shotRate), pct.shotRate, index.stars(pct.shotRate),
          'Dribbles after which the same player shot inside the window'),
      ),
      visionBlock(),
      el('div', { class: 'pp-facts' },
        el('span', {}, el('strong', { text: `${stats.takeonWins}/${stats.takeons}` }), ' take-ons won'),
        el('span', {}, el('strong', { text: String(stats.lost) }), ' dribbles lost'),
        el('span', {}, el('strong', { text: stats.avgGain == null ? '—' : `${stats.avgGain.toFixed(1)} m` }),
          ` avg gain (${stats.gainSamples}/${stats.attempts} measured)`),
      ),
      zoneMap,
      strip,
      el('div', { class: 'pp-block' },
        el('div', { class: 'pp-block-title', text: 'By third' }),
        thirdValue(def, 'Defensive third'),
        thirdValue(mid, 'Middle third'),
        thirdValue(fin, 'Final third'),
      ),
      insightList(notes),
      compareBlock,
    ];
  }

  function renderVisionBody(data) {
    if (!liveVisionBody) return;
    if (data && pid && data.pid !== pid) data = null;
    const sig = data
      ? `${data.pid}:${Math.round((data.coverage ?? 0) * 100)}:${data.visible ?? 0}:${data.blind ?? 0}:${data.best?.shirt ?? ''}:${Math.round((data.best?.p ?? 0) * 100)}`
      : 'none';
    if (sig === liveVisionSig) return;
    liveVisionSig = sig;
    liveVisionBody.innerHTML = '';
    if (!data) {
      liveVisionBody.append(el('span', { class: 'pp-empty-body', text: 'Not on the ball right now.' }));
      return;
    }
    liveVisionBody.append(
      el('span', {}, el('strong', { text: `${Math.round((data.coverage ?? 0) * 100)}%` }), ' coverage'),
      el('span', {}, el('strong', { text: String(data.visible ?? 0) }), ' visible'),
      el('span', {}, el('strong', { text: String(data.blind ?? 0) }), ' blind'),
    );
    if (data.best && Number.isFinite(data.best.shirt)) {
      liveVisionBody.append(el('span', {},
        el('strong', { text: `#${data.best.shirt} ${Math.round((data.best.p ?? 0) * 100)}%` }),
        ' best option'));
    }
  }

  function visionBlock() {
    liveVisionBody = el('div', { class: 'pp-facts' });
    renderVisionBody(liveVision);
    return el('div', { class: 'pp-block' },
      el('div', { class: 'pp-block-title' },
        'Live vision ',
        el('span', { class: 'hint', text: 'while this player is on the ball' })),
      liveVisionBody,
    );
  }

  // ── Render ────────────────────────────────────────────────────
  function activeStats() {
    if (!index) return null;
    const scope = scopeFilters();
    if (view === 'player' && pid) return index.playerStats(pid, scope);
    return index.teamStats(team, scope);
  }

  function render() {
    if (!index) return;
    liveVisionBody = null;
    liveVisionSig = '';
    pendingLive = false;
    // The live view re-renders at stat boundaries only; remember which one
    // this render shows so setNow can tell whether anything has moved.
    lastHorizon = index.horizonAt(horizonMode === 'sofar' ? nowSecs : null);
    // The tooltip element is reused across renders: a filter change while
    // hovering the zone map must not leave it stuck open over new content.
    // Hide it immediately — the 60 ms hover-out delay would show stale text.
    if (tooltipTimer) clearTimeout(tooltipTimer);
    tooltip.hidden = true;
    clockEl = null;
    for (const tab of ui.tabs.querySelectorAll('[role="tab"]')) {
      const active = tab.dataset.tab === view;
      tab.setAttribute('aria-selected', String(active));
      tab.tabIndex = active ? 0 : -1;
    }
    if (view === 'player' && !pid) view = 'team';

    const entry = pid ? index.playerById(pid) : null;
    ui.context.textContent = view === 'player' && entry
      ? [entry.label, entry.name ? `#${entry.shirt}` : null, team === 0 ? 'Home' : 'Away'].filter(Boolean).join(' · ')
      : `${team === 0 ? 'Home' : 'Away'} · ${filters.period === 'all' ? 'both halves' : filters.period === '1' ? 'first half' : 'second half'}`;

    const blocks = [scopeRow(), ...(view === 'player' ? playerView() : teamView()).filter(Boolean)];
    // Live updates re-render mid-read; keep the reader's scroll position.
    const scrollTop = ui.body.scrollTop;
    ui.body.innerHTML = '';
    const redraws = [];
    for (const block of blocks) {
      // `{ node, redraw }` pairs carry a canvas that needs a second pass
      // once it is in the document; plain nodes are appended as they are.
      if (block && block.nodeType !== 1 && typeof block.redraw === 'function') {
        redraws.push(block.redraw);
        if (block.node) ui.body.append(block.node);
        continue;
      }
      ui.body.append(block);
    }
    ui.body.classList.toggle('is-player', view === 'player');
    if (scrollTop) ui.body.scrollTop = scrollTop;

    ui.actions.innerHTML = '';
    const closeBtn = el('button', { type: 'button', class: 'pp-btn', text: 'Close', onclick: close });
    if (view === 'player') {
      ui.actions.append(
        el('button', { type: 'button', class: 'pp-btn pp-btn-primary', text: 'Show dribble zones', onclick: () => callbacks.onToggleZones?.() }),
        el('button', { type: 'button', class: 'pp-btn', 'aria-pressed': String(compare), text: compare ? 'Hide comparison' : 'Compare to average', onclick: () => { compare = !compare; render(); } }),
        el('button', { type: 'button', class: 'pp-btn', text: 'Export data', onclick: exportData }),
      );
    } else {
      ui.actions.append(
        el('button', { type: 'button', class: 'pp-btn', text: 'Export data', onclick: exportData }),
      );
    }
    ui.actions.append(closeBtn);

    // Draw canvases that were appended just now (zone map, strip).
    requestAnimationFrame(() => redraws.forEach((fn) => fn()));
  }

  // ── Public intents ────────────────────────────────────────────
  function setZone(zone) {
    selectedZone = zone;
    render();
    callbacks.onZoneSelection?.(zone);
  }

  function selectPlayer(nextPid) {
    pid = nextPid;
    view = 'player';
    compare = false;
    const entry = index.playerById(pid);
    if (entry) team = entry.team;
    render();
    callbacks.onSelection?.(pid ? { scope: 'player', pid, team } : { scope: 'team', team });
  }

  function exportData() {
    const stats = activeStats();
    if (!stats) return;
    const entry = view === 'player' ? index.playerById(pid) : null;
    const payload = {
      exported: new Date().toISOString(),
      view,
      team,
      player: view === 'player' ? pid : null,
      filters: { ...filters },
      horizon: {
        mode: horizonMode,
        untilSeconds: horizonMode === 'sofar' && Number.isFinite(nowSecs) ? Math.round(nowSecs) : null,
      },
      zone: selectedZone,
      attempts: stats.attempts,
      success: stats.success,
      zones: stats.zones.map((z, i) => ({ zone: i + 1, label: zoneLabel(i + 1), attempts: z.attempts, success: z.success, avgGain: z.avgGain })),
      dribbles: stats.dribbles.map((d) => ({
        minute: formatClock(d.t), team: d.team, player: d.name, kind: d.kind,
        x: d.x, y: d.y, zone: d.zone, ok: d.ok, gain: d.gain, shotAfter: d.shotAfter,
      })),
    };
    const blob = new Blob([JSON.stringify(payload, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    // Feed names can carry characters that break a file name; keep it plain.
    const who = (entry?.label ?? (team === 0 ? 'home' : 'away')).replace(/[^\p{L}\p{N} .-]+/gu, '-');
    a.download = `dribbles-${who}.json`;
    document.body.append(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 2000);
    callbacks.status?.('Dribble data exported as JSON.');
  }

  function open(nextView, { team: nextTeam = team, player = pid } = {}) {
    if (!index) return;
    if (nextTeam != null) team = nextTeam;
    view = nextView;
    pid = nextView === 'player' ? player : pid;
    if (view === 'player' && !pid) view = 'team';
    shell.classList.add('is-open');
    render();
    // Focus the panel so keyboard users land inside it.
    const activeTab = ui.tabs.querySelector('[role="tab"][aria-selected="true"]');
    (activeTab ?? ui.close)?.focus?.();
  }

  function close() {
    shell.classList.remove('is-open');
    pid = null;
    view = 'team';
    selectedZone = null;
    compare = false;
    clockEl = null;
    callbacks.onSelection?.(null);
    callbacks.onZoneSelection?.(null);
  }

  ui.close?.addEventListener('click', close);
  for (const tab of ui.tabs.querySelectorAll('[role="tab"]')) {
    tab.addEventListener('click', () => {
      view = tab.dataset.tab;
      if (view === 'player' && !pid) {
        // Fall back to the first squad player with dribbles.
        const pick = index && index.rosterFor(team).find((p) => index.playerStats(p.pid).attempts > 0);
        pid = pick ? pick.pid : null;
      }
      render();
    });
    tab.addEventListener('keydown', (ev) => {
      if (ev.key !== 'ArrowLeft' && ev.key !== 'ArrowRight') return;
      const tabs = Array.from(ui.tabs.querySelectorAll('[role="tab"]'));
      const i = tabs.indexOf(ev.currentTarget);
      const next = tabs[(i + (ev.key === 'ArrowRight' ? 1 : tabs.length - 1)) % tabs.length];
      next.focus();
      next.click();
    });
  }
  shell.addEventListener('keydown', (ev) => {
    if (ev.key === 'Escape') close();
  });
  // A live update deferred while focus was inside a rebuilt section catches
  // up when focus leaves it — but not when tabbing between its own controls.
  function catchUpLive(ev) {
    if (!pendingLive) return;
    const next = ev.relatedTarget;
    if (next && (ui.body.contains(next) || ui.actions.contains(next))) return;
    pendingLive = false;
    if (isOpen() && index) render();
  }
  ui.body.addEventListener('focusout', catchUpLive);
  ui.actions.addEventListener('focusout', catchUpLive);

  return {
    setIndex(nextIndex, nextTeam = 0) {
      index = nextIndex;
      team = nextTeam;
      pid = null;
      selectedZone = null;
      compare = false;
      filters = { period: 'all', state: 'all' };
      // Unknown clock until the app reports one; a fresh match never inherits
      // the previous match's horizon.
      nowSecs = Infinity;
      lastHorizon = undefined;
      if (isOpen()) render();
    },
    open,
    close,
    isOpen,
    selectPlayer,
    setZone,
    /** Live sim clock (match seconds) — keeps "So far" stats current. */
    setNow,
    setHorizonMode,
    setLiveVision(data) {
      liveVision = data ?? null;
      if (view === 'player' && isOpen()) {
        if (liveVision && pid && liveVision.pid !== pid) liveVision = null;
        renderVisionBody(liveVision);
      }
    },
    get selection() {
      if (!index) return null;
      if (view === 'player' && pid) return { scope: 'player', pid, team };
      return { scope: 'team', team };
    },
    get selectedZone() { return selectedZone; },
    get view() { return view; },
    get horizonMode() { return horizonMode; },
    setTeam(nextTeam) {
      if (team === nextTeam) return;
      team = nextTeam;
      pid = null;
      if (isOpen()) render();
    },
    /** Current selection for the pitch layer (team or player scope). */
    layerStats() {
      return activeStats();
    },
    layerTeam: () => team,
  };
}
