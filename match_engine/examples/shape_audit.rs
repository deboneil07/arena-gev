//! Shape audit — the adversarial diagnostic. It answers the only question that
//! matters: *do the players actually behave like a football team?*
//!
//! ```text
//! cargo run --release --example shape_audit
//! ```

use match_engine::{parse_opta_f24, AgentState, PlayerRole, SimulationEngine, TacticalIntent};
use std::path::PathBuf;

fn default_xml() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join("2647319.xml")
}

#[derive(Default)]
struct RoleStats {
    n: u64,
    to_anchor_sum: f64,
    anticipating: u64,
    max_to_anchor: f32,
}

fn main() {
    let path = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(default_xml);
    let xml = std::fs::read_to_string(&path).expect("read xml");
    let ctx = parse_opta_f24(&xml).expect("parse");
    let mut sim = SimulationEngine::from_context(ctx).expect("build");

    let mut by_role: std::collections::HashMap<&'static str, RoleStats> = Default::default();
    let mut by_intent: std::collections::HashMap<(&'static str, &'static str), u64> = Default::default();
    let mut samples = 0u64;
    let mut home_line_spread = 0.0f64;
    let mut away_line_spread = 0.0f64;
    let mut home_def_to_line = 0.0f64;
    let mut away_def_to_line = 0.0f64;
    let mut home_len = 0.0f64;
    let mut away_len = 0.0f64;
    let mut home_centroid_ball = 0.0f64;
    let mut away_centroid_ball = 0.0f64;
    let mut tick = 0u64;
    let mut corner_snapshot_at: Option<f32> = None;
    let mut kickoff_printed = false;
    let mut swarm_sum = 0.0f64;
    let mut swarm_max = 0u32;
    let mut role_order_ok = 0u64;
    let mut role_order_samples = 0u64;
    let mut defs_ahead_of_ball = 0u64;
    let mut defs_team_samples = 0u64;
    let mut open_def_ahead = 0u64;
    let mut open_def_samples = 0u64;
    let trace_start = 600.0f32;
    let mut traced = false;
    let end = sim.timeline.duration_secs() + 1.0;

    while sim.sim_time < end {
        sim.step();
        tick += 1;

        // Print the first kick-off and the first attacking corner so the
        // set-piece shape can be inspected directly.
        if !kickoff_printed && sim.sim_time > 4.0 && sim.set_piece_active() {
            kickoff_printed = true;
            println!();
            println!("── set-piece at t={:.1}s ball=({:.1},{:.1}) ──", sim.sim_time, sim.ball.position.x, sim.ball.position.y);
            print_shape(&sim);
        }
        if corner_snapshot_at.is_none()
            && sim
                .last_actions
                .iter()
                .any(|a| a.type_id == 6 && a.origin.x.abs() > 40.0)
        {
            corner_snapshot_at = Some(sim.sim_time + 8.0);
        }
        if let Some(t) = corner_snapshot_at {
            if sim.sim_time >= t {
                corner_snapshot_at = None;
                println!();
                println!("── attacking corner at t={:.1}s ball=({:.1},{:.1}) ──", sim.sim_time, sim.ball.position.x, sim.ball.position.y);
                print_shape(&sim);
            }
        }

        if !traced && sim.sim_time >= trace_start {
            traced = true;
            println!();
            println!("── trace at t={:.1}s  ball=({:.1},{:.1}) possession={:?} ──",
                sim.sim_time, sim.ball.position.x, sim.ball.position.y, sim.possession);
            for team in [&sim.home_team_id, &sim.away_team_id] {
                let tag = if team == &sim.home_team_id { "H" } else { "A" };
                print!("  {tag}: ");
                for a in sim.agents.iter().filter(|a| &a.team_id == team) {
                    let st = match a.state {
                        AgentState::InFormation => "I".to_string(),
                        AgentState::AnticipatingEvent { .. } => "A".to_string(),
                        AgentState::ExecutingAction { .. } => "E".to_string(),
                        AgentState::Recovering { .. } => "R".to_string(),
                    };
                    print!("{:?}{}({:.0},{:.0}) ", a.role, st, a.position.x, a.position.y);
                }
                println!();
            }
        }

        if tick.is_multiple_of(30) {
            samples += 1;
            let anchors = sim.anchors();
            // ├── swarm: how many of a team's outfielders are within 8 m of the ball
            for (team, attack_dx) in [(&sim.home_team_id, 1.0f32), (&sim.away_team_id, -1.0f32)] {
                let ball = sim.ball.position;
                let mut swarm = 0u32;
                let mut def_prog = 0.0f32;
                let mut mid_prog = 0.0f32;
                let mut fwd_prog = 0.0f32;
                let (mut dn, mut mn, mut fn_) = (0f32, 0f32, 0f32);
                let mut def_ahead = 0u32;
                let mut def_total = 0u32;
                for a in sim.agents.iter().filter(|a| &a.team_id == team) {
                    if a.role == PlayerRole::Goalkeeper {
                        continue;
                    }
                    if a.position.distance(ball) < 8.0 {
                        swarm += 1;
                    }
                    let prog = a.position.x * attack_dx;
                    match a.role {
                        PlayerRole::Defender => {
                            def_prog += prog;
                            dn += 1.0;
                            def_total += 1;
                            if prog > ball.x * attack_dx {
                                def_ahead += 1;
                            }
                        }
                        PlayerRole::Midfielder => {
                            mid_prog += prog;
                            mn += 1.0;
                        }
                        PlayerRole::Forward => {
                            fwd_prog += prog;
                            fn_ += 1.0;
                        }
                        _ => {}
                    }
                }
                swarm_sum += swarm as f64;
                swarm_max = swarm_max.max(swarm);
                if dn > 0.0 && mn > 0.0 && fn_ > 0.0 {
                    role_order_samples += 1;
                    let dm = def_prog / dn;
                    let mm = mid_prog / mn;
                    let fm = fwd_prog / fn_;
                    if dm < mm && mm < fm {
                        role_order_ok += 1;
                    }
                }
                if def_total > 0 {
                    defs_team_samples += 1;
                    if def_ahead == def_total {
                        defs_ahead_of_ball += 1;
                    }
                    if !sim.set_piece_active() {
                        open_def_samples += 1;
                        if def_ahead == def_total {
                            open_def_ahead += 1;
                        }
                    }
                }
            }

            for (team, is_home) in [(&sim.home_team_id, true), (&sim.away_team_id, false)] {
                let (mut minx, mut maxx) = (f32::MAX, f32::MIN);
                let (mut defmin, mut defmax) = (f32::MAX, f32::MIN);
                let mut def_n = 0f32;
                let mut def_sum = 0f32;
                let mut cx = 0f32;
                let mut cy = 0f32;
                let mut n = 0f32;
                for a in sim.agents.iter().filter(|a| &a.team_id == team) {
                    if a.role == PlayerRole::Goalkeeper {
                        continue;
                    }
                    minx = minx.min(a.position.x);
                    maxx = maxx.max(a.position.x);
                    cx += a.position.x;
                    cy += a.position.y;
                    n += 1.0;
                    if a.role == PlayerRole::Defender {
                        defmin = defmin.min(a.position.x);
                        defmax = defmax.max(a.position.x);
                        def_sum += a.position.x;
                        def_n += 1.0;
                    }
                    let key = role_key(a.role);
                    let s = by_role.entry(key).or_default();
                    s.n += 1;
                    if let Some(anchor) = anchors.get(&a.id) {
                        let d = a.position.distance(*anchor);
                        s.to_anchor_sum += d as f64;
                        s.max_to_anchor = s.max_to_anchor.max(d);
                    }
                    if matches!(a.state, AgentState::AnticipatingEvent { .. }) {
                        s.anticipating += 1;
                    }
                    let ik = intent_key(a.brain.current_intent);
                    *by_intent.entry((key, ik)).or_insert(0) += 1;
                }
                let mean_x = if def_n > 0.0 { def_sum / def_n } else { 0.0 };
                let mut dev = 0f32;
                for a in sim.agents.iter().filter(|a| &a.team_id == team && a.role == PlayerRole::Defender) {
                    dev += (a.position.x - mean_x).abs();
                }
                let line_dev = if def_n > 0.0 { (dev / def_n) as f64 } else { 0.0 };
                let len = (maxx - minx) as f64;
                let cball = ((cx / n.max(1.0) - sim.ball.position.x).powi(2)
                    + (cy / n.max(1.0) - sim.ball.position.y).powi(2))
                .sqrt() as f64;
                if is_home {
                    home_line_spread += (defmax - defmin).max(0.0) as f64;
                    home_def_to_line += line_dev;
                    home_len += len;
                    home_centroid_ball += cball;
                } else {
                    away_line_spread += (defmax - defmin).max(0.0) as f64;
                    away_def_to_line += line_dev;
                    away_len += len;
                    away_centroid_ball += cball;
                }
            }
        }
    }

    let s = samples as f64;
    println!();
    println!("=== shape audit ({} samples) ===", samples);
    println!(
        "backline max-min x   : home {:.1} m   away {:.1} m",
        home_line_spread / s,
        away_line_spread / s
    );
    println!(
        "backline mean |x-mean|: home {:.1} m   away {:.1} m",
        home_def_to_line / s,
        away_def_to_line / s
    );
    println!(
        "team length (x)      : home {:.1} m   away {:.1} m",
        home_len / s,
        away_len / s
    );
    println!(
        "centroid→ball dist   : home {:.1} m   away {:.1} m",
        home_centroid_ball / s,
        away_centroid_ball / s
    );
    println!(
        "ball swarm (<8m)     : mean {:.1}   max {}",
        swarm_sum / (s * 2.0),
        swarm_max
    );
    println!(
        "role line ordering   : {:.0}% of samples (Def<Mid<Fwd)",
        100.0 * role_order_ok as f64 / role_order_samples.max(1) as f64
    );
    println!(
        "whole backline ahead of ball: {:.1}% of team-samples",
        100.0 * defs_ahead_of_ball as f64 / defs_team_samples.max(1) as f64
    );
    println!(
        "  (open play only)       : {:.1}% of {}",
        100.0 * open_def_ahead as f64 / open_def_samples.max(1) as f64,
        open_def_samples
    );
    let mut rows: Vec<_> = by_role.iter().collect();
    rows.sort_by_key(|(k, _)| *k);
    println!("role                : mean→anchor   max→anchor   % anticipating");
    for (role, st) in rows {
        println!(
            "  {:<16}: {:>8.1} m   {:>8.1} m   {:>8.0}%",
            role,
            st.to_anchor_sum / st.n.max(1) as f64,
            st.max_to_anchor,
            100.0 * st.anticipating as f64 / st.n.max(1) as f64
        );
    }

    println!();
    println!("intent mix by role (% of role samples):");
    for role in ["Goalkeeper", "Defender", "Midfielder", "Forward"] {
        let total: u64 = by_intent
            .iter()
            .filter(|((r, _), _)| *r == role)
            .map(|(_, c)| *c)
            .sum();
        if total == 0 {
            continue;
        }
        let mut parts: Vec<(&str, u64)> = by_intent
            .iter()
            .filter(|((r, _), _)| *r == role)
            .map(|((_, i), c)| (*i, *c))
            .collect();
        parts.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
        let text: Vec<String> = parts
            .iter()
            .map(|(i, c)| format!("{i} {:.0}%", 100.0 * *c as f64 / total as f64))
            .collect();
        println!("  {role:<11}: {}", text.join(", "));
    }
}

fn intent_key(i: TacticalIntent) -> &'static str {
    match i {
        TacticalIntent::AnticipateEvent { .. } => "Anticipate",
        TacticalIntent::HoldShape => "HoldShape",
        TacticalIntent::ManMark { .. } => "ManMark",
        TacticalIntent::SupportTriangle { .. } => "Support",
        TacticalIntent::PressCarrier { .. } => "Press",
        TacticalIntent::CoverSpace { .. } => "Cover",
        TacticalIntent::TakeRun { .. } => "Run",
    }
}

fn role_key(r: PlayerRole) -> &'static str {
    match r {
        PlayerRole::Goalkeeper => "Goalkeeper",
        PlayerRole::Defender => "Defender",
        PlayerRole::Midfielder => "Midfielder",
        PlayerRole::Forward => "Forward",
        PlayerRole::Unknown => "Unknown",
    }
}

/// Print both teams' shapes in the attacking half, for set-piece inspection.
fn print_shape(sim: &SimulationEngine) {
    for (team, tag) in [(&sim.home_team_id, "H"), (&sim.away_team_id, "A")] {
        print!("  {tag}: ");
        let mut ps: Vec<_> = sim
            .agents
            .iter()
            .filter(|a| &a.team_id == team)
            .map(|a| (a.role, a.position))
            .collect();
        ps.sort_by(|a, b| b.1.x.partial_cmp(&a.1.x).unwrap());
        for (r, p) in ps {
            print!("{:?}({:.0},{:.0}) ", r, p.x, p.y);
        }
        println!();
    }
}
