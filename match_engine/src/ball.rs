//! Ball trajectory & choreography — continuous 3D kinematics `(x, y, z)` on
//! top of the 2D pitch.
//!
//! The ball is modelled as a small state machine:
//!
//! ```text
//!            attach_to                launch
//!   Loose ◀──────────────▶ AttachedToPlayer ──────▶ InFlight
//!     ▲                                                  │
//!     └──────────────── lands (residual roll) ◀──────────┘
//! ```
//!
//! * **InFlight** integrates a quadratic ease-out horizontally
//!   (`u_drag = 2u − u²`, aerodynamic drag) and a parabolic arc vertically
//!   (`z = 4·h·u·(1−u)`), so the God's-eye view can render drop shadows and
//!   scale changes for crosses, lofted through-balls and clearances.
//! * **Loose** rolls with exponential-ish friction until it stops.
//!
//! Everything is deterministic and allocation-free during [`Ball::update`].

use crate::formation::Vector2;

/// Minimum flight time regardless of distance (s).
pub const MIN_FLIGHT_TIME: f32 = 0.35;
/// Ground-pass launch speed (m/s).
pub const GROUND_PASS_SPEED: f32 = 19.0;
/// Aerial/lofted launch speed (m/s).
pub const AERIAL_PASS_SPEED: f32 = 20.0;
/// Shot launch speed (m/s).
pub const SHOT_SPEED: f32 = 28.0;
/// Distance beyond which a pass is automatically lofted (m).
pub const AUTO_AERIAL_DISTANCE: f32 = 28.0;
/// Residual roll speed after a ball lands (m/s).
pub const LANDING_RESIDUAL_SPEED: f32 = 3.0;
/// Friction (deceleration) applied to a loose ball (m/s²).
pub const LOOSE_FRICTION: f32 = 6.0;
/// Below this speed a loose ball is considered stopped (m/s).
pub const REST_SPEED: f32 = 0.05;
/// Top speed at which a carried ball closes the gap to the carrier's boots.
/// Bounded so an attach never reads as a teleport.
pub const CLOSE_TO_FEET_SPEED: f32 = 6.0;

// ── Types ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BallFlightType {
    /// Stays on the ground (`z = 0`).
    Ground,
    /// Parabolic arc peaking at `max_height` metres.
    Aerial { max_height: f32 },
    /// Ground reposition toward a *future event spot*: constant speed (no
    /// drag ease) so the arrival can be timed exactly, and a dead stop on
    /// landing — it has to be resting on that spot, not roll past it.
    Reposition,
}

impl BallFlightType {
    /// Short label for diagnostics / accuracy snapshots.
    #[inline]
    pub fn label(&self) -> &'static str {
        match self {
            BallFlightType::Ground => "Ground",
            BallFlightType::Aerial { .. } => "Aerial",
            BallFlightType::Reposition => "Reposition",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BallState {
    /// Carried by a player; position tracks the player each tick.
    AttachedToPlayer { player_id: String, offset: Vector2 },
    /// Airborne / travelling between two points.
    InFlight {
        start_pos: Vector2,
        target_pos: Vector2,
        total_time: f32,
        elapsed_time: f32,
        flight_type: BallFlightType,
    },
    /// Rolling freely with friction.
    Loose { velocity: Vector2, friction: f32 },
}

impl BallState {
    /// Short label for diagnostics / accuracy snapshots.
    #[inline]
    pub fn label(&self) -> &'static str {
        match self {
            BallState::AttachedToPlayer { .. } => "AttachedToPlayer",
            BallState::InFlight { .. } => "InFlight",
            BallState::Loose { .. } => "Loose",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Ball {
    pub position: Vector2,
    /// Height off the ground (z-axis, metres).
    pub altitude: f32,
    pub state: BallState,
}

impl Ball {
    /// A stationary ball at rest.
    pub fn new(initial_pos: Vector2) -> Self {
        Self {
            position: initial_pos,
            altitude: 0.0,
            state: BallState::Loose {
                velocity: Vector2::zero(),
                friction: LOOSE_FRICTION,
            },
        }
    }

    /// True while the ball is travelling between two points.
    #[inline]
    pub fn is_in_flight(&self) -> bool {
        matches!(self.state, BallState::InFlight { .. })
    }

    /// True while carried by a player.
    #[inline]
    pub fn is_attached(&self) -> bool {
        matches!(self.state, BallState::AttachedToPlayer { .. })
    }

    /// True while the ball is loose and (near-)stationary.
    #[inline]
    pub fn is_resting(&self) -> bool {
        matches!(&self.state, BallState::Loose { velocity, .. } if velocity.length() <= REST_SPEED)
    }

    /// Flight progress `u ∈ [0, 1]` (1.0 when not in flight).
    #[inline]
    pub fn progress(&self) -> f32 {
        match self.state {
            BallState::InFlight {
                total_time,
                elapsed_time,
                ..
            } if total_time > 0.0 => (elapsed_time / total_time).clamp(0.0, 1.0),
            _ => 1.0,
        }
    }

    /// Attach the ball to a player **without moving it**.
    ///
    /// The stored offset is the ball's existing position relative to the
    /// player, so `position = player_pos + offset` is a no-op: the ball stays
    /// exactly where Opta said the action happened. It then rolls to the
    /// boots at [`CLOSE_TO_FEET_SPEED`] in [`Ball::update`], which is what
    /// stops a pass from ever appearing to teleport from empty space.
    pub fn attach_to(&mut self, player_id: String, player_pos: Vector2, _heading: f32) {
        let offset = self.position - player_pos;
        self.position = (player_pos + offset).clamp_on_pitch();
        self.altitude = 0.0;
        self.state = BallState::AttachedToPlayer { player_id, offset };
    }

    /// Place a stationary ball at a position (dead-ball restart). The ball is
    /// loose and at rest, so it never drags the formation while play is stopped.
    pub fn place_at(&mut self, pos: Vector2) {
        self.position = pos.clamp_on_pitch();
        self.altitude = 0.0;
        self.state = BallState::Loose {
            velocity: Vector2::zero(),
            friction: LOOSE_FRICTION,
        };
    }

    /// Move the ball *continuously* toward `target`, never teleporting. Short
    /// distances collapse into a stationary placement.
    ///
    /// The InFlight profile front-loads speed (its initial derivative is 2), so
    /// the duration is derived from `max_speed` to keep the *peak* speed (and
    /// therefore the per-tick movement) bounded.
    pub fn travel_to(&mut self, target: Vector2, max_speed: f32, is_aerial: bool) {
        let target = target.clamp_on_pitch();
        let dist = self.position.distance(target);
        if dist < 0.6 {
            self.place_at(target);
            return;
        }
        let flight_type = if is_aerial {
            BallFlightType::Aerial {
                max_height: (dist * 0.12).clamp(1.0, 6.5),
            }
        } else {
            BallFlightType::Ground
        };
        let duration = (2.0 * dist / max_speed.max(1.0)).max(0.2);
        self.begin_flight(target, duration, flight_type);
    }

    /// Roll the ball to `target` so that it arrives `eta` seconds from now.
    ///
    /// The InFlight profile normally eases out (`u_drag = 2u − u²`), which
    /// halves the achievable average speed for a given peak. Staging toward a
    /// known future event does not need a kick-like profile — it needs to be
    /// *there* on time — so this uses [`BallFlightType::Reposition`]: linear
    /// motion at `dist / eta`, capped at `max_speed` (if that is not enough,
    /// it goes as fast as allowed and arrives as early as possible).
    pub fn travel_to_eta(&mut self, target: Vector2, eta: f32, max_speed: f32) {
        let target = target.clamp_on_pitch();
        let dist = self.position.distance(target);
        if dist < 0.6 {
            self.place_at(target);
            return;
        }
        // No `MIN_FLIGHT_TIME` floor here: staging is not a kick. The floor
        // would make every correction in the last third of a second land after
        // the event fires — the ball is then still in flight at dispatch.
        let duration = eta.max(dist / max_speed.max(1.0)).max(1e-3);
        self.begin_flight(target, duration, BallFlightType::Reposition);
    }

    /// Launch the ball toward `target`, deriving the flight time from the
    /// distance and an event-appropriate speed.
    pub fn launch(&mut self, target: Vector2, is_aerial: bool, is_shot: bool) {
        let total_time = self.natural_flight_time(target, is_aerial, is_shot);
        let (_, flight_type) = self.classify(target, is_aerial, is_shot);
        self.begin_flight(target, total_time, flight_type);
    }

    /// The physical flight time for a launch: `max(0.35, distance / speed)`.
    pub fn natural_flight_time(&self, target: Vector2, is_aerial: bool, is_shot: bool) -> f32 {
        let (speed, _) = self.classify(target, is_aerial, is_shot);
        (self.position.distance(target) / speed).max(MIN_FLIGHT_TIME)
    }

    /// Like [`Ball::launch`] but with an explicit flight duration — used to
    /// align the ball's arrival with the next event on the timeline.
    pub fn launch_with_duration(
        &mut self,
        target: Vector2,
        is_aerial: bool,
        is_shot: bool,
        duration: f32,
    ) {
        let (_, flight_type) = self.classify(target, is_aerial, is_shot);
        self.begin_flight(target, duration.max(MIN_FLIGHT_TIME), flight_type);
    }

    fn classify(&self, target: Vector2, is_aerial: bool, is_shot: bool) -> (f32, BallFlightType) {
        let dist = self.position.distance(target);
        if is_shot {
            let ft = if is_aerial {
                BallFlightType::Aerial { max_height: 2.2 }
            } else {
                BallFlightType::Ground
            };
            (SHOT_SPEED, ft)
        } else if is_aerial || dist > AUTO_AERIAL_DISTANCE {
            let height = (dist * 0.12).clamp(2.5, 6.5);
            (AERIAL_PASS_SPEED, BallFlightType::Aerial { max_height: height })
        } else {
            (GROUND_PASS_SPEED, BallFlightType::Ground)
        }
    }

    fn begin_flight(&mut self, target: Vector2, total_time: f32, flight_type: BallFlightType) {
        let start = self.position.clamp_on_pitch();
        let target = target.clamp_on_pitch();
        self.position = start;
        self.altitude = 0.0;
        // Callers floor the duration themselves (`natural_flight_time`,
        // `launch_with_duration`, `travel_to`); a blanket floor here would
        // silently break staged arrivals, which must land on the exact tick.
        self.state = BallState::InFlight {
            start_pos: start,
            target_pos: target,
            total_time,
            elapsed_time: 0.0,
            flight_type,
        };
    }

    /// Advance the ball one tick. `player_at` resolves an attached player's
    /// current `(position, heading)`; return `None` if the player is unknown.
    pub fn update<F>(&mut self, dt: f32, player_at: F)
    where
        F: Fn(&str) -> Option<(Vector2, f32)>,
    {
        // Move the old state out without cloning (the placeholder is a cheap
        // `Copy`-only Loose state); this avoids a `String` allocation per tick
        // while the ball is attached to a player.
        let old = std::mem::replace(
            &mut self.state,
            BallState::Loose {
                velocity: Vector2::zero(),
                friction: LOOSE_FRICTION,
            },
        );
        match old {
            BallState::AttachedToPlayer { player_id, offset } => {
                let offset = match player_at(&player_id) {
                    Some((pos, heading)) => {
                        // Where the ball settles once it reaches the carrier:
                        // just ahead of the boots, in the facing direction.
                        let settle = Vector2::new(heading.cos() * 0.4, heading.sin() * 0.4);
                        // Close the gap at a bounded roll speed so a ball that
                        // was still on its way to the carrier never teleports.
                        let offset = offset + (settle - offset).truncate(CLOSE_TO_FEET_SPEED * dt);
                        self.position = (pos + offset).clamp_on_pitch();
                        offset
                    }
                    None => offset,
                };
                self.altitude = 0.0;
                self.state = BallState::AttachedToPlayer { player_id, offset };
            }

            BallState::InFlight {
                start_pos,
                target_pos,
                total_time,
                elapsed_time,
                flight_type,
            } => {
                let next_elapsed = elapsed_time + dt;
                let u = (next_elapsed / total_time.max(1e-4)).clamp(0.0, 1.0);

                // Horizontal motion: quadratic drag ease-out for kicks and
                // passes, exact constant speed for event staging.
                let u_h = match flight_type {
                    BallFlightType::Reposition => u,
                    _ => 2.0 * u - u * u,
                };
                self.position = start_pos + (target_pos - start_pos) * u_h;

                // Vertical parabola (0 at both ends, `h` at u = 0.5).
                self.altitude = match flight_type {
                    BallFlightType::Ground => 0.0,
                    BallFlightType::Aerial { max_height } => 4.0 * max_height * u * (1.0 - u),
                    BallFlightType::Reposition => 0.0,
                };

                if u >= 1.0 {
                    // Landed at the destination: a staged ball stops dead on
                    // the spot, a kicked one rolls on with residual pace.
                    self.position = target_pos;
                    self.altitude = 0.0;
                    let velocity = match flight_type {
                        BallFlightType::Reposition => Vector2::zero(),
                        _ => (target_pos - start_pos).normalize() * LANDING_RESIDUAL_SPEED,
                    };
                    self.state = BallState::Loose {
                        velocity,
                        friction: LOOSE_FRICTION,
                    };
                } else {
                    self.state = BallState::InFlight {
                        start_pos,
                        target_pos,
                        total_time,
                        elapsed_time: next_elapsed,
                        flight_type,
                    };
                }
            }

            BallState::Loose { velocity, friction } => {
                self.altitude = 0.0;
                let speed = velocity.length();
                if speed > REST_SPEED {
                    self.position = (self.position + velocity * dt).clamp_on_pitch();
                    let decay = (friction * dt).min(speed);
                    let new_speed = speed - decay;
                    self.state = BallState::Loose {
                        velocity: velocity.normalize() * new_speed,
                        friction,
                    };
                } else {
                    self.state = BallState::Loose {
                        velocity: Vector2::zero(),
                        friction,
                    };
                }
            }
        }

        // Never let a numerical edge case poison the renderer.
        if !self.position.is_finite() {
            self.position = Vector2::zero();
        }
        if !self.altitude.is_finite() || self.altitude < 0.0 {
            self.altitude = 0.0;
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod ball_tests {
    use super::*;
    use crate::parser::{PITCH_HALF_LENGTH, PITCH_HALF_WIDTH};
    use std::collections::HashMap;

    fn lookup<'a>(
        players: &'a HashMap<String, (Vector2, f32)>,
    ) -> impl Fn(&str) -> Option<(Vector2, f32)> + 'a {
        move |id: &str| players.get(id).copied()
    }

    /// Step a ball `seconds` at 60 Hz with no players.
    fn run(ball: &mut Ball, seconds: f32) {
        let none: HashMap<String, (Vector2, f32)> = HashMap::new();
        let steps = (seconds * 60.0).round() as i32;
        for _ in 0..steps {
            ball.update(1.0 / 60.0, lookup(&none));
        }
    }

    #[test]
    fn test_new_ball_at_rest() {
        let b = Ball::new(Vector2::new(1.0, 2.0));
        assert_eq!(b.position, Vector2::new(1.0, 2.0));
        assert_eq!(b.altitude, 0.0);
        assert!(!b.is_in_flight());
        assert!(!b.is_attached());
        assert_eq!(b.progress(), 1.0);
    }

    #[test]
    fn test_ground_pass_stays_on_ground_and_lands_on_target() {
        let mut b = Ball::new(Vector2::new(0.0, 0.0));
        b.launch(Vector2::new(20.0, 0.0), false, false);
        assert!(matches!(
            b.state,
            BallState::InFlight {
                flight_type: BallFlightType::Ground,
                ..
            }
        ));
        // Never leaves the ground, and the landing point is the target (the
        // ball then rolls on, which is checked separately).
        let none: HashMap<String, (Vector2, f32)> = HashMap::new();
        let mut max_z = 0.0f32;
        let mut landed_at = None;
        for _ in 0..240 {
            b.update(1.0 / 60.0, lookup(&none));
            max_z = max_z.max(b.altitude);
            if !b.is_in_flight() {
                landed_at = Some(b.position);
                break;
            }
        }
        assert_eq!(max_z, 0.0);
        let landed = landed_at.expect("ball never landed");
        assert!(landed.distance(Vector2::new(20.0, 0.0)) < 0.05, "landed at {landed:?}");
    }

    #[test]
    fn test_aerial_parabola_peaks_and_returns() {
        let mut b = Ball::new(Vector2::new(0.0, 0.0));
        b.launch(Vector2::new(30.0, 0.0), true, false);
        let height = match b.state {
            BallState::InFlight {
                flight_type: BallFlightType::Aerial { max_height },
                ..
            } => max_height,
            _ => panic!("expected aerial"),
        };
        let none: HashMap<String, (Vector2, f32)> = HashMap::new();
        let mut max_z = 0.0f32;
        let mut landed_at = None;
        for _ in 0..300 {
            b.update(1.0 / 60.0, lookup(&none));
            max_z = max_z.max(b.altitude);
            if !b.is_in_flight() {
                landed_at = Some(b.position);
                break;
            }
        }
        assert!((max_z - height).abs() < 0.2, "apex {max_z} vs expected {height}");
        assert_eq!(b.altitude, 0.0);
        let landed = landed_at.expect("never landed");
        assert!(landed.distance(Vector2::new(30.0, 0.0)) < 0.05);
    }

    #[test]
    fn test_flight_time_matches_distance_over_speed() {
        let mut b = Ball::new(Vector2::zero());
        b.launch(Vector2::new(19.0, 0.0), false, false);
        match b.state {
            BallState::InFlight { total_time, .. } => {
                assert!((total_time - 1.0).abs() < 1e-3, "t={total_time}");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_short_flight_has_minimum_duration() {
        let mut b = Ball::new(Vector2::zero());
        b.launch(Vector2::new(0.5, 0.0), false, false);
        match b.state {
            BallState::InFlight { total_time, .. } => assert_eq!(total_time, MIN_FLIGHT_TIME),
            _ => panic!(),
        }
    }

    #[test]
    fn test_long_pass_becomes_aerial() {
        let mut b = Ball::new(Vector2::zero());
        b.launch(Vector2::new(40.0, 0.0), false, false);
        assert!(matches!(
            b.state,
            BallState::InFlight {
                flight_type: BallFlightType::Aerial { .. },
                ..
            }
        ));
    }

    #[test]
    fn test_shot_is_faster_than_pass() {
        let mut pass = Ball::new(Vector2::zero());
        pass.launch(Vector2::new(20.0, 0.0), false, false);
        let mut shot = Ball::new(Vector2::zero());
        shot.launch(Vector2::new(20.0, 0.0), false, true);
        let t_pass = match pass.state {
            BallState::InFlight { total_time, .. } => total_time,
            _ => panic!(),
        };
        let t_shot = match shot.state {
            BallState::InFlight { total_time, .. } => total_time,
            _ => panic!(),
        };
        assert!(t_shot < t_pass, "shot {t_shot} should arrive before pass {t_pass}");
    }

    #[test]
    fn test_horizontal_motion_uses_quadratic_ease_out() {
        let mut b = Ball::new(Vector2::zero());
        // In-pitch target (off-pitch targets are clamped).
        b.launch_with_duration(Vector2::new(40.0, 0.0), false, false, 1.0);
        // After exactly half the flight, u_drag = 2(0.5) - 0.25 = 0.75.
        run(&mut b, 0.5);
        assert!(
            (b.position.x - 30.0).abs() < 0.5,
            "expected ~30 m at u=0.5, got {}",
            b.position.x
        );
    }

    #[test]
    fn test_landing_transitions_to_loose_then_rests() {
        let mut b = Ball::new(Vector2::zero());
        b.launch(Vector2::new(10.0, 0.0), false, false);
        run(&mut b, 2.0);
        assert!(matches!(b.state, BallState::Loose { .. }));
        run(&mut b, 5.0);
        match b.state {
            BallState::Loose { velocity, .. } => assert!(velocity.length() <= REST_SPEED),
            _ => panic!("expected loose ball"),
        }
    }

    #[test]
    fn test_attach_never_moves_the_ball() {
        // The event origin is ground truth: binding the ball to a player must
        // leave it exactly where it was — no snap to the boots.
        let carrier = Vector2::new(5.0, 5.0);
        let mut b = Ball::new(Vector2::zero());
        b.attach_to("p".into(), carrier, 0.0);
        assert!(b.is_attached());
        assert_eq!(b.position, Vector2::zero());

        // It then rolls to the carrier's feet at a bounded speed (the carrier
        // stands still, so every tick must stay under the roll budget).
        let mut players = HashMap::new();
        players.insert("p".to_string(), (carrier, 0.0));
        let mut max_step = 0.0f32;
        let mut prev = b.position;
        for _ in 0..600 {
            b.update(1.0 / 60.0, lookup(&players));
            max_step = max_step.max(prev.distance(b.position));
            prev = b.position;
        }
        let boots = carrier + Vector2::new(0.4, 0.0);
        assert!(
            b.position.distance(boots) < 0.05,
            "never reached the boots: {:?}",
            b.position
        );
        assert!(max_step < CLOSE_TO_FEET_SPEED / 60.0 + 1e-3, "closed {max_step} m in a tick");
        assert_eq!(b.altitude, 0.0);
    }

    #[test]
    fn test_attach_unknown_player_keeps_position() {
        let mut b = Ball::new(Vector2::new(2.0, 2.0));
        b.attach_to("ghost".into(), Vector2::new(0.0, 0.0), 0.0);
        let none: HashMap<String, (Vector2, f32)> = HashMap::new();
        for _ in 0..30 {
            b.update(1.0 / 60.0, lookup(&none));
        }
        // Keeps the initial attach offset; no panic, no NaN.
        assert!(b.position.is_finite());
    }

    #[test]
    fn test_ball_stays_within_pitch_bounds() {
        let mut b = Ball::new(Vector2::zero());
        // Launch hard at a corner far off-pitch.
        b.launch(Vector2::new(500.0, 500.0), true, true);
        let none: HashMap<String, (Vector2, f32)> = HashMap::new();
        for _ in 0..600 {
            b.update(1.0 / 60.0, lookup(&none));
            assert!(b.position.x >= -PITCH_HALF_LENGTH - 1e-3);
            assert!(b.position.x <= PITCH_HALF_LENGTH + 1e-3);
            assert!(b.position.y >= -PITCH_HALF_WIDTH - 1e-3);
            assert!(b.position.y <= PITCH_HALF_WIDTH + 1e-3);
            assert!(b.altitude >= 0.0);
        }
    }

    #[test]
    fn test_no_nan_over_long_run() {
        let mut b = Ball::new(Vector2::zero());
        for i in 0..200 {
            let target = Vector2::new((i as f32).sin() * 40.0, (i as f32).cos() * 25.0);
            b.launch(target, i % 2 == 0, i % 5 == 0);
            run(&mut b, 1.0);
            assert!(b.position.is_finite(), "position NaN at iter {i}");
            assert!(b.altitude.is_finite() && b.altitude >= 0.0);
        }
    }

    #[test]
    fn test_update_is_deterministic() {
        let run_once = || {
            let mut b = Ball::new(Vector2::new(-5.0, 2.0));
            b.launch(Vector2::new(30.0, -10.0), true, false);
            let none: HashMap<String, (Vector2, f32)> = HashMap::new();
            for _ in 0..120 {
                b.update(1.0 / 60.0, lookup(&none));
            }
            (b.position, b.altitude)
        };
        assert_eq!(run_once(), run_once());
    }
}
