//! End-to-end integration tests against the real Opta F24 match file.
//!
//! These tests exercise the full pipeline:
//! `XML → loader → MatchContext → Timeline → FormationEngine → MatchSimulation`
//! and assert both correctness invariants and several regression behaviours
//! that were previously broken.

use match_engine::{
    is_gameplay_event, parse_opta_f24, AttackDirection, MatchSimulation, SimulationConfig,
    Timeline,
};
use std::collections::HashSet;
use std::path::PathBuf;

/// Locate the bundled match XML relative to the crate manifest.
fn xml_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join("2647319.xml")
}

fn load_xml() -> String {
    let path = xml_path();
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

// ── Parsing invariants ─────────────────────────────────────

#[test]
fn integration_real_xml_parses() {
    let ctx = parse_opta_f24(&load_xml()).expect("parse");
    assert!(!ctx.events.is_empty());
    assert!(!ctx.directions.is_empty());
    assert_eq!(ctx.home_team_name, "West Ham United");
    assert_eq!(ctx.away_team_name, "Wrexham");
    assert_eq!(ctx.match_id, "2647319");
}

#[test]
fn integration_event_ids_unique_and_nonzero() {
    // Regression: ids were parsed as u16, overflowed and became 0.
    let ctx = parse_opta_f24(&load_xml()).expect("parse");
    let ids: HashSet<u64> = ctx.events.iter().map(|e| e.id).collect();
    assert!(
        ctx.events.iter().all(|e| e.id != 0),
        "every event must have a non-zero 64-bit id"
    );
    assert_eq!(
        ids.len(),
        ctx.events.len(),
        "event ids must be unique ({} ids for {} events)",
        ids.len(),
        ctx.events.len()
    );
}

#[test]
fn integration_goals_are_retained() {
    // Regression: type 16 (Goal) was filtered out as "match end".
    let ctx = parse_opta_f24(&load_xml()).expect("parse");
    let goals: Vec<_> = ctx.events.iter().filter(|e| e.type_id == 16).collect();
    assert_eq!(goals.len(), 6, "expected 6 goals in the fixture");
    assert!(goals.iter().all(|g| g.outcome));
}

#[test]
fn integration_deleted_and_meta_events_excluded() {
    // Regression: deleted-event markers (43) and period markers (30/32/37…)
    // were leaking into the gameplay stream at placeholder coordinates.
    let ctx = parse_opta_f24(&load_xml()).expect("parse");
    for e in &ctx.events {
        assert!(
            is_gameplay_event(e.type_id),
            "meta event type {} leaked into gameplay",
            e.type_id
        );
        assert_ne!(e.type_id, 43, "deleted event leaked");
        assert_ne!(e.type_id, 30, "period-end leaked");
        assert_ne!(e.type_id, 32, "period-start leaked");
        assert_ne!(e.type_id, 34, "lineup leaked");
        assert_ne!(e.type_id, 37, "collection-end leaked");
        assert_ne!(e.type_id, 40, "formation-change leaked");
        assert_ne!(e.type_id, 70, "injury-time leaked");
    }
}

#[test]
fn integration_lineups_are_built() {
    // Regression: lineup construction was unreachable dead code.
    let ctx = parse_opta_f24(&load_xml()).expect("parse");
    assert_eq!(ctx.lineups.len(), 2, "both teams should have a lineup");
    for (team, roster) in &ctx.lineups {
        assert!(
            roster.len() >= 11,
            "team {team} should have at least 11 players, got {}",
            roster.len()
        );
        let starters = roster.iter().filter(|p| p.is_starter()).count();
        assert_eq!(starters, 11, "team {team} should have exactly 11 starters");
    }
    assert_eq!(ctx.starting_xi(&ctx.home_team_id).len(), 11);
    assert_eq!(ctx.starting_xi(&ctx.away_team_id).len(), 11);
}

#[test]
fn integration_xml_entities_unescaped() {
    // Regression: &apos; was left verbatim in player names.
    let ctx = parse_opta_f24(&load_xml()).expect("parse");
    assert!(
        ctx.events.iter().any(|e| e.player_name.contains('\'')),
        "expected at least one decoded apostrophe in a player name"
    );
    assert!(
        ctx.events.iter().all(|e| !e.player_name.contains("&apos;")),
        "raw XML entities must not survive into player names"
    );
}

#[test]
fn integration_all_coords_on_pitch() {
    let ctx = parse_opta_f24(&load_xml()).expect("parse");
    for e in &ctx.events {
        assert!(
            match_engine::MatchContext::is_on_pitch(e.origin_x, e.origin_y),
            "event {} origin ({}, {}) off pitch",
            e.id,
            e.origin_x,
            e.origin_y
        );
        if let (Some(tx), Some(ty)) = (e.target_x, e.target_y) {
            assert!(
                match_engine::MatchContext::is_on_pitch(tx, ty),
                "event {} target ({}, {}) off pitch",
                e.id,
                tx,
                ty
            );
        }
    }
}

#[test]
fn integration_both_directions_detected() {
    let ctx = parse_opta_f24(&load_xml()).expect("parse");
    let values: Vec<AttackDirection> = ctx.directions.values().copied().collect();
    assert!(values.contains(&AttackDirection::LeftToRight));
    assert!(values.contains(&AttackDirection::RightToLeft));
    // Each team has exactly one direction.
    assert_eq!(ctx.directions.len(), 2);
}

#[test]
fn integration_canonical_frame_is_period_stable() {
    // Regression: Opta swaps direction at half-time, so a single per-team
    // direction taken from the *last* period used to mirror period-1 events to
    // the wrong side. The match frame must be canonical (home +x, away -x)
    // and stable, while the raw per-period directions are kept separately.
    let ctx = parse_opta_f24(&load_xml()).expect("parse");
    assert_eq!(
        ctx.directions.get(&ctx.home_team_id),
        Some(&AttackDirection::LeftToRight)
    );
    assert_eq!(
        ctx.directions.get(&ctx.away_team_id),
        Some(&AttackDirection::RightToLeft)
    );
    assert_eq!(ctx.canonical_direction(&ctx.home_team_id), AttackDirection::LeftToRight);
    assert_eq!(ctx.canonical_direction(&ctx.away_team_id), AttackDirection::RightToLeft);
    // The raw feed directions are still captured (and genuinely swap).
    assert!(!ctx.physical_directions.is_empty());
}

#[test]
fn integration_goalkeepers_are_on_their_own_sides() {
    // The home keeper (attacking +x) defends -x; the away keeper defends +x.
    use match_engine::{FormationEngine, TeamTacticalPhase, Vector2};
    let ctx = parse_opta_f24(&load_xml()).expect("parse");
    let engine = FormationEngine::new();
    let home = engine.calculate_team_anchors(
        "4-3-3",
        &ctx.starting_xi(&ctx.home_team_id).into_iter().cloned().collect::<Vec<_>>(),
        Vector2::zero(),
        TeamTacticalPhase::OutOfPossession,
        ctx.canonical_direction(&ctx.home_team_id),
    );
    let away = engine.calculate_team_anchors(
        "4-3-3",
        &ctx.starting_xi(&ctx.away_team_id).into_iter().cloned().collect::<Vec<_>>(),
        Vector2::zero(),
        TeamTacticalPhase::OutOfPossession,
        ctx.canonical_direction(&ctx.away_team_id),
    );
    let home_gk = home.iter().find(|a| a.role_weight > 1.0).expect("home GK");
    let away_gk = away.iter().find(|a| a.role_weight > 1.0).expect("away GK");
    assert!(home_gk.target_pos.x < 0.0, "home GK at {}", home_gk.target_pos.x);
    assert!(away_gk.target_pos.x > 0.0, "away GK at {}", away_gk.target_pos.x);
}

#[test]
fn integration_aerial_events_flagged() {
    let ctx = parse_opta_f24(&load_xml()).expect("parse");
    let aerials = ctx.events.iter().filter(|e| e.is_aerial).count();
    assert!(aerials > 0, "aerial events should be detected");
    // Every type-44 event must be flagged aerial.
    for e in ctx.events.iter().filter(|e| e.type_id == 44) {
        assert!(e.is_aerial, "type 44 must be aerial");
    }
}

// ── Timeline invariants ────────────────────────────────────

#[test]
fn integration_timeline_monotonic_and_queryable() {
    let ctx = parse_opta_f24(&load_xml()).expect("parse");
    let timeline = Timeline::from_events(ctx.events.clone());

    assert_eq!(timeline.len(), ctx.events.len());
    assert!(timeline.duration_secs() > 5000.0, "full match length");

    // Timestamps must be non-decreasing.
    let times: Vec<f32> = timeline.events().iter().map(|e| e.match_time_secs).collect();
    assert!(times.windows(2).all(|w| w[0] <= w[1]));

    // Lookahead windows are a subset of a superset window.
    let w3 = timeline.lookahead(600.0, 3.0);
    let w5 = timeline.lookahead(600.0, 5.0);
    for ev in w3 {
        assert!(w5.iter().any(|e| e.id == ev.id));
    }

    // `at()` round-trips an event's own timestamp.
    if let Some(ev) = timeline.events().first() {
        let at = timeline.at(ev.match_time_secs);
        assert!(at.iter().any(|e| e.id == ev.id));
    }
}

#[test]
fn integration_timeline_range_boundaries() {
    let ctx = parse_opta_f24(&load_xml()).expect("parse");
    let timeline = Timeline::from_events(ctx.events.clone());

    // Every event in a returned range must fall inside [start, end).
    let start = 300.0;
    let end = 330.0;
    for ev in timeline.range(start..end) {
        assert!(ev.match_time_secs >= start && ev.match_time_secs < end);
    }
    // And the count must match a brute-force filter.
    let brute = timeline
        .events()
        .iter()
        .filter(|e| e.match_time_secs >= start && e.match_time_secs < end)
        .count();
    assert_eq!(timeline.range(start..end).len(), brute);
}

// ── Full-match simulation ──────────────────────────────────

fn build_simulation() -> MatchSimulation {
    let ctx = parse_opta_f24(&load_xml()).expect("parse");
    MatchSimulation::from_context(ctx).expect("build simulation")
}

#[test]
fn integration_dispatch_keeps_actors_near_their_events() {
    // End-to-end check that the canonical frame and the dispatcher cooperate:
    // over the whole match, the acting player should be close to the event
    // origin when it fires. The mean is dominated by the 3.5 s lookahead
    // window; a regression in either frame bug pushes it above ~10 m.
    let mut sim = build_simulation();
    let mut distances: Vec<f32> = Vec::new();
    while sim.sim_time < sim.timeline.duration_secs() + 1.0 {
        sim.step();
        for act in &sim.last_actions {
            if act.type_id == 18 || act.type_id == 19 {
                continue; // substitutions happen off the pitch
            }
            if let Some(i) = sim.agent_index(&act.player_id) {
                distances.push(sim.agents[i].position.distance(act.origin));
            }
        }
    }
    assert!(!distances.is_empty());
    let mean = distances.iter().sum::<f32>() / distances.len() as f32;
    assert!(mean < 8.0, "mean actor→origin distance too high: {mean:.2} m");
    let close = distances.iter().filter(|d| **d < 5.0).count();
    let ratio = close as f32 / distances.len() as f32;
    assert!(ratio > 0.6, "only {:.0}% of actors within 5 m", ratio * 100.0);
}

#[test]
fn integration_ball_never_teleports() {
    // Regression: long passes used to snap / rocket across the pitch.
    let mut sim = build_simulation();
    let mut prev = sim.ball.position;
    let mut max_move = 0.0f32;
    while sim.sim_time < sim.timeline.duration_secs() + 1.0 {
        sim.step();
        // Dead-ball restarts (kick-off, corner, free kick, throw-in) are
        // allowed to place the ball; live play must never teleport it.
        if !sim.set_piece_active() {
            max_move = max_move.max(prev.distance(sim.ball.position));
        }
        prev = sim.ball.position;
    }
    // A driven pass can legitimately hit ~1 m in a single frame; a teleport
    // would be tens of metres.
    assert!(max_move < 2.5, "ball teleported {max_move:.2} m in one tick");
}

#[test]
fn integration_heading_rate_is_bounded() {
    // Regression: the heading used to flip-flop and spin at up to 8 rad/s.
    use match_engine::agent::TURN_SPEED;
    let mut sim = build_simulation();
    let dt = sim.config.dt;
    let mut prev: Vec<f32> = sim.agents.iter().map(|a| a.heading).collect();
    let mut max_rate = 0.0f32;
    for _ in 0..(60 * 60) {
        sim.step();
        for (i, a) in sim.agents.iter().enumerate() {
            let mut d = (a.heading - prev[i]) % (2.0 * std::f32::consts::PI);
            if d > std::f32::consts::PI {
                d -= 2.0 * std::f32::consts::PI;
            } else if d < -std::f32::consts::PI {
                d += 2.0 * std::f32::consts::PI;
            }
            max_rate = max_rate.max(d.abs() / dt);
            prev[i] = a.heading;
        }
    }
    assert!(max_rate <= TURN_SPEED + 1e-3, "heading rate {max_rate} rad/s");
}

#[test]
fn integration_players_never_freeze() {
    // Regression: a 73 s event gap used to leave every player perfectly still.
    let mut sim = build_simulation();
    let n = sim.agent_count();
    sim.run_for(5.0); // warm up: every agent starts from rest at t=0
    // Measure the slowest sustained 1.5 s window (single-tick dips right before
    // an event are expected and harmless).
    const WINDOW: usize = 90;
    let mut window: std::collections::VecDeque<f32> = std::collections::VecDeque::new();
    let mut sum = 0.0f32;
    let mut min_window = f32::MAX;
    let mut min_at = 0.0f32;
    while sim.sim_time < sim.timeline.duration_secs() + 1.0 {
        sim.step();
        // Players are *supposed* to stand still during a set-piece stoppage.
        // Only open play must always show movement.
        if sim.set_piece_active() {
            window.clear();
            sum = 0.0;
            continue;
        }
        let avg = sim.agents.iter().map(|a| a.speed()).sum::<f32>() / n as f32;
        window.push_back(avg);
        sum += avg;
        if window.len() > WINDOW {
            sum -= window.pop_front().unwrap();
        }
        if window.len() == WINDOW {
            let w = sum / WINDOW as f32;
            if w < min_window {
                min_window = w;
                min_at = sim.sim_time;
            }
        }
    }
    assert!(
        min_window > 0.1,
        "players froze for 1.5s at t={min_at:.0}s (avg {min_window:.3} m/s)"
    );
}

#[test]
fn integration_dead_ball_events_do_not_attach() {
    // Regression: an Out event used to attach the ball to an off-pitch actor
    // and drag the whole formation into the corner for the whole stoppage.
    use match_engine::is_dead_ball_event;
    let mut sim = build_simulation();
    while sim.sim_time < sim.timeline.duration_secs() + 1.0 {
        sim.step();
        let acts: Vec<u16> = sim.last_actions.iter().map(|a| a.type_id).collect();
        if !acts.is_empty() && acts.iter().all(|t| is_dead_ball_event(*t)) {
            assert!(!sim.ball.is_attached(), "dead-ball event attached the ball");
        }
    }
}

#[test]
fn integration_event_feed_is_populated() {
    let mut sim = build_simulation();
    sim.run_to_end();
    assert!(sim.event_seq > 1000, "expected many recorded events");
    assert!(sim.recent_events.iter().all(|r| r.label.len() > 1));
    // The fixture contains goals and cards, which must be flagged notable.
    assert!(sim.recent_events.iter().any(|r| r.type_id == 16 || r.type_id == 17));
}

#[test]
fn integration_corners_reach_the_flag() {
    use match_engine::{Vector2, EVENT_TYPE_CORNER};
    let mut sim = build_simulation();
    let mut pending: Option<(Vector2, f32)> = None;
    let (mut attempts, mut successes) = (0u32, 0u32);
    while sim.sim_time < sim.timeline.duration_secs() + 1.0 {
        sim.step();
        for a in &sim.last_actions {
            if a.type_id != EVENT_TYPE_CORNER {
                continue;
            }
            // The kept row names whoever touched the ball out, so the team
            // credited says nothing about who attacks that end — every
            // corner restart belongs at its flag.
            attempts += 1;
            let x = if a.origin.x > 0.0 { 52.0 } else { -52.0 };
            let y = if a.origin.y >= 0.0 { 33.5 } else { -33.5 };
            pending = Some((Vector2::new(x, y), sim.sim_time + 3.0));
        }
        if let Some((corner, until)) = pending {
            if sim.ball.position.distance(corner) < 2.0 {
                successes += 1;
                pending = None;
            } else if sim.sim_time > until {
                pending = None;
            }
        }
    }
    assert!(attempts > 0, "no corners in the fixture");
    assert!(
        successes as f32 >= attempts as f32 * 0.8,
        "only {successes}/{attempts} attacking corners reached the flag"
    );
}

#[test]
fn integration_actors_reach_their_events() {
    // The lookahead + arrival budget + primary-override should put the acting
    // player on the event spot the vast majority of the time (dead-ball events
    // excluded: their actor never needs to be at the restart spot).
    use match_engine::is_dead_ball_event;
    let mut sim = build_simulation();
    let mut dists: Vec<f32> = Vec::new();
    while sim.sim_time < sim.timeline.duration_secs() + 1.0 {
        sim.step();
        for a in &sim.last_actions {
            if a.type_id == 18 || a.type_id == 19 || is_dead_ball_event(a.type_id) {
                continue;
            }
            if let Some(i) = sim.agent_index(&a.player_id) {
                dists.push(sim.agents[i].position.distance(a.origin));
            }
        }
    }
    assert!(!dists.is_empty());
    let close = dists.iter().filter(|d| **d < 1.0).count() as f32 / dists.len() as f32;
    let near = dists.iter().filter(|d| **d < 2.0).count() as f32 / dists.len() as f32;
    assert!(close > 0.8, "only {:.0}% of actors within 1 m", close * 100.0);
    assert!(near > 0.88, "only {:.0}% of actors within 2 m", near * 100.0);
}

#[test]
fn integration_simulation_builds_two_full_teams() {
    let sim = build_simulation();
    assert_eq!(sim.agent_count(), 22, "11 players per side");
    let home = sim.home_team_id.clone();
    let away = sim.away_team_id.clone();
    let home_count = (0..sim.agent_count())
        .filter(|&i| sim.agent_team(i) == home)
        .count();
    let away_count = (0..sim.agent_count())
        .filter(|&i| sim.agent_team(i) == away)
        .count();
    assert_eq!(home_count, 11);
    assert_eq!(away_count, 11);
}

#[test]
fn integration_simulation_full_match_is_consistent() {
    let mut sim = build_simulation();
    let expected_end = sim.timeline.duration_secs();

    // Run the whole match at 60 Hz.
    sim.run_to_end();

    assert!(sim.sim_time >= expected_end, "should reach the final whistle");
    assert!(sim.is_consistent(), "every agent must stay finite + on pitch");
    assert!(sim.events_dispatched > 0, "events should be dispatched");
    assert!(
        sim.events_dispatched > 1000,
        "expected most events dispatched, got {}",
        sim.events_dispatched
    );
}

#[test]
fn integration_simulation_is_deterministic() {
    // Two identical runs must produce byte-identical agent states.
    let mut a = build_simulation();
    let mut b = build_simulation();
    a.run_for(120.0);
    b.run_for(120.0);

    assert_eq!(a.agents.len(), b.agents.len());
    for (x, y) in a.agents.iter().zip(b.agents.iter()) {
        assert_eq!(x.id, y.id);
        assert_eq!(x.position, y.position, "position diverged for {}", x.id);
        assert_eq!(x.velocity, y.velocity, "velocity diverged for {}", x.id);
        assert_eq!(x.state, y.state);
    }
    assert_eq!(a.ball.position, b.ball.position);
    assert_eq!(a.ball.altitude, b.ball.altitude);
    assert_eq!(a.possession, b.possession);
}

#[test]
fn integration_simulation_never_teleports() {
    let mut sim = build_simulation();
    let mut prev: Vec<_> = sim.agents.iter().map(|a| a.position).collect();

    // Sample every tick for the first 60 seconds and bound displacement.
    for _ in 0..(60 * 60) {
        sim.step();
        for (i, agent) in sim.agents.iter().enumerate() {
            let d = prev[i].distance(agent.position);
            assert!(
                d <= match_engine::MAX_SPEED * sim.config.dt + 1e-3,
                "agent {} moved {d} m in one tick",
                agent.id
            );
        }
        prev = sim.agents.iter().map(|a| a.position).collect();
    }
}

#[test]
fn integration_simulation_lookahead_config_effect() {
    // A longer lookahead must dispatch events at least as early.
    let ctx = parse_opta_f24(&load_xml()).expect("parse");

    let mut short = MatchSimulation::with_config(
        ctx.clone(),
        SimulationConfig {
            lookahead_secs: 1.0,
            ..Default::default()
        },
    )
    .unwrap();
    let mut long = MatchSimulation::with_config(
        ctx,
        SimulationConfig {
            lookahead_secs: 8.0,
            ..Default::default()
        },
    )
    .unwrap();

    short.run_for(20.0);
    long.run_for(20.0);

    assert!(
        long.events_dispatched >= short.events_dispatched,
        "longer lookahead should not dispatch fewer events ({} vs {})",
        long.events_dispatched,
        short.events_dispatched
    );
}

#[test]
fn integration_simulation_performance_budget() {
    // Guards against accidental O(n²)-per-agent cloning blowups. The budget is
    // deliberately generous so it stays stable on slow CI hardware.
    use std::time::Instant;
    let mut sim = build_simulation();
    let start = Instant::now();
    sim.run_to_end();
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_secs() < 60,
        "full-match simulation took {elapsed:?} (budget 60s)"
    );
    // Keep the value used so the compiler does not optimise the run away.
    assert!(sim.agent_count() == 22);
}

#[test]
fn integration_receivers_and_shape_hold() {
    // The foundation test for the tactical rewrite: passes must have a
    // teammate arriving at the landing point, and the ball must never be
    // "passed to ghosts".
    use match_engine::PlayerRole;
    let mut sim = build_simulation();
    let mut pass_n = 0u32;
    let mut near = 0u32;
    let mut ghost = 0u32;
    let mut landings = 0u32;
    let mut pending: Vec<match_engine::Vector2> = Vec::new();
    let mut pending_age: Vec<f32> = Vec::new();
    while sim.sim_time < sim.timeline.duration_secs() + 1.0 {
        sim.step();
        for a in &sim.last_actions {
            if a.type_id == 1 {
                if let Some(t) = a.target {
                    pass_n += 1;
                    let best = sim
                        .agents
                        .iter()
                        .filter(|p| p.team_id == a.team_id)
                        .map(|p| p.position.distance(t))
                        .fold(f32::MAX, f32::min);
                    if best < 2.0 {
                        near += 1;
                    }
                    pending.push(t);
                    pending_age.push(0.0);
                }
            }
        }
        let ball = sim.ball.position;
        for i in 0..pending.len() {
            pending_age[i] += sim.config.dt;
            if ball.distance(pending[i]) < 1.0 {
                landings += 1;
                let best = sim
                    .agents
                    .iter()
                    .map(|p| p.position.distance(pending[i]))
                    .fold(f32::MAX, f32::min);
                if best > 3.0 {
                    ghost += 1;
                }
                pending_age[i] = -1.0;
            }
        }
        // Compact the lists.
        let mut keep = Vec::with_capacity(pending.len());
        for (i, age) in pending_age.iter().enumerate() {
            if *age > 0.0 && *age < 3.0 {
                keep.push((pending[i], *age));
            }
        }
        pending.clear();
        pending_age.clear();
        for (p, a) in keep {
            pending.push(p);
            pending_age.push(a);
        }
    }
    assert!(pass_n > 200, "expected many passes, got {pass_n}");
    let near_ratio = near as f32 / pass_n as f32;
    let ghost_ratio = ghost as f32 / landings.max(1) as f32;
    assert!(
        near_ratio > 0.30,
        "only {:.0}% of pass targets had a teammate within 2 m",
        near_ratio * 100.0
    );
    assert!(
        ghost_ratio < 0.30,
        "{ghost}/{landings} ball arrivals had nobody within 3 m ({:.0}%)",
        ghost_ratio * 100.0
    );

    // Every team keeps a recognisable defensive line (not a random swarm).
    let mut sim2 = build_simulation();
    let mut spreads: Vec<f32> = Vec::new();
    for _ in 0..(300 * 60) {
        sim2.step();
        for team in [&sim2.home_team_id, &sim2.away_team_id] {
            let xs: Vec<f32> = sim2
                .agents
                .iter()
                .filter(|a| &a.team_id == team && a.role == PlayerRole::Defender)
                .map(|a| a.position.x)
                .collect();
            if xs.len() >= 2 {
                let hi = xs.iter().cloned().fold(f32::MIN, f32::max);
                let lo = xs.iter().cloned().fold(f32::MAX, f32::min);
                spreads.push(hi - lo);
            }
        }
    }
    spreads.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = spreads[spreads.len() / 2];
    let mean = spreads.iter().sum::<f32>() / spreads.len().max(1) as f32;
    assert!(
        median < 12.0,
        "defensive line median spread {median:.1} m (mean {mean:.1})"
    );
}
