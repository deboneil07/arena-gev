//! Outcome-aware event graph — what each on-ball event *resolves into*.
//!
//! The dispatcher used to treat every pass as completable and every shot as a
//! generic launch. The feed already knows better (`outcome`, reason
//! qualifiers, link ids), so this module resolves each event to its true
//! endpoint *before* anyone is routed or anything flies:
//!
//! * completed pass → the receiver's touch (next same-team event, or the
//!   nearest role-biased teammate as fallback)
//! * failed pass → the interception point (next opponent touch on the lane,
//!   else the nearest defender projected onto the lane, else the
//!   touchline/out spot) — never a team-mate running onto it
//! * shot → goal / saved / post / miss branch from the type id
//! * cross → a target *zone*, not necessarily a player
//!
//! Pure geometry over the event list (no agents): the dispatcher and the
//! engine refine the points with live positions.

use crate::formation::Vector2;
use crate::parser::{NormalizedEvent, OutcomeReason};

/// How close (seconds) the next event must be to count as the reception.
pub const RECEPTION_WINDOW_SECS: f32 = 8.0;
/// How close (seconds) an opponent's next touch must be to own the
/// interception.
pub const INTERCEPT_WINDOW_SECS: f32 = 4.0;
/// How close (metres) a pass landing point may sit to the receiver's own
/// next-event origin before they are considered the same spot.
pub const RECEIVER_ORIGIN_TOL: f32 = 1.5;

/// What a pass resolves into.
#[derive(Debug, Clone, PartialEq)]
pub enum PassResolution {
    /// Completed: the ball goes to `target`, owned by receiver event `index`.
    Complete { receiver_idx: usize },
    /// Completed but nobody in particular owns it (dead heat fallback).
    CompleteLoose,
    /// Failed: intercepted — the ball ends at the interceptor's touch.
    Intercept { by_idx: usize },
    /// Failed: intercepted by an unnamed defender at `point` on the lane.
    InterceptPoint { point: Vector2 },
    /// Failed: out of play at `point` (touchline clamp).
    Out { point: Vector2 },
}

/// What a shot resolves into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotResolution {
    Goal,
    Saved,
    Post,
    Miss,
}

impl ShotResolution {
    /// From the feed's type id — the truth, not a guess.
    #[inline]
    pub fn from_type(type_id: u16) -> Option<Self> {
        match type_id {
            16 => Some(ShotResolution::Goal),
            15 => Some(ShotResolution::Saved),
            14 => Some(ShotResolution::Post),
            13 => Some(ShotResolution::Miss),
            _ => None,
        }
    }
}

/// Resolve a pass event at `i` to its endpoint.
pub fn resolve_pass(events: &[NormalizedEvent], i: usize) -> PassResolution {
    let event = &events[i];
    let target = match (event.target_x, event.target_y) {
        (Some(x), Some(y)) => Vector2::new(x, y),
        _ => return PassResolution::CompleteLoose,
    };

    if is_completed_pass(event) {
        // The Opta reception: the next same-team touch owns it.
        if let Some(j) = next_touch(events, i, &event.team_id, &event.player_id) {
            return PassResolution::Complete { receiver_idx: j };
        }
        return PassResolution::CompleteLoose;
    }

    // Failed pass: the next opponent touch within the window owns the
    // interception — that is where the ball must end, not at the original
    // target with a team-mate running onto it.
    if event.type_id == 2 {
        // Offside passes never complete: hold at the flag spot.
        return PassResolution::Out {
            point: target.clamp_on_pitch(),
        };
    }
    if let Some(j) = next_opponent_touch(events, i, &event.team_id) {
        return PassResolution::Intercept { by_idx: j };
    }
    match event.outcome_reason {
        OutcomeReason::Out => PassResolution::Out {
            point: target.clamp_on_pitch(),
        },
        _ => PassResolution::InterceptPoint {
            point: lane_intercept_point(
                Vector2::new(event.origin_x, event.origin_y),
                target,
            ),
        },
    }
}

/// True when the feed says this pass reached a team-mate.
#[inline]
pub fn is_completed_pass(event: &NormalizedEvent) -> bool {
    is_completed_pass_like(event.type_id, event.outcome, event.outcome_reason)
}

/// Predicate version for dispatched actions (which carry the decision, not
/// the whole event).
#[inline]
pub fn is_completed_pass_like(type_id: u16, outcome: bool, reason: OutcomeReason) -> bool {
    type_id == 1 && outcome && reason != OutcomeReason::Offside
}

/// Next event by the same team (different player) within the window.
fn next_touch(
    events: &[NormalizedEvent],
    i: usize,
    team_id: &str,
    player_id: &str,
) -> Option<usize> {
    events
        .iter()
        .enumerate()
        .skip(i + 1)
        .take_while(|(_, e)| e.match_time_secs - events[i].match_time_secs < RECEPTION_WINDOW_SECS)
        .find(|(_, e)| {
            e.team_id == team_id && !e.player_id.is_empty() && e.player_id != player_id
        })
        .map(|(j, _)| j)
}

/// Next opponent touch (interception / tackle / recovery class) within the
/// intercept window.
fn next_opponent_touch(events: &[NormalizedEvent], i: usize, team_id: &str) -> Option<usize> {
    events
        .iter()
        .enumerate()
        .skip(i + 1)
        .take_while(|(_, e)| {
            e.match_time_secs - events[i].match_time_secs < INTERCEPT_WINDOW_SECS
        })
        .find(|(_, e)| {
            e.team_id != team_id
                && !e.player_id.is_empty()
                && matches!(e.type_id, 7 | 8 | 12 | 45 | 49 | 61 | 74)
        })
        .map(|(j, _)| j)
}

/// Where an unnamed defender meets a failed pass: 60% along the lane, kept
/// on the pitch. The engine refines it with live defender positions.
fn lane_intercept_point(origin: Vector2, target: Vector2) -> Vector2 {
    (origin + (target - origin) * 0.6).clamp_on_pitch()
}

/// Project `p` onto the segment `a→b` (clamped): the defender's meeting
/// point with the pass lane.
pub fn project_on_lane(p: Vector2, a: Vector2, b: Vector2) -> Vector2 {
    let ab = b - a;
    let len_sq = ab.length_sq();
    if len_sq < 1e-6 {
        return a;
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    a + ab * t
}

#[cfg(test)]
mod event_graph_tests {
    use super::*;
    use crate::parser::NormalizedEvent;

    fn pass(id: u64, time: f32, team: &str, player: &str, outcome: bool) -> NormalizedEvent {
        let mut ev = NormalizedEvent::test(id, 1, time, team, player, 0.0, 0.0);
        ev.target_x = Some(20.0);
        ev.target_y = Some(0.0);
        ev.outcome = outcome;
        ev
    }

    fn touch(id: u64, time: f32, type_id: u16, team: &str, player: &str) -> NormalizedEvent {
        NormalizedEvent::test(id, type_id, time, team, player, 20.0, 0.0)
    }

    #[test]
    fn test_completed_pass_links_next_touch() {
        let events = vec![
            pass(1, 1.0, "h", "p1", true),
            touch(2, 1.6, 1, "h", "p2"),
        ];
        assert_eq!(
            resolve_pass(&events, 0),
            PassResolution::Complete { receiver_idx: 1 }
        );
    }

    #[test]
    fn test_failed_pass_links_interceptor() {
        let mut failed = pass(1, 1.0, "h", "p1", false);
        failed.outcome_reason = OutcomeReason::Intercepted;
        let events = vec![failed, touch(2, 1.6, 8, "a", "d1")];
        assert_eq!(
            resolve_pass(&events, 0),
            PassResolution::Intercept { by_idx: 1 }
        );
    }

    #[test]
    fn test_failed_pass_without_interceptor_goes_to_lane_point() {
        let mut failed = pass(1, 1.0, "h", "p1", false);
        failed.outcome_reason = OutcomeReason::Intercepted;
        let events = vec![failed, touch(2, 6.0, 1, "h", "p2")];
        match resolve_pass(&events, 0) {
            PassResolution::InterceptPoint { point } => {
                assert!((point.x - 12.0).abs() < 0.01, "point {point:?}");
            }
            other => panic!("expected lane point, got {other:?}"),
        }
    }

    #[test]
    fn test_out_pass_goes_to_touchline() {
        let mut failed = pass(1, 1.0, "h", "p1", false);
        failed.outcome_reason = OutcomeReason::Out;
        failed.target_x = Some(60.0);
        let events = vec![failed];
        match resolve_pass(&events, 0) {
            PassResolution::Out { point } => {
                assert!(point.x.abs() <= 52.5, "point {point:?}");
            }
            other => panic!("expected out, got {other:?}"),
        }
    }

    #[test]
    fn test_shot_resolution_from_type() {
        assert_eq!(ShotResolution::from_type(16), Some(ShotResolution::Goal));
        assert_eq!(ShotResolution::from_type(15), Some(ShotResolution::Saved));
        assert_eq!(ShotResolution::from_type(14), Some(ShotResolution::Post));
        assert_eq!(ShotResolution::from_type(13), Some(ShotResolution::Miss));
        assert_eq!(ShotResolution::from_type(1), None);
    }

    #[test]
    fn test_project_on_lane() {
        let a = Vector2::new(0.0, 0.0);
        let b = Vector2::new(10.0, 0.0);
        assert_eq!(
            project_on_lane(Vector2::new(5.0, 3.0), a, b),
            Vector2::new(5.0, 0.0)
        );
        assert_eq!(
            project_on_lane(Vector2::new(15.0, 0.0), a, b),
            Vector2::new(10.0, 0.0)
        );
    }
}
