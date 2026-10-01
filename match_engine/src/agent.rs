//! Player Agent Kinematics & Steering Behaviors — the micro layer.
//!
//! Each player is a physical vehicle driven by Craig Reynolds steering
//! behaviours (arrival + separation) integrated with a deterministic fixed
//! 60 Hz timestep. The macro layer (the elastic formation grid) supplies a
//! dynamic anchor; this layer turns that into smooth, rate-limited motion.
//!
//! ```text
//!                 AgentState
//!                      │
//!    ┌─────────────────┼──────────────────┐
//!    ▼                 ▼                  ▼
//! InFormation   AnticipatingEvent   ExecutingAction
//! (pulls to     (sprints to x,y)    (locks / kicks)
//!  anchor)             │                  │
//!    ▲                 └──────► Recovering ◄┘
//!    └────────────────────────── (returns to shape)
//! ```
//!
//! Steering = Arrive + w·Separation, capped by `max_accel`; velocity is then
//! capped by the state's speed limit (`max_speed_jog` / `max_speed_sprint`).

use crate::brain::{TacticalBrain, RAY_COUNT};
use crate::formation::{TacticalAnchor, Vector2};
use std::collections::HashMap;

// ── Defaults / constants ───────────────────────────────────────

/// Fixed physics timestep (60 Hz).
pub const DT: f32 = 1.0 / 60.0;
/// Fastest a player may ever move (sprint).
pub const MAX_SPEED: f32 = 8.5;
/// Default jog speed used while holding shape.
pub const DEFAULT_JOG_SPEED: f32 = 4.5;
/// Default sprint speed used while anticipating an event.
pub const DEFAULT_SPRINT_SPEED: f32 = 8.5;
/// Default maximum acceleration.
pub const DEFAULT_MAX_ACCEL: f32 = 6.0;
/// Personal space radius for separation forces (m).
pub const SEPARATION_RADIUS: f32 = 3.5;
/// How strongly separation is weighted against the arrive force.
pub const SEPARATION_WEIGHT: f32 = 2.5;
/// Angular turn-rate (rad/s) used for heading smoothing.
pub const TURN_SPEED: f32 = 4.0;
/// Above this speed a player faces the direction of travel.
pub const FACE_MOTION_SPEED: f32 = 1.0;
/// Below this speed an off-ball player turns to face the ball.
pub const FACE_BALL_SPEED: f32 = 0.3;
/// Default recovery duration after an action (s).
pub const RECOVER_SECS: f32 = 0.6;
/// Below this distance `arrive` applies a hard dynamic brake.
pub const BRAKE_RADIUS: f32 = 0.05;

// ── State machine ──────────────────────────────────────────────

/// Per-player personality attributes in `0..1`. Derived deterministically from
/// the player id, so an individual always reacts the same way across runs
/// while different players behave differently (individualism / flair).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerAttributes {
    /// Top-speed multiplier.
    pub pace: f32,
    /// Willingness to press / close down.
    pub aggression: f32,
    /// Willingness to make support runs.
    pub workrate: f32,
    /// Idle movement / showmanship.
    pub flair: f32,
    /// How tightly the player holds their tactical zone (high = disciplined).
    pub positioning: f32,
    /// How hard the player chases loose balls / late runs.
    pub determination: f32,
    /// Composure: less fidgeting, more deliberate movement.
    pub composure: f32,
    /// Stamina: sustains urgency sprints.
    pub stamina: f32,
    /// Reaction speed: how early the player commits to runs/presses.
    pub anticipation: f32,
    /// Tactical discipline: how tightly the player holds team shape and the
    /// back line rather than chasing the ball.
    pub teamwork: f32,
    /// Scanning / passing vision: how early support options are recognised.
    pub vision: f32,
}

impl Default for PlayerAttributes {
    fn default() -> Self {
        Self {
            pace: 0.5,
            aggression: 0.5,
            workrate: 0.5,
            flair: 0.5,
            positioning: 0.5,
            determination: 0.5,
            composure: 0.5,
            stamina: 0.5,
            anticipation: 0.5,
            teamwork: 0.5,
            vision: 0.5,
        }
    }
}

impl PlayerAttributes {
    /// Deterministic FNV-1a hash of the id → four independent `0..1` values.
    pub fn from_id(id: &str) -> Self {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in id.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let mut h2: u64 = h ^ 0x9e37_79b9_7f4a_7c15;
        h2 ^= h2 >> 29;
        h2 = h2.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        let v = |shift: u32| ((h >> shift) & 0xFF) as f32 / 255.0;
        let v2 = |shift: u32| ((h2 >> shift) & 0xFF) as f32 / 255.0;
        Self {
            pace: v(0),
            aggression: v(8),
            workrate: v(16),
            flair: v(24),
            positioning: v(32),
            determination: v(40),
            composure: v(48),
            stamina: v(56),
            anticipation: v2(0),
            teamwork: v2(8),
            vision: v2(16),
        }
    }
}

/// Coarse playing role, used to differentiate off-ball behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerRole {
    Goalkeeper,
    Defender,
    Midfielder,
    Forward,
    Unknown,
}

impl PlayerRole {
    /// Map an Opta `position_category` (qualifier 44) to a role.
    pub fn from_category(category: u8) -> Self {
        match category {
            1 => PlayerRole::Goalkeeper,
            2 => PlayerRole::Defender,
            3 => PlayerRole::Midfielder,
            4 => PlayerRole::Forward,
            _ => PlayerRole::Unknown,
        }
    }
}

/// The agent's behavioural state. Variants carry the data needed to act,
/// so an agent is self-contained (no external target table required).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AgentState {
    /// Holding the elastic formation shape (jog speed).
    InFormation,
    /// Sprinting to an anticipated event coordinate.
    AnticipatingEvent { target: Vector2 },
    /// Performing the action (on the ball); velocity is killed.
    ExecutingAction { remaining_secs: f32 },
    /// Brief recovery before returning to shape.
    Recovering { remaining_secs: f32 },
}

impl AgentState {
    /// Short label for diagnostics / accuracy snapshots.
    #[inline]
    pub fn label(&self) -> &'static str {
        match self {
            AgentState::InFormation => "InFormation",
            AgentState::AnticipatingEvent { .. } => "AnticipatingEvent",
            AgentState::ExecutingAction { .. } => "ExecutingAction",
            AgentState::Recovering { .. } => "Recovering",
        }
    }
}

// ── Agent ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PlayerAgent {
    pub id: String,
    pub team_id: String,
    pub shirt_number: u8,
    pub position: Vector2,
    pub velocity: Vector2,
    pub acceleration: Vector2,
    pub heading: f32, // facing angle in radians

    // Physical limits
    pub max_speed_jog: f32,
    pub max_speed_sprint: f32,
    pub max_accel: f32,
    pub is_goalkeeper: bool,
    /// Coarse playing role (set from the Opta position category).
    pub role: PlayerRole,
    /// Individual personality attributes.
    pub attributes: PlayerAttributes,
    /// Temporary sprint multiplier used when a player is running late to an
    /// event (set by the dispatcher, reset on arrival).
    pub sprint_boost: f32,
    /// Utility-AI tactical decision state.
    pub brain: TacticalBrain,
    /// Cached context-steering direction (obstacles move slowly).
    context_dir: Vector2,
    /// Seconds until the cached direction is refreshed.
    context_timer: f32,
    /// Event this player is currently scheduled to execute (if any).
    pub assigned_event_id: Option<u64>,
    /// Receiver routing: `(landing target, absolute sim time by which to
    /// arrive)`. Set by the dispatcher so an intended pass receiver is on the
    /// landing spot when the ball gets there.
    pub receiver_task: Option<(Vector2, f32)>,
    /// Position at the start of the current tick (used to cap per-tick
    /// displacement so nothing can ever teleport).
    pub prev_position: Vector2,

    pub state: AgentState,

    /// Heading mode latch: `true` while facing the ball, `false` while facing
    /// the direction of travel. A single boolean with different enter/exit
    /// thresholds gives hysteresis, which prevents the heading from
    /// flip-flopping (and the player from spinning) around a speed threshold.
    pub facing_ball: bool,
}

impl PlayerAgent {
    pub fn new(
        id: String,
        team_id: String,
        shirt_number: u8,
        start_pos: Vector2,
        is_goalkeeper: bool,
    ) -> Self {
        let attributes = PlayerAttributes::from_id(&id);
        // Pace spreads the top speed across ~7.9–8.5 m/s (still capped by
        // MAX_SPEED) so some players are visibly quicker than others.
        let max_speed_sprint = (7.9 + attributes.pace * 0.6).min(MAX_SPEED);
        Self {
            id,
            team_id,
            shirt_number,
            position: start_pos,
            velocity: Vector2::new(0.0, 0.0),
            acceleration: Vector2::new(0.0, 0.0),
            heading: 0.0,
            max_speed_jog: DEFAULT_JOG_SPEED,
            max_speed_sprint,
            max_accel: DEFAULT_MAX_ACCEL,
            is_goalkeeper,
            role: if is_goalkeeper {
                PlayerRole::Goalkeeper
            } else {
                PlayerRole::Unknown
            },
            attributes,
            sprint_boost: 1.0,
            brain: TacticalBrain::new(),
            context_dir: Vector2::zero(),
            context_timer: 0.0,
            assigned_event_id: None,
            receiver_task: None,
            prev_position: start_pos,
            state: AgentState::InFormation,
            facing_ball: false,
        }
    }

    /// Current speed (m/s).
    #[inline]
    pub fn speed(&self) -> f32 {
        self.velocity.length()
    }

    /// Distance from this agent to a point.
    #[inline]
    pub fn distance_to(&self, other: Vector2) -> f32 {
        self.position.distance(other)
    }

    /// The anticipated event target, if any.
    #[inline]
    pub fn target(&self) -> Option<Vector2> {
        match self.state {
            AgentState::AnticipatingEvent { target } => Some(target),
            _ => None,
        }
    }

    /// Whether the agent is currently bound to an event/action.
    #[inline]
    pub fn is_engaged(&self) -> bool {
        matches!(
            self.state,
            AgentState::AnticipatingEvent { .. } | AgentState::ExecutingAction { .. }
        )
    }

    /// Command the agent to sprint toward an anticipated event.
    #[inline]
    pub fn assign_anticipation(&mut self, target: Vector2) {
        self.state = AgentState::AnticipatingEvent { target };
    }

    /// Command the agent to begin executing the action for `secs` seconds.
    #[inline]
    pub fn begin_action(&mut self, secs: f32) {
        self.state = AgentState::ExecutingAction {
            remaining_secs: secs,
        };
    }

    // ── Steering forces ────────────────────────────────────────

    /// The arrival *desired velocity*: full speed toward the target, bounded
    /// by the largest speed from which the agent can still stop within the
    /// remaining distance (`sqrt(2·a·d)`), so it never overruns.
    #[inline]
    pub fn desired_velocity(&self, target: Vector2, max_speed: f32) -> Vector2 {
        let to_target = target - self.position;
        let dist = to_target.length();
        if dist < BRAKE_RADIUS {
            return Vector2::zero();
        }
        to_target.normalize() * braking_speed(dist, max_speed, self.max_accel)
    }

    /// Reynolds *arrival* steering: decelerates along the physically correct
    /// braking curve instead of overshooting and orbiting the target.
    #[inline]
    pub fn arrive(&self, target: Vector2, max_speed: f32) -> Vector2 {
        let to_target = target - self.position;
        if to_target.length() < BRAKE_RADIUS {
            // At the target: kill residual velocity.
            return self.velocity * -1.0;
        }
        self.desired_velocity(target, max_speed) - self.velocity
    }

    /// Reynolds separation, averaged over neighbours inside `sep_radius`.
    ///
    /// `all_positions` is a slice of `(position, id)` for every player on the
    /// pitch; the agent excludes itself by id.
    pub fn calculate_separation(
        &self,
        all_positions: &[(Vector2, &str)],
        sep_radius: f32,
    ) -> Vector2 {
        let mut steer = Vector2::new(0.0, 0.0);
        let mut count: u32 = 0;

        for (j, (other_pos, other_id)) in all_positions.iter().enumerate() {
            if *other_id == self.id {
                continue;
            }
            let diff = self.position - *other_pos;
            let dist = diff.length();
            if dist < sep_radius {
                if dist > 0.0001 {
                    // Stronger repulsion the closer the neighbour is.
                    steer = steer + diff.normalize() / dist;
                } else {
                    // Exactly coincident: a deterministic tie-break so they
                    // can never lock together. Large magnitude so the steering
                    // layer treats it as an emergency.
                    let a = (j as f32) * 2.399_963_2;
                    steer = steer + Vector2::new(a.cos(), a.sin()) * 10.0;
                }
                count += 1;
            }
        }

        if count > 0 {
            (steer / count as f32).truncate(self.max_accel)
        } else {
            steer
        }
    }

    /// Allocation-free separation using a positional snapshot and index
    /// (used by the batch [`simulate_step`] driver).
    #[inline]
    pub fn separation_by_index(
        &self,
        positions: &[Vector2],
        self_idx: usize,
        sep_radius: f32,
    ) -> Vector2 {
        let mut steer = Vector2::new(0.0, 0.0);
        let mut count: u32 = 0;

        for (j, other) in positions.iter().enumerate() {
            if j == self_idx {
                continue;
            }
            let diff = self.position - *other;
            let dist = diff.length();
            if dist < sep_radius {
                if dist > 0.0001 {
                    steer = steer + diff.normalize() / dist;
                } else {
                    let a = (j as f32) * 2.399_963_2;
                    steer = steer + Vector2::new(a.cos(), a.sin()) * 10.0;
                }
                count += 1;
            }
        }

        if count > 0 {
            (steer / count as f32).truncate(self.max_accel)
        } else {
            steer
        }
    }

    // ── Integration ────────────────────────────────────────────

    /// Advance the agent one timestep using an explicit neighbour list.
    pub fn update_kinematics(
        &mut self,
        dt: f32,
        anchor_pos: Vector2,
        all_positions: &[(Vector2, &str)],
        ball_pos: Vector2,
    ) {
        let sep = self.calculate_separation(all_positions, SEPARATION_RADIUS);
        self.apply_steering(dt, anchor_pos, sep, ball_pos);
    }

    /// Apply the blended steering forces, integrate, clamp, and orient.
    ///
    /// Separated out so the batch driver can reuse it with an index-based
    /// separation force (avoids allocating a neighbour list per agent).
    pub fn apply_steering(
        &mut self,
        dt: f32,
        anchor_pos: Vector2,
        separation: Vector2,
        ball_pos: Vector2,
    ) {
        // 1. Pick the active target + speed limit from the state machine and
        //    advance any timers.
        let (target, max_speed) = match self.state {
            AgentState::InFormation => (anchor_pos, self.max_speed_jog),
            AgentState::AnticipatingEvent { target } => {
                (target, self.max_speed_sprint * self.sprint_boost)
            }
            AgentState::ExecutingAction { remaining_secs } => {
                let next = remaining_secs - dt;
                if next <= 0.0 {
                    self.state = AgentState::Recovering {
                        remaining_secs: RECOVER_SECS,
                    };
                } else {
                    self.state = AgentState::ExecutingAction {
                        remaining_secs: next,
                    };
                }
                // Target = current position with a 0 speed limit: the agent
                // decelerates to a stop *at the acceleration cap* (no snap).
                self.sprint_boost = 1.0;
                (self.position, 0.0)
            }
            AgentState::Recovering { remaining_secs } => {
                let next = remaining_secs - dt;
                if next <= 0.0 {
                    self.state = AgentState::InFormation;
                } else {
                    self.state = AgentState::Recovering {
                        remaining_secs: next,
                    };
                }
                (anchor_pos, self.max_speed_jog * 0.7)
            }
        };

        // 2. Blend the arrival desired-velocity with the separation force.
        let arrive_vel = self.desired_velocity(target, max_speed);
        let target_vel = (arrive_vel + separation * SEPARATION_WEIGHT).truncate(max_speed);

        // 3. Accelerate toward the target velocity at the accel cap (bang-bang
        //    with a snap that lands exactly on the target without overshoot).
        let dv = target_vel - self.velocity;
        let accel = if dv.length() > self.max_accel * dt {
            dv.normalize() * self.max_accel
        } else {
            dv / dt.max(1e-6)
        };
        self.acceleration = accel;
        // Hard physical ceiling only. The state speed limit is enforced by the
        // desired-velocity profile, so switching to a slower state decelerates
        // at the cap rather than snapping to the new limit.
        self.velocity = (self.velocity + self.acceleration * dt).truncate(self.max_speed_sprint);

        // 4. Integrate position. No separate clamp is needed: the target
        //    velocity already follows the braking curve, and the single
        //    cap-limited acceleration above is what tracks it, so the agent
        //    stays on-curve (and cannot overshoot) without exceeding the cap.
        self.position = (self.position + self.velocity * dt).clamp_on_pitch();

        // 5. Orientation. Face the direction of travel while moving, and the
        //    ball while (near-)stationary. The `facing_ball` latch uses
        //    different enter/exit speeds (hysteresis) so the target never
        //    flip-flops, which used to make players spin on the spot.
        let speed = self.velocity.length();
        if self.facing_ball {
            if speed > FACE_MOTION_SPEED {
                self.facing_ball = false;
            }
        } else if speed < FACE_BALL_SPEED {
            self.facing_ball = true;
        }
        let target_heading = if self.facing_ball {
            let to_ball = ball_pos - self.position;
            if to_ball.length_sq() > 1e-4 {
                to_ball.y.atan2(to_ball.x)
            } else {
                self.heading
            }
        } else {
            self.velocity.y.atan2(self.velocity.x)
        };
        let angle_diff =
            (target_heading - self.heading + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI;
        self.heading += angle_diff.clamp(-TURN_SPEED * dt, TURN_SPEED * dt);
    }

    /// Utility-AI locomotion: context-steer toward `target` at `target_speed`
    /// with a burst-and-hold cadence, braking hard while executing an action.
    #[allow(clippy::too_many_arguments)]
    pub fn steer_toward(
        &mut self,
        dt: f32,
        target: Vector2,
        target_speed: f32,
        positions: &[Vector2],
        is_home: &[bool],
        self_idx: usize,
        ball_pos: Vector2,
        direct: bool,
    ) {
        // Advance the action / recovery timers.
        match self.state {
            AgentState::ExecutingAction { remaining_secs } => {
                if direct {
                    // An urgent scheduled event wins over the tail of the
                    // previous action: drop it and sprint to the event spot so
                    // the actor is on the ball when the action fires.
                    self.state = AgentState::InFormation;
                } else {
                    let next = remaining_secs - dt;
                    self.state = if next <= 0.0 {
                        AgentState::Recovering { remaining_secs: RECOVER_SECS }
                    } else {
                        AgentState::ExecutingAction { remaining_secs: next }
                    };
                    self.brake(dt);
                    return;
                }
            }
            AgentState::Recovering { remaining_secs } => {
                let next = remaining_secs - dt;
                self.state = if next <= 0.0 {
                    AgentState::InFormation
                } else {
                    AgentState::Recovering { remaining_secs: next }
                };
            }
            _ => {}
        }

        let to_target = target - self.position;
        let dist = to_target.length();
        let target_dir = if dist > 1e-3 {
            to_target / dist
        } else {
            Vector2::new(self.heading.cos(), self.heading.sin())
        };
        // Refresh the avoidance direction ~10x/s (obstacles barely move in
        // between); hard event runs go straight.
        let dir = if direct {
            self.context_dir = target_dir;
            target_dir
        } else {
            self.context_timer -= dt;
            if self.context_timer <= 0.0 || self.context_dir.length_sq() < 1e-6 {
                self.context_dir = self.context_direction(target_dir, positions, is_home, self_idx);
                self.context_timer = 0.1;
            }
            self.context_dir
        };
        let speed = braking_speed(dist, target_speed, self.max_accel);
        // Real players do not run through each other. Apply separation even on
        // event runs (halved so the actor still reaches the spot) and let a
        // planted player shuffle aside rather than be frozen in an overlap.
        let sep = self
            .separation_by_index(positions, self_idx, SEPARATION_RADIUS)
            * if direct { 0.6 } else { SEPARATION_WEIGHT };
        let mut desired = dir * speed;
        let sep_mag = sep.length();
        if sep_mag > 2.5 {
            // Emergency: a neighbour is almost on top of us. Separation
            // outranks the target so bodies can never stack, while staying
            // inside the normal per-tick speed budget (no teleporting).
            let mut sdir = sep.normalize();
            // Never push into a boundary: slide along it instead. Otherwise the
            // pitch clamp cancels the push and two players pin together on the
            // touchline.
            if (self.position.y + sdir.y).abs() > 33.4 && sdir.y.abs() > 0.3 {
                sdir.y = 0.0;
            }
            if (self.position.x + sdir.x).abs() > 51.8 && sdir.x.abs() > 0.3 {
                sdir.x = 0.0;
            }
            if sdir.length_sq() < 1e-6 {
                sdir = Vector2::new(-sep.y, sep.x).normalize();
            }
            desired = (sdir * self.max_speed_sprint * 0.8 + dir * speed * 0.2)
                .truncate(self.max_speed_sprint);
        } else if sep_mag > 1e-8 {
            desired = (desired + sep).truncate(target_speed.max(2.2));
        }

        let dv = desired - self.velocity;
        let accel = if dv.length() > self.max_accel * dt {
            dv.normalize() * self.max_accel
        } else {
            dv / dt.max(1e-6)
        };
        self.acceleration = accel;
        self.velocity = (self.velocity + accel * dt).truncate(self.max_speed_sprint);
        self.position = (self.position + self.velocity * dt).clamp_on_pitch();

        // Heading: face travel while moving, the ball while (near-)still.
        let sp = self.velocity.length();
        if self.facing_ball {
            if sp > FACE_MOTION_SPEED {
                self.facing_ball = false;
            }
        } else if sp < FACE_BALL_SPEED {
            self.facing_ball = true;
        }
        let desired_heading = if self.facing_ball {
            let to_ball = ball_pos - self.position;
            if to_ball.length_sq() > 1e-4 {
                to_ball.y.atan2(to_ball.x)
            } else {
                self.heading
            }
        } else {
            self.velocity.y.atan2(self.velocity.x)
        };
        let d = (desired_heading - self.heading + std::f32::consts::PI)
            .rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        self.heading += d.clamp(-TURN_SPEED * dt, TURN_SPEED * dt);
    }

    /// Brake to a stop at the acceleration cap.
    fn brake(&mut self, dt: f32) {
        let speed = self.velocity.length();
        if speed > 1e-4 {
            let decel = (self.max_accel * dt).min(speed);
            self.velocity = self.velocity - self.velocity.normalize() * decel;
        } else {
            self.velocity = Vector2::zero();
        }
        self.position = (self.position + self.velocity * dt).clamp_on_pitch();
        self.acceleration = Vector2::zero();
    }

    /// Sample `RAY_COUNT` directions and pick the one that best balances
    /// "toward the target" (interest) against "into bodies" (danger).
    pub fn context_direction(
        &self,
        target_dir: Vector2,
        positions: &[Vector2],
        is_home: &[bool],
        self_idx: usize,
    ) -> Vector2 {
        if target_dir.length_sq() < 1e-6 {
            return Vector2::zero();
        }
        let my_home = is_home[self_idx];
        let mut best = target_dir;
        let mut best_score = f32::NEG_INFINITY;
        for r in 0..RAY_COUNT {
            let angle = (r as f32 / RAY_COUNT as f32) * std::f32::consts::TAU;
            let dir = Vector2::new(angle.cos(), angle.sin());
            let interest = dir.dot(target_dir);
            let mut danger = 0.0;
            for (j, p) in positions.iter().enumerate() {
                if j == self_idx {
                    continue;
                }
                let to_p = *p - self.position;
                // Early-out on squared distance so distant bodies never pay
                // for a sqrt: only players inside the 6 m bubble matter.
                let d_sq = to_p.length_sq();
                if d_sq < 1e-6 || d_sq > 36.0 {
                    continue;
                }
                let d = d_sq.sqrt();
                let align = dir.dot(to_p / d);
                if align > 0.8 {
                    // Team-mates are avoided gently; opponents much harder.
                    let w = if is_home[j] == my_home { 1.1 } else { 2.0 };
                    danger += (1.0 - d / 6.0) * w;
                }
            }
            let score = interest * 1.6 - danger;
            if score > best_score {
                best_score = score;
                best = dir;
            }
        }
        best
    }
}


// ── Arrival profile ────────────────────────────────────────────

/// Largest speed from which the agent can still stop within `dist` under the
/// acceleration cap: `min(max_speed, sqrt(2·max_accel·dist))`.
///
/// This is the time-optimal braking curve. Its slope is exactly
/// `-max_accel`, so following it both respects the cap and never overshoots.
#[inline]
pub fn braking_speed(dist: f32, max_speed: f32, max_accel: f32) -> f32 {
    if dist <= 0.0 || max_speed <= 0.0 || max_accel <= 0.0 {
        0.0
    } else {
        max_speed.min((2.0 * max_accel * dist).sqrt())
    }
}

// ── Batch driver ───────────────────────────────────────────────

/// Advance every agent one tick using an explicit snapshot for neighbour
/// positions (Jacobi-style update → deterministic, order-independent).
///
/// `positions_snapshot` is a reusable scratch buffer: pass the same `Vec`
/// every tick to avoid per-tick allocation.
pub fn simulate_step(
    agents: &mut [PlayerAgent],
    anchors: &HashMap<String, Vector2>,
    ball_pos: Vector2,
    dt: f32,
    positions_snapshot: &mut Vec<Vector2>,
) {
    positions_snapshot.clear();
    positions_snapshot.extend(agents.iter().map(|a| a.position));

    for (i, agent) in agents.iter_mut().enumerate() {
        let anchor = anchors.get(&agent.id).copied().unwrap_or(Vector2::zero());
        let sep = agent.separation_by_index(positions_snapshot, i, SEPARATION_RADIUS);
        agent.apply_steering(dt, anchor, sep, ball_pos);
    }
}

/// Build agents for one team from its formation anchors.
///
/// `goalkeeper` should be true for the anchor belonging to the keeper.
pub fn init_agents(
    team_id: &str,
    anchors: &[TacticalAnchor],
    goalkeeper_id: Option<&str>,
) -> Vec<PlayerAgent> {
    anchors
        .iter()
        .map(|a| {
            let is_gk = goalkeeper_id
                .map(|gk| gk == a.player_id)
                .unwrap_or(a.role_weight > 1.0);
            PlayerAgent::new(
                a.player_id.clone(),
                team_id.to_string(),
                a.shirt_number,
                a.target_pos,
                is_gk,
            )
        })
        .collect()
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod agent_tests {
    use super::*;
    use std::f32::consts::PI;

    fn agent_at(id: &str, x: f32, y: f32) -> PlayerAgent {
        PlayerAgent::new(id.into(), "team1".into(), 9, Vector2::new(x, y), false)
    }

    // ── Spec test 1: smooth braking, no oscillation ────────────

    #[test]
    fn test_agent_kinematics_and_braking() {
        let target = Vector2::new(10.0, 0.0);
        let mut agent = agent_at("p1", 0.0, 0.0);
        agent.state = AgentState::AnticipatingEvent { target };

        let dt = DT;
        let ball_pos = Vector2::new(12.0, 0.0);

        // NOTE: the brief's example used 120 ticks (2 s), but with
        // v_max=8.5 m/s and a_max=6 m/s² the stop-to-stop minimum over 10 m is
        // 2·√(D/a) = 2.58 s, so 2 s is physically impossible. 3 s gives the
        // required margin while still asserting smooth, oscillaton-free arrival.
        for _ in 0..180 {
            let other_positions: Vec<(Vector2, &str)> = vec![];
            agent.update_kinematics(dt, target, &other_positions, ball_pos);
            assert!(
                agent.velocity.length() <= agent.max_speed_sprint + 0.001,
                "velocity {} exceeded sprint limit",
                agent.velocity.length()
            );
        }

        assert!(
            agent.position.distance(target) < 0.2,
            "expected to arrive, dist={}",
            agent.position.distance(target)
        );
        assert!(
            agent.velocity.length() < 0.2,
            "expected to brake, speed={}",
            agent.velocity.length()
        );
    }

    #[test]
    fn test_physical_stop_time_bound() {
        // Documents *why* a 2 s window cannot satisfy the arrival assertion:
        // the fastest way to cover 10 m and stop under a 6 m/s² cap is a
        // triangular profile taking 2·sqrt(D/a) ≈ 2.58 s.
        let d = 10.0f32;
        let a = DEFAULT_MAX_ACCEL;
        let t_min = 2.0 * (d / a).sqrt();
        assert!((t_min - 2.582).abs() < 0.01, "t_min={t_min}");
        assert!(t_min > 2.0, "2 s window is below the physical minimum");
    }

    // ── Spec test 2: separation repulsion direction ────────────

    #[test]
    fn test_agent_separation_repulsion() {
        let agent1 = agent_at("p1", 0.0, 0.0);
        let agent2_pos = Vector2::new(0.5, 0.0);
        let others = vec![(agent2_pos, "p2")];
        let sep = agent1.calculate_separation(&others, 2.5);
        assert!(sep.x < 0.0, "repulsion must push along -x, got {sep:?}");
    }

    // ── Extended coverage ──────────────────────────────────────

    #[test]
    fn test_no_arrival_overshoot() {
        // The classic "satellite orbit" failure: pure seek overshoots.
        let target = Vector2::new(8.0, 0.0);
        let mut agent = agent_at("p", 0.0, 0.0);
        agent.state = AgentState::AnticipatingEvent { target };
        let mut max_x = f32::MIN;
        for _ in 0..600 {
            agent.update_kinematics(DT, target, &[], Vector2::zero());
            max_x = max_x.max(agent.position.x);
        }
        assert!(
            max_x <= target.x + 0.2,
            "agent overshot the target: max_x={max_x}"
        );
        assert!(agent.position.distance(target) < 0.2);
    }

    #[test]
    fn test_jog_speed_slower_than_sprint() {
        let mut a = agent_at("p", -40.0, 0.0);
        a.state = AgentState::InFormation;
        let mut max_jog = 0.0f32;
        for _ in 0..600 {
            a.apply_steering(DT, Vector2::new(40.0, 0.0), Vector2::zero(), Vector2::zero());
            max_jog = max_jog.max(a.speed());
        }
        assert!(
            max_jog <= a.max_speed_jog + 1e-3,
            "jogging exceeded jog limit: {max_jog}"
        );
        assert!(max_jog < a.max_speed_sprint);
    }

    #[test]
    fn test_acceleration_is_limited() {
        let mut a = agent_at("p", 50.0, 30.0);
        a.state = AgentState::AnticipatingEvent {
            target: Vector2::new(-50.0, -30.0),
        };
        for _ in 0..1200 {
            a.update_kinematics(DT, Vector2::zero(), &[], Vector2::zero());
            assert!(
                a.acceleration.length() <= a.max_accel + 1e-3,
                "accel {} exceeded cap",
                a.acceleration.length()
            );
        }
    }

    #[test]
    fn test_executing_then_recovering_then_formation() {
        let mut a = agent_at("p", 0.0, 0.0);
        a.begin_action(0.2);
        let mut saw_recovering = false;
        let mut saw_formation = false;
        for _ in 0..120 {
            a.update_kinematics(DT, Vector2::new(0.0, 0.0), &[], Vector2::zero());
            match a.state {
                AgentState::Recovering { .. } => saw_recovering = true,
                AgentState::InFormation => saw_formation = true,
                _ => {}
            }
        }
        assert!(saw_recovering, "never entered Recovering");
        assert!(saw_formation, "never returned to InFormation");
    }

    #[test]
    fn test_position_stays_on_pitch() {
        let mut a = agent_at("p", 45.0, 30.0);
        a.state = AgentState::AnticipatingEvent {
            target: Vector2::new(100.0, 100.0), // deliberately off-pitch
        };
        for _ in 0..600 {
            a.update_kinematics(DT, Vector2::zero(), &[], Vector2::zero());
            assert!(a.position.x <= 52.5 + 1e-3);
            assert!(a.position.y <= 34.0 + 1e-3);
            assert!(a.position.x >= -52.5 - 1e-3);
            assert!(a.position.y >= -34.0 - 1e-3);
        }
    }

    #[test]
    fn test_heading_faces_ball_when_still() {
        let mut a = agent_at("p", 0.0, 0.0);
        let ball = Vector2::new(0.0, 10.0); // straight "up"
        for _ in 0..120 {
            a.update_kinematics(DT, Vector2::new(0.0, 0.0), &[], ball);
        }
        assert!(
            (a.heading - PI / 2.0).abs() < 0.2,
            "heading {} should approach +pi/2",
            a.heading
        );
    }

    #[test]
    fn test_separation_falls_off_with_distance() {
        let a = agent_at("p1", 0.0, 0.0);
        let near = a.calculate_separation(&[(Vector2::new(0.4, 0.0), "b")], 2.5);
        let far = a.calculate_separation(&[(Vector2::new(2.2, 0.0), "b")], 2.5);
        assert!(near.length() > far.length());
        // Outside the radius there is no force at all.
        let outside = a.calculate_separation(&[(Vector2::new(3.0, 0.0), "b")], 2.5);
        assert_eq!(outside, Vector2::zero());
    }

    #[test]
    fn test_batch_driver_matches_single_agent_update() {
        // simulate_step must agree with per-agent update_kinematics when the
        // neighbour set is identical.
        let mk = || {
            vec![
                agent_at("a", -5.0, 0.0),
                agent_at("b", 5.0, 0.0),
                agent_at("c", 0.0, 5.0),
            ]
        };
        let mut batch = mk();
        let mut single = mk();
        let ids: Vec<String> = single.iter().map(|a| a.id.clone()).collect();
        let anchors: HashMap<String, Vector2> = batch
            .iter()
            .map(|a| (a.id.clone(), Vector2::new(0.0, 0.0)))
            .collect();
        let ball = Vector2::new(1.0, 1.0);
        let mut snap = Vec::new();

        for _ in 0..120 {
            simulate_step(&mut batch, &anchors, ball, DT, &mut snap);

            // Neighbour list borrows only `ids`, not `single`, so the mutable
            // pass below is allowed.
            let positions: Vec<(Vector2, &str)> = single
                .iter()
                .enumerate()
                .map(|(i, a)| (a.position, ids[i].as_str()))
                .collect();
            for a in single.iter_mut() {
                a.update_kinematics(DT, Vector2::new(0.0, 0.0), &positions, ball);
            }
        }

        for (x, y) in batch.iter().zip(single.iter()) {
            assert!(
                (x.position.x - y.position.x).abs() < 1e-4
                    && (x.position.y - y.position.y).abs() < 1e-4,
                "batch/single diverged for {}: {:?} vs {:?}",
                x.id,
                x.position,
                y.position
            );
        }
    }

    #[test]
    fn test_agents_do_not_overlap() {
        // Two agents commanded to the same point must not occupy it together.
        let mut agents = vec![agent_at("a", -1.0, 0.0), agent_at("b", 1.0, 0.0)];
        for a in &mut agents {
            a.assign_anticipation(Vector2::new(0.0, 0.0));
        }
        let anchors: HashMap<String, Vector2> = HashMap::new();
        let mut snap = Vec::new();
        for _ in 0..600 {
            simulate_step(&mut agents, &anchors, Vector2::zero(), DT, &mut snap);
        }
        let d = agents[0].position.distance(agents[1].position);
        assert!(d > 0.5, "agents clumped together: {d} m apart");
    }

    #[test]
    fn test_init_agents_flags_goalkeeper() {
        let anchors = vec![
            TacticalAnchor {
                player_id: "gk".into(),
                shirt_number: 1,
                target_pos: Vector2::new(-47.0, 0.0),
                role_weight: 1.5,
            },
            TacticalAnchor {
                player_id: "d".into(),
                shirt_number: 3,
                target_pos: Vector2::new(-36.0, 0.0),
                role_weight: 1.0,
            },
        ];
        let agents = init_agents("team1", &anchors, None);
        assert!(agents[0].is_goalkeeper);
        assert!(!agents[1].is_goalkeeper);
        assert!(agents.iter().all(|a| a.team_id == "team1"));
    }

    #[test]
    fn test_heading_wraps_short_way() {
        let mut a = agent_at("p", 0.0, 0.0);
        a.heading = 3.0;
        // Target just across the -pi/+pi seam.
        let target = Vector2::new(-1.0, -0.05);
        a.assign_anticipation(target);
        a.apply_steering(DT, target, Vector2::zero(), Vector2::zero());
        assert!((a.heading - 3.0).abs() < 1.0, "turn took the long way");
    }
}
