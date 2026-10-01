//! Utility-AI tactical brain + context-steered locomotion.
//!
//! The engine's macro layer already produces a **coherent** formation anchor
//! per player (flat back line, onside forwards, elastic block). This module
//! adds the micro behaviour *around* that anchor:
//!
//! * **Hard constraint** — a scheduled event (the player's own action, or a
//!   pass they are the intended receiver of) overrides everything so the actor
//!   is guaranteed to be on the spot when the ball arrives.
//! * **Team plans** — at most one presser, one cover, two support options and
//!   one runner per team. Everyone else holds shape. This is what makes a team
//!   play *as one* instead of swarming the ball.
//! * **Role / style / attribute weighted scoring** — defenders hold the line,
//!   midfielders contain and support, forwards take runs. The same decision is
//!   shaded by team style (mentality, pressing, width, tempo) and by the
//!   player's own attributes (teamwork, positioning, aggression, pace, flair…).
//!
//! ```text
//!  Upcoming event ──▶ [ hard constraint ] ──▶ sprint to the spot
//!                          │ else
//!                          ▼
//!                   [ team plan + utility ]──▶ intent + micro-target
//!                          │
//!                          ▼
//!                   [ context steering + burst/hold cadence ]
//! ```

use crate::agent::{PlayerAgent, PlayerAttributes, PlayerRole};
use crate::dispatcher::{arrival_time, PRE_POSITION_SECS};
use crate::formation::Vector2;
use crate::tactics::{Line, TeamContext, TeamPlan, TeamStyle};
use std::collections::HashMap;

/// Directions sampled by context steering.
pub const RAY_COUNT: usize = 16;

/// The discrete tactical actions an off-ball player can choose.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TacticalIntent {
    /// Sprint to an event the player is scheduled to execute (hard constraint).
    AnticipateEvent { target: Vector2 },
    /// Hold the elastic formation anchor.
    HoldShape,
    /// Soft-mark an opponent while staying anchored to the zone.
    ManMark { target: Vector2 },
    /// Move into a passing pocket to support the ball carrier.
    SupportTriangle { target: Vector2 },
    /// Close down the ball carrier (the designated presser).
    PressCarrier { target: Vector2 },
    /// Screen the space goal-side of the ball (the designated cover).
    CoverSpace { target: Vector2 },
    /// Make a run in behind the opposition line (the designated runner).
    TakeRun { target: Vector2 },
}

/// Per-player decision state.
#[derive(Debug, Clone)]
pub struct TacticalBrain {
    pub current_intent: TacticalIntent,
    /// Seconds until the player may reconsider their chosen action.
    pub commitment_timer: f32,
    /// While > 0 the player stands still and scans (stop-and-go cadence).
    pub scan_timer: f32,
}

impl Default for TacticalBrain {
    fn default() -> Self {
        Self::new()
    }
}

impl TacticalBrain {
    pub fn new() -> Self {
        Self {
            current_intent: TacticalIntent::HoldShape,
            commitment_timer: 0.0,
            scan_timer: 0.0,
        }
    }

    /// Evaluate the tactical utility of each candidate action and return the
    /// winning `(target position, target speed)`.
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate_intent(
        &mut self,
        dt: f32,
        attributes: &PlayerAttributes,
        role: PlayerRole,
        max_speed: f32,
        max_accel: f32,
        pos: Vector2,
        anchor: Vector2,
        ball: Vector2,
        upcoming_event: Option<(Vector2, f32)>,
        positions: &[Vector2],
        is_home: &[bool],
        self_idx: usize,
        plan: &TeamPlan,
    ) -> (Vector2, f32) {
        self.commitment_timer -= dt;
        self.scan_timer -= dt;

        // ── 1. HARD CONSTRAINT — a scheduled event must be reached in time. ──
        if let Some((event_target, time_until)) = upcoming_event {
            let dist = pos.distance(event_target);
            let buffer = (0.5 - attributes.anticipation * 0.15).max(0.3);
            let time_needed = arrival_time(dist, max_speed, max_accel) + buffer;
            if time_until <= time_needed {
                self.current_intent = TacticalIntent::AnticipateEvent { target: event_target };
                self.scan_timer = 0.0;
                return (event_target, max_speed);
            }
        }

        // ── 2. STOP-AND-GO — plant and scan while the timer runs. ──
        if self.scan_timer > 0.0 {
            return (pos, 0.0);
        }

        // ── 3. RE-EVALUATE at the commitment boundary. ──
        if self.commitment_timer <= 0.0 {
            self.commitment_timer = 0.75 + (1.0 - attributes.anticipation) * 0.55;
            self.current_intent = self.choose_intent(
                role, attributes, pos, positions, is_home, self_idx, plan,
            );
        }

        // ── 4. EXECUTE the chosen intent with a live target. ──
        let (target, speed) = self.plan_intent(
            attributes, role, max_speed, pos, anchor, ball, positions, is_home, plan,
        );

        // ── 5. PRE-POSITION — an event is known but not yet urgent, so drift
        // toward it now. By the time the sprint is needed the player is already
        // close, which is how real players "read the game" early.
        if let Some((ev, t)) = upcoming_event {
            if t > 0.0 && t <= PRE_POSITION_SECS && !plan.ctx.set_piece {
                let line = Line::from_role(role);
                // Defenders hold the line and only move on an urgent event;
                // pre-positioning them would drag the back four out of shape.
                if matches!(line, Line::Defense | Line::Goalkeeper) {
                    return (target, speed);
                }
                let zone = zone_radius(role, plan.ctx.style);
                let ev_target = clamp_to_zone(ev, anchor, zone * 1.15);
                let w = ((PRE_POSITION_SECS - t) / PRE_POSITION_SECS).clamp(0.0, 1.0) * 0.45;
                let blended = target + (ev_target - target) * w;
                let speed = if speed < 1.0 && w > 0.08 {
                    jog_speed(Line::from_role(role), plan.ctx.style)
                } else {
                    speed
                };
                return (blended, speed);
            }
        }
        (target, speed)
    }

    /// Choose the intent for this commitment window.
    #[allow(clippy::too_many_arguments)]
    fn choose_intent(
        &self,
        role: PlayerRole,
        attributes: &PlayerAttributes,
        pos: Vector2,
        positions: &[Vector2],
        is_home: &[bool],
        self_idx: usize,
        plan: &TeamPlan,
    ) -> TacticalIntent {
        if role == PlayerRole::Goalkeeper {
            return TacticalIntent::HoldShape;
        }
        // During a set piece every player holds their assigned position; only
        // the taker (handled by the hard event constraint) acts.
        if plan.ctx.set_piece {
            return TacticalIntent::HoldShape;
        }
        let line = Line::from_role(role);

        if plan.ctx.in_possession {
            if plan.is_runner(self_idx) {
                return TacticalIntent::TakeRun {
                    target: Vector2::zero(),
                };
            }
            if plan.support_rank(self_idx).is_some() {
                let ball = Vector2::new(plan.ctx.ball_x, plan.ctx.ball_y);
                let pocket = self.find_support_pocket(
                    pos, ball, positions, is_home, self_idx, plan.ctx.is_home,
                );
                return TacticalIntent::SupportTriangle { target: pocket };
            }
            return TacticalIntent::HoldShape;
        }

        // Out of possession: the designated pair presses / screens.
        if plan.is_presser(self_idx) {
            return TacticalIntent::PressCarrier {
                target: Vector2::zero(),
            };
        }
        if plan.is_cover(self_idx) {
            return TacticalIntent::CoverSpace {
                target: Vector2::zero(),
            };
        }

        // Defenders / midfielders soft-mark a nearby threat. Defenders mark
        // **zonally**: they slide along the back line but never step off it.
        // Formation stability is the backs' default: only low-teamwork
        // defenders freelance into an individual drift, so the unit never
        // collapses around one opposition carrier.
        let may_mark = line != Line::Defense || attributes.teamwork < 0.6;
        if matches!(line, Line::Defense | Line::Midfield) && may_mark {
            let mark_range = if line == Line::Defense {
                3.5 + (1.0 - plan.ctx.style.compactness) * 1.5
            } else {
                4.5 + (1.0 - plan.ctx.style.compactness) * 2.0
            };
            if let Some((_opp, d)) = nearest_opponent(pos, positions, is_home, plan.ctx.is_home) {
                if d < mark_range {
                    return TacticalIntent::ManMark {
                        target: Vector2::zero(),
                    };
                }
            }
        }

        TacticalIntent::HoldShape
    }

    /// Turn the committed intent into a live `(target, speed)`, recomputing
    /// moving targets each tick so marks and runs track the play.
    #[allow(clippy::too_many_arguments)]
    fn plan_intent(
        &mut self,
        attributes: &PlayerAttributes,
        role: PlayerRole,
        max_speed: f32,
        pos: Vector2,
        anchor: Vector2,
        ball: Vector2,
        positions: &[Vector2],
        is_home: &[bool],
        plan: &TeamPlan,
    ) -> (Vector2, f32) {
        let style = plan.ctx.style;
        let line = Line::from_role(role);
        let zone = zone_radius(role, style);
        let clamped = |t: Vector2| clamp_to_zone(t, anchor, zone);

        match self.current_intent {
            TacticalIntent::AnticipateEvent { target } => {
                if pos.distance(target) < 0.4 {
                    (pos, 0.0)
                } else {
                    (target, max_speed)
                }
            }
            TacticalIntent::HoldShape => {
                let d = pos.distance(anchor);
                if d < 0.6 {
                    (pos, 0.0) // in position: hold the line
                } else {
                    // Recover faster the further out of position we are, so a
                    // back line that has been turned can sprint back into shape.
                    let mut urgency = (1.0 + d / 7.0).min(2.4);
                    if line == Line::Defense
                        && pos.x * plan.ctx.attack_dx > plan.ctx.ball_x * plan.ctx.attack_dx
                    {
                        // Caught ahead of the ball: sprint back immediately.
                        urgency = 3.0;
                    }
                    (anchor, jog_speed(line, style) * urgency)
                }
            }
            TacticalIntent::ManMark { .. } => {
                if let Some((opp, d)) = nearest_opponent(pos, positions, is_home, plan.ctx.is_home)
                {
                    if line == Line::Defense {
                        // Zonal: hold the line's x, and only mirror the runner
                        // a short way laterally — a small drift keeps the back
                        // four from bunching around one opposition player.
                        let lateral = (opp.y - anchor.y).clamp(-4.0, 4.0);
                        let target = Vector2::new(anchor.x, anchor.y + lateral * 0.4);
                        return (target, 4.2 + (1.0 - d / 10.0).max(0.0) * 2.0);
                    }
                    // Midfielders screen the goal side, blended back to the
                    // anchor so they do not abandon the block.
                    let goal_dir = (plan.ctx.own_goal() - opp).normalize();
                    let sticky = opp + goal_dir * 1.6;
                    let target = anchor + (sticky - anchor) * (0.25 + attributes.positioning * 0.25);
                    (clamped(target), 4.6 + (1.0 - d / 12.0).max(0.0) * 2.0)
                } else {
                    (anchor, 3.0)
                }
            }
            TacticalIntent::PressCarrier { .. } => {
                // Close down the ball; sprint the last few metres.
                let target = clamped(ball);
                (target, 3.6 + attributes.aggression * 1.6 + style.pressing * 0.8)
            }
            TacticalIntent::CoverSpace { .. } => {
                // Screen the space between the ball and our goal.
                let goal_dir = (plan.ctx.own_goal() - ball).normalize();
                let target = clamped(ball + goal_dir * 5.0);
                (target, 3.6)
            }
            TacticalIntent::SupportTriangle { target } => {
                let target = clamped(target);
                if pos.distance(target) < 0.8 {
                    self.scan_timer = 0.9; // arrived: plant and scan
                    (pos, 0.0)
                } else {
                    (target, 3.2 + style.tempo * 0.6)
                }
            }
            TacticalIntent::TakeRun { .. } => {
                let target = clamped(run_target(anchor, plan.ctx, attributes));
                if pos.distance(target) < 0.9 {
                    (pos, 0.0)
                } else {
                    (target, 4.6 + attributes.pace * 1.2)
                }
            }
        }
    }

    /// A pocket of space offset from the ball carrier to form a passing
    /// triangle, avoiding the nearest opponent.
    fn find_support_pocket(
        &self,
        current: Vector2,
        ball: Vector2,
        positions: &[Vector2],
        is_home: &[bool],
        self_idx: usize,
        my_home: bool,
    ) -> Vector2 {
        let to_ball = (ball - current).normalize();
        if to_ball.length_sq() < 1e-6 {
            return current;
        }
        let lateral = Vector2::new(-to_ball.y, to_ball.x) * 4.0;
        let candidate = current + lateral;
        let crowded = positions.iter().enumerate().any(|(j, p)| {
            j != self_idx && is_home[j] != my_home && p.distance(candidate) < 3.0
        });
        if crowded {
            current - lateral
        } else {
            candidate
        }
    }
}

/// Jog speed for a line, shaded by team tempo.
#[inline]
fn jog_speed(line: Line, style: TeamStyle) -> f32 {
    let base = match line {
        Line::Goalkeeper => 2.4,
        Line::Defense => 3.3,
        Line::Midfield => 3.7,
        Line::Attack => 4.0,
    };
    base + style.tempo * 0.9
}

/// How far a player may roam from their anchor, by line and style.
#[inline]
fn zone_radius(role: PlayerRole, style: TeamStyle) -> f32 {
    match role {
        PlayerRole::Goalkeeper => 0.0,
        // Compact teams keep tighter zones; expansive teams allow more.
        PlayerRole::Defender => 5.5 + (1.0 - style.compactness) * 3.0,
        PlayerRole::Midfielder => 10.0 + style.width * 6.0,
        PlayerRole::Forward => 16.0 + style.directness * 10.0,
        PlayerRole::Unknown => 12.0,
    }
}

/// Keep a target inside a circle around the anchor.
#[inline]
fn clamp_to_zone(target: Vector2, anchor: Vector2, radius: f32) -> Vector2 {
    if radius <= 0.0 {
        return anchor;
    }
    let delta = target - anchor;
    let d = delta.length();
    if d > radius {
        anchor + delta * (radius / d)
    } else {
        target
    }
}

/// A run into the channel, staying onside with the opposition line.
#[inline]
fn run_target(anchor: Vector2, ctx: TeamContext, attributes: &PlayerAttributes) -> Vector2 {
    // Attackers must stay onside: never beyond the opponent's back line.
    let offside = ctx.offside_limit_x - ctx.attack_dx * 0.35;
    let channel = anchor.y + (attributes.flair - 0.5) * 6.0;
    let x = if ctx.attack_dx > 0.0 {
        offside.min(anchor.x + 6.0 * (0.5 + attributes.pace))
    } else {
        offside.max(anchor.x - 6.0 * (0.5 + attributes.pace))
    };
    Vector2::new(x, channel)
}

/// Progress-frame x of the *second-last defender* of the defending team —
/// the second-highest `(x · attack_dx)` among that team's players, goalkeeper
/// included. Returns `f32::NEG_INFINITY` when fewer than two defenders are
/// known.
///
/// `players` yields `(world_x, is_home)` for every player on the pitch.
#[inline]
pub fn second_last_progress(
    players: impl Iterator<Item = (f32, bool)>,
    defending_home: bool,
    attack_dx: f32,
) -> f32 {
    let mut best = f32::NEG_INFINITY;
    let mut second = f32::NEG_INFINITY;
    for (x, is_home) in players {
        if is_home != defending_home {
            continue;
        }
        let v = x * attack_dx;
        if v > best {
            second = best;
            best = v;
        } else if v > second {
            second = v;
        }
    }
    second
}

/// The offside limit in **world x** for a team attacking along `attack_dx`:
/// the law's rule — a player may not be *both* ahead of the second-last
/// defender *and* ahead of the ball, so the legal bound is the goal-ward
/// maximum of the two.
#[inline]
pub fn offside_limit_world(
    players: impl Iterator<Item = (f32, bool)>,
    defending_home: bool,
    attack_dx: f32,
    ball_x: f32,
) -> f32 {
    let second = second_last_progress(players, defending_home, attack_dx);
    if !second.is_finite() {
        return ball_x;
    }
    second.max(ball_x * attack_dx) * attack_dx
}

/// Nearest opponent `(position, distance)` to `pos`.
fn nearest_opponent(
    pos: Vector2,
    positions: &[Vector2],
    is_home: &[bool],
    my_home: bool,
) -> Option<(Vector2, f32)> {
    let mut best: Option<(Vector2, f32)> = None;
    for (j, p) in positions.iter().enumerate() {
        if is_home[j] == my_home {
            continue;
        }
        let d = pos.distance(*p);
        if best.map(|(_, bd)| d < bd).unwrap_or(true) {
            best = Some((*p, d));
        }
    }
    best
}

// ── Team plan builder ──────────────────────────────────────────

/// Build the per-team activation plan (presser / cover / support / runner).
///
/// This is where "the team plays as one" is enforced: only a handful of
/// players are ever allowed to leave their shape at a time.
fn build_plan(
    ctx: TeamContext,
    positions: &[Vector2],
    is_home: &[bool],
    roles: &[PlayerRole],
    attributes: &[PlayerAttributes],
    ball: Vector2,
    team_home: bool,
) -> TeamPlan {
    let mut presser: Option<(f32, usize)> = None;
    let mut cover: Option<(f32, usize)> = None;
    let mut support: [(f32, usize); 2] = [(f32::MAX, usize::MAX); 2];
    let mut runner: Option<(f32, usize)> = None;

    let press_reach = 8.0 + ctx.style.pressing * 24.0;

    for (i, p) in positions.iter().enumerate() {
        if is_home[i] != team_home || roles[i] == PlayerRole::Goalkeeper {
            continue;
        }
        let dist = p.distance(ball);
        let line = Line::from_role(roles[i]);

        // Pressing cost: nearest to the ball, with a bias that keeps defenders
        // home and prefers the midfield/attack to close down.
        let role_bias = match line {
            Line::Defense => 5.0,
            Line::Midfield => 0.0,
            Line::Attack => 1.5,
            Line::Goalkeeper => f32::MAX,
        };
        let press_cost = dist + role_bias - attributes[i].aggression * 4.0;
        if dist <= press_reach {
            consider_pair(&mut presser, press_cost, i);
        }

        // Support: nearest two to the ball, but defenders only offer a short
        // outlet (they do not push up as passing options like midfielders).
        let support_bias = match line {
            Line::Defense => 14.0,
            Line::Midfield => 0.0,
            Line::Attack => 5.0,
            Line::Goalkeeper => f32::MAX,
        };
        consider_duo(
            &mut support,
            dist + support_bias - attributes[i].vision * 3.0,
            i,
        );

        // Runner: the most direct forward, if the ball is advanced enough.
        if line == Line::Attack {
            let progress = ctx.attack_progress();
            let run_drive = attributes[i].pace * 0.6
                + attributes[i].flair * 0.5
                + progress * 0.8;
            if progress > -0.15 {
                consider_pair(&mut runner, -run_drive, i);
            }
        }
    }

    // Cover is the next-best presser candidate (reuse the same cost).
    if let Some((_, pi)) = presser {
        let mut second: Option<(f32, usize)> = None;
        for (i, p) in positions.iter().enumerate() {
            if is_home[i] != team_home || roles[i] == PlayerRole::Goalkeeper || i == pi {
                continue;
            }
            let dist = p.distance(ball);
            if dist > press_reach + 12.0 {
                continue;
            }
            let line = Line::from_role(roles[i]);
            let role_bias = match line {
                Line::Defense => 5.0,
                Line::Midfield => 0.0,
                Line::Attack => 1.5,
                Line::Goalkeeper => f32::MAX,
            };
            consider_pair(
                &mut second,
                dist + role_bias - attributes[i].aggression * 3.0,
                i,
            );
        }
        cover = second;
    }

    let support = support.map(|(_, i)| if i == usize::MAX { None } else { Some(i) });
    TeamPlan {
        ctx,
        presser: presser.map(|(_, i)| i),
        cover: cover.map(|(_, i)| i),
        support,
        runner: runner.map(|(_, i)| i),
    }
}

#[inline]
fn consider_pair(slot: &mut Option<(f32, usize)>, cost: f32, idx: usize) {
    match slot {
        Some((best, _)) if *best <= cost => {}
        _ => *slot = Some((cost, idx)),
    }
}

#[inline]
fn consider_duo(slot: &mut [(f32, usize); 2], cost: f32, idx: usize) {
    if cost < slot[0].0 {
        slot[1] = slot[0];
        slot[0] = (cost, idx);
    } else if cost < slot[1].0 {
        slot[1] = (cost, idx);
    }
}

// ── Batch driver ───────────────────────────────────────────────

/// Run one Utility-AI + steering step for every agent.
///
/// `event_lookup` maps an event id → `(origin, match_time_secs)`, used for the
/// hard anticipation constraint. `styles` and `hold_lines` are the per-team
/// `[home, away]` tactical identity and defensive-line x, computed by the
/// engine's macro layer.
#[allow(clippy::too_many_arguments)]
pub fn step_agents(
    agents: &mut [PlayerAgent],
    anchors: &HashMap<String, Vector2>,
    event_lookup: &HashMap<u64, (Vector2, f32)>,
    sim_time: f32,
    ball: Vector2,
    possession_team: Option<&str>,
    home_team_id: &str,
    styles: &[TeamStyle; 2],
    hold_lines: &[f32; 2],
    set_piece: bool,
    dt: f32,
) {
    // Snapshot positions/teams so every agent reads a consistent world.
    let positions: Vec<Vector2> = agents.iter().map(|a| a.position).collect();
    let is_home: Vec<bool> = agents.iter().map(|a| a.team_id == home_team_id).collect();
    let roles: Vec<PlayerRole> = agents.iter().map(|a| a.role).collect();
    let attributes: Vec<PlayerAttributes> = agents.iter().map(|a| a.attributes).collect();
    for agent in agents.iter_mut() {
        agent.prev_position = agent.position;
    }

    // ── Build both team plans from the snapshot. ──
    let mut plans: [Option<TeamPlan>; 2] = [None, None];
    for team in 0..2 {
        let team_home = team == 0;
        let attack_dx = if team_home { 1.0 } else { -1.0 };
        let team_id = if team_home {
            home_team_id
        } else {
            // Any agent not on the home team.
            agents
                .iter()
                .find(|a| a.team_id != home_team_id)
                .map(|a| a.team_id.as_str())
                .unwrap_or("")
        };
        let in_possession = possession_team == Some(team_id);
        let own = hold_lines[team];
        let opp = hold_lines[1 - team];
        // Onside bound = the opponent's *actual* second-last defender (law
        // rule: never beyond it and the ball at once), not the block proxy.
        let offside_limit_x = offside_limit_world(
            positions.iter().zip(is_home.iter()).map(|(p, h)| (p.x, *h)),
            !team_home,
            attack_dx,
            ball.x,
        );
        let ctx = TeamContext {
            is_home: team_home,
            attack_dx,
            own_back_line_x: own,
            opp_back_line_x: opp,
            offside_limit_x,
            ball_x: ball.x,
            ball_y: ball.y,
            in_possession,
            set_piece,
            style: styles[team],
        };
        plans[team] = Some(build_plan(
            ctx,
            &positions,
            &is_home,
            &roles,
            &attributes,
            ball,
            team_home,
        ));
    }

    for (i, agent) in agents.iter_mut().enumerate() {
        let anchor = anchors.get(&agent.id).copied().unwrap_or(Vector2::zero());
        let attributes = agent.attributes; // Copy
        let role = agent.role;
        let max_speed = agent.max_speed_sprint * agent.sprint_boost;
        let max_accel = agent.max_accel;
        let pos = agent.position;
        let team_home = agent.team_id == home_team_id;
        let plan = plans[usize::from(!team_home)].as_ref().unwrap();

        // Prefer whichever deadline is sooner: the player's own scheduled
        // action, or a pass they are routed to receive.
        let own = agent.assigned_event_id.and_then(|eid| {
            event_lookup
                .get(&eid)
                .map(|(t, tm)| (*t, tm - sim_time))
        });
        let recv = agent
            .receiver_task
            .filter(|(_, deadline)| sim_time <= *deadline + 0.5)
            .map(|(target, deadline)| (target, deadline - sim_time));
        let upcoming = match (own, recv) {
            // The receiver task wins whenever the ball lands no later than the
            // player's own event: a man about to receive a pass runs to the
            // ball, not away from it toward a spot he reaches a beat later.
            (Some(o), Some(r)) => Some(if o.1 < r.1 { o } else { r }),
            (Some(o), None) => Some(o),
            (None, Some(r)) => Some(r),
            (None, None) => None,
        };

        let (target, target_speed) = agent.brain.evaluate_intent(
            dt,
            &attributes,
            role,
            max_speed,
            max_accel,
            pos,
            anchor,
            ball,
            upcoming,
            &positions,
            &is_home,
            i,
            plan,
        );

        // Hard event runs ignore avoidance so the actor always reaches the spot.
        let direct = matches!(
            agent.brain.current_intent,
            TacticalIntent::AnticipateEvent { .. }
        );
        agent.steer_toward(dt, target, target_speed, &positions, &is_home, i, ball, direct);
    }

    // Final positional depenetration: a hard guarantee that two bodies can
    // never share the same spot, regardless of intent or inertia. Runs after
    // integration so the recorded positions are always legal.
    depenetrate(agents, dt);

    // Hard per-tick displacement cap: the agent's own integration plus any
    // depenetration correction can never exceed one sprinting step. This makes
    // teleporting physically impossible no matter what set-piece reshaping did.
    let max_step = crate::agent::MAX_SPEED * dt;
    for agent in agents.iter_mut() {
        let delta = agent.position - agent.prev_position;
        agent.position = (agent.prev_position + delta.truncate(max_step)).clamp_on_pitch();
    }

    // ── Offside hard limit (open play only).
    //
    // The team in possession may never place a player beyond the opponent's
    // second-last defender *and* the ball, so nobody can ever sit in an
    // offside-looking position while the log shows no offside. The correction
    // is bounded to one sprint step per tick, so it eases players back behind
    // the line instead of dragging them (no teleport). Deliberate set-piece
    // layouts (corners, free kicks) are exempt.
    if !set_piece {
        if let Some(poss_id) = possession_team {
            let poss_home = poss_id == home_team_id;
            let mut clamped: Vec<(usize, f32)> = Vec::new();
            for (i, agent) in agents.iter().enumerate() {
                if is_home[i] != poss_home {
                    continue;
                }
                // The hard event constraint wins: a logged spot is where the
                // real player stood, so the actor is allowed to reach it even
                // if our simulated line sits shallower than the real one.
                if matches!(
                    agent.brain.current_intent,
                    TacticalIntent::AnticipateEvent { .. }
                ) {
                    continue;
                }
                let attack_dx = if is_home[i] { 1.0 } else { -1.0 };
                let limit = offside_limit_world(
                    agents.iter().map(|a| (a.position.x, a.team_id == home_team_id)),
                    !is_home[i],
                    attack_dx,
                    ball.x,
                ) - 0.35 * attack_dx;
                let x = agent.position.x;
                let over = (x - limit) * attack_dx; // > 0 → beyond the limit
                // A 1 m deadband absorbs depenetration jitter and event-spot
                // arrivals; only real violations are pulled back.
                const DEADBAND: f32 = 1.0;
                if over > DEADBAND {
                    let push = (over - DEADBAND).min(max_step);
                    clamped.push((i, x - attack_dx * push));
                }
            }
            for (i, new_x) in clamped {
                agents[i].position.x = new_x;
            }
        }
    }
}

/// Resolve any residual body overlap. Pushes are capped to the per-tick speed
/// budget (so this can never read as a teleport) and slide along the touchline
/// instead of pinning players against it.
fn depenetrate(agents: &mut [PlayerAgent], dt: f32) {
    use crate::parser::MatchContext;
    const MIN_SEP: f32 = 1.0;
    let max_push = crate::agent::MAX_SPEED * dt;
    let n = agents.len();
    for i in 0..n {
        for j in (i + 1)..n {
            let a = agents[i].position;
            let b = agents[j].position;
            let d = a.distance(b);
            if d >= MIN_SEP {
                continue;
            }
            let (mut dx, mut dy) = if d > 1e-4 {
                ((a.x - b.x) / d, (a.y - b.y) / d)
            } else {
                let ang = ((i * 37 + j * 101) as f32) * 0.7;
                (ang.cos(), ang.sin())
            };
            let push = ((MIN_SEP - d) * 0.5).min(max_push);
            let na = a + Vector2::new(dx * push, dy * push);
            let nb = b - Vector2::new(dx * push, dy * push);
            if !MatchContext::is_on_pitch(na.x, na.y)
                || !MatchContext::is_on_pitch(nb.x, nb.y)
            {
                let (tx, ty) = (-dy, dx);
                let ta = a + Vector2::new(tx * push, ty * push);
                let tb = b - Vector2::new(tx * push, ty * push);
                if MatchContext::is_on_pitch(ta.x, ta.y)
                    && MatchContext::is_on_pitch(tb.x, tb.y)
                {
                    dx = tx;
                    dy = ty;
                }
            }
            agents[i].position = (a + Vector2::new(dx * push, dy * push)).clamp_on_pitch();
            agents[j].position = (b - Vector2::new(dx * push, dy * push)).clamp_on_pitch();
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod brain_tests {
    use super::*;
    use crate::agent::PlayerAgent;

    fn attrs() -> PlayerAttributes {
        PlayerAttributes::default()
    }

    fn ctx(in_possession: bool) -> TeamContext {
        TeamContext {
            is_home: true,
            attack_dx: 1.0,
            own_back_line_x: -30.0,
            opp_back_line_x: 10.0,
            offside_limit_x: 10.0,
            ball_x: 0.0,
            ball_y: 0.0,
            in_possession,
            set_piece: false,
            style: TeamStyle::default(),
        }
    }

    fn plan(in_possession: bool) -> TeamPlan {
        TeamPlan {
            ctx: ctx(in_possession),
            presser: None,
            cover: None,
            support: [None; 2],
            runner: None,
        }
    }

    fn eval(
        b: &mut TacticalBrain,
        role: PlayerRole,
        pos: Vector2,
        anchor: Vector2,
        plan: &TeamPlan,
        positions: &[Vector2],
        is_home: &[bool],
    ) -> (Vector2, f32) {
        b.evaluate_intent(
            1.0 / 60.0,
            &attrs(),
            role,
            8.5,
            6.0,
            pos,
            anchor,
            Vector2::zero(),
            None,
            positions,
            is_home,
            0,
            plan,
        )
    }

    #[test]
    fn test_undelegated_defender_holds_shape() {
        let mut b = TacticalBrain::new();
        let p = plan(false);
        let pos = vec![Vector2::new(-30.0, -10.0), Vector2::new(20.0, 0.0)];
        let home = vec![true, false];
        let (_t, speed) = eval(&mut b, PlayerRole::Defender, pos[0], pos[0], &p, &pos, &home);
        assert!(matches!(b.current_intent, TacticalIntent::HoldShape));
        assert!(speed <= 5.0, "shape jog too fast: {speed}");
    }

    #[test]
    fn test_designated_presser_closes_down_ball() {
        let mut b = TacticalBrain::new();
        let mut p = plan(false);
        p.presser = Some(0);
        let pos = vec![Vector2::new(-5.0, 0.0), Vector2::new(20.0, 0.0)];
        let home = vec![true, false];
        let (target, speed) = eval(
            &mut b,
            PlayerRole::Midfielder,
            pos[0],
            Vector2::new(-10.0, 0.0),
            &p,
            &pos,
            &home,
        );
        assert!(matches!(b.current_intent, TacticalIntent::PressCarrier { .. }));
        assert!(target.distance(Vector2::zero()) < 3.0, "press target {target:?}");
        // The press is deliberately not a full sprint — a high press that
        // sends everyone running clutters the pitch. It closes down at a
        // strong-but-controlled pace instead.
        assert!(speed > 3.0, "press should close down, got {speed}");
    }

    #[test]
    fn test_runner_stays_onside() {
        let mut b = TacticalBrain::new();
        let mut p = plan(true);
        p.runner = Some(0);
        let pos = vec![Vector2::new(0.0, 0.0)];
        let home = vec![true];
        let (target, _speed) = eval(
            &mut b,
            PlayerRole::Forward,
            pos[0],
            Vector2::new(0.0, 0.0),
            &p,
            &pos,
            &home,
        );
        assert!(matches!(b.current_intent, TacticalIntent::TakeRun { .. }));
        assert!(target.x <= 10.0 + 1e-3, "run offside at x={}", target.x);
    }

    #[test]
    fn test_zone_clamp_keeps_targets_near_anchor() {
        let mut b = TacticalBrain::new();
        let mut p = plan(false);
        p.presser = Some(0);
        let anchor = Vector2::new(0.0, 0.0);
        let ball = Vector2::new(60.0, 0.0);
        let pos = vec![Vector2::new(0.0, 0.0)];
        let home = vec![true];
        let (target, _) = b.evaluate_intent(
            1.0 / 60.0,
            &attrs(),
            PlayerRole::Defender,
            8.5,
            6.0,
            pos[0],
            anchor,
            ball,
            None,
            &pos,
            &home,
            0,
            &p,
        );
        assert!(target.distance(anchor) <= 14.0, "zone clamp failed: {target:?}");
    }

    #[test]
    fn test_hard_event_overrides_everything() {
        let mut b = TacticalBrain::new();
        let p = plan(false);
        let pos = vec![Vector2::new(0.0, 0.0)];
        let home = vec![true];
        let (target, speed) = b.evaluate_intent(
            1.0 / 60.0,
            &attrs(),
            PlayerRole::Forward,
            8.5,
            6.0,
            Vector2::zero(),
            Vector2::new(-40.0, 0.0),
            Vector2::new(40.0, 0.0),
            Some((Vector2::new(5.0, 0.0), 0.2)),
            &pos,
            &home,
            0,
            &p,
        );
        assert_eq!(target, Vector2::new(5.0, 0.0));
        assert!(speed >= 8.0);
        assert!(matches!(b.current_intent, TacticalIntent::AnticipateEvent { .. }));
    }

    #[test]
    fn test_scan_cadence_after_support() {
        let mut b = TacticalBrain::new();
        let mut p = plan(true);
        p.support = [Some(0), None];
        let anchor = Vector2::new(0.0, 0.0);
        let mut pos = Vector2::new(-8.0, 0.0);
        let mut planted = false;
        for _ in 0..600 {
            let positions = vec![pos];
            let home = vec![true];
            let (t, _s) = eval(&mut b, PlayerRole::Midfielder, pos, anchor, &p, &positions, &home);
            pos = t;
            if b.scan_timer > 0.0 {
                planted = true;
                break;
            }
        }
        assert!(planted, "never entered the stop-and-scan cadence");
    }

    #[test]
    fn test_build_plan_limits_activations() {
        let positions: Vec<Vector2> = (0..10).map(|i| Vector2::new(i as f32, 0.0)).collect();
        let is_home = vec![true; 10];
        let roles = vec![PlayerRole::Midfielder; 10];
        let attributes = vec![PlayerAttributes::default(); 10];
        let plan = build_plan(
            ctx(false),
            &positions,
            &is_home,
            &roles,
            &attributes,
            Vector2::zero(),
            true,
        );
        assert!(plan.presser.is_some());
        assert!(plan.cover.is_some());
        assert_ne!(plan.presser, plan.cover);
    }

    #[test]
    fn test_step_agents_is_deterministic() {
        let build = || {
            let mut a = PlayerAgent::new("a".into(), "h".into(), 1, Vector2::new(-10.0, 0.0), false);
            a.role = PlayerRole::Defender;
            let mut b = PlayerAgent::new("b".into(), "h".into(), 2, Vector2::new(-5.0, 5.0), false);
            b.role = PlayerRole::Midfielder;
            let mut c = PlayerAgent::new("c".into(), "a".into(), 3, Vector2::new(5.0, 0.0), false);
            c.role = PlayerRole::Forward;
            vec![a, b, c]
        };
        let styles = [TeamStyle::default(); 2];
        let lines = [-30.0, 30.0];
        let run = |agents: &mut Vec<PlayerAgent>| {
            let anchors: HashMap<String, Vector2> =
                agents.iter().map(|a| (a.id.clone(), a.position)).collect();
            let lookup: HashMap<u64, (Vector2, f32)> = HashMap::new();
            for _ in 0..300 {
                step_agents(
                    agents,
                    &anchors,
                    &lookup,
                    0.0,
                    Vector2::zero(),
                    Some("h"),
                    "h",
                    &styles,
                    &lines,
                    false,
                    1.0 / 60.0,
                );
            }
        };
        let mut a = build();
        let mut b = build();
        run(&mut a);
        run(&mut b);
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.position, y.position);
            assert_eq!(x.state, y.state);
        }
    }

    #[test]
    fn test_receiver_task_routes_agent_to_landing() {
        let mut agents = vec![PlayerAgent::new(
            "p".into(),
            "h".into(),
            9,
            Vector2::new(-30.0, 0.0),
            false,
        )];
        agents[0].receiver_task = Some((Vector2::new(20.0, 0.0), 5.0));
        let anchors: HashMap<String, Vector2> =
            agents.iter().map(|a| (a.id.clone(), a.position)).collect();
        let lookup: HashMap<u64, (Vector2, f32)> = HashMap::new();
        let styles = [TeamStyle::default(); 2];
        let lines = [-30.0, 30.0];
        let start_x = agents[0].position.x;
        for _ in 0..60 {
            step_agents(
                &mut agents,
                &anchors,
                &lookup,
                0.0,
                Vector2::new(20.0, 0.0),
                Some("h"),
                "h",
                &styles,
                &lines,
                false,
                1.0 / 60.0,
            );
        }
        assert!(
            agents[0].position.x > start_x + 2.0,
            "receiver did not move toward the landing point"
        );
    }
}
