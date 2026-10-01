//! Diagnostic: measure PRE-event doer/ball positioning (the moment each event
//! fires), pass landing/receiver quality, kick-off correctness, ball-stuck
//! episodes and — crucially — *why* each accuracy failure happened.
//!
//! Event/landing metrics come from the engine's ground-truth snapshots so the
//! numbers here are exactly what `tests/event_accuracy_test.rs` asserts.

use match_engine::{parse_opta_f24, BallState, SimulationEngine, Vector2};
use std::collections::HashMap;
use std::path::PathBuf;

fn build() -> SimulationEngine {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join("2647319.xml");
    let xml = std::fs::read_to_string(&path).expect("read fixture");
    let ctx = parse_opta_f24(&xml).expect("parse");
    SimulationEngine::from_context(ctx).expect("build")
}

#[derive(Default, Clone)]
struct TypeStat {
    n: u32,
    missing: u32,
    doer_ok: u32,
    ball_ok: u32,
    both_ok: u32,
    both_den: u32,
    doer_sum: f64,
    ball_sum: f64,
}

/// Why a doer/ball/... missed the origin, from feed-side feasibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Fail {
    Missing,
    SameTick,
    FeedInfeasible,
    LateProfile,
    Staging,
}

fn main() {
    let mut sim = build();
    let dur = sim.timeline.duration_secs();

    // Ball-stuck detection.
    let mut stuck: Vec<(f32, f32, f32, f32, u32)> = Vec::new(); // (t, x, y, nearest, still)
    let mut prev_ball = (f32::NAN, f32::NAN);
    let mut still_ticks = 0u32;
    let mut was_stuck = false;

    // State of each entity at the tick its event fired (for failure reasons).

    // Kick-off samples (only while the ball really is on the centre spot).
    let mut ko: Vec<(f32, String, String, f32)> = Vec::new(); // (t, piece_team, nearest_team, dist)
    let mut was_piece = false;
    let mut piece_times: Vec<(f32, String, String)> = Vec::new();

    let mut prev_score = (0u32, 0u32);
    let mut goal_kickoffs: Vec<(f32, String)> = Vec::new();

    while sim.sim_time < dur + 1.0 {
        sim.step();

        // ── ball-stuck ──────────────────────────────────────────────
        let flying = sim.ball.is_in_flight();
        let attached = matches!(sim.ball.state, BallState::AttachedToPlayer { .. });
        let moving =
            (sim.ball.position.x - prev_ball.0).abs() + (sim.ball.position.y - prev_ball.1).abs();
        if !flying && !attached && moving < 0.001 {
            still_ticks += 1;
            if still_ticks == 120 {
                let nearest = nearest_player(&sim, sim.ball.position);
                stuck.push((
                    sim.sim_time,
                    sim.ball.position.x,
                    sim.ball.position.y,
                    nearest,
                    1,
                ));
                was_stuck = true;
            } else if still_ticks > 120 && was_stuck {
                if let Some(last) = stuck.last_mut() {
                    last.4 = still_ticks;
                }
            }
        } else {
            still_ticks = 0;
            was_stuck = false;
        }
        prev_ball = (sim.ball.position.x, sim.ball.position.y);

        // ── set pieces / kick-off ───────────────────────────────────
        if sim.set_piece_active() {
            was_piece = true;
            if let (Some(team), Some(kind)) = (sim.set_piece_team(), sim.set_piece_kind()) {
                piece_times.push((sim.sim_time, kind.to_string(), team.to_string()));
                if kind == "kickoff" && sim.ball.position.length() < 2.0 {
                    let home = &sim.home_team_id;
                    let mut kd = f32::INFINITY;
                    let mut kt = "?";
                    for a in sim.agents.iter() {
                        let d = a.position.length();
                        if d < kd {
                            kd = d;
                            kt = if a.team_id == *home { "home" } else { "away" };
                        }
                    }
                    let piece_side = if team == home.as_str() { "home" } else { "away" };
                    ko.push((sim.sim_time, piece_side.to_string(), kt.to_string(), kd));
                }
            }
        } else if was_piece {
            was_piece = false;
        }
        if sim.score[0] != prev_score.0 || sim.score[1] != prev_score.1 {
            let conceding = if sim.score[0] > prev_score.0 {
                sim.away_team_id.clone()
            } else {
                sim.home_team_id.clone()
            };
            goal_kickoffs.push((sim.sim_time, conceding));
            prev_score = (sim.score[0], sim.score[1]);
        }

    }

    // ── feed-side reference data ───────────────────────────────────
    let events = sim.dispatcher.events().to_vec();
    let mut idx_of: HashMap<u64, usize> = HashMap::new();
    for (i, e) in events.iter().enumerate() {
        idx_of.insert(e.id, i);
    }
    // previous event / previous same-player event / previous ball event
    let mut prev_same: Vec<Option<usize>> = vec![None; events.len()];
    let mut prev_ball_ev: Vec<Option<usize>> = vec![None; events.len()];
    let mut last_player: HashMap<&str, usize> = HashMap::new();
    let mut last_ball: Option<usize> = None;
    for (i, e) in events.iter().enumerate() {
        prev_same[i] = if e.player_id.is_empty() {
            None
        } else {
            last_player.get(e.player_id.as_str()).copied()
        };
        prev_ball_ev[i] = last_ball;
        if !e.player_id.is_empty() {
            last_player.insert(e.player_id.as_str(), i);
        }
        if match_engine::ball_spot_expected(e.type_id) {
            last_ball = Some(i);
        }
    }

    report_snapshots(
        &sim,
        &events,
        &idx_of,
        &prev_same,
        &prev_ball_ev,
    );
    report_landings(&sim);
    report_kickoffs(&ko);
    report_set_pieces(&piece_times);
    report_stuck(&stuck);
    report_goals(&goal_kickoffs, &sim);
}

fn report_snapshots(
    sim: &SimulationEngine,
    events: &[match_engine::NormalizedEvent],
    idx_of: &HashMap<u64, usize>,
    prev_same: &[Option<usize>],
    prev_ball_ev: &[Option<usize>],
) {
    let snaps = &sim.event_snapshots;
    let mut n = 0u32;
    let mut doer_at_org = 0u32;
    let mut ball_at_org = 0u32;
    let mut both_ready = 0u32;
    let mut both_n = 0u32;
    let mut ball_n = 0u32;
    let mut doer_dists: Vec<f32> = Vec::new();
    let mut ball_dists: Vec<f32> = Vec::new();
    let mut by_type: HashMap<u16, TypeStat> = HashMap::new();
    let mut missing_ids: HashMap<String, u32> = HashMap::new();
    let mut ghost_pass = 0u32;
    let mut touch_n = 0u32;
    let mut max_slip = 0.0f32;
    let mut slipped = 0u32;

    let mut doer_fails: HashMap<Fail, u32> = HashMap::new();
    let mut ball_fails: HashMap<Fail, u32> = HashMap::new();
    let mut doer_fail_examples: Vec<(f32, u16, f32, Fail, String, Option<f32>)> = Vec::new();
    let mut ball_fail_examples: Vec<(f32, u16, f32, Fail, String, f32)> = Vec::new();
    let mut ball_staging_detail: Vec<String> = Vec::new();
    let mut ball_far_detail: Vec<String> = Vec::new();

    for s in snaps {
        n += 1;
        let slip = s.sim_time - s.feed_time;
        max_slip = max_slip.max(slip);
        if slip > 2.0 {
            slipped += 1;
        }

        let st = by_type.entry(s.type_id).or_default();
        st.n += 1;
        let doer_d = s.doer_pos.map(|p| p.distance(s.origin));
        let ball_d = s.ball_pos.distance(s.origin);
        let doer_ok = doer_d.is_some_and(|d| d <= 1.5);

        match doer_d {
            Some(d) => {
                st.doer_sum += d as f64;
                doer_dists.push(d);
                if d <= 1.5 {
                    st.doer_ok += 1;
                    doer_at_org += 1;
                } else {
                    let reason = classify_doer(
                        s.event_id,
                        d,
                        s.feed_time,
                        events,
                        idx_of,
                        prev_same,
                    );
                    *doer_fails.entry(reason).or_default() += 1;
                    doer_fail_examples.push((
                        s.feed_time,
                        s.type_id,
                        d,
                        reason,
                        s.doer_state.unwrap_or("MISSING").to_string(),
                        s.doer_stage_target.map(|t| t.distance(s.origin)),
                    ));
                }
                if match_engine::is_ball_event(s.type_id) {
                    touch_n += 1;
                    if s.doer_pos.unwrap().distance(s.ball_pos) > 2.0 {
                        ghost_pass += 1;
                    }
                }
            }
            None => {
                st.missing += 1;
                *missing_ids.entry(s.doer_id.clone()).or_default() += 1;
                *doer_fails.entry(Fail::Missing).or_default() += 1;
            }
        }

        if s.ball_expected {
            st.ball_sum += ball_d as f64;
            ball_n += 1;
            ball_dists.push(ball_d);
            if ball_d <= 1.5 {
                st.ball_ok += 1;
                ball_at_org += 1;
            } else {
                let (reason, need_speed) = classify_ball(
                    s.event_id,
                    s.feed_time,
                    events,
                    idx_of,
                    prev_ball_ev,
                );
                *ball_fails.entry(reason).or_default() += 1;
                {
                    ball_staging_detail.push(if s.pending_kickoff {
                        "kickoff pending".into()
                    } else if s.ball_carrier.as_deref() == Some(s.doer_id.as_str()) {
                        "carrier is the doer".into()
                    } else if s.ball_carrier.is_some() {
                        "carrier is someone else".into()
                    } else if let Some(t) = s.ball_flight_target {
                        let fd = t.distance(s.origin);
                        let rem = s.ball_flight_remaining.unwrap_or(0.0);
                        let kind = s.ball_flight_kind.unwrap_or("?");
                        let st = s.staging_decision;
                        if fd <= 1.5 {
                            if rem > 0.0 {
                                format!("{st}/{kind}→spot, {rem:.2}s")
                            } else {
                                format!("{kind}→spot, landed")
                            }
                        } else if rem > 0.0 {
                            format!("{kind} elsewhere {} ({rem:.1}s)", bucket(fd))
                        } else {
                            format!("{kind} elsewhere {} (landed)", bucket(fd))
                        }
                    } else {
                        "not in flight".into()
                    });
                }
                ball_far_detail.push(bucket(ball_d));
                ball_fail_examples.push((
                    s.feed_time,
                    s.type_id,
                    ball_d,
                    reason,
                    s.ball_state_head.to_string(),
                    need_speed,
                ));
            }
            if doer_d.is_some() {
                both_n += 1;
                st.both_den += 1;
            }
            if doer_ok {
                st.both_ok += 1;
                both_ready += 1;
            }
        }
    }

    doer_dists.sort_by(|a, b| a.partial_cmp(b).unwrap());
    ball_dists.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |v: &[f32], p: f32| {
        if v.is_empty() {
            f32::NAN
        } else {
            v[((v.len() - 1) as f32 * p) as usize]
        }
    };

    println!("=== PRE-event positioning ({n} events) ===");
    println!(
        "doer  within 1.5m of origin: {doer_at_org} ({:.0}%)",
        100.0 * doer_at_org as f32 / n as f32
    );
    println!(
        "ball  within 1.5m of origin: {ball_at_org} ({:.0}%)  [of {ball_n} ball events]",
        100.0 * ball_at_org as f32 / ball_n.max(1) as f32
    );
    println!(
        "BOTH ready:                 {both_ready} ({:.0}%)  [of {both_n} with a doer]",
        100.0 * both_ready as f32 / both_n.max(1) as f32
    );
    println!("dispatch slip: max={max_slip:.2}s, >2s on {slipped} ({:.1}%)",
        100.0 * slipped as f32 / n as f32);
    if !doer_dists.is_empty() {
        println!(
            "doer→origin: mean={:.2} p50={:.2} p90={:.2} max={:.2}",
            doer_dists.iter().sum::<f32>() / doer_dists.len() as f32,
            pct(&doer_dists, 0.5),
            pct(&doer_dists, 0.9),
            doer_dists.last().unwrap()
        );
    }
    if !ball_dists.is_empty() {
        println!(
            "ball→origin: mean={:.2} p50={:.2} p90={:.2} max={:.2}",
            ball_dists.iter().sum::<f32>() / ball_dists.len() as f32,
            pct(&ball_dists, 0.5),
            pct(&ball_dists, 0.9),
            ball_dists.last().unwrap()
        );
    }

    println!("\n=== by event type ===");
    let mut types: Vec<u16> = by_type.keys().copied().collect();
    types.sort_unstable();
    for t in types {
        let s = &by_type[&t];
        let denom = (s.n - s.missing).max(1);
        println!(
            "  type {t:3} ({:<22}) n={:4} missing={:3} doer≤1.5={:5.1}% ball≤1.5={:5.1}% both={:5.1}%",
            event_name(t),
            s.n,
            s.missing,
            100.0 * s.doer_ok as f32 / denom as f32,
            100.0 * s.ball_ok as f32 / s.n as f32,
            100.0 * s.both_ok as f32 / s.both_den.max(1) as f32,
        );
    }

    println!("\n=== FAILURE REASONS ===");
    println!("doer misses ({}):", doer_fails.values().sum::<u32>());
    print_reasons(&doer_fails);
    println!("ball misses ({}):", ball_fails.values().sum::<u32>());
    print_reasons(&ball_fails);

    println!("\n  fixable ('Staging') doer misses by agent state:");
    let hist: Vec<String> = doer_fail_examples
        .iter()
        .filter(|e| e.3 == Fail::Staging)
        .map(|e| state_head(&e.4))
        .collect();
    print_hist(&hist);
    println!("  fixable ('Staging') doer misses: stage-target → true origin:");
    let hist: Vec<String> = doer_fail_examples
        .iter()
        .filter(|e| e.3 == Fail::Staging)
        .map(|e| match e.5 {
            None => "no stage target".into(),
            Some(d) => bucket(d),
        })
        .collect();
    print_hist(&hist);
    println!("  fixable ('Staging') doer misses: distance bucket:");
    let hist: Vec<String> = doer_fail_examples
        .iter()
        .filter(|e| e.3 == Fail::Staging)
        .map(|e| bucket(e.2))
        .collect();
    print_hist(&hist);
    println!("  fixable ('Staging') ball misses by ball state:");
    let hist: Vec<String> = ball_fail_examples
        .iter()
        .filter(|e| e.3 == Fail::Staging)
        .map(|e| state_head(&e.4))
        .collect();
    print_hist(&hist);
    println!("  'SameTick' ball misses by ball state:");
    let hist: Vec<String> = ball_fail_examples
        .iter()
        .filter(|e| e.3 == Fail::SameTick)
        .map(|e| state_head(&e.4))
        .collect();
    print_hist(&hist);
    println!("  'SameTick' ball misses: distance bucket:");
    let hist: Vec<String> = ball_fail_examples
        .iter()
        .filter(|e| e.3 == Fail::SameTick)
        .map(|e| bucket(e.2))
        .collect();
    print_hist(&hist);
    println!("  ball misses by required average speed (m/s):");
    let hist: Vec<String> = ball_fail_examples
        .iter()
        .map(|e| match e.5 {
            x if !x.is_finite() => "n/a".into(),
            x if x <= 13.0 => "<=13".into(),
            x if x <= 26.0 => "13-26".into(),
            x if x <= 40.0 => "26-40".into(),
            x if x <= 70.0 => "40-70".into(),
            _ => ">70".into(),
        })
        .collect();
    print_hist(&hist);
    println!("  ALL ball misses: situation at dispatch:");
    print_hist(&ball_staging_detail);

    doer_fail_examples.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
    println!("\n  worst doer misses:");
    for (t, ty, d, r, st, sd) in doer_fail_examples.iter().take(12) {
        let sdst = match sd {
            Some(x) => format!("{x:5.1}m→"),
            None => "  n/a→".into(),
        };
        println!("    t={t:7.1} type={ty:3} d={d:6.1}m [{r:?}] stage {sdst} {st}");
    }
    println!("  ALL ball misses: distance bucket:");
    print_hist(&ball_far_detail);

    ball_fail_examples.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
    println!("\n  worst ball misses:");
    for (t, ty, d, r, st, sp) in ball_fail_examples.iter().take(12) {
        let spd = if sp.is_finite() {
            format!("{sp:5.1}m/s")
        } else {
            "  n/a".into()
        };
        println!("    t={t:7.1} type={ty:3} d={d:6.1}m [{r:?}] {spd} {st}");
    }

    println!("\n=== players with no agent (doer MISSING) ===");
    let mut mv: Vec<(u32, String)> = missing_ids
        .into_iter()
        .map(|(k, v)| (v, k))
        .collect();
    mv.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (c, id) in mv.iter().take(12) {
        println!("  {id}  {c} events");
    }

    println!("\n=== passes / touches ({touch_n}) ===");
    println!(
        "doer >2m from the ball: {ghost_pass} ({:.0}%)",
        100.0 * ghost_pass as f32 / touch_n.max(1) as f32
    );
}

fn print_reasons(map: &HashMap<Fail, u32>) {
    let mut v: Vec<(Fail, u32)> = map.iter().map(|(k, v)| (*k, *v)).collect();
    v.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    for (r, c) in v {
        println!("    {r:<15?} {c}");
    }
}

/// Distance bucket labels for stage-target analysis.
fn bucket(d: f32) -> String {
    if d <= 1.0 {
        "<=1m".to_string()
    } else if d <= 3.0 {
        "1-3m".to_string()
    } else if d <= 8.0 {
        "3-8m".to_string()
    } else {
        ">8m".to_string()
    }
}

/// `Loose { .. }` → `Loose`, so states group into a histogram.
fn state_head(s: &str) -> String {
    s.split(" {").next().unwrap_or(s).to_string()
}

fn print_hist(items: &[String]) {
    let mut counts: HashMap<&str, u32> = HashMap::new();
    for i in items {
        *counts.entry(i.as_str()).or_default() += 1;
    }
    let mut v: Vec<(&str, u32)> = counts.into_iter().collect();
    v.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    if v.is_empty() {
        println!("    (none)");
    }
    for (k, c) in v {
        println!("    {k:<24} {c}");
    }
}

/// Classify why the doer was not on the origin when his event fired.
fn classify_doer(
    event_id: u64,
    dist: f32,
    feed_time: f32,
    events: &[match_engine::NormalizedEvent],
    idx_of: &HashMap<u64, usize>,
    prev_same: &[Option<usize>],
) -> Fail {
    let Some(&i) = idx_of.get(&event_id) else {
        return Fail::Staging;
    };
    let Some(p) = prev_same[i] else {
        return Fail::Staging;
    };
    let dt = feed_time - events[p].match_time_secs;
    let d = Vector2::new(
        events[i].origin_x - events[p].origin_x,
        events[i].origin_y - events[p].origin_y,
    )
    .length();
    if dt <= 0.0 {
        if d > 1.5 {
            Fail::SameTick
        } else {
            Fail::Staging
        }
    } else if d > 8.5 * dt + 2.0 {
        Fail::FeedInfeasible
    } else {
        let _ = dist;
        Fail::Staging
    }
}

/// Classify why the ball was not on the origin when its event fired.
/// Returns `(why, required average speed in m/s)`; speed is infinite when
/// there is no previous ball event to travel from.
fn classify_ball(
    event_id: u64,
    feed_time: f32,
    events: &[match_engine::NormalizedEvent],
    idx_of: &HashMap<u64, usize>,
    prev_ball_ev: &[Option<usize>],
) -> (Fail, f32) {
    let Some(&i) = idx_of.get(&event_id) else {
        return (Fail::Staging, f32::INFINITY);
    };
    let Some(p) = prev_ball_ev[i] else {
        return (Fail::Staging, f32::INFINITY);
    };
    let dt = feed_time - events[p].match_time_secs;
    let d = Vector2::new(
        events[i].origin_x - events[p].origin_x,
        events[i].origin_y - events[p].origin_y,
    )
    .length();
    if dt <= 0.0 {
        (Fail::SameTick, f32::INFINITY)
    } else {
        let avg = d / dt;
        if avg > 26.0 {
            (Fail::FeedInfeasible, avg)
        } else if avg > 13.0 {
            // Front-loaded profile (2·dist/speed) is clamped at 26 m/s, so
            // anything needing more than 13 m/s average arrives late.
            (Fail::LateProfile, avg)
        } else {
            (Fail::Staging, avg)
        }
    }
}

fn report_landings(sim: &SimulationEngine) {
    let land = &sim.landing_snapshots;
    let n = land.len();
    if n == 0 {
        println!("\n=== pass landings: none ===");
        return;
    }
    let (mut recv_2, mut recv_5, mut anyone_2, mut with_recv) = (0u32, 0u32, 0u32, 0u32);
    let mut recv_dists: Vec<f32> = Vec::new();
    let mut anyone_dists: Vec<f32> = Vec::new();
    let mut wrong_recv = 0u32;
    let mut wrong_examples: Vec<(f32, f32, f32)> = Vec::new();
    for l in land {
        if let Some(rp) = l.receiver_pos {
            with_recv += 1;
            let d = rp.distance(l.landing);
            recv_dists.push(d);
            if d <= 2.0 {
                recv_2 += 1;
            } else if d > 5.0 {
                wrong_recv += 1;
                wrong_examples.push((l.sim_time, d, rp.distance(l.target)));
            }
            if d <= 5.0 {
                recv_5 += 1;
            }
        }
        if let Some(np) = l.nearest_pos {
            let d = np.distance(l.landing);
            anyone_dists.push(d);
            if d <= 2.0 {
                anyone_2 += 1;
            }
        }
    }
    let pct = |v: &[f32], p: f32| {
        if v.is_empty() {
            f32::NAN
        } else {
            v[((v.len() - 1) as f32 * p) as usize]
        }
    };
    println!("\n=== pass landings ({n}) ===");
    println!(
        "  intended receiver within 2m: {recv_2}/{with_recv} ({:.0}%)",
        100.0 * recv_2 as f32 / with_recv.max(1) as f32
    );
    println!(
        "  intended receiver within 5m: {recv_5}/{with_recv} ({:.0}%)",
        100.0 * recv_5 as f32 / with_recv.max(1) as f32
    );
    println!(
        "  ANY player    within 2m:     {anyone_2}/{n} ({:.0}%)",
        100.0 * anyone_2 as f32 / n as f32
    );
    if !recv_dists.is_empty() {
        println!(
            "  receiver→ball: mean={:.2} p50={:.2} p90={:.2} max={:.2}",
            recv_dists.iter().sum::<f32>() / recv_dists.len() as f32,
            pct(&recv_dists, 0.5),
            pct(&recv_dists, 0.9),
            recv_dists.last().unwrap()
        );
    }
    if !anyone_dists.is_empty() {
        println!(
            "  nearest→ball:  mean={:.2} p50={:.2} p90={:.2}",
            anyone_dists.iter().sum::<f32>() / anyone_dists.len() as f32,
            pct(&anyone_dists, 0.5),
            pct(&anyone_dists, 0.9)
        );
    }
    println!("  receiver misses >5m (WRONG_RECEIVER): {wrong_recv}");
    wrong_examples.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    for (t, d, dt) in wrong_examples.iter().take(8) {
        println!("    t={t:7.1} receiver {d:.1}m from landing (target→recv {dt:.1}m)");
    }
}

fn report_kickoffs(ko: &[(f32, String, String, f32)]) {
    println!("\n=== kick-off: is the RIGHT team on the centre spot? ===");
    let mut seen = Vec::new();
    let mut ok = 0u32;
    for (t, team, kt, kd) in ko.iter() {
        if seen.iter().any(|s: &f32| (s - t).abs() < 3.0) {
            continue;
        }
        seen.push(*t);
        let good = *kd < 2.0 && kt == team;
        if good {
            ok += 1;
        }
        println!(
            "  t={t:7.1} piece={team} nearest_center={kt} dist={kd:.1}m  [{}]",
            if good { "OK" } else { "WRONG" }
        );
    }
    println!("kick-off samples OK: {ok}/{}", seen.len().max(1));
}

fn report_set_pieces(piece_times: &[(f32, String, String)]) {
    println!("\n=== set-piece kinds ===");
    let mut kinds: HashMap<String, u32> = HashMap::new();
    for (_, k, _) in piece_times.iter() {
        *kinds.entry(k.clone()).or_default() += 1;
    }
    for (k, c) in kinds.iter() {
        println!("  {k}: {c} ticks");
    }
}

fn report_stuck(stuck: &[(f32, f32, f32, f32, u32)]) {
    println!("\n=== ball stuck on ground (2s+ no flight/attach/movement) ===");
    println!("episodes: {}", stuck.len());
    let mut worst_stuck = stuck.to_vec();
    worst_stuck.sort_by_key(|a| std::cmp::Reverse(a.4));
    for (t, x, y, nearest, ticks) in worst_stuck.iter().take(25) {
        println!(
            "  t={t:7.1} ball=({x:.1},{y:.1}) nearest_player={nearest:.1}m duration={:.1}s",
            *ticks as f32 / 60.0
        );
    }
}

fn report_goals(goal_kickoffs: &[(f32, String)], sim: &SimulationEngine) {
    if goal_kickoffs.is_empty() {
        return;
    }
    println!("\n=== goals → conceding team kicks off ===");
    for (t, team) in goal_kickoffs.iter() {
        let side = if *team == sim.home_team_id { "home" } else { "away" };
        println!("  t={t:7.1} conceding={side}");
    }
}

fn nearest_player(sim: &SimulationEngine, p: Vector2) -> f32 {
    sim.agents
        .iter()
        .map(|a| a.position.distance(p))
        .fold(f32::INFINITY, f32::min)
}

fn event_name(t: u16) -> &'static str {
    match t {
        1 => "Pass",
        2 => "Offside Pass",
        3 => "Take On",
        4 => "Foul",
        5 => "Out",
        6 => "Corner Awarded",
        7 => "Tackle",
        8 => "Interception",
        10 => "Save",
        12 => "Clearance",
        13 => "Miss",
        15 => "Attempt Saved",
        16 => "Goal",
        17 => "Card",
        41 => "Punch",
        43 => "Deleted event",
        44 => "Aerial",
        45 => "Challenge",
        49 => "Ball recovery",
        50 => "Dispossessed",
        51 => "Error",
        52 => "Keeper pick-up",
        55 => "Offside provoked",
        56 => "Shield ball opp",
        59 => "Keeper Sweeper",
        61 => "Ball touch",
        74 => "Blocked Pass",
        80 => "Unknown",
        83 => "Unknown",
        _ => "Other",
    }
}
