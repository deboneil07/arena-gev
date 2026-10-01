//! End-to-end demo: parse a real Opta F24 match, build the simulation and
//! print statistics.
//!
//! ```text
//! cargo run --release --example match_stats -- "path/to/match.xml"
//! ```
//!
//! With no argument it falls back to the fixture bundled with the crate at
//! `data/2647319.xml`.

use match_engine::{parse_opta_f24, SimulationEngine};
use std::path::PathBuf;

fn default_xml() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join("2647319.xml")
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_xml);

    let xml = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));

    // ── Parse ──────────────────────────────────────────────
    let ctx = parse_opta_f24(&xml).expect("parse failed");
    println!("Match  : {} vs {}", ctx.home_team_name, ctx.away_team_name);
    println!("Date   : {}  Season {}", ctx.game_date, ctx.season);
    println!("Events : {} gameplay events retained", ctx.events.len());
    println!("Lineups: {} team(s)", ctx.lineups.len());
    for (team, roster) in &ctx.lineups {
        let starters = roster.iter().filter(|p| p.is_starter()).count();
        println!("         {team}: {starters} starters, {} total", roster.len());
    }
    for (team, dir) in &ctx.directions {
        println!("Direction: {team} -> {dir:?}");
    }

    let goals = ctx.events.iter().filter(|e| e.type_id == 16).count();
    let aerials = ctx.events.iter().filter(|e| e.is_aerial).count();
    println!("Goals  : {goals}   Aerial events: {aerials}");

    // Event-type histogram, most common first.
    let mut types: Vec<(u16, usize)> = ctx
        .events
        .iter()
        .fold(std::collections::HashMap::<u16, usize>::new(), |mut m, e| {
            *m.entry(e.type_id).or_insert(0) += 1;
            m
        })
        .into_iter()
        .collect();
    types.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    println!("Top event types (id:count): {:?}", &types[..types.len().min(8)]);

    // ── Simulate ───────────────────────────────────────────
    let mut sim = SimulationEngine::from_context(ctx).expect("build simulation");
    println!("\nSimulating {} agents + 3D ball at 60 Hz…", sim.agent_count());
    let start = std::time::Instant::now();

    // Track ball trajectory stats across the whole match.
    let mut max_altitude = 0.0f32;
    let mut flights = 0u64;
    let mut was_flying = false;
    let end = sim.timeline.duration_secs() + 2.0;
    while sim.sim_time < end {
        sim.step();
        max_altitude = max_altitude.max(sim.ball.altitude);
        let flying = sim.ball.is_in_flight();
        if flying && !was_flying {
            flights += 1;
        }
        was_flying = flying;
    }
    let elapsed = start.elapsed();

    println!("Sim time      : {:.1}s", sim.sim_time);
    println!("Wall time     : {:.2}s", elapsed.as_secs_f32());
    println!("Events sent   : {}", sim.events_dispatched);
    println!("Consistent    : {}", sim.is_consistent());
    println!("Ball flights  : {flights}   Max altitude: {max_altitude:.2} m");
    println!("Ball position : ({:.1}, {:.1}) z={:.2}", sim.ball.position.x, sim.ball.position.y, sim.ball.altitude);
    println!("Render frame  : {} entities", sim.get_render_state().len());
    println!("Possession    : {:?}", sim.possession);

    if !sim.is_consistent() {
        std::process::exit(1);
    }
}
