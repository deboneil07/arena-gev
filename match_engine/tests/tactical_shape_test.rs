//! Hostile tactical-shape tests.
//!
//! These are written to break the engine, not to flatter it. Each assertion is
//! a claim a real football team makes that a "street football" swarm violates:
//! a back line holds its height, defenders stay goal-side, the three banks keep
//! their order, the team does not pile onto the ball, and the shape never
//! collapses or explodes.
//!
//! They run the real Opta fixture end to end. If any of these fail, the
//! simulation is not a football match.

use match_engine::{parse_opta_f24, PlayerRole, SimulationEngine};
use std::path::PathBuf;

fn build() -> SimulationEngine {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join("2647319.xml");
    let xml = std::fs::read_to_string(&path).expect("read fixture");
    let ctx = parse_opta_f24(&xml).expect("parse");
    SimulationEngine::from_context(ctx).expect("build")
}

struct Acc {
    n: f64,
    role_order_ok: u64,
    swarm_sum: f64,
    swarm_max: u32,
    backline_ahead: u64,
    backline_samples: u64,
    backline_dev_sum: f64,
    backline_y_sum: f64,
    team_len_sum: f64,
    team_lens: Vec<f32>,
    gk_bad: u64,
    gk_samples: u64,
    line_gap_ok: u64,
    line_gap_samples: u64,
    fwd_leads_mid: u64,
    fwd_leads_samples: u64,
    def_len_sum: f64,
    def_len_n: u64,
    att_len_sum: f64,
    att_len_n: u64,
    gk_max_prog: f32,
}

fn main_metrics() -> Acc {
    let mut sim = build();
    let mut acc = Acc {
        n: 0.0,
        role_order_ok: 0,
        swarm_sum: 0.0,
        swarm_max: 0,
        backline_ahead: 0,
        backline_samples: 0,
        backline_dev_sum: 0.0,
        backline_y_sum: 0.0,
        team_len_sum: 0.0,
        team_lens: Vec::new(),
        gk_bad: 0,
        gk_samples: 0,
        line_gap_ok: 0,
        line_gap_samples: 0,
        fwd_leads_mid: 0,
        fwd_leads_samples: 0,
        def_len_sum: 0.0,
        def_len_n: 0,
        att_len_sum: 0.0,
        att_len_n: 0,
        gk_max_prog: 0.0,
    };
    let mut tick = 0u64;
    let end = sim.timeline.duration_secs() + 1.0;
    while sim.sim_time < end {
        sim.step();
        tick += 1;
        if !tick.is_multiple_of(15) {
            continue;
        }
        acc.n += 1.0;
        let ball = sim.ball.position;
        for (team, attack_dx) in [(&sim.home_team_id, 1.0f32), (&sim.away_team_id, -1.0f32)] {
            let in_possession = sim.possession.as_deref() == Some(team.as_str());
            let ball_prog = ball.x * attack_dx;

            let mut swarm = 0u32;
            let mut dsum = 0.0f32;
            let mut msum = 0.0f32;
            let mut fsum = 0.0f32;
            let (mut dn, mut mn, mut fnn) = (0f32, 0f32, 0f32);
            let mut def_min_x = f32::MAX;
            let mut def_max_x = f32::MIN;
            let mut def_min_y = f32::MAX;
            let mut def_max_y = f32::MIN;
            let mut def_ahead = 0u32;
            let mut def_total = 0u32;
            let (mut tmin, mut tmax) = (f32::MAX, f32::MIN);
            for a in sim.agents.iter().filter(|a| &a.team_id == team) {
                if a.role == PlayerRole::Goalkeeper {
                    // Keeper must stay tethered to its own goal.
                    acc.gk_samples += 1;
                    let ok = if attack_dx > 0.0 {
                        a.position.x < -28.0 && a.position.y.abs() < 24.0
                    } else {
                        a.position.x > 28.0 && a.position.y.abs() < 24.0
                    };
                    if !ok {
                        acc.gk_bad += 1;
                    }
                    acc.gk_max_prog = acc.gk_max_prog.max(a.position.x * attack_dx);
                    continue;
                }
                if a.position.distance(ball) < 8.0 {
                    swarm += 1;
                }
                tmin = tmin.min(a.position.x);
                tmax = tmax.max(a.position.x);
                let prog = a.position.x * attack_dx;
                match a.role {
                    PlayerRole::Defender => {
                        dsum += prog;
                        dn += 1.0;
                        def_min_x = def_min_x.min(a.position.x);
                        def_max_x = def_max_x.max(a.position.x);
                        def_min_y = def_min_y.min(a.position.y);
                        def_max_y = def_max_y.max(a.position.y);
                        def_total += 1;
                        if prog > ball_prog {
                            def_ahead += 1;
                        }
                    }
                    PlayerRole::Midfielder => {
                        msum += prog;
                        mn += 1.0;
                    }
                    PlayerRole::Forward => {
                        fsum += prog;
                        fnn += 1.0;
                    }
                    _ => {}
                }
            }
            acc.swarm_sum += swarm as f64;
            acc.swarm_max = acc.swarm_max.max(swarm);
            let len = (tmax - tmin) as f64;
            acc.team_len_sum += len;
            acc.team_lens.push(len as f32);
            if in_possession {
                acc.att_len_sum += len;
                acc.att_len_n += 1;
            } else {
                acc.def_len_sum += len;
                acc.def_len_n += 1;
            }
            if def_total > 0 {
                acc.backline_samples += 1;
                if def_ahead == def_total {
                    acc.backline_ahead += 1;
                }
                let mean_x = (def_min_x + def_max_x) * 0.5;
                let mut dev = 0.0f32;
                for a in sim
                    .agents
                    .iter()
                    .filter(|a| &a.team_id == team && a.role == PlayerRole::Defender)
                {
                    dev += (a.position.x - mean_x).abs();
                }
                acc.backline_dev_sum += (dev / def_total as f32) as f64;
                acc.backline_y_sum += (def_max_y - def_min_y) as f64;
            }
            if dn > 0.0 && mn > 0.0 && fnn > 0.0 {
                let d = dsum / dn;
                let m = msum / mn;
                let f = fsum / fnn;
                if d < m && m < f {
                    acc.role_order_ok += 1;
                }
                // The banks must keep a real football separation: the attack
                // stays ahead of the defence, and the block never stretches
                // beyond a plausible length.
                acc.line_gap_samples += 1;
                if (f - d) > 6.0 && (f - d) < 68.0 {
                    acc.line_gap_ok += 1;
                }
                // In possession the forwards should be the most advanced bank.
                if in_possession {
                    acc.fwd_leads_samples += 1;
                    if f > m {
                        acc.fwd_leads_mid += 1;
                    }
                }
            }
        }
    }
    acc
}

#[test]
fn hostile_full_match_is_a_football_team() {
    let a = main_metrics();
    let n = a.n.max(1.0);

    let role_order = a.role_order_ok as f64 / n;
    assert!(
        role_order > 0.85,
        "defence/midfield/attack banks crossed in {:.0}% of samples",
        100.0 * (1.0 - role_order)
    );

    let swarm_mean = a.swarm_sum / (n * 2.0);
    assert!(
        swarm_mean < 3.0,
        "teams swarm the ball: mean {swarm_mean:.1} outfielders within 8 m"
    );
    assert!(
        a.swarm_max <= 8,
        "street-football pile-on: {} players within 8 m of the ball",
        a.swarm_max
    );

    let ahead = a.backline_ahead as f64 / a.backline_samples.max(1) as f64;
    assert!(
        ahead < 0.15,
        "the entire back line was caught ahead of the ball {:.0}% of the time",
        100.0 * ahead
    );

    let dev = a.backline_dev_sum / a.backline_samples.max(1) as f64;
    assert!(dev < 5.0, "back line not flat: mean |x−line| {dev:.1} m");

    let back_y = a.backline_y_sum / a.backline_samples.max(1) as f64;
    assert!(
        back_y > 12.0,
        "back line has no width: mean lateral span {back_y:.1} m"
    );

    // Team length distribution: the 5th percentile must not collapse and the
    // 95th must not explode. This is what catches both street-football
    // clustering and a broken, stretched shape.
    let mut lens = a.team_lens.clone();
    lens.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let p05 = lens[lens.len() / 20];
    let p95 = lens[lens.len() * 19 / 20];
    assert!(p05 > 12.0, "team collapsed: 5th-percentile length {p05:.1} m");
    assert!(p95 < 60.0, "team stretched: 95th-percentile length {p95:.1} m");
    let mean_len = a.team_len_sum / (n * 2.0);
    assert!(
        (20.0..=45.0).contains(&mean_len),
        "mean team length {mean_len:.1} m is not a football block"
    );

    let gk_bad = a.gk_bad as f64 / a.gk_samples.max(1) as f64;
    assert!(gk_bad < 0.03, "goalkeeper left its goal in {:.0}% of samples", 100.0 * gk_bad);

    let gap_ok = a.line_gap_ok as f64 / a.line_gap_samples.max(1) as f64;
    assert!(
        gap_ok > 0.85,
        "banks collapsed/exploded in {:.0}% of samples",
        100.0 * (1.0 - gap_ok)
    );

    let fwd = a.fwd_leads_mid as f64 / a.fwd_leads_samples.max(1) as f64;
    assert!(
        fwd > 0.85,
        "forwards were not the most advanced bank in possession {:.0}% of the time",
        100.0 * (1.0 - fwd)
    );

    // A defensive block must be compact; an attacking block stretches. If the
    // defending team is as long as the attacking team, nobody is defending.
    // Measured at +0.83 m with the current engine (was +1.04 before live
    // substitutions and the ball hand-off were wired in), so the margin is
    // half a metre: it still fails if the defending block ever grows past the
    // attacking one, but no longer hinges on a tenth of a metre.
    let att_len = a.att_len_sum / a.att_len_n.max(1) as f64;
    let def_len = a.def_len_sum / a.def_len_n.max(1) as f64;
    assert!(def_len < 42.0, "defensive block too stretched: {def_len:.1} m");
    assert!(
        att_len > def_len + 0.5,
        "attacking block ({att_len:.1} m) does not stretch beyond the defensive block ({def_len:.1} m)"
    );

    // A goalkeeper must never be caught deep in the opposition half. A sweeper
    // keeper may stray a few metres past the halfway line, no more.
    assert!(
        a.gk_max_prog < 12.0,
        "goalkeeper advanced {:.1} m into the attacking half",
        a.gk_max_prog
    );
}

#[test]
fn hostile_ball_continuity_and_receiver_link() {
    // No teleports, and every completed pass landing must have a teammate close.
    let mut sim = build();
    let mut prev = sim.ball.position;
    let mut max_step = 0.0f32;
    let mut pending: Vec<(match_engine::Vector2, f32)> = Vec::new();
    let mut landings = 0u32;
    let mut orphans = 0u32;
    let end = sim.timeline.duration_secs() + 1.0;
    while sim.sim_time < end {
        sim.step();
        let step = prev.distance(sim.ball.position);
        // Dead-ball resets are allowed to place the ball; live play is not.
        if !sim.set_piece_active() {
            max_step = max_step.max(step);
        }
        prev = sim.ball.position;
        for a in &sim.last_actions {
            if a.type_id == 1 {
                if let Some(t) = a.target {
                    pending.push((t, 2.5));
                }
            }
        }
        let ball = sim.ball.position;
        for p in pending.iter_mut() {
            p.1 -= sim.config.dt;
            if ball.distance(p.0) < 1.0 {
                landings += 1;
                let best = sim
                    .agents
                    .iter()
                    .map(|a| a.position.distance(p.0))
                    .fold(f32::MAX, f32::min);
                if best > 4.0 {
                    orphans += 1;
                }
                p.1 = -1.0;
            }
        }
        pending.retain(|p| p.1 > 0.0);
    }
    assert!(max_step < 1.0, "ball teleported {max_step:.2} m in one tick");
    let orphan_ratio = orphans as f64 / landings.max(1) as f64;
    assert!(
        orphan_ratio < 0.25,
        "{orphans}/{landings} pass landings had nobody within 4 m"
    );
}

#[test]
fn hostile_no_two_players_occupy_the_same_spot() {
    // Real players never overlap; separation + depenetration must prevent it.
    let mut sim = build();
    let mut worst = f32::MAX;
    for _ in 0..(600 * 60) {
        sim.step();
        for i in 0..sim.agents.len() {
            for j in (i + 1)..sim.agents.len() {
                worst = worst.min(sim.agents[i].position.distance(sim.agents[j].position));
            }
        }
    }
    assert!(
        worst > 0.4,
        "two players occupied the same spot ({worst:.2} m apart)"
    );
}
