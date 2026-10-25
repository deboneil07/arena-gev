// Highlight replay: a second minimap-like canvas that cycles through the most
// recent important events (goals, shots, corners, penalties, fouls in the
// final third) so viewers never miss them. Each highlight is a frozen frame
// with positions captured at event time, replayed for ~5 s with a fade.

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

export function createHighlightTracker(canvas, titleEl) {
  if (!canvas) return null;
  const ctx = canvas.getContext('2d');
  const W = canvas.width;
  const H = canvas.height;
  const buffer = []; // {t, type, label, team, players:[], ball:{x,y}, x, y}
  const MAX = 12;
  let idx = 0;       // current highlight index
  let phase = 'idle'; // idle | fadein | hold | fadeout
  let phaseStart = 0;
  let holdDur = 5;
  let lastNow = 0;

  function push(ev) {
    if (!ev) return;
    buffer.push(ev);
    if (buffer.length > MAX) buffer.shift();
    // Jump to newest on push.
    idx = buffer.length - 1;
    phase = 'fadein';
    phaseStart = lastNow || (performance.now() / 1000);
  }

  function drawHighlight(hl, alpha, now) {
    ctx.save();
    ctx.globalAlpha = alpha;
    miniPitch(ctx, W, H);
    if (!hl) { ctx.restore(); return; }
    const players = hl.players || [];
    for (const p of players) {
      const [sx, sy] = toMini(p.x, p.y, W, H);
      ctx.beginPath();
      ctx.arc(sx, sy, 2.2, 0, Math.PI * 2);
      ctx.fillStyle = p.team === 0 ? 'rgba(242,242,242,0.85)' : 'rgba(47,129,247,0.85)';
      ctx.fill();
    }
    const blink = 0.5 + 0.5 * Math.sin((now || 0) * 5);
    // ball + event-specific pointer
    if (hl.ball) {
      const [bx, by] = toMini(hl.ball.x, hl.ball.y, W, H);
      // goal: directional arrow toward the goal the ball entered
      const isGoal = hl.type === 16;
      const isShot = hl.type === 13 || hl.type === 14 || hl.type === 15;
      const isSave = hl.type === 10;
      const isCorner = hl.type === 4 || hl.type === 5;
      if (isGoal) {
        const goalX = hl.ball.x >= 0 ? 52.5 : -52.5;
        const [gx, gy] = toMini(goalX, hl.ball.y * 0.3, W, H);
        ctx.strokeStyle = `rgba(255,234,0,${0.7 + 0.3 * blink})`;
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.moveTo(bx, by);
        ctx.lineTo(gx, gy);
        ctx.stroke();
        // arrowhead
        const ang = Math.atan2(gy - by, gx - bx);
        ctx.beginPath();
        ctx.moveTo(gx, gy);
        ctx.lineTo(gx - 6 * Math.cos(ang - 0.4), gy - 6 * Math.sin(ang - 0.4));
        ctx.lineTo(gx - 6 * Math.cos(ang + 0.4), gy - 6 * Math.sin(ang + 0.4));
        ctx.closePath();
        ctx.fillStyle = `rgba(255,234,0,${0.9})`;
        ctx.fill();
      } else if (isShot) {
        const goalX = hl.ball.x >= 0 ? 52.5 : -52.5;
        const [gx, gy] = toMini(goalX, 0, W, H);
        ctx.strokeStyle = `rgba(255,200,80,${0.4 + 0.3 * blink})`;
        ctx.lineWidth = 1.2;
        ctx.setLineDash([3, 3]);
        ctx.beginPath();
        ctx.moveTo(bx, by);
        ctx.lineTo(gx, gy);
        ctx.stroke();
        ctx.setLineDash([]);
      } else if (isSave) {
        // save burst ring
        ctx.beginPath();
        ctx.arc(bx, by, 6 + 3 * blink, 0, Math.PI * 2);
        ctx.strokeStyle = `rgba(100,255,150,${0.9 - 0.4 * blink})`;
        ctx.lineWidth = 2;
        ctx.stroke();
      } else if (isCorner) {
        // corner flag pointer
        ctx.strokeStyle = `rgba(255,255,255,${0.6 + 0.4 * blink})`;
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        ctx.moveTo(bx, by);
        ctx.lineTo(bx, by - 8);
        ctx.stroke();
        ctx.fillStyle = `rgba(255,200,80,${0.8})`;
        ctx.beginPath();
        ctx.moveTo(bx, by - 8);
        ctx.lineTo(bx + 6, by - 5);
        ctx.lineTo(bx, by - 2);
        ctx.closePath();
        ctx.fill();
      }
      // ball dot
      ctx.beginPath();
      ctx.arc(bx, by, 3, 0, Math.PI * 2);
      ctx.fillStyle = '#ffea00';
      ctx.fill();
      // pulse ring
      ctx.beginPath();
      ctx.arc(bx, by, 5 + 2 * blink, 0, Math.PI * 2);
      ctx.strokeStyle = `rgba(255,234,0,${0.5 - 0.3 * blink})`;
      ctx.lineWidth = 1;
      ctx.stroke();
    }
    // label
    ctx.fillStyle = hl.team === 0 ? '#f2f2f2' : '#2f81f7';
    ctx.font = 'bold 10px ui-sans-serif, system-ui, sans-serif';
    ctx.textAlign = 'left';
    ctx.textBaseline = 'top';
    const mm = String(Math.floor(hl.t / 60)).padStart(2, '0');
    const ss = String(Math.floor(hl.t % 60)).padStart(2, '0');
    ctx.fillText(`${hl.label || 'Event'}  ${mm}:${ss}`, 4, 4);
    ctx.restore();
  }

  function update(now, simTime) {
    lastNow = now;
    if (!buffer.length) {
      // idle: draw empty pitch
      ctx.clearRect(0, 0, W, H);
      miniPitch(ctx, W, H);
      if (titleEl) titleEl.textContent = 'no highlights yet';
      return;
    }
    if (phase === 'idle') {
      phase = 'fadein';
      phaseStart = now;
    }
    const elapsed = now - phaseStart;
    let alpha = 1;
    if (phase === 'fadein') {
      alpha = Math.min(1, elapsed / 0.4);
      if (alpha >= 1) { phase = 'hold'; phaseStart = now; }
    } else if (phase === 'hold') {
      alpha = 1;
      if (elapsed >= holdDur) { phase = 'fadeout'; phaseStart = now; }
    } else if (phase === 'fadeout') {
      alpha = Math.max(0, 1 - elapsed / 0.4);
      if (alpha <= 0) {
        // advance
        idx = (idx + 1) % buffer.length;
        phase = 'fadein';
        phaseStart = now;
        alpha = 0;
      }
    }
    ctx.clearRect(0, 0, W, H);
    drawHighlight(buffer[idx], alpha, now);
    if (titleEl) titleEl.textContent = `${idx + 1}/${buffer.length} highlights`;
  }

  return { push, update, buffer };
}