//! Step 5 verification — the master engine + 3D ball.
//!
//! Contains the brief's test verbatim followed by additional dynamic coverage.

use match_engine::ball::{BallFlightType, BallState};
use match_engine::engine::{SimulationEngine, TEAM_BALL, TEAM_HOME};
use match_engine::loader::parse_opta_f24;

const SAMPLE_XML: &str = r#"
<Games>
  <Game id="1" home_team_id="home" away_team_id="away">
    <Event id="1" type_id="34" period_id="16" team_id="home">
      <Q qualifier_id="30" value="h1,h2,h3,h4,h5,h6,h7,h8,h9,h10,h11"/>
      <Q qualifier_id="44" value="1,2,2,2,2,3,3,3,4,4,4"/>
      <Q qualifier_id="59" value="1,2,3,4,5,6,7,8,9,10,11"/>
      <Q qualifier_id="131" value="1,2,3,4,5,6,7,8,9,10,11"/>
    </Event>
    <Event id="2" type_id="34" period_id="16" team_id="away">
      <Q qualifier_id="30" value="a1,a2,a3,a4,a5,a6,a7,a8,a9,a10,a11"/>
      <Q qualifier_id="44" value="1,2,2,2,2,3,3,3,4,4,4"/>
      <Q qualifier_id="59" value="1,2,3,4,5,6,7,8,9,10,11"/>
      <Q qualifier_id="131" value="1,2,3,4,5,6,7,8,9,10,11"/>
    </Event>
    <Event id="3" type_id="32" period_id="1" team_id="home">
      <Q qualifier_id="127" value="Left to Right"/>
    </Event>
    <Event id="4" type_id="32" period_id="1" team_id="away">
      <Q qualifier_id="127" value="Right to Left"/>
    </Event>
    <Event id="5" type_id="1" period_id="1" min="0" sec="2" team_id="home" player_id="h10" x="50" y="50">
      <Q qualifier_id="140" value="70"/>
      <Q qualifier_id="141" value="80"/>
      <Q qualifier_id="1"/>
    </Event>
  </Game>
</Games>
"#;

#[test]
fn test_full_simulation_engine_run() {
    let ctx = parse_opta_f24(SAMPLE_XML).expect("Parser failed");
    let mut engine = SimulationEngine::new(ctx);

    assert_eq!(engine.agents.len(), 22);

    let dt = 1.0 / 60.0;
    let mut ball_flew_in_air = false;

    // Run 5 seconds of match simulation (300 frames)
    for _ in 0..300 {
        engine.tick(dt);
        let render_state = engine.get_render_state();
        assert_eq!(render_state.len(), 23);

        let ball_render = &render_state[22];
        if ball_render.z > 0.5 {
            ball_flew_in_air = true;
        }
    }

    assert!(ball_flew_in_air, "Ball failed to launch with parabolic aerial arc");
}

// ── Extra dynamic coverage ─────────────────────────────────────

fn engine() -> SimulationEngine {
    SimulationEngine::new(parse_opta_f24(SAMPLE_XML).expect("parse"))
}

#[test]
fn test_ball_launch_is_aerial_for_long_pass() {
    // The 29 m pass is automatically lofted (distance > 28 m).
    let mut e = engine();
    let mut seen_aerial = false;
    for _ in 0..240 {
        e.step();
        if let BallState::InFlight {
            flight_type: BallFlightType::Aerial { max_height },
            ..
        } = e.ball.state
        {
            assert!(max_height >= 2.5);
            seen_aerial = true;
        }
    }
    assert!(seen_aerial, "long pass was not lofted");
}

#[test]
fn test_ball_altitude_returns_to_ground() {
    let mut e = engine();
    e.run_for(6.0);
    assert_eq!(e.ball.altitude, 0.0, "ball should have landed");
}

#[test]
fn test_render_snapshot_is_flat_and_finite() {
    let mut e = engine();
    e.run_for(3.0);
    let rs = e.get_render_state();
    assert_eq!(rs.len(), 23);
    for r in &rs {
        assert!(r.x.is_finite() && r.y.is_finite() && r.z.is_finite());
    }
    // Exactly one ball entity, last.
    assert_eq!(rs.iter().filter(|r| r.team_index == TEAM_BALL).count(), 1);
    assert_eq!(rs[22].team_index, TEAM_BALL);
    // 11 home players.
    assert_eq!(rs.iter().filter(|r| r.team_index == TEAM_HOME).count(), 11);
}

#[test]
fn test_engine_is_deterministic_over_full_run() {
    let mut a = engine();
    let mut b = engine();
    a.run_for(6.0);
    b.run_for(6.0);
    assert_eq!(a.get_render_state(), b.get_render_state());
}

#[test]
fn test_ball_never_leaves_pitch_or_nan() {
    let mut e = engine();
    for _ in 0..600 {
        e.step();
        let b = &e.ball;
        assert!(b.position.is_finite() && b.altitude.is_finite() && b.altitude >= 0.0);
        assert!(match_engine::MatchContext::is_on_pitch(b.position.x, b.position.y));
    }
}

#[test]
fn test_engine_agents_obey_motion_caps() {
    let mut e = engine();
    for _ in 0..600 {
        e.step();
        for a in &e.agents {
            assert!(a.speed() <= match_engine::MAX_SPEED + 1e-3);
            assert!(a.acceleration.length() <= a.max_accel + 1e-3);
            assert!(a.position.is_finite());
        }
    }
}

#[test]
fn test_ball_eventually_comes_to_rest() {
    let mut e = engine();
    // The pass is deliberately held in the air until its receiver arrives, so
    // the ball may still be settling well after its natural flight time.
    e.run_for(10.0);
    // After the only pass has landed and rolled, the ball must stop.
    match e.ball.state {
        BallState::Loose { velocity, .. } => {
            assert!(velocity.length() <= match_engine::ball::REST_SPEED + 1e-3);
        }
        BallState::InFlight { .. } => panic!("ball still in flight after 10 s"),
        BallState::AttachedToPlayer { .. } => {}
    }
}

#[test]
fn test_engine_run_to_end_is_consistent() {
    let mut e = engine();
    e.run_to_end();
    assert!(e.is_consistent());
    assert!(e.dispatcher.is_finished());
}
