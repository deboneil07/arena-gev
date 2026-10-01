//! Tactical diagnostic harness — quantifies formation discipline, receiver
//! arrival and ball continuity against the bundled Opta fixture.
//!
//! ```text
//! cargo run --release --example tactical_diag
//! ```

use match_engine::{parse_opta_f24, PlayerRole, SimulationEngine, Vector2};
use std::path::PathBuf;

fn default_xml() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join("2647319.xml")
}

#[derive(Default)]
struct Stats {
    samples: u64,
    anchor_err_sum: f64,
    anchor_err_max: f32,
    line_flat_sum: f64,
    line_flat_count: u64,
    home_len_sum: f64,
    home_wid_sum: f64,
    away_len_sum: f64,
    away_wid_sum: f64,
    ball_max_step: f32,
    ball_big_steps: u64,
    pass_samples: u64,
    receiver_near: u64,
    receiver_mid: u64,
    pass_land_target: u64,
    ghost_land: u64,
    actor_dists: Vec<f32>,
}

fn main() {
    let path = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(default_xml);
    let xml = std::fs::read_to_string(&path).expect("read xml");
    let ctx = parse_opta_f24(&xml).expect("parse");
    let mut sim = SimulationEngine::from_context(ctx).expect("build");

    let mut pending: Vec<(Vector2, f32)> = Vec::new();
    let mut stats = Stats::default();
    let mut prev_ball = sim.ball.position;
    let mut tick: u64 = 0;
    let end = sim.timeline.duration_secs() + 1.0;

    while sim.sim_time < end {
        sim.step();
        tick += 1;

        // ── ball continuity ────────────────────────────────────
        let step = prev_ball.distance(sim.ball.position);
        stats.ball_max_step = stats.ball_max_step.max(step);
        if step > 1.2 {
            stats.ball_big_steps += 1;
        }
        prev_ball = sim.ball.position;

        // ── actor arrival (the integration-test metric) ───────
        for a in &sim.last_actions {
            if a.type_id == 18 || a.type_id == 19 || a.type_id == 5 || a.type_id == 6 {
                continue;
            }
            if let Some(i) = sim.agent_index(&a.player_id) {
                stats.actor_dists.push(sim.agents[i].position.distance(a.origin));
            }
        }

        // ── passes: nearest teammate to target at release ──────
        for a in &sim.last_actions {
            if a.type_id == 1 {
                if let Some(t) = a.target {
                    stats.pass_samples += 1;
                    let mut best = f32::MAX;
                    for p in sim.agents.iter().filter(|p| p.team_id == a.team_id) {
                        best = best.min(p.position.distance(t));
                    }
                    if best < 2.0 {
                        stats.receiver_near += 1;
                    }
                    if best < 6.0 {
                        stats.receiver_mid += 1;
                    }
                    pending.push((t, 3.0));
                }
            }
        }

        // ── ghost passes: ball arrived, nobody there ───────────
        let ball = sim.ball.position;
        for item in pending.iter_mut() {
            item.1 -= sim.config.dt;
            if ball.distance(item.0) < 1.0 {
                stats.pass_land_target += 1;
                let mut best = f32::MAX;
                for p in sim.agents.iter() {
                    best = best.min(p.position.distance(item.0));
                }
                if best > 3.0 {
                    stats.ghost_land += 1;
                }
                item.1 = -1.0;
            }
        }
        pending.retain(|p| p.1 > 0.0);

        // ── shape discipline (sampled every 30 ticks) ──────────
        if tick.is_multiple_of(30) {
            let anchors = sim.anchors();
            for (team, is_home) in [(&sim.home_team_id, true), (&sim.away_team_id, false)] {
                let mut sum = 0.0f32;
                let mut n = 0f32;
                let (mut minx, mut maxx, mut miny, mut maxy) =
                    (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
                let mut def_x: Vec<f32> = Vec::new();
                for a in sim.agents.iter().filter(|a| &a.team_id == team) {
                    if a.role == PlayerRole::Goalkeeper {
                        continue;
                    }
                    if let Some(t) = anchors.get(&a.id) {
                        let d = a.position.distance(*t);
                        sum += d;
                        n += 1.0;
                        stats.anchor_err_max = stats.anchor_err_max.max(d);
                    }
                    if a.role == PlayerRole::Defender {
                        def_x.push(a.position.x);
                    }
                    minx = minx.min(a.position.x);
                    maxx = maxx.max(a.position.x);
                    miny = miny.min(a.position.y);
                    maxy = maxy.max(a.position.y);
                }
                let len = maxx - minx;
                let wid = maxy - miny;
                if is_home {
                    stats.home_len_sum += len as f64;
                    stats.home_wid_sum += wid as f64;
                } else {
                    stats.away_len_sum += len as f64;
                    stats.away_wid_sum += wid as f64;
                }
                if n > 0.0 {
                    stats.anchor_err_sum += (sum / n) as f64;
                    stats.samples += 1;
                }
                if def_x.len() >= 2 {
                    let mean = def_x.iter().sum::<f32>() / def_x.len() as f32;
                    let flat = def_x.iter().map(|x| (x - mean).abs()).sum::<f32>()
                        / def_x.len() as f32;
                    stats.line_flat_sum += flat as f64;
                    stats.line_flat_count += 1;
                }
            }
        }
    }

    let s = &stats;
    println!("=== tactical diagnostic ===");
    println!("samples           : {}", s.samples);
    println!(
        "anchor adherence  : mean {:.2} m  max {:.2} m",
        s.anchor_err_sum / s.samples.max(1) as f64,
        s.anchor_err_max
    );
    println!(
        "backline flatness : mean |x - line| {:.2} m",
        s.line_flat_sum / s.line_flat_count.max(1) as f64
    );
    println!(
        "team length (x)   : home {:.1} m  away {:.1} m",
        s.home_len_sum / s.samples.max(1) as f64,
        s.away_len_sum / s.samples.max(1) as f64
    );
    println!(
        "team width  (y)   : home {:.1} m  away {:.1} m",
        s.home_wid_sum / s.samples.max(1) as f64,
        s.away_wid_sum / s.samples.max(1) as f64
    );
    println!(
        "ball max step     : {:.2} m/tick   big(>1.2m) {}",
        s.ball_max_step, s.ball_big_steps
    );
    println!(
        "pass receiver     : {} passes, {:.0}% <2m, {:.0}% <6m at release",
        s.pass_samples,
        100.0 * s.receiver_near as f64 / s.pass_samples.max(1) as f64,
        100.0 * s.receiver_mid as f64 / s.pass_samples.max(1) as f64,
    );
    println!(
        "ghost landings    : {}/{} ball arrivals had nobody within 3 m",
        s.ghost_land, s.pass_land_target
    );
    let n = s.actor_dists.len().max(1) as f64;
    let close = s.actor_dists.iter().filter(|d| **d < 1.0).count() as f64 / n;
    let near = s.actor_dists.iter().filter(|d| **d < 2.0).count() as f64 / n;
    let mean = s.actor_dists.iter().sum::<f32>() as f64 / n;
    println!(
        "actor arrival     : {} acts, {:.0}% <1m, {:.0}% <2m, mean {:.2} m",
        s.actor_dists.len(),
        close * 100.0,
        near * 100.0,
        mean
    );
    println!("consistent        : {}", sim.is_consistent());
}
