// Smoothed, held team-shape geometry.
//
// Raw player positions jitter and units can empty for a frame (substitutions,
// dismissals, role updates). This tracker eases each unit's polyline, holds
// the last readable line for a moment after a member breaks formation, and
// fades units/triangles in and out instead of popping.

const KEYS = ['def', 'mid', 'fwd'];

function makeUnit() {
  return { points: [], alpha: 0, hold: 0 };
}

function makeTeam() {
  return {
    units: { def: makeUnit(), mid: makeUnit(), fwd: makeUnit() },
    tri: { points: null, alpha: 0, hold: 0 },
  };
}

function centroid(points) {
  let x = 0;
  let y = 0;
  for (const p of points) {
    x += p.x;
    y += p.y;
  }
  return { x: x / points.length, y: y / points.length };
}

export function createShapeTracker({ tau = 0.5, hold = 0.9, fade = 0.3 } = {}) {
  const teams = [makeTeam(), makeTeam()];

  function ease(points, target, dt) {
    const k = 1 - Math.exp(-dt / tau);
    for (let i = 0; i < target.length; i++) {
      points[i].x += (target[i].x - points[i].x) * k;
      points[i].y += (target[i].y - points[i].y) * k;
    }
  }

  function updateUnit(unit, target, dt) {
    if (target.length >= 2) {
      if (unit.points.length === target.length) {
        ease(unit.points, target, dt);
        unit.alpha = Math.min(1, unit.alpha + dt / fade);
        unit.hold = 0;
      } else if (unit.points.length >= 2) {
        // Membership changed: keep the previous line readable for a moment.
        if (unit.hold <= 0) unit.hold = hold;
        unit.hold -= dt;
        unit.alpha = 1;
        if (unit.hold <= 0) {
          unit.points = target.map((p) => ({ x: p.x, y: p.y }));
          unit.alpha = 0;
        }
      } else {
        unit.points = target.map((p) => ({ x: p.x, y: p.y }));
        unit.alpha = Math.min(1, unit.alpha + dt / fade);
        unit.hold = 0;
      }
      return;
    }
    // Too few players for a line: hold, then fade the last readable line.
    if (unit.points.length >= 2) {
      if (unit.hold <= 0) unit.hold = hold;
      unit.hold -= dt;
      if (unit.hold <= 0) {
        unit.alpha -= dt / fade;
        if (unit.alpha <= 0) {
          unit.alpha = 0;
          unit.points = [];
        }
      }
    } else {
      unit.alpha = 0;
      unit.points = [];
    }
  }

  function updateTriangle(tri, target, dt) {
    if (target) {
      if (!tri.points) {
        tri.points = { def: { ...target.def }, mid: { ...target.mid }, fwd: { ...target.fwd } };
        tri.alpha = 0;
      } else {
        const k = 1 - Math.exp(-dt / tau);
        for (const key of KEYS) {
          tri.points[key].x += (target[key].x - tri.points[key].x) * k;
          tri.points[key].y += (target[key].y - tri.points[key].y) * k;
        }
      }
      tri.alpha = Math.min(1, tri.alpha + dt / fade);
      tri.hold = 0;
      return;
    }
    if (tri.points) {
      if (tri.hold <= 0) tri.hold = hold;
      tri.hold -= dt;
      if (tri.hold <= 0) {
        tri.alpha -= dt / fade;
        if (tri.alpha <= 0) {
          tri.alpha = 0;
          tri.points = null;
        }
      }
    }
  }

  function update(teamIndex, units, dt) {
    const team = teams[teamIndex === 1 ? 1 : 0];
    for (const key of KEYS) {
      updateUnit(team.units[key], units?.[key] ?? [], dt);
    }
    const visible = KEYS.every(
      (key) => team.units[key].points.length >= 2 && team.units[key].alpha > 0.05,
    );
    const target = visible
      ? {
          def: centroid(team.units.def.points),
          mid: centroid(team.units.mid.points),
          fwd: centroid(team.units.fwd.points),
        }
      : null;
    updateTriangle(team.tri, target, dt);
    return {
      units: {
        def: { points: team.units.def.points, alpha: team.units.def.alpha },
        mid: { points: team.units.mid.points, alpha: team.units.mid.alpha },
        fwd: { points: team.units.fwd.points, alpha: team.units.fwd.alpha },
      },
      triangle: team.tri.points,
      triangleAlpha: team.tri.alpha,
    };
  }

  function reset() {
    teams[0] = makeTeam();
    teams[1] = makeTeam();
  }

  return { update, reset };
}
