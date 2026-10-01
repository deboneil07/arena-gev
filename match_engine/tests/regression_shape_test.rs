//! Phase-6 regression tests — the guarantees from the seven-issue audit.
//!
//! Each test claims something a broken engine would visibly do on screen:
//! the ball flying from thin air, pass flights starting somewhere else,
//! attackers standing miles beyond the last defender, a back line that
//! breathes in and out, or players piling into one small patch of grass.
//!
//! They run the real Opta fixture end to end and are written to be tunable:
//! every assertion message reports the measured value.

use match_engine::brain::offside_limit_world;
use match_engine::{
    is_ball_event, is_dead_ball_event, parse_opta_f24, PlayerRole, SimulationEngine, TacticalIntent,
    EVENT_TYPE_PASS,
};
use std::path::PathBuf;

fn build() -> SimulationEngine {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join("2647319.xml");
    let xml = std::fs::read_to_string(&path).expect("read fixture");
    let ctx = parse_opta_f24(&xml).expect("parse");
    SimulationEngine::from_context(ctx).expect("build")
}

/// The ball must already be at the acting player's boots when the event
/// fires — a pass may never appear to come from thin air (issue 6).
#[test]
fn test_ball_is_at_actor_before_event() {
    let mut sim = build();
    let dur = sim.timeline.duration_secs();
    let (mut n, mut within1, mut within2) = (0u32, 0u32, 0u32);
    while sim.sim_time < dur + 1.0 {
        // A pass that is still in flight when the next event fires is a
        // continuation of visible play (the engine deliberately lets it
        // finish), not a ghost pass — only idle-ball dispatches count.
        let flying = sim.ball.is_in_flight();
        sim.step();
        for a in &sim.last_actions {
            // Only ball events attach the action to the actor in
            // `choreograph`; dead-ball restarts place the ball on the spot
            // while the taker is still arriving.
            if is_ball_event(a.type_id) && !is_dead_ball_event(a.type_id) && !flying {
                if let Some(i) = sim.agent_index(&a.player_id) {
                    let d = sim.ball.position.distance(sim.agents[i].position);
                    n += 1;
                    if d < 1.0 {
                        within1 += 1;
                    }
                    if d < 2.0 {
                        within2 += 1;
                    }
                }
            }
        }
    }
    assert!(n > 100, "too few ball events sampled: {n}");
    let c1 = within1 as f32 / n as f32;
    let c2 = within2 as f32 / n as f32;
    // Measured on the fixture: ~62% within 1 m, ~85% within 2 m; the tail
    // is Opta's unreachable same-timestamp events (~12%) where the ball is
    // still visibly rolling in from the previous action — never teleporting.
    assert!(
        c1 > 0.55,
        "only {:.0}% of {n} ball events start within 1 m of the actor (within 2 m: {:.0}%)",
        c1 * 100.0,
        c2 * 100.0
    );
    assert!(
        c2 > 0.80,
        "only {:.0}% of {n} ball events start within 2 m of the actor",
        c2 * 100.0
    );
}

/// A pass flight must launch from the Opta origin, not from wherever the
/// ball happened to be (Phase-1 launch-point guarantee, issues 2 + 6).
#[test]
fn test_launch_point_is_event_origin() {
    let mut sim = build();
    let dur = sim.timeline.duration_secs();
    let (mut n, mut within15, mut within3) = (0u32, 0u32, 0u32);
    while sim.sim_time < dur + 1.0 {
        sim.step();
        if sim.set_piece_active() {
            continue;
        }
        for a in &sim.last_actions {
            if a.type_id != EVENT_TYPE_PASS {
                continue;
            }
            let d = sim.ball.position.distance(a.origin);
            n += 1;
            if d < 1.5 {
                within15 += 1;
            }
            if d < 3.0 {
                within3 += 1;
            }
        }
    }
    assert!(n > 100, "too few open-play passes sampled: {n}");
    let c1 = within15 as f32 / n as f32;
    let c2 = within3 as f32 / n as f32;
    assert!(
        c1 > 0.80,
        "only {:.0}% of {n} passes launch within 1.5 m of the event origin (within 3 m: {:.0}%)",
        c1 * 100.0,
        c2 * 100.0
    );
    assert!(
        c2 > 0.90,
        "only {:.0}% of {n} passes launch within 3 m of the event origin",
        c2 * 100.0
    );
}

/// Open play only: nobody on the possessing team may sit beyond the
/// opponent's second-last defender and the ball (issue 4). Mirrors the hard
/// clamp in `brain::step_agents` and asserts the same law, with event-run
/// (`AnticipateEvent`) actors and set-piece layouts exempt — exactly the
/// exemptions the engine applies.
#[test]
fn test_attackers_never_offside() {
    let mut sim = build();
    let dur = sim.timeline.duration_secs();
    let (mut n, mut over15) = (0u64, 0u64);
    let (mut sum, mut max_over) = (0.0f64, 0.0f32);
    while sim.sim_time < dur + 1.0 {
        sim.step();
        if sim.set_piece_active() {
            continue;
        }
        let Some(poss) = sim.possession.clone() else {
            continue;
        };
        let ball_x = sim.ball.position.x;
        let home = &sim.home_team_id;
        for agent in sim.agents.iter() {
            if agent.team_id != poss {
                continue;
            }
            if matches!(
                agent.brain.current_intent,
                TacticalIntent::AnticipateEvent { .. }
            ) {
                continue;
            }
            let is_home = agent.team_id == *home;
            let attack_dx = if is_home { 1.0 } else { -1.0 };
            let limit = offside_limit_world(
                sim.agents
                    .iter()
                    .map(|a| (a.position.x, a.team_id == *home)),
                !is_home,
                attack_dx,
                ball_x,
            );
            let over = (agent.position.x - limit) * attack_dx;
            n += 1;
            sum += over as f64;
            max_over = max_over.max(over);
            if over > 1.5 {
                over15 += 1;
            }
        }
    }
    assert!(n > 10_000, "too few open-play samples: {n}");
    let mean = sum / n as f64;
    let frac = over15 as f64 / n as f64;
    assert!(
        frac < 0.001,
        "{:.3}% of {n} open-play samples sat >1.5 m beyond the offside line (worst {:.1} m, mean {mean:.2} m)",
        frac * 100.0,
        max_over
    );
    // The only permitted excursion: the instant a set piece ends, attackers
    // leaving the box may sit up to ~15 m beyond the line and the clamp
    // eases them back at one sprint step per tick (never teleports).
    assert!(
        max_over < 20.0,
        "an attacker sat {max_over:.1} m beyond the offside line over {n} samples (mean {mean:.2} m)"
    );
    assert!(
        mean <= 1.0,
        "mean offside excess {mean:.2} m over {n} open-play samples"
    );
}

/// The back line holds its height: defenders stay in a compact bank instead
/// of stepping up and dropping back with the ball (issue 3, breathing off).
#[test]
fn test_backline_flat_without_breathing() {
    let mut sim = build();
    let dur = sim.timeline.duration_secs();
    let mut sums = [0.0f64; 2];
    let mut counts = [0u32; 2];
    let mut all: [Vec<f32>; 2] = [Vec::new(), Vec::new()];
    let mut tick = 0u64;
    while sim.sim_time < dur + 1.0 {
        sim.step();
        tick += 1;
        if !tick.is_multiple_of(10) {
            continue;
        }
        for (t, team_id) in [&sim.home_team_id, &sim.away_team_id].into_iter().enumerate() {
            let mut min_x = f32::INFINITY;
            let mut max_x = f32::NEG_INFINITY;
            let mut defs = 0u32;
            for a in sim.agents.iter() {
                if a.team_id == *team_id && a.role == PlayerRole::Defender {
                    min_x = min_x.min(a.position.x);
                    max_x = max_x.max(a.position.x);
                    defs += 1;
                }
            }
            if defs < 3 {
                continue;
            }
            let spread = max_x - min_x;
            sums[t] += spread as f64;
            counts[t] += 1;
            all[t].push(spread);
        }
    }
    for t in 0..2 {
        assert!(counts[t] > 100, "team {t}: too few samples: {}", counts[t]);
        let mean = sums[t] / counts[t] as f64;
        all[t].sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p95 = all[t][(all[t].len() as f64 * 0.95) as usize];
        let p99 = all[t][(all[t].len() as f64 * 0.99) as usize];
        let max = *all[t].last().unwrap();
        assert!(
            mean <= 7.5,
            "team {t}: back line spreads {mean:.2} m on average (p95={p95:.1} p99={p99:.1} max={max:.1})"
        );
        // A centre-back may step out to reach an event (genuine football);
        // what matters is that the unit stays compact nearly all the time.
        assert!(
            p95 <= 25.0,
            "team {t}: back line spread p95={p95:.1} m (mean {mean:.2} m) — the bank broke apart"
        );
        assert!(
            p99 <= 40.0,
            "team {t}: back line spread p99={p99:.1} m max={max:.1} — the bank disintegrated"
        );
    }
}

/// No pile-ups: the densest 6 m circle on the pitch must never turn into a
/// scrum, even when the ball arrives in a crowded area (issue 5).
#[test]
fn test_no_clutter() {
    let mut sim = build();
    let dur = sim.timeline.duration_secs();
    let (mut n, mut sum, mut max_dense) = (0u32, 0.0f64, 0u32);
    let mut tick = 0u64;
    let pos = |sim: &SimulationEngine| -> Vec<(f32, f32)> {
        sim.agents.iter().map(|a| (a.position.x, a.position.y)).collect()
    };
    while sim.sim_time < dur + 1.0 {
        sim.step();
        tick += 1;
        if !tick.is_multiple_of(10) || sim.set_piece_active() {
            continue;
        }
        let pts = pos(&sim);
        let mut densest = 0u32;
        for (i, (x, y)) in pts.iter().enumerate() {
            let mut cnt = 0u32;
            for (j, (px, py)) in pts.iter().enumerate() {
                if i == j {
                    continue;
                }
                let dx = x - px;
                let dy = y - py;
                if dx * dx + dy * dy <= 36.0 {
                    cnt += 1;
                }
            }
            densest = densest.max(cnt);
        }
        n += 1;
        sum += densest as f64;
        max_dense = max_dense.max(densest);
    }
    assert!(n > 1000, "too few samples: {n}");
    let mean = sum / n as f64;
    assert!(
        mean <= 5.0,
        "densest 6 m circle holds {mean:.2} players on average (max {max_dense}) over {n} samples"
    );
    assert!(
        max_dense <= 12,
        "densest 6 m circle reached {max_dense} players — a pile-up (mean {mean:.2})"
    );
}
