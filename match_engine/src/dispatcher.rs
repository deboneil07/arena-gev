//! The Lookahead Dispatcher — bridges sparse Opta events and continuous
//! agent physics.
//!
//! Each tick it peeks ahead into the timeline and pre-routes players so they
//! *sprint* early enough to arrive at the event coordinate exactly when the
//! event fires. When an event's timestamp is reached it emits an execution
//! signal (→ [`AgentState::ExecutingAction`]) and a [`DispatchedAction`].
//!
//! ```text
//!  t_sim ─────────────────────────────────────────► t_event
//!         │                                          │
//!         │            lookahead window              │
//!         ▼                                          ▼
//!   d = ‖pos − origin‖                        state → ExecutingAction
//!   t_need = travel_time(d, v, a) + buffer     emit DispatchedAction
//!   if t_event − t_sim ≤ t_need: anticipate    ball begins flight
//! ```
//!
//! ## Travel-time estimate
//!
//! The brief's formula `d / v_sprint + buffer` ignores acceleration and
//! therefore dispatches **too late**: a 20 m sprint needs ~3.06 s of pure
//! kinematics at `a = 6 m/s²`, not `20/8.5 = 2.35 s`. We use the exact
//! point-mass time (triangular below the accel-limited distance, trapezoidal
//! above it), which is the correct "dynamic travel calculation".

use crate::ball_spot_expected;
use crate::agent::{AgentState, PlayerAgent, PlayerRole};
use crate::formation::Vector2;
use crate::parser::{NormalizedEvent, EVENT_TYPE_PLAYER_OFF, EVENT_TYPE_PLAYER_ON};
use std::collections::HashMap;

/// Extra settle time added to the kinematic travel time so a player arrives
/// a beat early and can plant for the action (spec: 0.3–0.5 s).
pub const PRIMARY_BUFFER_SECS: f32 = 0.5;
/// How far ahead (seconds) a player is told about their next event so they can
/// drift into position before they have to sprint.
pub const PRE_POSITION_SECS: f32 = 5.0;
/// How close the doer must be to his event origin before the event may fire.
pub const DOER_READY_TOL: f32 = 1.5;
/// Tighter tolerance for a player who just received a pass and is also the
/// next event's doer — he must finish at his origin, not drift past it.
pub const DOER_READY_TOL_RECEIVER: f32 = 1.0;
/// How close the ball must be to the origin of an event that plays it.
pub const BALL_READY_TOL: f32 = 1.0;
/// The most a due event may be held back waiting for its actors. Beyond it
/// the event fires anyway — a wrong-position beat beats a stalled timeline.
pub const SLIP_MAX_SECS: f32 = 2.0;

/// Settle buffer for the intended receiver of a pass.
pub const RECEIVER_BUFFER_SECS: f32 = 0.30;
/// How far a receiver's own next-event origin may sit from the pass landing
/// point before he is routed to the landing point instead of his origin.
pub const RECEIVER_ORIGIN_TOL: f32 = 1.5;
/// Fallback lookahead horizon when a caller passes a non-positive window.
pub const DEFAULT_LOOKAHEAD_SECS: f32 = 3.5;

/// Largest possible travel time on the pitch — the scan is always at least
/// this long so a distant actor can be dispatched early enough to arrive.
fn max_lead_secs() -> f32 {
    let diag = (crate::parser::PITCH_LENGTH.powi(2) + crate::parser::PITCH_WIDTH.powi(2)).sqrt();
    arrival_time(diag, 8.5, 6.0) + 0.5
}

/// A fully-resolved event emitted the moment its timestamp is reached.
#[derive(Debug, Clone, PartialEq)]
pub struct DispatchedAction {
    pub event_id: u64,
    pub type_id: u16,
    pub team_id: String,
    pub player_id: String,
    pub origin: Vector2,
    /// The Opta timestamp of the event (mm:ss of the feed), which can lead
    /// the simulation clock when dispatch had to wait for an actor/ball to
    /// get into place. The UI renders *this*, not `sim_time`.
    pub feed_time: f32,
    /// The doer's tactical state *before* `begin_action` put him into the
    /// action pose — diagnostic for accuracy measurements.
    pub doer_state: Option<&'static str>,
    /// Where the doer was *heading* when the event fired (`None` unless he
    /// was in `AnticipatingEvent`) — diagnostic for accuracy measurements.
    pub doer_stage_target: Option<Vector2>,
    pub target: Option<Vector2>,
    /// Intended receiver of this pass (if one could be identified).
    pub receiver_id: Option<String>,
    pub outcome: bool,
    pub is_aerial: bool,
    /// True when this Goal was scored in the doer's own net (qualifier 28).
    /// The engine credits the opponent and flies the ball into the scorer's
    /// own goal.
    pub is_own_goal: bool,
}

/// Exact time for a point mass at rest to travel `dist` metres given a top
/// speed `max_speed` and acceleration `max_accel`.
///
/// * Triangular profile when `dist ≤ v²/2a`:  `t = 2·√(dist/a)`
/// * Trapezoidal profile otherwise:           `t = v/a + (dist − v²/2a)/v`
#[inline]
pub fn travel_time(dist: f32, max_speed: f32, max_accel: f32) -> f32 {
    if dist <= 0.0 || max_speed <= 0.0 || max_accel <= 0.0 {
        return 0.0;
    }
    let d_accel = max_speed * max_speed / (2.0 * max_accel);
    if dist <= d_accel {
        2.0 * (dist / max_accel).sqrt()
    } else {
        max_speed / max_accel + (dist - d_accel) / max_speed
    }
}

/// Time for a point mass to travel `dist` **and come to rest** (accelerate,
/// cruise, decelerate) at the acceleration cap. This is the conservative
/// budget the dispatcher uses so an actor is guaranteed to be standing on the
/// event spot the moment it fires.
#[inline]
pub fn arrival_time(dist: f32, max_speed: f32, max_accel: f32) -> f32 {
    if dist <= 0.0 || max_speed <= 0.0 || max_accel <= 0.0 {
        return 0.0;
    }
    let d_accel = max_speed * max_speed / (2.0 * max_accel);
    if dist <= 2.0 * d_accel {
        2.0 * (dist / max_accel).sqrt()
    } else {
        2.0 * max_speed / max_accel + (dist - 2.0 * d_accel) / max_speed
    }
}

/// How long a player "holds" an action before recovery, by event type.
#[inline]
pub fn action_duration(type_id: u16) -> f32 {
    match type_id {
        1 => 0.4,                 // Pass release
        3 => 0.6,                 // Take-on / dribble
        7 => 0.5,                 // Tackle
        13..=16 => 0.7,           // Shot
        _ => 0.3,
    }
}

// ── Dispatcher ─────────────────────────────────────────────────

/// Time-indexed lookahead dispatcher.
pub struct LookaheadDispatcher {
    events: Vec<NormalizedEvent>,
    cursor: usize,
    pub sim_time: f32,
    /// How far ahead (seconds) events are inspected.
    pub lookahead_window: f32,
    /// Number of anticipation assignments made.
    pub dispatched: u64,
    /// Number of actions executed.
    pub executed: u64,
    /// Pass event id → intended receiver player id (routed during lookahead).
    receiver_of: HashMap<u64, String>,
    /// Receiver of the most recently dispatched pass. That player is also the
    /// next event's doer, so his restart is held to a tighter tolerance: he
    /// must finish at his own origin, not merely pass within 1.5 m of it on
    /// the way to the landing point.
    last_pass_receiver: Option<String>,
}

impl LookaheadDispatcher {
    /// Build a dispatcher over `events` (sorted internally by timestamp).
    pub fn new(mut events: Vec<NormalizedEvent>, lookahead_window: f32) -> Self {
        events.sort_by(|a, b| {
            a.match_time_secs
                .partial_cmp(&b.match_time_secs)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Self {
            events,
            cursor: 0,
            sim_time: 0.0,
            lookahead_window: if lookahead_window > 0.0 {
                lookahead_window
            } else {
                DEFAULT_LOOKAHEAD_SECS
            },
            dispatched: 0,
            executed: 0,
            receiver_of: HashMap::new(),
            last_pass_receiver: None,
        }
    }

    /// All events (chronological).
    #[inline]
    pub fn events(&self) -> &[NormalizedEvent] {
        &self.events
    }

    /// Next event not yet executed.
    #[inline]
    pub fn peek_next(&self) -> Option<&NormalizedEvent> {
        self.events.get(self.cursor)
    }

    /// Index of the next event to be executed.
    #[inline]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// True once every event has been executed.
    #[inline]
    pub fn is_finished(&self) -> bool {
        self.cursor >= self.events.len()
    }

    /// Assign an anticipation target to a player if idle.
    /// Returns `true` when the player actually transitioned into anticipation.
    #[inline]
    fn anticipate(
        &self,
        idx: usize,
        agents: &mut [PlayerAgent],
        target: Vector2,
        boost: f32,
        force: bool,
        event_id: u64,
    ) -> bool {
        match agents[idx].state {
            AgentState::InFormation | AgentState::Recovering { .. } => {
                agents[idx].state = AgentState::AnticipatingEvent { target };
                agents[idx].sprint_boost = boost;
                agents[idx].assigned_event_id = Some(event_id);
                // The receiver task is NOT cleared here: the brain's own-vs-recv
                // arbitration decides which target is aimed for, so a receiver
                // whose own event is also due still runs to the landing point
                // when the ball arrives first.
                true
            }
            // A primary dispatch (the agent's own event) always wins over an
            // earlier speculative receiver target, so the true event origin is
            // what the actor aims for.
            AgentState::AnticipatingEvent { .. } if force => {
                let newly_assigned = agents[idx].assigned_event_id != Some(event_id);
                agents[idx].state = AgentState::AnticipatingEvent { target };
                agents[idx].sprint_boost = boost;
                agents[idx].assigned_event_id = Some(event_id);
                newly_assigned
            }
            // An urgent primary event interrupts the tail of the previous
            // action so the actor can still reach the spot in time.
            AgentState::ExecutingAction { .. } if force => {
                agents[idx].state = AgentState::AnticipatingEvent { target };
                agents[idx].sprint_boost = boost;
                agents[idx].assigned_event_id = Some(event_id);
                true
            }
            _ => false,
        }
    }

    /// Choose the intended receiver of a pass with a known landing point.
    ///
    /// Priority: the next same-team event's player (the Opta reception), then
    /// the best-placed teammate by role-weighted proximity to the target. This
    /// is what stops "ghost" passes into empty space.
    fn pick_receiver(&self, i: usize, target: Vector2, agents: &[PlayerAgent]) -> Option<String> {
        let event = &self.events[i];
        if i + 1 < self.events.len() {
            let next = &self.events[i + 1];
            if next.team_id == event.team_id
                && !next.player_id.is_empty()
                && next.player_id != event.player_id
                && next.match_time_secs - event.match_time_secs < 8.0
            {
                return Some(next.player_id.clone());
            }
        }
        let mut best: Option<(f32, &str)> = None;
        for a in agents.iter() {
            if a.team_id != event.team_id
                || a.id == event.player_id
                || a.role == PlayerRole::Goalkeeper
            {
                continue;
            }
            let bias = match a.role {
                PlayerRole::Forward => 0.0,
                PlayerRole::Midfielder => 8.0,
                PlayerRole::Defender => 20.0,
                _ => 12.0,
            };
            let cost = a.position.distance(target) + bias;
            if best.map(|(c, _)| cost < c).unwrap_or(true) {
                best = Some((cost, a.id.as_str()));
            }
        }
        best.map(|(_, id)| id.to_string())
    }

    /// Pre-route a receiver toward the pass landing point. Does not touch the
    /// dispatcher itself, so it can run while event borrows are live.
    fn route_receiver(
        &self,
        idx: usize,
        agents: &mut [PlayerAgent],
        target: Vector2,
        deadline: f32,
        boost: f32,
    ) {
        if matches!(agents[idx].state, AgentState::ExecutingAction { .. }) {
            return;
        }
        agents[idx].receiver_task = Some((target, deadline));
        agents[idx].sprint_boost = boost;
        agents[idx].state = AgentState::AnticipatingEvent { target };
    }

    /// Advance the clock by `dt`, pre-route players, and execute due events.
    pub fn tick(
        &mut self,
        dt: f32,
        agents: &mut [PlayerAgent],
        ball_pos: Option<Vector2>,
    ) -> Vec<DispatchedAction> {
        self.sim_time += dt;
        let mut executed_actions = Vec::new();

        // Scan at least the longest possible travel budget so a player can be
        // pre-routed early enough to reach the event on time (10/10 arrival).
        let scan_end = self.sim_time + self.lookahead_window.max(max_lead_secs());

        // ── 1. Lookahead pass: pre-route agents for upcoming events ──
        // Each player is told about the *soonest* event they must act on, far
        // enough ahead that they can position themselves calmly. Receivers are
        // routed to the landing point early as well.
        let mut soonest: Vec<Option<(f32, Vector2, u64)>> = vec![None; agents.len()];
        for i in self.cursor..self.events.len() {
            let event = &self.events[i];
            if event.match_time_secs > scan_end {
                break;
            }
            let origin = Vector2::new(event.origin_x, event.origin_y);

            // Primary actor: remember the soonest event only.
            if let Some(idx) = agents.iter().position(|a| a.id == event.player_id) {
                let sooner = soonest[idx]
                    .map(|(t, _, _)| event.match_time_secs < t)
                    .unwrap_or(true);
                if sooner {
                    soonest[idx] = Some((event.match_time_secs, origin, event.id));
                }
            }

            // Receiver routing: identify the intended receiver of a pass and
            // pre-route them to the landing point.
            if let (Some(tx), Some(ty)) = (event.target_x, event.target_y) {
                let target_pos = Vector2::new(tx, ty);
                let ev_id = event.id;
                let ev_time = event.match_time_secs;
                let flight = ((target_pos - origin).length() / 20.0).max(0.35);
                let deadline = ev_time + flight;
                if let Some(rid) = self.pick_receiver(i, target_pos, agents) {
                    if let Some(idx) = agents.iter().position(|a| a.id == rid) {
                        // A receiver who is also the *next* event's doer must
                        // end up at his own origin anyway. When that origin is
                        // within receiver tolerance of the landing point, route
                        // him there instead: he is at his event spot (doer
                        // accuracy) *and* within 2 m of the ball (receiver
                        // accuracy) — the feed's pass-end and the next touch
                        // are the same place for most passes.
                        let route_target = match self.events.get(i + 1) {
                            Some(next)
                                if next.player_id == rid
                                    && !next.player_id.is_empty()
                                    && next.match_time_secs - ev_time < 8.0 =>
                            {
                                let origin =
                                    Vector2::new(next.origin_x, next.origin_y);
                                if origin.distance(target_pos) <= RECEIVER_ORIGIN_TOL {
                                    origin
                                } else {
                                    target_pos
                                }
                            }
                            _ => target_pos,
                        };
                        let dist = agents[idx].position.distance(route_target);
                        let strict = arrival_time(
                            dist,
                            agents[idx].max_speed_sprint,
                            agents[idx].max_accel,
                        );
                        let needed = strict + RECEIVER_BUFFER_SECS;
                        let time_until = deadline - self.sim_time;
                        // Route early so the receiver can make the run in time.
                        if time_until <= needed.max(PRE_POSITION_SECS) {
                            let boost = if time_until < strict {
                                1.0 + 0.3 * agents[idx].attributes.stamina
                            } else {
                                1.0
                            };
                            let already = agents[idx]
                                .receiver_task
                                .map(|(t, _)| t.distance(route_target) < 0.05)
                                .unwrap_or(false);
                            self.route_receiver(idx, agents, route_target, deadline, boost);
                            if !already {
                                self.dispatched += 1;
                            }
                            self.receiver_of.insert(ev_id, agents[idx].id.clone());
                        }
                    }
                }
            }
        }

        // Apply each player's soonest event. Urgent events set the sprint
        // boost; anything within the pre-position horizon is recorded so the
        // brain drifts toward it well before it is due.
        for (idx, entry) in soonest.iter().enumerate() {
            let Some((etime, target, id)) = *entry else {
                continue;
            };
            let time_until = etime - self.sim_time;
            let dist = agents[idx].position.distance(target);
            let strict = arrival_time(dist, agents[idx].max_speed_sprint, agents[idx].max_accel);
            let needed = strict + PRIMARY_BUFFER_SECS;
            if time_until <= needed {
                let boost = if time_until < strict {
                    1.0 + 0.3 * agents[idx].attributes.stamina
                } else {
                    1.0
                };
                if self.anticipate(idx, agents, target, boost, true, id) {
                    self.dispatched += 1;
                }
            } else if time_until <= PRE_POSITION_SECS {
                // Know it early; do not sprint yet.
                if agents[idx].assigned_event_id != Some(id) {
                    agents[idx].assigned_event_id = Some(id);
                    self.dispatched += 1;
                }
                agents[idx].sprint_boost = 1.0;
            }
        }

        // ── 2. Execution pass: fire events whose timestamp has arrived ──
        while self.cursor < self.events.len() {
            let idx = self.cursor;
            if self.events[idx].match_time_secs > self.sim_time {
                break;
            }
            // ── readiness gate ─────────────────────────────────────────
            // The feed time is a promise about *when*; accuracy is a promise
            // about *where*. When the doer or the ball cannot yet be at the
            // origin, hold the event back — up to SLIP_MAX_SECS — so the
            // world catches up instead of staging the action in the wrong
            // place. Past the budget the event fires anyway.
            if self.sim_time - self.events[idx].match_time_secs < SLIP_MAX_SECS
                && !self.event_is_ready(&self.events[idx], agents, ball_pos)
            {
                break;
            }
            let event = &self.events[idx];
            let origin = Vector2::new(event.origin_x, event.origin_y);
            let target = match (event.target_x, event.target_y) {
                (Some(x), Some(y)) => Some(Vector2::new(x, y)),
                _ => None,
            };

            let (doer_state, doer_stage_target) = agents
                .iter_mut()
                .find(|a| a.id == event.player_id)
                .map(|agent| {
                    let before = agent.state.label();
                    let heading = match agent.state {
                        crate::agent::AgentState::AnticipatingEvent { target } => Some(target),
                        _ => None,
                    };
                    agent.begin_action(action_duration(event.type_id));
                    agent.assigned_event_id = None;
                    (Some(before), heading)
                })
                .unwrap_or((None, None));
            if doer_state.is_some() {
                self.executed += 1;
            }

            let receiver_id = self.receiver_of.remove(&event.id);
            if matches!(event.type_id, 1 | 2) {
                self.last_pass_receiver = receiver_id.clone();
            }
            executed_actions.push(DispatchedAction {
                event_id: event.id,
                type_id: event.type_id,
                team_id: event.team_id.clone(),
                player_id: event.player_id.clone(),
                origin,
                feed_time: event.match_time_secs,
                doer_state,
                doer_stage_target,
                target,
                receiver_id,
                outcome: event.outcome,
                is_aerial: event.is_aerial,
                is_own_goal: event.is_own_goal,
            });

            self.cursor += 1;
        }

        executed_actions
    }

    /// True when the doer and (for events that play it) the ball are already
    /// at the event origin, i.e. firing now would be recorded in the right
    /// place on the pitch.
    fn event_is_ready(
        &self,
        ev: &NormalizedEvent,
        agents: &[PlayerAgent],
        ball_pos: Option<Vector2>,
    ) -> bool {
        // Substitutions have no on-pitch positional contract: the outgoing
        // player walks off at the bench and the incoming player's agent is
        // only re-identified after dispatch, so gating on the origin would
        // stall the whole timeline for the full slip budget on every change.
        if ev.type_id == EVENT_TYPE_PLAYER_OFF || ev.type_id == EVENT_TYPE_PLAYER_ON {
            return true;
        }
        let origin = Vector2::new(ev.origin_x, ev.origin_y);
        if !ev.player_id.is_empty() {
            // A player who just received a pass is held to a tighter tolerance:
            // he is the next event's doer and must settle at his own origin,
            // not fire while still drifting toward the landing point.
            let tol = if self.last_pass_receiver.as_deref() == Some(ev.player_id.as_str())
            {
                DOER_READY_TOL_RECEIVER
            } else {
                DOER_READY_TOL
            };
            let at_origin = agents
                .iter()
                .find(|a| a.id == ev.player_id)
                .is_some_and(|a| a.position.distance(origin) <= tol);
            if !at_origin {
                return false;
            }
        }
        // `None` = no ball modelled (pure-agent unit tests): the ball check
        // cannot be satisfied by something that does not exist there.
        match ball_pos {
            Some(bp) => !ball_spot_expected(ev.type_id) || bp.distance(origin) <= BALL_READY_TOL,
            None => true,
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod dispatcher_tests {
    use super::*;

    fn event(id: u64, time: f32, team: &str, player: &str, x: f32, y: f32) -> NormalizedEvent {
        NormalizedEvent {
            id,
            type_id: 1,
            period_id: 1,
            match_time_secs: time,
            team_id: team.into(),
            player_id: player.into(),
            player_name: player.into(),
            origin_x: x,
            origin_y: y,
            target_x: None,
            target_y: None,
            outcome: true,
            is_aerial: false,
            is_own_goal: false,
        }
    }

    fn agent(id: &str, x: f32, y: f32) -> PlayerAgent {
        PlayerAgent::new(id.into(), "team_a".into(), 4, Vector2::new(x, y), false)
    }

    #[test]
    fn test_arrival_time_covers_deceleration() {
        // Arrival (stop-to-stop) must be at least the reach time...
        for d in [1.0, 6.0, 20.0, 50.0] {
            assert!(arrival_time(d, 8.5, 6.0) >= travel_time(d, 8.5, 6.0) - 1e-4);
        }
        // ...and a 20 m stop-to-stop needs ~3.77 s under the caps.
        let t = arrival_time(20.0, 8.5, 6.0);
        assert!(t > 3.7 && t < 3.85, "arrival_time(20) = {t}");
        assert_eq!(arrival_time(0.0, 8.5, 6.0), 0.0);
    }

    #[test]
    fn test_primary_actor_reaches_the_event() {
        // An actor 25 m away with 6 s of warning must be on the spot when the
        // event fires (the whole point of the arrival budget + wide scan).
        let mut d = LookaheadDispatcher::new(
            vec![event(1, 6.0, "team_a", "p", 20.0, 0.0)],
            3.5,
        );
        let mut agents = vec![agent("p", -5.0, 0.0)];
        let dt = 1.0 / 60.0;
        let mut arrived_when_fired = None;
        for _ in 0..420 {
            let actions = d.tick(dt, &mut agents, None);
            agents[0].update_kinematics(dt, Vector2::new(-5.0, 0.0), &[], Vector2::zero());
            if !actions.is_empty() {
                arrived_when_fired = Some(agents[0].position.distance(Vector2::new(20.0, 0.0)));
            }
        }
        let dist = arrived_when_fired.expect("event never fired");
        assert!(dist < 1.0, "actor was {dist:.2} m from the event when it fired");
    }

    #[test]
    fn test_travel_time_triangular_and_trapezoidal() {
        // Short distance → triangular (no cruise).
        let d_accel = 8.5f32 * 8.5 / (2.0 * 6.0);
        let t_tri = travel_time(d_accel, 8.5, 6.0);
        assert!((t_tri - 2.0 * (d_accel / 6.0).sqrt()).abs() < 1e-4);
        // Long distance → trapezoidal, and strictly less than d/v.
        let t = travel_time(20.0, 8.5, 6.0);
        assert!(t > 20.0 / 8.5, "travel time must exceed d/v");
        assert!(t < 20.0 / 8.5 + 8.5 / 6.0, "but stay bounded");
        // Degenerate inputs.
        assert_eq!(travel_time(0.0, 8.5, 6.0), 0.0);
        assert_eq!(travel_time(10.0, 0.0, 6.0), 0.0);
        assert_eq!(travel_time(10.0, 8.5, 0.0), 0.0);
    }

    #[test]
    fn test_anticipates_before_event_time() {
        let mut d = LookaheadDispatcher::new(
            vec![event(1, 3.0, "team_a", "p", -10.0, 0.0)],
            3.5,
        );
        let mut agents = vec![agent("p", -30.0, 0.0)];
        let dt = 1.0 / 60.0;
        let mut anticipated_at = None;
        for tick in 0..210 {
            d.tick(dt, &mut agents, None);
            if anticipated_at.is_none()
                && matches!(agents[0].state, AgentState::AnticipatingEvent { .. })
            {
                anticipated_at = Some(tick as f32 * dt);
            }
            agents[0].update_kinematics(dt, Vector2::new(-30.0, 0.0), &[], Vector2::zero());
        }
        let t = anticipated_at.expect("never anticipated");
        assert!(t < 3.0, "anticipation must start before the event");
        // 20 m needs ~3.06 s of kinematics, so anticipation must begin early.
        assert!(
            t < 0.5,
            "anticipation started too late ({t:.2}s) — travel time under-estimated"
        );
    }

    #[test]
    fn test_executes_on_time_and_arrives_near_origin() {
        let mut d = LookaheadDispatcher::new(
            vec![event(101, 3.0, "team_a", "p", -10.0, 0.0)],
            3.5,
        );
        let mut agents = vec![agent("p", -30.0, 0.0)];
        let dt = 1.0 / 60.0;
        let mut executed = false;
        for _ in 0..210 {
            let actions = d.tick(dt, &mut agents, None);
            agents[0].update_kinematics(dt, Vector2::new(-30.0, 0.0), &[], Vector2::zero());
            if !actions.is_empty() {
                executed = true;
                assert_eq!(actions[0].event_id, 101);
                // The player is 20 m away with 3.0 s before the event. Under the
                // realistic caps (v=8.5, a=6) a 20 m stop-to-stop takes
                // 2·v/a + (d − v²/a)/v = 3.77 s, so the player physically cannot
                // be within 1 m by t=3.0 s; the time-optimal arrival gets as
                // close as the kinematics allow (~1.7 m).
                let dist = agents[0].position.distance(Vector2::new(-10.0, 0.0));
                assert!(dist < 2.5, "arrived at {dist:.2} m from origin");
            }
        }
        assert!(executed);
        assert!(d.is_finished());
    }

    #[test]
    fn test_does_not_dispatch_when_event_is_far_away() {
        // Event 60 s in the future: nobody should pre-route on tick 1.
        let mut d = LookaheadDispatcher::new(
            vec![event(1, 60.0, "team_a", "p", 0.0, 0.0)],
            3.5,
        );
        let mut agents = vec![agent("p", 0.0, 0.0)];
        d.tick(1.0 / 60.0, &mut agents, None);
        assert!(
            matches!(agents[0].state, AgentState::InFormation),
            "player should not anticipate an event 60s away"
        );
    }

    #[test]
    fn test_receiver_chained_lookahead() {
        // Pass from p1 to landing point (0, 0); next event is p2's reception.
        let mut pass = event(1, 2.0, "team_a", "p1", -5.0, 0.0);
        pass.target_x = Some(0.0);
        pass.target_y = Some(0.0);
        let mut reception = event(2, 2.6, "team_a", "p2", 0.0, 0.0);
        reception.type_id = 1;

        let mut d = LookaheadDispatcher::new(vec![pass, reception], 3.5);
        let mut agents = vec![agent("p1", -5.0, 0.0), agent("p2", 0.0, 30.0)];
        let dt = 1.0 / 60.0;
        // Run ~1s; the receiver should already be routed toward (0,0).
        for _ in 0..60 {
            d.tick(dt, &mut agents, None);
            for a in agents.iter_mut() {
                a.update_kinematics(dt, a.position, &[], Vector2::zero());
            }
        }
        match agents[1].state {
            AgentState::AnticipatingEvent { target } => {
                assert!((target.x - 0.0).abs() < 1e-3);
            }
            other => panic!("receiver not routed: {other:?}"),
        }
    }

    #[test]
    fn test_no_teleportation() {
        let mut d = LookaheadDispatcher::new(
            vec![event(1, 1.0, "team_a", "p", 20.0, 10.0)],
            3.5,
        );
        let mut agents = vec![agent("p", 0.0, 0.0)];
        let dt = 1.0 / 60.0;
        let mut prev = agents[0].position;
        for _ in 0..300 {
            d.tick(dt, &mut agents, None);
            agents[0].update_kinematics(dt, Vector2::zero(), &[], Vector2::zero());
            let step = prev.distance(agents[0].position);
            assert!(step <= 8.5 * dt + 1e-3, "teleport of {step} m in one tick");
            prev = agents[0].position;
        }
    }

    #[test]
    fn test_is_finished_when_all_executed() {
        let mut d = LookaheadDispatcher::new(
            vec![
                event(1, 0.5, "team_a", "p", 0.0, 0.0),
                event(2, 1.0, "team_a", "p", 0.0, 0.0),
            ],
            3.5,
        );
        let mut agents = vec![agent("p", 0.0, 0.0)];
        for _ in 0..90 {
            d.tick(1.0 / 60.0, &mut agents, None);
        }
        assert!(d.is_finished());
        assert_eq!(d.executed, 2);
    }

    #[test]
    fn test_deterministic() {
        let build = || {
            LookaheadDispatcher::new(
                vec![
                    event(1, 1.0, "team_a", "p1", 5.0, 0.0),
                    event(2, 2.0, "team_a", "p2", -5.0, 0.0),
                ],
                3.5,
            )
        };
        let mut a = build();
        let mut b = build();
        let mut aa = vec![agent("p1", 0.0, 0.0), agent("p2", 0.0, 0.0)];
        let mut bb = aa.clone();
        for _ in 0..180 {
            a.tick(1.0 / 60.0, &mut aa, None);
            b.tick(1.0 / 60.0, &mut bb, None);
        }
        for (x, y) in aa.iter().zip(bb.iter()) {
            assert_eq!(x.position, y.position);
            assert_eq!(x.state, y.state);
        }
    }

    #[test]
    fn test_pass_routes_intended_receiver() {
        // A pass with a destination followed by a team-mate's reception must
        // route that receiver toward the landing point and name them on the
        // emitted action (so the ball can be timed to their arrival).
        let mut pass = event(1, 3.0, "team_a", "p1", -5.0, 0.0);
        pass.target_x = Some(10.0);
        pass.target_y = Some(0.0);
        let mut recv = event(2, 3.6, "team_a", "p2", 10.0, 0.0);
        recv.type_id = 1;

        let mut d = LookaheadDispatcher::new(vec![pass, recv], 3.5);
        let mut agents = vec![agent("p1", -5.0, 0.0), agent("p2", -20.0, 0.0)];
        let dt = 1.0 / 60.0;
        let mut receiver_id: Option<String> = None;
        let mut routed = false;
        for _ in 0..240 {
            let actions = d.tick(dt, &mut agents, None);
            // The receiver may be routed via a receiver task, or promoted to the
            // primary actor of the reception event (which clears the task).
            if agents[1].receiver_task.is_some()
                || agents[1].assigned_event_id == Some(2)
            {
                routed = true;
            }
            for a in &actions {
                if a.event_id == 1 {
                    receiver_id = a.receiver_id.clone();
                }
            }
            for a in agents.iter_mut() {
                a.update_kinematics(dt, a.position, &[], Vector2::zero());
            }
        }
        assert!(routed, "receiver was never routed to the pass landing point");
        assert_eq!(receiver_id.as_deref(), Some("p2"));
    }

    #[test]
    fn test_primary_event_keeps_receiver_task_for_arbitration() {
        // The receiver task is no longer wiped by a primary dispatch: the
        // brain's own-vs-recv arbitration decides which target is aimed for,
        // so a receiver whose own event is also due still runs to the
        // landing point when the ball arrives first.
        let mut agent = PlayerAgent::new(
            "p".into(),
            "team_a".into(),
            9,
            Vector2::zero(),
            false,
        );
        agent.receiver_task = Some((Vector2::new(5.0, 0.0), 10.0));
        let mut agents = vec![agent];
        let d = LookaheadDispatcher::new(vec![event(1, 0.0, "team_a", "p", 0.0, 0.0)], 3.5);
        d.anticipate(0, &mut agents, Vector2::zero(), 1.0, true, 1);
        assert!(agents[0].receiver_task.is_some());
        assert_eq!(agents[0].assigned_event_id, Some(1));
    }

    #[test]
    fn test_players_are_told_about_events_early() {
        // An event 8 s away. The actor is already close, so it is not urgent;
        // nevertheless the dispatcher must record the event well beforehand
        // (within the pre-position horizon) so the brain can drift toward it.
        let mut d =
            LookaheadDispatcher::new(vec![event(1, 8.0, "team_a", "p", 1.0, 0.0)], 8.0);
        let mut agents = vec![agent("p", 1.0, 0.0)];
        let mut assigned_at = None;
        for tick in 0..(5 * 60) {
            d.tick(1.0 / 60.0, &mut agents, None);
            if assigned_at.is_none() && agents[0].assigned_event_id == Some(1) {
                assigned_at = Some(tick as f32 / 60.0);
            }
            for a in agents.iter_mut() {
                a.update_kinematics(1.0 / 60.0, a.position, &[], Vector2::zero());
            }
        }
        let t = assigned_at.expect("event was never assigned");
        assert!(
            t < 5.5,
            "event only assigned {t:.1}s before it was due (must be >=5 s early)"
        );
    }
}
