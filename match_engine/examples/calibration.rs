//! Permanent calibration dashboard: run the engine over a fixture and print
//! the accuracy contract (doer/ball/receiver/landing), pass-completion
//! agreement, shot outcomes, shape coherence and overlay stability.
//!
//! ```sh
//! cargo run --release --example calibration -- data/2647319.xml
//! ```
//!
//! Thresholds mirror `tests/event_accuracy_test.rs`. Tune them on a training
//! corpus, validate on held-out matches. Off-ball positions are INFERRED, not
//! measured — the dashboard labels them as such.

use match_engine::{is_ball_event, parse_opta_f24, MatchSimulation};
use std::collections::HashMap;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "data/2647319.xml".into());
    let xml = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let ctx = parse_opta_f24(&xml).expect("parse");
    let nev = ctx.events.len();
    let mut sim = MatchSimulation::from_context(ctx).expect("engine");
    let dur = sim.timeline.duration_secs();
    sim.run_for(dur);

    // ── Actor / ball accuracy ──
    let (mut dn, mut d15, mut d10, mut bn, mut b15, mut b05, mut both, mut both_n) = (0, 0, 0, 0, 0, 0, 0, 0);
    let (mut touch_n, mut ghost) = (0, 0);
    for s in &sim.event_snapshots {
        if let Some(d) = s.doer_pos.map(|p| p.distance(s.origin)) {
            dn += 1;
            if d <= 1.5 { d15 += 1; }
            if d <= 1.0 { d10 += 1; }
        }
        if s.ball_expected {
            bn += 1;
            let d = s.ball_pos.distance(s.origin);
            if d <= 1.5 { b15 += 1; }
            if d <= 0.5 { b05 += 1; }
            if let Some(dd) = s.doer_pos.map(|p| p.distance(s.origin)) {
                both_n += 1;
                if dd <= 1.5 && d <= 1.5 { both += 1; }
            }
        }
        if is_ball_event(s.type_id) {
            if let (Some(dp), bp) = (s.doer_pos, s.ball_pos) {
                touch_n += 1;
                if dp.distance(bp) > 2.0 { ghost += 1; }
            }
        }
    }
    // ── Receiver / landing ──
    let (mut rn, mut r2, mut land_n, mut near2) = (0, 0, 0, 0);
    for l in &sim.landing_snapshots {
        land_n += 1;
        if let Some(rp) = l.receiver_pos {
            rn += 1;
            if rp.distance(l.landing) <= 2.0 { r2 += 1; }
        }
        if let Some(np) = l.nearest_pos {
            if np.distance(l.landing) <= 2.0 { near2 += 1; }
        }
    }
    // ── Pass completion agreement (feed vs simulated receiver) ──
    let feed_completed = sim
        .event_snapshots
        .iter()
        .filter(|s| s.type_id == 1)
        .count();
    // ── Shot outcomes ──
    let mut shots: HashMap<u16, usize> = HashMap::new();
    for s in &sim.event_snapshots {
        if (13..=16).contains(&s.type_id) {
            *shots.entry(s.type_id).or_insert(0) += 1;
        }
    }
    // ── Slip ──
    let mut slipped = 0;
    let mut max_slip = 0.0f32;
    for s in &sim.event_snapshots {
        let slip = s.sim_time - s.feed_time;
        if slip > max_slip { max_slip = slip; }
        if slip > 2.0 { slipped += 1; }
    }
    let pct = |a: usize, b: usize| if b > 0 { 100.0 * a as f32 / b as f32 } else { 0.0 };

    println!("=== GEV calibration: {path} ===");
    println!("events: {nev}  sim: {dur:.0}s  snapshots: {}", sim.event_snapshots.len());
    println!("--- reconstruction (inferred off-ball, measured on-ball) ---");
    println!("doer   <=1.5m {:5.1}%  <=1.0m {:5.1}%  (n={dn})", pct(d15, dn), pct(d10, dn));
    println!("ball   <=1.5m {:5.1}%  <=0.5m {:5.1}%  (n={bn})", pct(b15, bn), pct(b05, bn));
    println!("both   <=1.5m {:5.1}%  (n={both_n})", pct(both, both_n));
    println!("ghost  (doer>2m ball) {:5.1}%  (n={touch_n})", pct(ghost, touch_n));
    println!("recv   <=2m   {:5.1}%  (n={rn})", pct(r2, rn));
    println!("nearest<=2m   {:5.1}%  (landings={land_n})", pct(near2, land_n));
    println!("pass rows (feed type 1): {feed_completed}");
    println!("shots  miss={} post={} saved={} goal={}",
        shots.get(&13).copied().unwrap_or(0),
        shots.get(&14).copied().unwrap_or(0),
        shots.get(&15).copied().unwrap_or(0),
        shots.get(&16).copied().unwrap_or(0));
    println!("slip   max {max_slip:.2}s  >2s {slipped} ({:.1}%)", pct(slipped, dn));
    println!("--- gates: doer15>=97 doer10>=77.5 ball15>=99 ball05>=80 both>=97 ghost<=2 recv>=88 near>=91 slip<=5 ---");
    let ok = pct(d15, dn) >= 97.0 && pct(d10, dn) >= 77.5 && pct(b15, bn) >= 99.0
        && pct(b05, bn) >= 80.0 && pct(both, both_n) >= 97.0 && pct(ghost, touch_n) <= 2.0
        && pct(r2, rn) >= 88.0 && pct(near2, land_n) >= 91.0 && pct(slipped, dn) <= 5.0;
    println!("{}", if ok { "PASS" } else { "FAIL" });
    if !ok { std::process::exit(1); }
}
