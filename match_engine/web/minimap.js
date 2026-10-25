// Broadcast-style minimap: a small pitch that randomly shows one of several
// analytical views for 20-30 s, then fades out and is replaced by another.
//
// Variations are static (no moving nodes) and drawn on a mini pitch.

const VARIATIONS = [
  'homeHeat', 'awayHeat', 'homeShots', 'awayShots',
  'homePasses', 'awayPasses', 'homeAttacks', 'awayAttacks',
  'homeTackles', 'awayTackles', 'homeRecoveries', 'awayRecoveries',
  'homeProgressive', 'awayProgressive', 'setPieces',
  'combinedHeat', 'homeDefensive', 'awayDefensive',
  'homeShotMap', 'awayShotMap',
];

function miniPitch(ctx, w, h) {
  ctx.fillStyle = '#1e3822';
  ctx.fillRect(0, 0, w, h);
  ctx.strokeStyle = 'rgba(255,255,255,0.35)';
  ctx.lineWidth = 1;
  ctx.strokeRect(2, 2, w - 4, h - 4);
  ctx.beginPath();
  ctx.moveTo(w / 2, 2);
  ctx.lineTo(w / 2, h - 2);
  ctx.stroke();
  ctx.beginPath();
  ctx.arc(w / 2, h / 2, h * 0.12, 0, Math.PI * 2);
  ctx.stroke();
  // Goal posts.
  const gw = 3;
  const gh = h * 0.18;
  const gy = (h - gh) / 2;
  ctx.fillStyle = 'rgba(255,255,255,0.9)';
  ctx.fillRect(0, gy, 3, gh);
  ctx.fillRect(w - 3, gy, 3, gh);
  ctx.fillRect(0, gy, gw, 2);
  ctx.fillRect(0, gy + gh - 2, gw, 2);
  ctx.fillRect(w - gw, gy, gw, 2);
  ctx.fillRect(w - gw, gy + gh - 2, gw, 2);
}

function toMini(x, y, w, h) {
  return [2 + (x + 52.5) / 105 * (w - 4), 2 + (34 - y) / 68 * (h - 4)];
}

export function createMinimap(canvas, titleEl) {
  if (!canvas) return null;
  const ctx = canvas.getContext('2d');
  const W = canvas.width;
  const H = canvas.height;
  const data = {
    heat: { home: null, away: null },
    shots: { home: [], away: [] },
    passes: { home: [], away: [] },
    attacks: { home: [], away: [] },
    tackles: { home: [], away: [] },
    recoveries: { home: [], away: [] },
    progressive: { home: [], away: [] },
    setPieces: [],
  };

  let current = null;
  let alpha = 0;
  let phase = 'idle'; // idle | fadein | hold | fadeout
  let phaseStart = 0;
  let holdDuration = 12 + Math.random() * 6;
  let nextSpawn = 3 + Math.random() * 5;

  function scheduleNext(now) {
    nextSpawn = now + 3 + Math.random() * 8;
    phase = 'idle';
    current = null;
    alpha = 0;
    holdDuration = 12 + Math.random() * 6;
  }

  function pickVariation() {
    return VARIATIONS[Math.floor(Math.random() * VARIATIONS.length)];
  }

  function drawVariation(v, a) {
    ctx.save();
    ctx.globalAlpha = a;
    miniPitch(ctx, W, H);

    if (v === 'homeHeat' || v === 'awayHeat') {
      const team = v === 'homeHeat' ? 0 : 1;
      const heat = data.heat[team === 0 ? 'home' : 'away'];
      const rgb = team === 0 ? [255, 180, 80] : [120, 180, 255];
      if (heat && heat.max > 0) {
        const cols = heat.cols;
        const rows = heat.rows;
        const cw = (W - 4) / cols;
        const ch = (H - 4) / rows;
        const invMax = 1 / heat.max;
        for (let r = 0; r < rows; r++) {
          for (let c = 0; c < cols; c++) {
            const val = heat.grid[r * cols + c] * invMax;
            if (val < 0.04) continue;
            const a = Math.pow(val, 0.6) * 0.55;
            const cx = 2 + (c + 0.5) * cw;
            const cy = 2 + (r + 0.5) * ch;
            const rad = Math.max(cw, ch) * 1.6;
            const g = ctx.createRadialGradient(cx, cy, 0, cx, cy, rad);
            g.addColorStop(0, `rgba(${rgb[0]},${rgb[1]},${rgb[2]},${a})`);
            g.addColorStop(1, `rgba(${rgb[0]},${rgb[1]},${rgb[2]},0)`);
            ctx.fillStyle = g;
            ctx.fillRect(cx - rad, cy - rad, rad * 2, rad * 2);
          }
        }
      }
      titleEl.textContent = (team === 0 ? 'Home' : 'Away') + ' heat';
    } else if (v === 'homeShots' || v === 'awayShots') {
      const team = v === 'homeShots' ? 0 : 1;
      const shots = data.shots[team === 0 ? 'home' : 'away'];
      const rgb = team === 0 ? '255,255,255' : '120,180,255';
      const goalX = team === 0 ? 52.5 : -52.5;
      for (const s of shots) {
        const [sx, sy] = toMini(s.x, s.y, W, H);
        ctx.beginPath();
        ctx.arc(sx, sy, 3, 0, Math.PI * 2);
        ctx.fillStyle = `rgba(${rgb},0.9)`;
        ctx.fill();
        const [gx, gy] = toMini(goalX, s.targetY || 0, W, H);
        ctx.beginPath();
        ctx.moveTo(sx, sy);
        ctx.lineTo(gx, gy);
        ctx.strokeStyle = `rgba(${rgb},0.3)`;
        ctx.lineWidth = 0.8;
        ctx.stroke();
      }
      titleEl.textContent = (team === 0 ? 'Home' : 'Away') + ' shots';
    } else if (v === 'homePasses' || v === 'awayPasses') {
      const team = v === 'homePasses' ? 0 : 1;
      const passes = data.passes[team === 0 ? 'home' : 'away'];
      const rgb = team === 0 ? '255,255,255' : '120,180,255';
      for (const p of passes) {
        const [sx, sy] = toMini(p.x, p.y, W, H);
        ctx.beginPath();
        ctx.arc(sx, sy, 2, 0, Math.PI * 2);
        ctx.fillStyle = `rgba(${rgb},0.6)`;
        ctx.fill();
      }
      titleEl.textContent = (team === 0 ? 'Home' : 'Away') + ' touches';
    } else if (v === 'homeAttacks' || v === 'awayAttacks') {
      const team = v === 'homeAttacks' ? 0 : 1;
      const attacks = data.attacks[team === 0 ? 'home' : 'away'];
      const rgb = team === 0 ? '255,200,80' : '120,180,255';
      for (const a of attacks) {
        const [sx, sy] = toMini(a.x, a.y, W, H);
        ctx.beginPath();
        ctx.arc(sx, sy, 2.5, 0, Math.PI * 2);
        ctx.fillStyle = `rgba(${rgb},0.8)`;
        ctx.fill();
      }
      titleEl.textContent = (team === 0 ? 'Home' : 'Away') + ' attacks';
    } else if (v === 'homeTackles' || v === 'awayTackles') {
      const team = v === 'homeTackles' ? 0 : 1;
      const tackles = data.tackles[team === 0 ? 'home' : 'away'];
      const rgb = team === 0 ? '255,100,100' : '255,100,100';
      for (const t of tackles) {
        const [sx, sy] = toMini(t.x, t.y, W, H);
        ctx.beginPath();
        ctx.arc(sx, sy, 2.5, 0, Math.PI * 2);
        ctx.fillStyle = `rgba(${rgb},0.8)`;
        ctx.fill();
      }
      titleEl.textContent = (team === 0 ? 'Home' : 'Away') + ' tackles';
    } else if (v === 'homeRecoveries' || v === 'awayRecoveries') {
      const team = v === 'homeRecoveries' ? 0 : 1;
      const recs = data.recoveries[team === 0 ? 'home' : 'away'];
      const rgb = team === 0 ? '100,255,150' : '100,255,150';
      for (const r of recs) {
        const [sx, sy] = toMini(r.x, r.y, W, H);
        ctx.beginPath();
        ctx.arc(sx, sy, 2, 0, Math.PI * 2);
        ctx.fillStyle = `rgba(${rgb},0.7)`;
        ctx.fill();
      }
      titleEl.textContent = (team === 0 ? 'Home' : 'Away') + ' recoveries';
    } else if (v === 'homeProgressive' || v === 'awayProgressive') {
      const team = v === 'homeProgressive' ? 0 : 1;
      const progs = data.progressive[team === 0 ? 'home' : 'away'];
      const rgb = team === 0 ? '255,220,100' : '255,220,100';
      for (const p of progs) {
        const [sx, sy] = toMini(p.x, p.y, W, H);
        ctx.beginPath();
        ctx.arc(sx, sy, 2, 0, Math.PI * 2);
        ctx.fillStyle = `rgba(${rgb},0.7)`;
        ctx.fill();
      }
      titleEl.textContent = (team === 0 ? 'Home' : 'Away') + ' progressive';
    } else if (v === 'combinedHeat') {
      // Both teams' heat overlaid.
      const homeHeat = data.heat.home;
      const awayHeat = data.heat.away;
      if (homeHeat && homeHeat.max > 0) {
        const cols = homeHeat.cols;
        const rows = homeHeat.rows;
        const cw = (W - 4) / cols;
        const ch = (H - 4) / rows;
        const invMax = 1 / homeHeat.max;
        for (let r = 0; r < rows; r++) {
          for (let c = 0; c < cols; c++) {
            const val = homeHeat.grid[r * cols + c] * invMax;
            if (val < 0.04) continue;
            const a = Math.pow(val, 0.6) * 0.45;
            const cx = 2 + (c + 0.5) * cw;
            const cy = 2 + (r + 0.5) * ch;
            const rad = Math.max(cw, ch) * 1.6;
            const g = ctx.createRadialGradient(cx, cy, 0, cx, cy, rad);
            g.addColorStop(0, `rgba(255,180,80,${a})`);
            g.addColorStop(1, 'rgba(255,180,80,0)');
            ctx.fillStyle = g;
            ctx.fillRect(cx - rad, cy - rad, rad * 2, rad * 2);
          }
        }
      }
      if (awayHeat && awayHeat.max > 0) {
        const cols = awayHeat.cols;
        const rows = awayHeat.rows;
        const cw = (W - 4) / cols;
        const ch = (H - 4) / rows;
        const invMax = 1 / awayHeat.max;
        for (let r = 0; r < rows; r++) {
          for (let c = 0; c < cols; c++) {
            const val = awayHeat.grid[r * cols + c] * invMax;
            if (val < 0.04) continue;
            const a = Math.pow(val, 0.6) * 0.4;
            const cx = 2 + (c + 0.5) * cw;
            const cy = 2 + (r + 0.5) * ch;
            const rad = Math.max(cw, ch) * 1.6;
            const g = ctx.createRadialGradient(cx, cy, 0, cx, cy, rad);
            g.addColorStop(0, `rgba(120,180,255,${a})`);
            g.addColorStop(1, 'rgba(120,180,255,0)');
            ctx.fillStyle = g;
            ctx.fillRect(cx - rad, cy - rad, rad * 2, rad * 2);
          }
        }
      }
      titleEl.textContent = 'Combined heat';
    } else if (v === 'homeDefensive' || v === 'awayDefensive') {
      const team = v === 'homeDefensive' ? 0 : 1;
      const teamKey = team === 0 ? 'home' : 'away';
      const tackles = data.tackles[teamKey] || [];
      const recs = data.recoveries[teamKey] || [];
      const rgb = team === 0 ? '255,100,100' : '255,100,100';
      for (const t of tackles) {
        const [sx, sy] = toMini(t.x, t.y, W, H);
        ctx.beginPath();
        ctx.arc(sx, sy, 2.5, 0, Math.PI * 2);
        ctx.fillStyle = `rgba(${rgb},0.8)`;
        ctx.fill();
      }
      for (const r of recs) {
        const [sx, sy] = toMini(r.x, r.y, W, H);
        ctx.beginPath();
        ctx.arc(sx, sy, 2, 0, Math.PI * 2);
        ctx.fillStyle = 'rgba(100,255,150,0.7)';
        ctx.fill();
      }
      titleEl.textContent = (team === 0 ? 'Home' : 'Away') + ' defence';
    } else if (v === 'homeShotMap' || v === 'awayShotMap') {
      const team = v === 'homeShotMap' ? 0 : 1;
      const shots = data.shots[team === 0 ? 'home' : 'away'];
      const rgb = team === 0 ? '255,255,255' : '120,180,255';
      const goalX = team === 0 ? 52.5 : -52.5;
      for (const s of shots) {
        const [sx, sy] = toMini(s.x, s.y, W, H);
        ctx.beginPath();
        ctx.arc(sx, sy, 3, 0, Math.PI * 2);
        ctx.fillStyle = `rgba(${rgb},0.9)`;
        ctx.fill();
        const [gx, gy] = toMini(goalX, 0, W, H);
        ctx.beginPath();
        ctx.moveTo(sx, sy);
        ctx.lineTo(gx, gy);
        ctx.strokeStyle = `rgba(${rgb},0.4)`;
        ctx.lineWidth = 0.8;
        ctx.stroke();
      }
      titleEl.textContent = (team === 0 ? 'Home' : 'Away') + ' shot map';
    }

    ctx.restore();
  }

  function update(now) {
    if (phase === 'idle') {
      if (now >= nextSpawn) {
        current = pickVariation();
        phase = 'fadein';
        phaseStart = now;
      }
      return;
    }
    const elapsed = now - phaseStart;
    if (phase === 'fadein') {
      alpha = Math.min(1, elapsed / 0.5);
      if (alpha >= 1) { phase = 'hold'; phaseStart = now; }
    } else if (phase === 'hold') {
      alpha = 1;
      if (elapsed >= holdDuration) { phase = 'fadeout'; phaseStart = now; }
    } else if (phase === 'fadeout') {
      alpha = Math.max(0, 1 - elapsed / 0.5);
      if (alpha <= 0) scheduleNext(now);
    }
    if (current) drawVariation(current, alpha);
  }

  return { update, data, canvas, ctx };
}
