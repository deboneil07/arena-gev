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
use crate::tactics::{Duty, EventCue, Line, LookaheadBlock, TeamContext, TeamPhase, TeamPlan, TeamStyle};
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
        duty: Duty,
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

        // Goalkeepers do not chase outfield feed events. Keep them anchored to
        // their goal; the engine stages the logged action at its own origin.
        let mut upcoming_event = upcoming_event;
        if role == PlayerRole::Goalkeeper {
            if let Some((target, _)) = upcoming_event {
                let own_goal_x = if plan.ctx.is_home {
                    -crate::parser::PITCH_HALF_LENGTH
                } else {
                    crate::parser::PITCH_HALF_LENGTH
                };
                if (target.x - own_goal_x).abs() > 22.0 {
                    upcoming_event = None;
                }
            }
        }

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
                role, duty, attributes, pos, positions, is_home, self_idx, plan,
            );
        }

        // ── 4. EXECUTE the chosen intent with a live target. ──
        let (mut target, mut speed) = self.plan_intent(
            attributes, role, duty, max_speed, max_accel, pos, anchor, ball, positions, is_home,
            self_idx, plan,
        );

        // ── 5. PRE-POSITION — an event is known but not yet urgent, so drift
        // toward it now. By the time the sprint is needed the player is already
        // close, which is how real players "read the game" early.
        if let Some((ev, t)) = upcoming_event {
            if t > 0.0 && t <= PRE_POSITION_SECS && !plan.ctx.set_piece {
                let line = Line::from_role(role);
                // Defenders hold the line and only move on an urgent event;
                // pre-positioning them would drag the back four out of shape.
                if !matches!(line, Line::Defense | Line::Goalkeeper) {
                    let zone = zone_radius(role, plan.ctx.style);
                    let ev_target = clamp_to_zone(ev, anchor, zone * 1.15);
                    let w =
                        ((PRE_POSITION_SECS - t) / PRE_POSITION_SECS).clamp(0.0, 1.0) * 0.45;
                    target = target + (ev_target - target) * w;
                    if speed < 1.0 && w > 0.08 {
                        speed = jog_speed(Line::from_role(role), plan.ctx.style);
                    }
                }
            }
        }

        // A move that is not itself a scheduled event may never drift beyond
        // the offside line. Event-bound players are exempt: the feed's spot
        // is ground truth and the dispatcher has already started their run.
        if plan.ctx.in_possession
            && upcoming_event.is_none()
            && !matches!(self.current_intent, TacticalIntent::AnticipateEvent { .. })
        {
            target = clamp_onside(
                target,
                plan.ctx.offside_limit_x,
                plan.ctx.attack_dx,
                0.05,
            );
        }

        (target, speed)
    }

    /// Choose the intent for this commitment window.
    #[allow(clippy::too_many_arguments)]
    fn choose_intent(
        &self,
        role: PlayerRole,
        duty: Duty,
        _attributes: &PlayerAttributes,
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

        // Everyone else takes the one-to-one mark the solver handed them,
        // provided it is still inside their zone. Presser and cover have no
        // mark, so the whole team can never converge on one opponent.
        if let Some(mark) = plan.mark_for(self_idx) {
            if let Some(&target) = positions.get(mark) {
                if pos.distance(target) <= mark_radius(duty) + 2.5 {
                    return TacticalIntent::ManMark { target };
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
        duty: Duty,
        max_speed: f32,
        max_accel: f32,
        pos: Vector2,
        anchor: Vector2,
        ball: Vector2,
        positions: &[Vector2],
        is_home: &[bool],
        self_idx: usize,
        plan: &TeamPlan,
    ) -> (Vector2, f32) {
        let style = plan.ctx.style;
        let line = Line::from_role(role);
        let zone = zone_radius(role, style);
        let clamped = |t: Vector2| clamp_to_zone(t, anchor, zone);
        // Every player reads the same team lookahead block: the next pass or
        // landing point his side must react to, and how soon. The weight is
        // the player's own travel-time runway, so he starts drifting early
        // instead of exploding out of shape when the ball is played.
        let event = event_anticipation(
            anchor,
            role,
            duty,
            plan,
            pos,
            max_speed,
            max_accel,
            attributes.anticipation,
        );

        match self.current_intent {
            TacticalIntent::AnticipateEvent { target } => {
                if pos.distance(target) < 0.4 {
                    (pos, 0.0)
                } else {
                    (target, max_speed)
                }
            }
            TacticalIntent::HoldShape => {
                let base = shape_target(anchor, duty, role, plan);
                let d0 = pos.distance(base);
                let mut speed = 0.0;
                if d0 >= 0.6 {
                    // Recover faster the further out of position we are, so a
                    // back line that has been turned can sprint back into shape.
                    let mut urgency = (1.0 + d0 / 7.0).min(2.4);
                    if line == Line::Defense
                        && pos.x * plan.ctx.attack_dx > plan.ctx.ball_x * plan.ctx.attack_dx
                    {
                        // Caught ahead of the ball: sprint back immediately.
                        urgency = 3.0;
                    }
                    speed = jog_speed(line, style) * plan.ctx.phase.tempo() * urgency;
                }
                let (target, speed) =
                    apply_event(base, speed, event, pos, max_speed, 1.0);
                if pos.distance(target) < 0.6 {
                    (pos, 0.0) // in position: hold the line
                } else {
                    (target, speed)
                }
            }
            TacticalIntent::ManMark { .. } => {
                let marked = plan
                    .mark_for(self_idx)
                    .and_then(|j| positions.get(j).copied())
                    .or_else(|| {
                        nearest_opponent(pos, positions, is_home, plan.ctx.is_home)
                            .map(|(p, _)| p)
                    });
                if let Some(opp) = marked {
                    let d = pos.distance(opp);
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
                // Screen the space between the ball and our goal, then shade
                // toward the opponent's next landing spot: the cover man is
                // the one who reads the pass before it is played.
                let goal_dir = (plan.ctx.own_goal() - ball).normalize();
                let base = clamped(ball + goal_dir * 5.0);
                apply_event(base, 3.6, event, pos, max_speed, 0.45)
            }
            TacticalIntent::SupportTriangle { target } => {
                let base = clamped(target);
                // Support stays a passing option first, but leans toward the
                // next landing point so the triangle is already forming when
                // the pass leaves the boot.
                let (target, speed) =
                    apply_event(base, 3.2 + style.tempo * 0.6, event, pos, max_speed, 0.55);
                if pos.distance(target) < 0.8 {
                    self.scan_timer = 0.9; // arrived: plant and scan
                    (pos, 0.0)
                } else {
                    (target, speed)
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

/// Pull a target back to the legal side of the offside line, keeping its
/// lateral coordinate. `margin` leaves a small cushion so numerical jitter
/// cannot put a toe beyond the line.
#[inline]
fn clamp_onside(target: Vector2, limit_x: f32, attack_dx: f32, margin: f32) -> Vector2 {
    if !limit_x.is_finite() {
        return target;
    }
    let bound = limit_x - attack_dx * margin;
    let x = if attack_dx > 0.0 {
        target.x.min(bound)
    } else {
        target.x.max(bound)
    };
    Vector2::new(x, target.y)
}

/// The team's next meaningful event, as seen by every player on the side.
///
/// In possession the cue is the team's own next pass/landing; out of
/// possession it is the opponent's. `None` means the next logged beat is not
/// a ball event or is too far away to matter yet.
#[inline]
fn team_event_cue(plan: &TeamPlan) -> Option<EventCue> {
    let block = plan.ctx.lookahead;
    let (next, landing) = if plan.ctx.in_possession {
        (block.team, block.team_landing)
    } else {
        (block.opponent, block.opponent_landing)
    };
    match next {
        // The landing is the moment that matters for a pass: use it.
        Some(n) if n.is_pass() => landing.or(Some(n)),
        // A shot/tackle/loose ball is the next beat; a later pass landing
        // must not shadow it.
        Some(n) => Some(n),
        None => landing,
    }
}

/// How strongly a player should already be moving toward the shared event
/// spot, from 0 (hold shape) to 1 (fully committed).
///
/// The trigger is the player's own travel time: `start` is the moment the
/// remaining clock first allows him to arrive with a small settling cushion.
/// Better anticipation buys a slightly longer runway, so good readers leave
/// a beat earlier without ever sprinting from a standing start.
fn event_anticipation(
    anchor: Vector2,
    role: PlayerRole,
    duty: Duty,
    plan: &TeamPlan,
    pos: Vector2,
    max_speed: f32,
    max_accel: f32,
    anticipation: f32,
) -> Option<(Vector2, f32, f32)> {
    let ctx = plan.ctx;
    // Set pieces are staged deliberately; nobody freelances before the ball
    // is live or the wall and box slots would dissolve.
    if ctx.set_piece {
        return None;
    }
    let cue = team_event_cue(plan)?;
    let time_until = cue.time - ctx.now;
    if !(0.05..=PRE_POSITION_SECS).contains(&time_until) {
        return None;
    }
    let target = event_shape_target(anchor, role, duty, plan, cue)?;
    let dist = pos.distance(target);
    if dist < 0.4 {
        return None;
    }
    let travel = arrival_time(dist, max_speed.max(3.0), max_accel.max(1.0));
    let cushion = 0.55 + anticipation.clamp(0.0, 1.0) * 0.35;
    let start = travel + cushion;
    let ramp = (start - cushion).max(0.5);
    let weight = ((start - time_until) / ramp).clamp(0.0, 1.0);
    if weight <= 0.0 {
        return None;
    }
    Some((target, weight, time_until))
}

/// Blend the current shape target toward the shared event spot and raise the
/// approach speed just enough to cover the remaining distance on time.
///
/// `scale` keeps secondary roles (support, cover) from abandoning their
/// primary job: they only lean toward the cue, while a player holding shape
/// can commit fully.
fn apply_event(
    target: Vector2,
    speed: f32,
    event: Option<(Vector2, f32, f32)>,
    pos: Vector2,
    max_speed: f32,
    scale: f32,
) -> (Vector2, f32) {
    let Some((spot, weight, time_until)) = event else {
        return (target, speed);
    };
    let w = (weight * scale).clamp(0.0, 1.0);
    if w <= 1e-3 {
        return (target, speed);
    }
    let blended = target + (spot - target) * w;
    let remaining = (time_until - 0.30).max(0.20);
    let needed = pos.distance(blended) / remaining;
    (blended, speed.max(needed.min(max_speed)))
}

/// Where a player should be when the next logged ball event arrives.
///
/// This is the off-ball layer's answer to "what is this pass for?". In
/// possession the strikers attack the box, the No.10 and midfielders form
/// the supporting triangle, the ball-side full-back overlaps and the holder
/// screens for the second ball. Out of possession everyone screens the
/// opponent's landing spot from the goal side, with defenders restricted to
/// a lateral shuffle so the back line never leaves its height.
fn event_shape_target(
    anchor: Vector2,
    role: PlayerRole,
    duty: Duty,
    plan: &TeamPlan,
    cue: EventCue,
) -> Option<Vector2> {
    let ctx = plan.ctx;
    if duty == Duty::Goalkeeper {
        return None;
    }
    let attack = ctx.attack_dx;
    let goal_x = attack * crate::parser::PITCH_HALF_LENGTH;
    let spot = cue.spot();
    let progress = spot.x * attack;
    let goal_dir = (ctx.own_goal() - spot).normalize();
    let zone = zone_radius(role, ctx.style) * ctx.phase.zone_scale();

    let mut target = if ctx.in_possession {
        match duty {
            Duty::Striker => {
                if progress > 25.0 {
                    // Attack the six-yard box, near post when the ball comes
                    // from wide, and split the posts when two strikers are
                    // central so they do not run into the same spot.
                    let post = if spot.y.abs() > 9.0 {
                        spot.y.signum() * 7.0
                    } else if anchor.y.abs() > 2.0 {
                        anchor.y.signum() * 4.0
                    } else {
                        0.0
                    };
                    Vector2::new(goal_x - attack * 9.0, post)
                } else if progress > -5.0 {
                    // Check toward the pass and stay available as the wall
                    // pass / lay-off option.
                    Vector2::new(spot.x + attack * 7.0, anchor.y * 0.85 + spot.y * 0.10)
                } else {
                    anchor
                }
            }
            Duty::AttackingMid => {
                if progress > 20.0 {
                    // Arrive underneath the landing point for the cut-back.
                    spot + goal_dir * 8.0
                } else {
                    Vector2::new(spot.x - attack * 7.0, anchor.y * 0.75 + spot.y * 0.15)
                }
            }
            Duty::Winger => {
                let weak = spot.y.abs() > 8.0 && spot.y.signum() != anchor.y.signum();
                if progress > 22.0 && weak {
                    // Weak-side winger attacks the far post when the ball is
                    // about to arrive in the box.
                    Vector2::new(goal_x - attack * 8.0, -spot.y.signum() * 7.0)
                } else if progress > -8.0 {
                    Vector2::new(spot.x + attack * 4.0, anchor.y)
                } else {
                    anchor
                }
            }
            Duty::CentralMid => {
                // Support pocket behind and to the side of the landing spot.
                Vector2::new(spot.x - attack * 6.0, anchor.y * 0.72 + spot.y * 0.12)
            }
            Duty::FullBack => {
                let same_flank = spot.y.signum() == anchor.y.signum();
                if same_flank && progress > -5.0 {
                    Vector2::new(spot.x + attack * 3.0, anchor.y)
                } else {
                    Vector2::new(anchor.x + attack * 0.5, anchor.y * 0.90)
                }
            }
            Duty::DefensiveMid => {
                // Screen the space the second ball will drop into.
                Vector2::new(spot.x - attack * 11.0, anchor.y * 0.55)
            }
            Duty::CentreBack => {
                Vector2::new(anchor.x, anchor.y + (spot.y - anchor.y) * 0.08)
            }
            _ => anchor,
        }
    } else {
        // Defending: get goal-side of the opponent's landing spot. The back
        // line only shuffles laterally; its height belongs to the block.
        let threat = if cue.is_shot() { cue.origin } else { spot };
        let screen = threat + goal_dir * 4.0;
        match duty {
            Duty::DefensiveMid => screen,
            Duty::CentralMid => anchor + (screen - anchor) * 0.40,
            Duty::AttackingMid => {
                anchor + (screen - anchor) * 0.25 - Vector2::new(attack * 1.0, 0.0)
            }
            Duty::Winger => {
                anchor + (screen - anchor) * 0.18 - Vector2::new(attack * 0.8, 0.0)
            }
            Duty::Striker => {
                anchor + (screen - anchor) * 0.10 - Vector2::new(attack * 0.5, 0.0)
            }
            Duty::CentreBack => {
                Vector2::new(anchor.x, anchor.y + (threat.y - anchor.y) * 0.30)
            }
            Duty::FullBack => {
                Vector2::new(anchor.x, anchor.y + (threat.y - anchor.y) * 0.25)
            }
            _ => anchor,
        }
    };

    // The back line's height is owned by the block, and a midfield bank's
    // depth is what makes it visible as a bank (4-2-3-1 double pivot,
    // 4-3-3 triangle). Event anticipation may shuffle those players
    // laterally, but never collapse their depth: only forwards attack a new
    // line on the cue.
    if duty.line() == Line::Defense || role == PlayerRole::Midfielder {
        target.x = anchor.x;
    }
    let clamped = clamp_to_zone(target, anchor, zone);
    if clamped.is_finite() {
        Some(clamped)
    } else {
        None
    }
}

/// Ball-relative micro-shape around the team anchor.
///
/// The elastic grid gives the team its macro block. This adds the details a
/// real side shows inside that block: the weak-side winger tucks in, the
/// ball-side full-back overlaps, the No.10 drifts into the half-space, and
/// the holding midfielder screens behind the ball. Defenders never change
/// their line's x — only their lateral slot — so the back four stays flat.
fn shape_target(anchor: Vector2, duty: Duty, role: PlayerRole, plan: &TeamPlan) -> Vector2 {
    let ctx = plan.ctx;
    let ball = Vector2::new(ctx.ball_x, ctx.ball_y);
    let phase = ctx.phase;
    let mut t = anchor;

    if duty == Duty::Goalkeeper {
        return anchor;
    }

    let side = if anchor.y >= 0.0 { 1.0 } else { -1.0 };
    let ball_central = ball.y.abs() < 4.0;
    let ball_side = if ball.y >= 0.0 { 1.0 } else { -1.0 };
    let weak = !ball_central && anchor.y.abs() > 10.0 && side != ball_side;
    let attack = ctx.attack_dx;

    if ctx.in_possession {
        match duty {
            Duty::Winger => {
                if weak {
                    // Weak-side winger: narrow, then attack the back post in
                    // the final third instead of hugging the far touchline.
                    t.y = anchor.y * 0.70;
                    t.x += attack * if phase == TeamPhase::FinalThird { 4.5 } else { 1.0 };
                } else {
                    t.y = anchor.y * 1.06 + (ball.y - anchor.y) * 0.05;
                    t.x += attack * 1.5;
                }
            }
            Duty::AttackingMid => {
                t.y += (ball.y - anchor.y) * 0.16;
                t.x += attack
                    * match phase {
                        TeamPhase::FinalThird => 3.0,
                        TeamPhase::AttackingTransition => 2.5,
                        TeamPhase::BuildUp => 0.0,
                        _ => 1.5,
                    };
            }
            Duty::Striker => {
                if phase == TeamPhase::FinalThird {
                    t.x += attack * 2.5;
                    t.y += (ball.y * 0.25 - t.y) * 0.18;
                } else {
                    t.y += (ball.y - anchor.y) * 0.06;
                }
            }
            Duty::FullBack => {
                if weak {
                    t.y = anchor.y * 0.84;
                } else {
                    t.y = anchor.y * 1.03 + (ball.y - anchor.y) * 0.04;
                    t.x += attack * 3.0;
                }
            }
            Duty::DefensiveMid => {
                t.y += (ball.y - anchor.y) * 0.12;
                if phase == TeamPhase::BuildUp {
                    t.x -= attack * 1.5;
                }
            }
            Duty::CentralMid => {
                t.y += (ball.y - anchor.y) * 0.12;
                if phase == TeamPhase::FinalThird {
                    t.x += attack * 2.0;
                }
            }
            Duty::CentreBack => {
                t.y += (ball.y - anchor.y) * 0.05;
            }
            _ => {}
        }
    } else {
        match duty {
            Duty::Winger => {
                if weak {
                    t.y = anchor.y * 0.66;
                    t.x -= attack * 1.0;
                } else {
                    t.y = anchor.y * 1.02;
                }
            }
            Duty::AttackingMid => {
                t.y += (ball.y - anchor.y) * 0.18;
                t.x -= attack * 1.0;
            }
            Duty::Striker => {
                t.y += (ball.y - anchor.y) * 0.08;
                if phase == TeamPhase::LowBlock {
                    t.x -= attack * 2.0;
                }
            }
            Duty::FullBack => {
                if weak {
                    t.y = anchor.y * 0.76;
                } else {
                    t.y += (ball.y - anchor.y) * 0.05;
                }
            }
            Duty::DefensiveMid => {
                t.y += (ball.y - anchor.y) * 0.10;
                if weak {
                    t.y -= side * 1.0;
                }
            }
            Duty::CentralMid => {
                t.y += (ball.y - anchor.y) * 0.10;
                if weak {
                    t.y -= side * 1.5;
                }
            }
            Duty::CentreBack => {
                t.y += (ball.y - anchor.y) * 0.04;
            }
            _ => {}
        }
    }

    // The back line's height belongs to the line block, never to micro-shape.
    if duty.line() == Line::Defense {
        t.x = anchor.x;
    }

    clamp_to_zone(
        t,
        anchor,
        zone_radius(role, ctx.style) * phase.zone_scale(),
    )
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

/// Maximum distance at which a duty will pick up a man. Beyond this a player
/// stays in his zone instead of chasing a stranger across the pitch.
fn mark_radius(duty: Duty) -> f32 {
    match duty {
        Duty::Goalkeeper => 0.0,
        Duty::CentreBack => 13.5,
        Duty::FullBack => 17.0,
        Duty::DefensiveMid => 17.0,
        Duty::CentralMid => 15.0,
        Duty::AttackingMid => 13.0,
        Duty::Winger => 17.0,
        Duty::Striker => 9.0,
        Duty::Unknown => 11.0,
    }
}

/// One-to-one man-marking assignments for the defending team.
///
/// Greedy over the cheapest legal `(marker, opponent)` pairs, which is enough
/// for 11 v 11 and keeps the result deterministic. A pair is legal when the
/// opponent is within the marker's duty radius and is level with or goal-side
/// of him — chasing a man back toward his own goal is pressing, not marking.
/// The presser and cover are excluded because the ball already occupies them.
fn assign_marks(
    ctx: TeamContext,
    positions: &[Vector2],
    is_home: &[bool],
    available: &[bool],
    duties: &[Duty],
    attributes: &[PlayerAttributes],
    presser: Option<usize>,
    cover: Option<usize>,
    team_home: bool,
) -> [Option<usize>; 22] {
    let mut marks = [None; 22];
    if ctx.in_possession || positions.len() < 2 {
        return marks;
    }

    struct Pair {
        cost: f32,
        marker: usize,
        opponent: usize,
    }
    let mut pairs: Vec<Pair> = Vec::new();

    for (i, p) in positions.iter().enumerate() {
        if !available[i] || is_home[i] != team_home || i >= marks.len() {
            continue;
        }
        if Some(i) == presser || Some(i) == cover {
            continue;
        }
        let duty = duties.get(i).copied().unwrap_or(Duty::Unknown);
        let radius = mark_radius(duty);
        if radius <= 0.0 {
            continue;
        }

        for (j, q) in positions.iter().enumerate() {
            if !available[j] || is_home[j] == team_home {
                continue;
            }
            let opp_duty = duties.get(j).copied().unwrap_or(Duty::Unknown);
            if opp_duty == Duty::Goalkeeper {
                continue;
            }
            let dist = p.distance(*q);
            if dist > radius {
                continue;
            }
            // `upfield > 0` means the opponent is further from our own goal
            // than the marker is — the marker is goal-side, which is exactly
            // where a defender wants to be. A metre of tolerance lets him
            // stay with a man who is level and just starting to spin in
            // behind.
            let upfield = (q.x - p.x) * ctx.attack_dx;
            if upfield < -1.0 {
                continue;
            }

            let lateral = (p.y - q.y).abs();
            let mut cost = dist
                + duty.mark_fit(opp_duty) * 0.55
                + lateral * 0.18
                + (-upfield).max(0.0) * 0.35
                - attributes[i].positioning * 1.2;

            // Keep central defenders central and full-backs wide unless the
            // distance really demands a switch.
            if duty.is_central() && opp_duty.is_wide() {
                cost += 3.0;
            }
            if duty.is_wide() && opp_duty.is_central() {
                cost += 2.0;
            }
            // A forward should not be dragged into a back-line marking job.
            if duty == Duty::Striker && opp_duty.line() == Line::Defense {
                cost += 6.0;
            }

            pairs.push(Pair {
                cost,
                marker: i,
                opponent: j,
            });
        }
    }

    pairs.sort_by(|a, b| {
        a.cost
            .partial_cmp(&b.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.marker.cmp(&b.marker))
            .then_with(|| a.opponent.cmp(&b.opponent))
    });

    let mut taken = [false; 22];
    let mut claimed = [false; 22];
    for pair in pairs {
        if pair.marker >= marks.len() || pair.opponent >= taken.len() {
            continue;
        }
        if taken[pair.marker] || claimed[pair.opponent] {
            continue;
        }
        taken[pair.marker] = true;
        claimed[pair.opponent] = true;
        marks[pair.marker] = Some(pair.opponent);
    }
    marks
}

/// Build the per-team activation plan (presser / cover / support / runner).
///
/// This is where "the team plays as one" is enforced: only a handful of
/// players are ever allowed to leave their shape at a time.
#[allow(clippy::too_many_arguments)]
fn build_plan(
    ctx: TeamContext,
    positions: &[Vector2],
    is_home: &[bool],
    available: &[bool],
    roles: &[PlayerRole],
    duties: &[Duty],
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
        if !available[i] || is_home[i] != team_home || roles[i] == PlayerRole::Goalkeeper {
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
        let duty_bias = duties.get(i).copied().unwrap_or(Duty::Unknown).press_bias();
        let press_cost = dist + role_bias - duty_bias * 0.5 - attributes[i].aggression * 4.0;
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
            if !available[i] || is_home[i] != team_home || roles[i] == PlayerRole::Goalkeeper || i == pi {
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
    let presser_idx = presser.map(|(_, i)| i);
    let cover_idx = cover.map(|(_, i)| i);
    let marks = assign_marks(
        ctx,
        positions,
        is_home,
        available,
        duties,
        attributes,
        presser_idx,
        cover_idx,
        team_home,
    );
    TeamPlan {
        ctx,
        presser: presser_idx,
        cover: cover_idx,
        support,
        runner: runner.map(|(_, i)| i),
        marks,
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
    phases: &[TeamPhase; 2],
    lookahead: &[LookaheadBlock; 2],
    hold_lines: &[f32; 2],
    set_piece: bool,
    set_piece_taker: Option<&str>,
    dt: f32,
) -> [Option<TeamPlan>; 2] {
    // Snapshot positions/teams so every agent reads a consistent world.
    // Sent-off players are excluded from plans: they walk to the bench and
    // the side plays short-handed.
    let positions: Vec<Vector2> = agents.iter().map(|a| a.position).collect();
    let is_home: Vec<bool> = agents.iter().map(|a| a.team_id == home_team_id).collect();
    let available: Vec<bool> = agents.iter().map(|a| !a.sent_off).collect();
    let roles: Vec<PlayerRole> = agents.iter().map(|a| a.role).collect();
    let duties: Vec<Duty> = agents
        .iter()
        .map(|a| {
            if a.duty == Duty::Unknown {
                Duty::from_role(a.role, a.position.y)
            } else {
                a.duty
            }
        })
        .collect();
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
        // Dismissed players walking off cannot play anyone onside.
        let offside_limit_x = offside_limit_world(
            positions
                .iter()
                .zip(is_home.iter())
                .zip(available.iter())
                .filter(|((_, _), a)| **a)
                .map(|((p, h), _)| (p.x, *h)),
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
            phase: phases[team],
            lookahead: lookahead[team],
            now: sim_time,
            style: styles[team],
        };
        plans[team] = Some(build_plan(
            ctx,
            &positions,
            &is_home,
            &available,
            &roles,
            &duties,
            &attributes,
            ball,
            team_home,
        ));
    }

    for (i, agent) in agents.iter_mut().enumerate() {
        let anchor = anchors.get(&agent.id).copied().unwrap_or(Vector2::zero());
        let attributes = agent.attributes; // Copy
        let role = agent.role;
        let duty = duties[i];
        let max_speed = agent.max_speed_sprint * agent.sprint_boost;
        let max_accel = agent.max_accel;
        let pos = agent.position;
        let team_home = agent.team_id == home_team_id;
        let plan = plans[usize::from(!team_home)].as_ref().unwrap();

        // Prefer whichever deadline is sooner: the player's own scheduled
        // action, or a pass they are routed to receive.
        let own_raw = agent.assigned_event_id.and_then(|eid| {
            event_lookup
                .get(&eid)
                .map(|(t, tm)| (*t, tm - sim_time))
        });
        let recv_raw = agent
            .receiver_task
            .filter(|(_, deadline)| sim_time <= *deadline + 0.5)
            .map(|(target, deadline)| (target, deadline - sim_time));
        // During a set piece, non-takers must not sprint onto the restart spot:
        // only the taker plays the ball. But a non-taker whose own event is
        // elsewhere (e.g. a cross from the box) still has to reach it, so we
        // only suppress routing when the upcoming event sits on the ball.
        let own_origin_on_ball = own_raw
            .as_ref()
            .map(|(t, _)| t.distance(ball) < 3.0)
            .unwrap_or(false);
        let recv_on_ball = recv_raw.is_some(); // receiver tasks are always ball-bound
        let hold_set_piece = false;
        let own = if hold_set_piece { None } else { own_raw };
        let recv = if hold_set_piece { None } else { recv_raw };
        let upcoming = match (own, recv) {
            (Some(o), Some(r)) => Some(if o.1 < r.1 { o } else { r }),
            (Some(o), None) => Some(o),
            (None, Some(r)) => Some(r),
            (None, None) => None,
        };

        let (target, target_speed) = agent.brain.evaluate_intent(
            dt,
            &attributes,
            role,
            duty,
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
                // `assigned_event_id` covers the whole lookahead window, not
                // just the instant the brain commits to `AnticipateEvent`.
                if agent.assigned_event_id.is_some()
                    || agent.receiver_task.is_some()
                    || matches!(
                        agent.brain.current_intent,
                        TacticalIntent::AnticipateEvent { .. }
                    )
                {
                    continue;
                }
                let attack_dx = if is_home[i] { 1.0 } else { -1.0 };
                let limit = offside_limit_world(
                    agents
                        .iter()
                        .filter(|a| !a.sent_off)
                        .map(|a| (a.position.x, a.team_id == home_team_id)),
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
    plans
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
            phase: TeamPhase::MidBlock,
            lookahead: LookaheadBlock::default(),
            now: 0.0,
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
            marks: [None; 22],
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
            Duty::from_role(role, anchor.y),
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
            Duty::CentreBack,
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
            Duty::Striker,
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
    fn test_unassigned_player_does_not_man_mark() {
        // A defender standing a metre from an opponent used to pick him up
        // individually. Marking must now come from the team's one-to-one
        // assignment, otherwise two defenders can converge on the same man.
        let mut b = TacticalBrain::new();
        let p = plan(false);
        let pos = vec![Vector2::new(-30.0, 0.0), Vector2::new(-29.0, 0.5)];
        let home = vec![true, false];
        let _ = eval(&mut b, PlayerRole::Defender, pos[0], pos[0], &p, &pos, &home);
        assert!(matches!(b.current_intent, TacticalIntent::HoldShape));
    }

    #[test]
    fn test_assigned_player_takes_his_mark() {
        let mut b = TacticalBrain::new();
        let mut p = plan(false);
        p.marks[0] = Some(1);
        let pos = vec![Vector2::new(-30.0, 0.0), Vector2::new(-27.0, 4.0)];
        let home = vec![true, false];
        let _ = eval(&mut b, PlayerRole::Defender, pos[0], pos[0], &p, &pos, &home);
        assert!(matches!(b.current_intent, TacticalIntent::ManMark { .. }));
    }

    #[test]
    fn test_marks_are_one_to_one_and_role_weighted() {
        let positions = vec![
            Vector2::new(-20.0, 0.0),   // 0 home DM (presser)
            Vector2::new(-22.0, 6.0),   // 1 home DM (cover)
            Vector2::new(-32.0, -8.0),  // 2 home CB
            Vector2::new(-32.0, 8.0),   // 3 home CB
            Vector2::new(-26.0, -20.0), // 4 home LB
            Vector2::new(-26.0, 20.0),  // 5 home RB
            Vector2::new(-28.0, -6.0),  // 6 away ST
            Vector2::new(-27.0, 7.0),   // 7 away ST
            Vector2::new(-24.0, -19.0), // 8 away LW
            Vector2::new(-25.0, 21.0),  // 9 away RW
        ];
        let is_home = vec![true, true, true, true, true, true, false, false, false, false];
        let duties = vec![
            Duty::DefensiveMid,
            Duty::DefensiveMid,
            Duty::CentreBack,
            Duty::CentreBack,
            Duty::FullBack,
            Duty::FullBack,
            Duty::Striker,
            Duty::Striker,
            Duty::Winger,
            Duty::Winger,
        ];
        let attributes = vec![PlayerAttributes::default(); positions.len()];
        let available = vec![true; positions.len()];
        let marks = assign_marks(
            ctx(false),
            &positions,
            &is_home,
            &available,
            &duties,
            &attributes,
            Some(0),
            Some(1),
            true,
        );

        // The presser and cover are occupied with the ball, not a man.
        assert_eq!(marks[0], None);
        assert_eq!(marks[1], None);
        // Centre-backs take the strikers, full-backs the wingers.
        assert_eq!(marks[2], Some(6));
        assert_eq!(marks[3], Some(7));
        assert_eq!(marks[4], Some(8));
        assert_eq!(marks[5], Some(9));

        let mut claimed: Vec<usize> = marks.iter().flatten().copied().collect();
        let count = claimed.len();
        claimed.sort_unstable();
        claimed.dedup();
        assert_eq!(claimed.len(), count, "an opponent was marked twice: {marks:?}");
    }

    #[test]
    fn test_marks_are_empty_in_possession_and_for_the_goalkeeper() {
        let positions = vec![
            Vector2::new(-40.0, 0.0),
            Vector2::new(-28.0, -4.0),
            Vector2::new(-28.0, 4.0),
        ];
        let is_home = vec![true, true, false];
        let duties = vec![Duty::Goalkeeper, Duty::CentreBack, Duty::Striker];
        let attributes = vec![PlayerAttributes::default(); 3];
        let available = vec![true; positions.len()];
        let marks = assign_marks(
            ctx(true),
            &positions,
            &is_home,
            &available,
            &duties,
            &attributes,
            None,
            None,
            true,
        );
        assert_eq!(marks, [None; 22]);

        let available = vec![true; positions.len()];
        let marks = assign_marks(
            ctx(false),
            &positions,
            &is_home,
            &available,
            &duties,
            &attributes,
            None,
            None,
            true,
        );
        assert_eq!(marks[0], None, "the goalkeeper must never be a marker");
        assert_eq!(marks[1], Some(2));
    }

    #[test]
    fn test_shape_target_tucks_in_the_weak_side_winger() {
        let mut p = plan(true);
        p.ctx.ball_y = 18.0;
        p.ctx.phase = TeamPhase::Progression;
        let weak_anchor = Vector2::new(-5.0, -18.0);
        let weak = shape_target(weak_anchor, Duty::Winger, PlayerRole::Midfielder, &p);
        assert!(
            weak.y.abs() < 18.0 * 0.8,
            "weak-side winger did not tuck in: {weak:?}"
        );

        let strong_anchor = Vector2::new(-5.0, 18.0);
        let strong = shape_target(strong_anchor, Duty::Winger, PlayerRole::Midfielder, &p);
        assert!(
            strong.y.abs() >= 18.0 * 0.95,
            "ball-side winger gave up the touchline: {strong:?}"
        );
        assert!(strong.x > strong_anchor.x, "ball-side winger did not advance");
    }

    #[test]
    fn test_weak_side_winger_attacks_the_back_post_in_the_final_third() {
        let mut p = plan(true);
        p.ctx.ball_y = 18.0;
        p.ctx.phase = TeamPhase::Progression;
        let anchor = Vector2::new(-5.0, -18.0);
        let progression = shape_target(anchor, Duty::Winger, PlayerRole::Midfielder, &p);

        p.ctx.phase = TeamPhase::FinalThird;
        let final_third = shape_target(anchor, Duty::Winger, PlayerRole::Midfielder, &p);
        assert!(
            final_third.x > progression.x + 1.0,
            "weak-side winger did not attack the box: {progression:?} → {final_third:?}"
        );
    }

    #[test]
    fn test_shape_target_never_moves_the_back_line() {
        let mut p = plan(true);
        p.ctx.ball_x = 30.0;
        p.ctx.ball_y = 25.0;
        p.ctx.phase = TeamPhase::FinalThird;
        for duty in [Duty::CentreBack, Duty::FullBack] {
            let anchor = Vector2::new(-25.0, if duty == Duty::CentreBack { -4.0 } else { -18.0 });
            let target = shape_target(anchor, duty, PlayerRole::Defender, &p);
            assert_eq!(
                target.x, anchor.x,
                "{duty:?} stepped off the back line: {target:?}"
            );
        }
    }

    #[test]
    fn test_shape_target_stays_inside_the_phase_zone() {
        let mut p = plan(false);
        p.ctx.ball_x = 10.0;
        p.ctx.ball_y = 30.0;
        p.ctx.phase = TeamPhase::LowBlock;
        let anchor = Vector2::new(-10.0, 18.0);
        let target = shape_target(anchor, Duty::Winger, PlayerRole::Forward, &p);
        let zone = zone_radius(PlayerRole::Forward, p.ctx.style) * p.ctx.phase.zone_scale();
        assert!(
            target.distance(anchor) <= zone + 1e-3,
            "shape target escaped its zone: {target:?} from {anchor:?} (r={zone})"
        );
    }

    #[test]
    fn test_build_plan_limits_activations() {
        let positions: Vec<Vector2> = (0..10).map(|i| Vector2::new(i as f32, 0.0)).collect();
        let is_home = vec![true; 10];
        let available = vec![true; 10];
        let roles = vec![PlayerRole::Midfielder; 10];
        let duties = vec![Duty::CentralMid; 10];
        let attributes = vec![PlayerAttributes::default(); 10];
        let plan = build_plan(
            ctx(false),
            &positions,
            &is_home,
            &available,
            &roles,
            &duties,
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
                    &[TeamPhase::MidBlock; 2],
                    &[LookaheadBlock::default(); 2],
                    &lines,
                    false,
                    None,
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
                &[TeamPhase::MidBlock; 2],
                &[LookaheadBlock::default(); 2],
                &lines,
                false,
                None,
                1.0 / 60.0,
            );
        }
        assert!(
            agents[0].position.x > start_x + 2.0,
            "receiver did not move toward the landing point"
        );
    }

    fn cue(origin: Vector2, target: Vector2, time: f32, type_id: u16) -> EventCue {
        EventCue {
            origin,
            target: Some(target),
            time,
            type_id,
        }
    }

    #[test]
    fn test_event_weight_uses_travel_time_not_distance_alone() {
        let mut p = plan(true);
        p.ctx.phase = TeamPhase::Progression;
        p.ctx.now = 0.0;
        let landing = cue(
            Vector2::new(-5.0, 0.0),
            Vector2::new(30.0, 10.0),
            3.0,
            1,
        );
        p.ctx.lookahead.team_landing = Some(landing);
        p.ctx.lookahead.team = Some(landing);

        // A striker far from the box needs to leave now; one already on the
        // edge of the box can wait a beat and still arrive in time.
        let far = event_anticipation(
            Vector2::new(5.0, 0.0),
            PlayerRole::Forward,
            Duty::Striker,
            &p,
            Vector2::new(5.0, 0.0),
            8.5,
            6.0,
            0.5,
        );
        let near = event_anticipation(
            Vector2::new(40.0, 8.0),
            PlayerRole::Forward,
            Duty::Striker,
            &p,
            Vector2::new(40.0, 8.0),
            8.5,
            6.0,
            0.5,
        );
        let (_, w, _) = far.expect("far striker should already be moving");
        assert!(w > 0.0 && w <= 1.0, "bad event weight {w}");
        assert!(
            near.is_none(),
            "a striker already at the box should not be dragged early"
        );
    }

    #[test]
    fn test_event_weight_ramps_smoothly_as_the_clock_runs_down() {
        let mut p = plan(true);
        p.ctx.phase = TeamPhase::Progression;
        let spot = Vector2::new(30.0, 10.0);
        let pos = Vector2::new(5.0, 0.0);
        let mut probe = |until: f32| {
            p.ctx.now = 0.0;
            let cue = cue(Vector2::new(-5.0, 0.0), spot, until, 1);
            p.ctx.lookahead.team_landing = Some(cue);
            p.ctx.lookahead.team = Some(cue);
            event_anticipation(
                Vector2::new(5.0, 0.0),
                PlayerRole::Forward,
                Duty::Striker,
                &p,
                pos,
                8.5,
                6.0,
                0.5,
            )
            .map(|(_, w, _)| w)
        };

        assert!(probe(5.0).is_none(), "movement started too early");
        let mut previous = 0.0;
        for until in [4.0, 3.0, 2.0, 1.0, 0.4] {
            let w = probe(until).unwrap_or_else(|| panic!("no movement at {until}s"));
            assert!(w >= previous, "weight went backwards at {until}s: {previous} → {w}");
            assert!(w - previous < 0.6, "weight jumped at {until}s: {previous} → {w}");
            previous = w;
        }
        assert!(previous > 0.8, "late weight too weak: {previous}");
    }

    #[test]
    fn test_event_target_keeps_the_back_line_height() {
        let mut p = plan(false);
        p.ctx.phase = TeamPhase::MidBlock;
        let cue = cue(
            Vector2::new(5.0, 25.0),
            Vector2::new(-10.0, 20.0),
            2.0,
            1,
        );
        p.ctx.lookahead.opponent_landing = Some(cue);
        p.ctx.lookahead.opponent = Some(cue);

        let anchor = Vector2::new(-25.0, -4.0);
        let target = event_shape_target(
            anchor,
            PlayerRole::Defender,
            Duty::CentreBack,
            &p,
            cue,
        )
        .expect("defender should react to the opponent landing");
        assert!(
            (target.x - anchor.x).abs() < 1e-4,
            "centre-back left the line: {target:?} vs {anchor:?}"
        );
        assert!(target.y > anchor.y, "centre-back did not shuffle across");

        let anchor = Vector2::new(-24.0, 18.0);
        let target = event_shape_target(
            anchor,
            PlayerRole::Defender,
            Duty::FullBack,
            &p,
            cue,
        )
        .expect("full-back should react to the opponent landing");
        assert!(
            (target.x - anchor.x).abs() < 1e-4,
            "full-back left the line: {target:?} vs {anchor:?}"
        );
    }

    #[test]
    fn test_event_targets_never_escape_the_phase_zone() {
        let anchors = [
            Vector2::new(-30.0, -4.0),
            Vector2::new(-24.0, 18.0),
            Vector2::new(-8.0, 2.0),
            Vector2::new(5.0, -18.0),
            Vector2::new(18.0, 0.0),
        ];
        let duties = [
            (Duty::CentreBack, PlayerRole::Defender),
            (Duty::FullBack, PlayerRole::Defender),
            (Duty::DefensiveMid, PlayerRole::Midfielder),
            (Duty::CentralMid, PlayerRole::Midfielder),
            (Duty::AttackingMid, PlayerRole::Midfielder),
            (Duty::Winger, PlayerRole::Forward),
            (Duty::Striker, PlayerRole::Forward),
        ];
        for in_possession in [false, true] {
            let mut p = plan(in_possession);
            p.ctx.phase = TeamPhase::FinalThird;
            let spot = Vector2::new(35.0, 14.0);
            let c = cue(Vector2::new(10.0, 8.0), spot, 2.0, 1);
            if in_possession {
                p.ctx.lookahead.team_landing = Some(c);
            } else {
                p.ctx.lookahead.opponent_landing = Some(c);
            }
            let radius =
                zone_radius(PlayerRole::Forward, p.ctx.style) * p.ctx.phase.zone_scale();
            for anchor in anchors {
                for (duty, role) in duties {
                    let target =
                        event_shape_target(anchor, role, duty, &p, c).expect("no target");
                    let zone =
                        zone_radius(role, p.ctx.style) * p.ctx.phase.zone_scale();
                    assert!(
                        target.distance(anchor) <= zone + 1e-3,
                        "{duty:?} escaped its zone: {target:?} from {anchor:?} (r={zone}, overall {radius})"
                    );
                }
            }
        }
    }

    #[test]
    fn test_hold_shape_moves_early_toward_the_next_landing() {
        let mut b = TacticalBrain::new();
        let mut p = plan(true);
        p.ctx.phase = TeamPhase::Progression;
        p.ctx.ball_x = 10.0;
        p.ctx.ball_y = 0.0;
        p.ctx.offside_limit_x = crate::parser::PITCH_HALF_LENGTH;
        p.ctx.now = 0.0;
        let landing = cue(Vector2::new(5.0, 0.0), Vector2::new(30.0, 8.0), 2.5, 1);
        p.ctx.lookahead.team_landing = Some(landing);
        p.ctx.lookahead.team = Some(landing);

        let anchor = Vector2::new(5.0, 0.0);
        let pos = vec![anchor, Vector2::new(30.0, 8.0)];
        let home = vec![true, false];
        let (target, speed) = eval(
            &mut b,
            PlayerRole::Forward,
            anchor,
            anchor,
            &p,
            &pos,
            &home,
        );
        let base = shape_target(anchor, Duty::Striker, PlayerRole::Forward, &p);
        assert!(
            target.distance(base) > 1.0,
            "non-tracked striker ignored the shared lookahead: {target:?} vs {base:?}"
        );
        assert!(
            target.x > base.x,
            "movement was not toward the next landing: {target:?} vs {base:?}"
        );
        assert!(speed > 0.0, "player started early but stood still");
        assert!(matches!(b.current_intent, TacticalIntent::HoldShape));
    }

    #[test]
    fn test_event_conditioned_defender_only_shuffles_laterally() {
        let mut b = TacticalBrain::new();
        let mut p = plan(false);
        p.ctx.now = 0.0;
        let landing = cue(Vector2::new(5.0, 25.0), Vector2::new(-8.0, 22.0), 2.0, 1);
        p.ctx.lookahead.opponent_landing = Some(landing);
        p.ctx.lookahead.opponent = Some(landing);

        let anchor = Vector2::new(-25.0, 0.0);
        let pos = vec![anchor, Vector2::new(-8.0, 22.0)];
        let home = vec![true, false];
        let (target, _) = eval(
            &mut b,
            PlayerRole::Defender,
            anchor,
            anchor,
            &p,
            &pos,
            &home,
        );
        assert!(
            (target.x - anchor.x).abs() < 1e-4,
            "defender stepped off the line reacting to the pass: {target:?}"
        );
        assert!(target.y > 0.5, "defender did not tuck toward the threat");
    }

    #[test]
    fn test_set_piece_disables_event_anticipation() {
        let mut p = plan(true);
        p.ctx.phase = TeamPhase::Progression;
        p.ctx.set_piece = true;
        p.ctx.now = 0.0;
        let landing = cue(Vector2::new(-5.0, 0.0), Vector2::new(30.0, 10.0), 1.0, 1);
        p.ctx.lookahead.team_landing = Some(landing);
        assert!(event_anticipation(
            Vector2::new(5.0, 0.0),
            PlayerRole::Forward,
            Duty::Striker,
            &p,
            Vector2::new(5.0, 0.0),
            8.5,
            6.0,
            0.5,
        )
        .is_none());
    }
}
