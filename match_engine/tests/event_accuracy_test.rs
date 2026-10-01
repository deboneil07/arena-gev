//! Accuracy contract for the Opta f24 feed: at the moment an event row
//! appears, the doer and the ball must be on the event origin, and at the
//! moment a pass lands the receiver must be on the ball.
//!
//! Measurements come from the engine's own ground-truth snapshots
//! (`SimulationEngine::event_snapshots` / `landing_snapshots`), captured at
//! the dispatch instant *before* choreography runs. The whole match is
//! simulated once and every threshold is asserted from that single run.
//!
//! Thresholds start at the measured baseline and are tightened as each phase
//! of the accuracy plan lands. The end targets are:
//!   doer ≤1.0 m ≥99%, ball ≤0.5 m 100% (ball events), both ≥98%,
//!   receiver ≤2 m ≥95%, nearest ≤2 m ≥99%, ghost pass ≤2%, slip ≤2.0 s.

use match_engine::{
    ball_spot_expected, is_ball_event, parse_opta_f24, MatchSimulation, PITCH_HALF_LENGTH,
    PITCH_HALF_WIDTH,
};
use std::path::PathBuf;

fn load_xml() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join("2647319.xml");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

struct Metrics {
    doer_n: usize,
    doer_15: usize,
    doer_10: usize,
    doer_worst: (f32, f32, u16),
    ball_n: usize,
    ball_15: usize,
    ball_05: usize,
    ball_worst: (f32, f32, u16),
    both_n: usize,
    both_ok: usize,
    touch_n: usize,
    ghost: usize,
    land_n: usize,
    recv_n: usize,
    recv_2: usize,
    recv_worst: (f32, f32),
    nearest_2: usize,
    max_slip: f32,
    slipped: usize,
}

fn collect() -> Metrics {
    let ctx = parse_opta_f24(&load_xml()).expect("parse");
    let mut sim = MatchSimulation::from_context(ctx).expect("engine");
    let dur = sim.timeline.duration_secs();
    sim.run_for(dur);

    let mut m = Metrics {
        doer_n: 0,
        doer_15: 0,
        doer_10: 0,
        doer_worst: (0.0, 0.0, 0),
        ball_n: 0,
        ball_15: 0,
        ball_05: 0,
        ball_worst: (0.0, 0.0, 0),
        both_n: 0,
        both_ok: 0,
        touch_n: 0,
        ghost: 0,
        land_n: sim.landing_snapshots.len(),
        recv_n: 0,
        recv_2: 0,
        recv_worst: (0.0, 0.0),
        nearest_2: 0,
        max_slip: 0.0,
        slipped: 0,
    };

    for s in &sim.event_snapshots {
        let slip = s.sim_time - s.feed_time;
        if slip > m.max_slip {
            m.max_slip = slip;
        }
        if slip > 2.0 {
            m.slipped += 1;
        }

        let doer_d = s.doer_pos.map(|p| p.distance(s.origin));
        if let Some(d) = doer_d {
            m.doer_n += 1;
            if d <= 1.5 {
                m.doer_15 += 1;
            }
            if d <= 1.0 {
                m.doer_10 += 1;
            }
            if d > m.doer_worst.1 {
                m.doer_worst = (s.feed_time, d, s.type_id);
            }
            if is_ball_event(s.type_id) {
                m.touch_n += 1;
                if s.doer_pos.unwrap().distance(s.ball_pos) > 2.0 {
                    m.ghost += 1;
                }
            }
        }

        if s.ball_expected {
            m.ball_n += 1;
            let d = s.ball_pos.distance(s.origin);
            if d <= 1.5 {
                m.ball_15 += 1;
            }
            if d <= 0.5 {
                m.ball_05 += 1;
            }
            if d > m.ball_worst.1 {
                m.ball_worst = (s.feed_time, d, s.type_id);
            }
            if let Some(dd) = doer_d {
                m.both_n += 1;
                if dd <= 1.5 && d <= 1.5 {
                    m.both_ok += 1;
                }
            }
        }
    }

    for l in &sim.landing_snapshots {
        if let Some(rp) = l.receiver_pos {
            m.recv_n += 1;
            let d = rp.distance(l.landing);
            if d <= 2.0 {
                m.recv_2 += 1;
            }
            if d > m.recv_worst.1 {
                m.recv_worst = (l.sim_time, d);
            }
        }
        if let Some(np) = l.nearest_pos {
            if np.distance(l.landing) <= 2.0 {
                m.nearest_2 += 1;
            }
        }
    }

    // Every snapshot must be on the pitch — nothing teleports off the field.
    for s in &sim.event_snapshots {
        assert!(
            s.ball_pos.x.abs() <= PITCH_HALF_LENGTH + 1.5
                && s.ball_pos.y.abs() <= PITCH_HALF_WIDTH + 1.5,
            "ball off pitch at t={:.1}: {:?}",
            s.feed_time,
            s.ball_pos
        );
        if let Some(p) = s.doer_pos {
            assert!(
                p.x.abs() <= PITCH_HALF_LENGTH + 1.5 && p.y.abs() <= PITCH_HALF_WIDTH + 1.5,
                "doer off pitch at t={:.1}: {:?}",
                s.feed_time,
                p
            );
        }
    }

    m
}

#[test]
fn accuracy_contract_holds_for_the_full_match() {
    let m = collect();

    let doer_15 = 100.0 * m.doer_15 as f32 / m.doer_n as f32;
    let doer_10 = 100.0 * m.doer_10 as f32 / m.doer_n as f32;
    let ball_15 = 100.0 * m.ball_15 as f32 / m.ball_n as f32;
    let ball_05 = 100.0 * m.ball_05 as f32 / m.ball_n as f32;
    let both = 100.0 * m.both_ok as f32 / m.both_n as f32;
    let ghost = 100.0 * m.ghost as f32 / m.touch_n as f32;
    let recv = 100.0 * m.recv_2 as f32 / m.recv_n as f32;
    let nearest = 100.0 * m.nearest_2 as f32 / m.land_n as f32;
    let slipped = 100.0 * m.slipped as f32 / m.doer_n as f32;

    println!("=== event accuracy contract ===");
    println!(
        "doer  ≤1.5m {doer_15:.1}%   ≤1.0m {doer_10:.1}%   (n={}, worst t={:.1}s d={:.1}m type={})",
        m.doer_n, m.doer_worst.0, m.doer_worst.1, m.doer_worst.2
    );
    println!(
        "ball  ≤1.5m {ball_15:.1}%   ≤0.5m {ball_05:.1}%   (n={}, worst t={:.1}s d={:.1}m type={})",
        m.ball_n, m.ball_worst.0, m.ball_worst.1, m.ball_worst.2
    );
    println!("both  ≤1.5m {both:.1}%   (n={})", m.both_n);
    println!("ghost (doer >2m from ball) {ghost:.1}%  (n={})", m.touch_n);
    println!(
        "recv  ≤2m {recv:.1}%  nearest ≤2m {nearest:.1}%  (n={}/{}, worst t={:.1}s d={:.1}m)",
        m.recv_n, m.land_n, m.recv_worst.0, m.recv_worst.1
    );
    println!("slip  max {:.2}s, >2s on {slipped:.1}%", m.max_slip);

    // ── Baseline guards (tightened phase by phase) ──
    // Measured after the readiness gate: doer 98.3, ball 99.7, both 98.7,
    // ghost 0.6, recv 89.6, nearest 91.9, slip max 2.02s / 1.8% over 2s.
    assert!(doer_15 >= 97.0, "doer ≤1.5m regressed: {doer_15:.1}%");
    assert!(doer_10 >= 77.5, "doer ≤1.0m regressed: {doer_10:.1}%");
    assert!(ball_15 >= 99.0, "ball ≤1.5m regressed: {ball_15:.1}%");
    assert!(ball_05 >= 80.0, "ball ≤0.5m regressed: {ball_05:.1}%");
    assert!(both >= 97.0, "BOTH regressed: {both:.1}%");
    assert!(ghost <= 2.0, "ghost passes regressed: {ghost:.1}%");
    assert!(recv >= 88.0, "receiver ≤2m regressed: {recv:.1}%");
    assert!(nearest >= 91.0, "nearest ≤2m regressed: {nearest:.1}%");
    assert!(m.max_slip <= 6.0, "dispatch slip blew up: {:.2}s", m.max_slip);
    assert!(slipped <= 5.0, "too many events slipped >2s: {slipped:.1}%");
}

// `ball_spot_expected` must mirror the engine's own notion of which events
// need the ball staged on their origin.
#[test]
fn ball_spot_expected_matches_ball_event_predicate() {
    for tid in 0..200u16 {
        if is_ball_event(tid) {
            assert!(ball_spot_expected(tid), "type {tid} is a ball event");
        }
    }
}
