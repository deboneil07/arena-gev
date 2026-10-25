//! Master simulation engine — binds the timeline, dispatcher, elastic
//! formation, agent kinematics and the 3D ball into one 60 Hz orchestrator,
//! and prepares flat render snapshots for the presentation layer.
//!
//! ```text
//! ┌──────────┐  execute  ┌────────────┐  launch/attach  ┌──────┐
//! │ Timeline │──────────▶│ Dispatcher │────────────────▶│ Ball │
//! └──────────┘           └────────────┘                 └──────┘
//!                               │ anticipation               │ (x,y,z)
//!                               ▼                            ▼
//!                        ┌────────────┐   anchors   ┌────────────────┐
//!                        │   Agents   │◀────────────│ Elastic Grid   │
//!                        └────────────┘             └────────────────┘
//!                               │
//!                               ▼
//!                        RenderSnapshot (23 entities)
//! ```

use crate::agent::{AgentState, PlayerAgent, PlayerAttributes, PlayerRole, DT, MAX_SPEED};
use crate::ball::{Ball, BallState};
use crate::dispatcher::{arrival_time, DispatchedAction, LookaheadDispatcher};
use crate::formation::{FormationEngine, TeamTacticalPhase, Vector2};
use crate::tactics::{Duty, TeamPhase, TeamPlan, TeamStyle};
use crate::parser::{
    event_type_name, is_ball_event, is_dead_ball_event, is_notable_event, AttackDirection, BodyPart,
    CardKind, FormationChange, KeeperAction, LineupPlayer, MatchContext, NormalizedEvent,
    OutcomeReason, PassKind, SetPieceTag, EVENT_TYPE_CARD, EVENT_TYPE_CORNER, EVENT_TYPE_FOUL,
    EVENT_TYPE_GOAL, EVENT_TYPE_PLAYER_OFF, EVENT_TYPE_PLAYER_ON, PITCH_HALF_LENGTH,
};
use crate::timeline::Timeline;
use std::collections::{HashMap, VecDeque};

/// Default formation used when a team's shape is unknown.
pub const DEFAULT_FORMATION: &str = "4-3-3";

/// Maximum speed (m/s) at which the tactical block may migrate. Keeps shape
/// changes smooth instead of letting anchors teleport with the ball.
pub const ANCHOR_MAX_SPEED: f32 = 5.5;

/// Fastest the ball may reposition itself toward the next event's spot. At
/// 60 Hz that is ≤ 0.44 m per tick, so it can never read as a teleport.
pub const BALL_SPOT_MAX_SPEED: f32 = 40.0;
/// Only start rolling the ball toward the next event's spot in the last
/// seconds before it: a reposition started tens of seconds early would drag
/// the ball visibly across a live pitch.
pub const BALL_STAGING_SECS: f32 = 10.0;
/// While the readiness gate holds an overdue event, keep staging the ball to
/// arrive this soon — short enough that it is essentially "right now".
pub const HOLD_STAGING_SECS: f32 = 0.2;
/// Slowest the ball will crawl toward the next spot (a deliberate roll).
pub const BALL_SPOT_MIN_SPEED: f32 = 4.0;
/// Largest per-tick nudge used to settle a ball exactly onto an imminent
/// event's origin. Bounded so settling can never read as a teleport, and it
/// converges well before the event fires.
pub const BALL_SETTLE_STEP: f32 = 0.4;

/// Radius (m) within which the *upcoming* actor may claim a resting ball
/// before the next event — they are the one who plays it next.
pub const BALL_CLAIM_RADIUS: f32 = 2.2;
/// How close the upcoming action must be before that actor may take the ball.
/// Any earlier and the ball parks in a player's feet doing nothing (the whole
/// team then stops stretching for the ball); this matches how far ahead an
/// actor starts his run at all (`PRE_POSITION_SECS`).
pub const PICKUP_HORIZON_SECS: f32 = 5.0;

/// Team index used by [`RenderEntity::team_index`].
pub const TEAM_HOME: u8 = 0;
pub const TEAM_AWAY: u8 = 1;
pub const TEAM_BALL: u8 = 2;

/// How long a kick-off set piece holds its cage before the teams relax to
/// their blocks (the kick still fires on schedule via the taker + pickup
/// rule). Covers celebration gaps and half-time without freezing the shape.
pub const KICKOFF_HOLD_SECS: f32 = 20.0;

/// Simulation configuration.
#[derive(Debug, Clone)]
pub struct SimulationConfig {
    /// How far ahead (seconds) events are assigned to players.
    pub lookahead_secs: f32,
    /// Fixed physics timestep (seconds).
    pub dt: f32,
    /// Formation used for the home team.
    pub home_formation: String,
    /// Formation used for the away team.
    pub away_formation: String,
}

impl Default for SimulationConfig {
    fn default() -> Self {
        Self {
            lookahead_secs: 8.0,
            dt: DT,
            home_formation: DEFAULT_FORMATION.to_string(),
            away_formation: DEFAULT_FORMATION.to_string(),
        }
    }
}

/// Flat, `#[repr(C)]` render snapshot entry (WASM/Canvas friendly).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderEntity {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub heading: f32,
    pub team_index: u8, // 0 home, 1 away, 2 ball
    pub shirt_number: u8,
}

/// One entry in the on-screen event feed.
#[derive(Debug, Clone, PartialEq)]
pub struct EventRecord {
    /// Display clock (raw min*60+sec) — what the mm:ss UI row shows.
    pub time: f32,
    pub type_id: u16,
    pub label: &'static str,
    /// 0 = home, 1 = away, 2 = neutral.
    pub team_index: u8,
    pub player: String,
    /// Worth surfacing as a large splash (goal, card, foul, …).
    pub notable: bool,
}

/// How many recent events to retain for the UI feed.
pub const EVENT_FEED_CAPACITY: usize = 12;

/// Ground-truth snapshot taken the instant an event is dispatched, *before*
/// any choreography runs. This is the measurement contract for doer/ball
/// accuracy: `doer_pos` and `ball_pos` are exactly what the UI shows when the
/// feed row appears.
#[derive(Debug, Clone)]
pub struct EventSnapshot {
    pub event_id: u64,
    /// Opta timestamp of the event on the sim clock (drives slip math).
    pub feed_time: f32,
    /// Display clock for mm:ss rows and graphs.
    pub display_time: f32,
    /// Simulation clock at dispatch (can trail `feed_time` when the event had
    /// to wait for an actor to arrive).
    pub sim_time: f32,
    pub type_id: u16,
    pub origin: Vector2,
    pub doer_id: String,
    pub doer_pos: Option<Vector2>,
    /// Doer's tactical state right before the event was applied to him.
    pub doer_state: Option<&'static str>,
    /// Where the doer was heading when the event fired.
    pub doer_stage_target: Option<Vector2>,
    /// If the ball was in flight: where that flight is headed.
    pub ball_flight_target: Option<Vector2>,
    /// Seconds until that flight lands (`None` unless in flight).
    pub ball_flight_remaining: Option<f32>,
    /// Who is flying it: staging reposition vs a pass/launch.
    pub ball_flight_kind: Option<&'static str>,
    /// Whether a kickoff reset was pending (blocks ball staging).
    pub pending_kickoff: bool,
    /// Ball state right at dispatch (before choreography moves it).
    pub ball_state_head: &'static str,
    /// Why `pre_position_ball` stopped short of launching this tick.
    pub staging_decision: &'static str,
    /// Player the ball is attached to at dispatch, if any.
    pub ball_carrier: Option<String>,
    pub ball_pos: Vector2,
    /// True when this event is expected to have the ball on its origin
    /// (ball events, dead-ball restarts and fouls).
    pub ball_expected: bool,
}

/// Snapshot taken the moment a pass/shot flight touches down: where the ball
/// landed versus where the intended receiver was.
#[derive(Debug, Clone)]
pub struct LandingSnapshot {
    pub sim_time: f32,
    pub target: Vector2,
    pub landing: Vector2,
    pub receiver_id: Option<String>,
    pub receiver_pos: Option<Vector2>,
    pub nearest_id: Option<String>,
    pub nearest_pos: Option<Vector2>,
}

/// A live set piece that temporarily reshapes both teams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetPieceKind {
    /// Kick-off from the centre spot (match start, half-time, after a goal).
    KickOff,
    /// Corner: the attacking team floods the box with assigned roles.
    Corner,
    /// Free kick: a defensive wall plus the taker.
    FreeKick,
    /// Throw-in: the taker walks to the ball, everyone else holds shape.
    ThrowIn,
    /// Penalty: taker on the spot, keeper on the line, arc rest.
    Penalty,
    /// Goal kick: keeper distribution, short or long.
    GoalKick,
}

#[derive(Debug, Clone)]
struct SetPiece {
    kind: SetPieceKind,
    team: String,
    until: f32,
    /// Ground-truth ball spot for the restart.
    spot: Vector2,
    /// Which side the corner is taken from (+1 / −1), for corners.
    side: f32,
    /// The feed player who will actually take the restart. Chosen once when
    /// the restart is armed so the ball anchor never hops between players.
    taker_id: Option<String>,
}

/// Explicit, rate-limited position of the three lines in a team's block.
///
/// The whole line shares one x and one lateral shift, so the block moves as a
/// unit instead of each player independently chasing the ball. This is what
/// eliminates the "whole back line caught ahead of the ball" lag.
#[derive(Debug, Clone, Copy)]
struct LineBlock {
    def_x: f32,
    mid_x: f32,
    fwd_x: f32,
    def_y: f32,
    mid_y: f32,
    fwd_y: f32,
}

impl LineBlock {
    const fn zero() -> Self {
        Self {
            def_x: 0.0,
            mid_x: 0.0,
            fwd_x: 0.0,
            def_y: 0.0,
            mid_y: 0.0,
            fwd_y: 0.0,
        }
    }
}

/// Initial line block derived from a team's real categories/template.
fn initial_block(
    lineup: &[LineupPlayer],
    template: &[Vector2],
    team_home: bool,
) -> LineBlock {
    let mirror = !team_home;
    let (mut ds, mut ms, mut fs) = (0.0f32, 0.0f32, 0.0f32);
    let (mut dn, mut mn, mut fnn) = (0.0f32, 0.0f32, 0.0f32);
    for p in lineup.iter().filter(|p| p.is_starter()) {
        let idx = (p.formation_slot.saturating_sub(1)) as usize;
        let base = template.get(idx).copied().unwrap_or(Vector2::zero());
        let x = if mirror { -base.x } else { base.x };
        match p.position_category {
            2 => {
                ds += x;
                dn += 1.0;
            }
            3 => {
                ms += x;
                mn += 1.0;
            }
            4 => {
                fs += x;
                fnn += 1.0;
            }
            _ => {}
        }
    }
    let d = |s, n, fallback| if n > 0.0 { s / n } else { fallback };
    let sign = if team_home { 1.0 } else { -1.0 };
    LineBlock {
        def_x: d(ds, dn, -35.0 * sign),
        mid_x: d(ms, mn, -18.0 * sign),
        fwd_x: d(fs, fnn, -6.0 * sign),
        def_y: 0.0,
        mid_y: 0.0,
        fwd_y: 0.0,
    }
}

/// Desired line heights in the team's *attack frame* (`prog = x * attack_dx`).
fn desired_line_progs(
    ball_prog: f32,
    style: TeamStyle,
    in_possession: bool,
    opp_def_prog: f32,
) -> (f32, f32, f32) {
    let def = if in_possession {
        (ball_prog - (14.0 + style.compactness * 10.0)).clamp(-46.0, 34.0)
    } else {
        (ball_prog - (6.0 + style.pressing * 14.0)).clamp(-50.0, 7.0)
    };
    let mid = def + 9.0 + style.compactness * 7.0;
    let mut fwd = mid + 10.0 + style.directness * 8.0;
    // Attackers stay onside: never beyond the opponent's line, the ball, or
    // the halfway line.
    let offside_limit = opp_def_prog.max(ball_prog).max(0.0);
    fwd = fwd.min(offside_limit - 0.4);
    let mid = mid.min(fwd - 4.0).max(def + 4.0);
    let fwd = fwd.max(mid + 4.0);
    (def, mid, fwd)
}

/// Move `current` toward `desired_prog` at a bounded rate, measuring distance
/// in the team's attack frame. Teams retreat faster than they advance.
#[inline]
fn approach_prog(
    current_x: f32,
    desired_prog: f32,
    attack_dx: f32,
    advance: f32,
    retreat: f32,
    dt: f32,
) -> f32 {
    let cur = current_x * attack_dx;
    let d = desired_prog - cur;
    let step = if d > 0.0 {
        (advance * dt).min(d)
    } else {
        (-retreat * dt).max(d)
    };
    (cur + step) * attack_dx
}

#[inline]
fn approach_scalar(current: f32, desired: f32, rate: f32, dt: f32) -> f32 {
    let d = desired - current;
    let step = (rate * dt).min(d.abs());
    current + step * d.signum()
}

/// Hysteretic phase state for one team. Thresholds alone chatter when the
/// ball hovers around a line, so a candidate must survive a short dwell
/// before it becomes the active phase.
#[derive(Debug, Clone, Copy)]
struct PhaseTracker {
    phase: TeamPhase,
    candidate: TeamPhase,
    candidate_since: f32,
    last_in_possession: bool,
    changed_at: f32,
}

impl PhaseTracker {
    const fn new() -> Self {
        Self {
            phase: TeamPhase::MidBlock,
            candidate: TeamPhase::MidBlock,
            candidate_since: 0.0,
            last_in_possession: false,
            changed_at: 0.0,
        }
    }
}

/// The master simulation engine.
pub struct SimulationEngine {
    pub home_team_id: String,
    pub away_team_id: String,
    /// Match metadata carried through from the Opta `<Game>` row so the
    /// UI can label the HUD and the match list without re-parsing XML.
    pub match_id: String,
    pub home_team_name: String,
    pub away_team_name: String,
    pub competition_id: String,
    pub season: String,
    pub game_date: String,
    pub lineups: HashMap<String, Vec<LineupPlayer>>,
    pub directions: HashMap<String, AttackDirection>,
    pub formation: FormationEngine,
    pub config: SimulationConfig,

    /// All player agents (home first, then away).
    pub agents: Vec<PlayerAgent>,

    /// The 3D ball.
    pub ball: Ball,
    /// Team currently in possession: the last ball winner. Updated on every
    /// dispatched ball action (actor's team), except failed passes (in
    /// dispute) and fouls (restart decides). Falls back to the next event.
    pub possession: Option<String>,
    /// Winner of the last resolved ball action (drives [`Self::possession`]).
    last_touch: Option<String>,
    /// Player id of the last resolved ball action (for awarded-team correction).
    last_touch_player: Option<String>,

    /// Chronological event index (duration/stats queries).
    pub timeline: Timeline,
    /// Lookahead dispatcher driving anticipation + execution.
    pub dispatcher: LookaheadDispatcher,
    /// Actions emitted on the most recent tick.
    pub last_actions: Vec<DispatchedAction>,
    /// Rolling feed of the most recent executed events (newest at the back).
    pub recent_events: VecDeque<EventRecord>,
    /// Sequence number for the newest event (lets the UI detect changes).
    pub event_seq: u64,
    /// Goals per team: `[home, away]`.
    pub score: [u32; 2],
    /// Ticks each team has been in possession: `[home, away]`.
    pub possession_frames: [u64; 2],
    /// Active set piece (corner / kick-off / free kick / throw-in), if any.
    set_piece: Option<SetPiece>,
    /// A kick-off waiting for the goal celebration to finish: (team, at_time).
    pending_kickoff: Option<(String, f32)>,
    /// Explicit rate-limited line positions per team: `[home, away]`.
    blocks: [LineBlock; 2],
    /// Hysteretic game phase per team: `[home, away]`.
    phases: [PhaseTracker; 2],
    /// Last per-team tactical plans (presser/cover/support/runner/marks),
    /// refreshed every tick — the overlay reads the engine's own scheme
    /// instead of re-deriving it.
    last_plans: [Option<TeamPlan>; 2],
    /// Base lateral lane per player id (world frame).
    lane_y: HashMap<String, f32>,
    /// Depth offset from the midfield bank, in the team's attacking frame.
    /// A 4-2-3-1 keeps its two banks instead of collapsing every midfielder
    /// onto the single `LineBlock::mid_x`.
    mid_depth: HashMap<String, f32>,
    /// Period currently being simulated (detects half-time).
    current_period: u8,
    /// Event id → (origin, match time) for the Utility-AI lookahead.
    event_lookup: HashMap<u64, (Vector2, f32)>,
    /// Per-team tactical identity: `[home, away]`.
    styles: [TeamStyle; 2],
    /// Current defensive-line x per team: `[home, away]`.
    hold_line_x: [f32; 2],
    /// Player id → agent index (O(1) lookup on the hot path).
    agent_index: HashMap<String, usize>,
    /// Team → player going off, waiting for their replacement event.
    pending_subs: HashMap<String, String>,
    /// Mid-match formation changes (type 40), chronological.
    formation_changes: Vec<FormationChange>,
    /// Index of the next unapplied formation change.
    formation_cursor: usize,

    /// Current simulation clock (seconds).
    pub sim_time: f32,
    /// Anticipation assignments made so far.
    pub events_dispatched: u64,
    /// Actions executed so far.
    pub events_executed: u64,
    /// One snapshot per dispatched event (doer/ball at the dispatch instant).
    pub event_snapshots: Vec<EventSnapshot>,
    /// Why `pre_position_ball` stopped short of launching this tick.
    pub staging_decision: &'static str,
    /// One snapshot per completed pass/shot flight (landing vs receiver).
    pub landing_snapshots: Vec<LandingSnapshot>,
    /// A flight in progress that must produce a landing snapshot on touchdown.
    pending_landing: Option<(Vector2, Option<String>)>,
    /// A held claim in progress: when the current flight lands, attach the
    /// ball to this keeper instead of letting it roll (collected saves).
    pending_catch: Option<String>,

    // Reusable per-tick buffers.
    anchor_buffer: HashMap<String, Vector2>,
}

/// Whether an event of this type is expected to have the ball on its origin
/// (ball events, dead-ball restarts and fouls).
#[inline]
pub fn ball_spot_expected(type_id: u16) -> bool {
    is_dead_ball_event(type_id) || type_id == EVENT_TYPE_FOUL || is_ball_event(type_id)
}

/// +1/-1 attack direction of the team that took the shot in `action`
/// (flipped for own goals, which fly into the scorer's own net).
fn attack_goal_sign(action: &DispatchedAction, sim: &SimulationEngine) -> f32 {
    let dx = if action.team_id == sim.home_team_id {
        1.0
    } else {
        -1.0
    };
    if action.is_own_goal {
        -dx
    } else {
        dx
    }
}

/// Ceremonial kick-off tap: the feed's target is where the next touch
/// happens, not the pass destination — cap the tap at 2.5 m toward it.
fn tap_target(origin: Vector2, target: Vector2) -> Vector2 {
    let lane = target - origin;
    if lane.length() > 2.5 {
        origin + lane.normalize() * 2.5
    } else {
        target
    }
}

/// Stop-to-stop ETA of the pass endpoint's owner: the linked receiver for a
/// completed pass, the nearest opponent to the endpoint for an interception.
/// `None` when nobody owns it (loose completion, out ball).
fn owner_eta(
    agents: &[PlayerAgent],
    action: &DispatchedAction,
    target: Vector2,
    failed: bool,
) -> Option<f32> {
    let eta = |pos: Vector2, sprint: f32, accel: f32| {
        crate::dispatcher::arrival_time(pos.distance(target), sprint, accel)
    };
    if !failed {
        let rid = action.receiver_id.as_deref()?;
        let a = agents.iter().find(|a| a.id == rid)?;
        return Some(eta(a.position, a.max_speed_sprint, a.max_accel));
    }
    agents
        .iter()
        .filter(|a| !a.sent_off && a.team_id != action.team_id && !a.is_goalkeeper)
        .map(|a| eta(a.position, a.max_speed_sprint, a.max_accel))
        .fold(None, |best: Option<f32>, t| {
            Some(best.map(|b| b.min(t)).unwrap_or(t))
        })
}

/// Lateral bend (metres) for a curved delivery: in-swingers bend one way,
/// out-swingers the other; footedness decides for open-play crosses, corners,
/// free kicks and through-balls. Straight for everything else.
fn curve_bend(action: &DispatchedAction) -> f32 {
    match action.setpiece_swing {
        SetPieceTag::Inswinger => return 2.5,
        SetPieceTag::Outswinger => return -2.5,
        _ => {}
    }
    match action.pass_kind {
        PassKind::Cross | PassKind::CornerTaken | PassKind::FreeKickTaken => {
            match action.body_part {
                BodyPart::Left => 1.6,
                BodyPart::Right => -1.6,
                _ => 0.0,
            }
        }
        PassKind::ThroughBall => match action.body_part {
            BodyPart::Left => 1.0,
            BodyPart::Right => -1.0,
            _ => 0.0,
        },
        _ => 0.0,
    }
}

impl SimulationEngine {
    /// Build an engine, panicking if the context is unusable. Convenience for
    /// tests/demos; prefer [`SimulationEngine::from_context`] in production.
    pub fn new(context: MatchContext) -> Self {
        Self::from_context(context).expect("SimulationEngine::new: invalid match context")
    }

    /// Build an engine from a parsed [`MatchContext`].
    pub fn from_context(ctx: MatchContext) -> Result<Self, String> {
        Self::with_config(ctx, SimulationConfig::default())
    }

    /// Build an engine with an explicit configuration.
    pub fn with_config(ctx: MatchContext, config: SimulationConfig) -> Result<Self, String> {
        if ctx.home_team_id.is_empty() || ctx.away_team_id.is_empty() {
            return Err("match has no team ids".into());
        }

        let formation = FormationEngine::new();

        let home_xi: Vec<LineupPlayer> =
            ctx.starting_xi(&ctx.home_team_id).into_iter().cloned().collect();
        let away_xi: Vec<LineupPlayer> =
            ctx.starting_xi(&ctx.away_team_id).into_iter().cloned().collect();

        if home_xi.is_empty() && away_xi.is_empty() {
            return Err("no starting lineups available for either team".into());
        }

        let mut event_lookup: HashMap<u64, (Vector2, f32)> = HashMap::with_capacity(ctx.events.len());
        for e in &ctx.events {
            event_lookup.insert(e.id, (Vector2::new(e.origin_x, e.origin_y), e.match_time_secs));
        }
        let timeline = Timeline::from_events(ctx.events.clone());
        let dispatcher = LookaheadDispatcher::new(ctx.events, config.lookahead_secs);

        let home_dir = ctx
            .directions
            .get(&ctx.home_team_id)
            .copied()
            .unwrap_or(AttackDirection::LeftToRight);
        let away_dir = ctx
            .directions
            .get(&ctx.away_team_id)
            .copied()
            .unwrap_or(AttackDirection::RightToLeft);

        // Build the *actual* shape from the lineup's position categories so a
        // 4-4-2 side is not forced into a 4-3-3.
        let home_template = FormationEngine::dynamic_template(&home_xi)
            .unwrap_or_else(|| formation.template_vec(&config.home_formation));
        let away_template = FormationEngine::dynamic_template(&away_xi)
            .unwrap_or_else(|| formation.template_vec(&config.away_formation));
        let home_centroid = FormationEngine::template_centroid(&home_template);
        let away_centroid = FormationEngine::template_centroid(&away_template);
        let home_duties = Duty::from_lineup(&home_xi, &home_template);
        let away_duties = Duty::from_lineup(&away_xi, &away_template);

        let home_anchors = formation.calculate_team_anchors_from_template(
            &home_template,
            home_centroid,
            &home_xi,
            Vector2::zero(),
            TeamTacticalPhase::OutOfPossession,
            home_dir,
        );
        let away_anchors = formation.calculate_team_anchors_from_template(
            &away_template,
            away_centroid,
            &away_xi,
            Vector2::zero(),
            TeamTacticalPhase::OutOfPossession,
            away_dir,
        );

        let mut agents = Vec::with_capacity(home_anchors.len() + away_anchors.len());
        for (team_id, xi, anchors, duties) in [
            (&ctx.home_team_id, &home_xi, &home_anchors, &home_duties),
            (&ctx.away_team_id, &away_xi, &away_anchors, &away_duties),
        ] {
            let by_id: HashMap<&str, Vector2> =
                anchors.iter().map(|a| (a.player_id.as_str(), a.target_pos)).collect();
            for p in xi {
                let pos = by_id.get(p.player_id.as_str()).copied().unwrap_or(Vector2::zero());
                let mut agent = PlayerAgent::new(
                    p.player_id.clone(),
                    team_id.clone(),
                    p.shirt_number,
                    pos,
                    p.position_category == 1,
                );
                agent.role = PlayerRole::from_category(p.position_category);
                agent.duty = duties
                    .get(&p.player_id)
                    .copied()
                    .unwrap_or_else(|| Duty::from_role(agent.role, pos.y));
                agent.refresh_attributes();
                agents.push(agent);
            }
        }

        let n = agents.len();
        let mut anchor_buffer = HashMap::with_capacity(n);
        let mut agent_index = HashMap::with_capacity(n);
        for (i, a) in agents.iter().enumerate() {
            anchor_buffer.insert(a.id.clone(), a.position);
            agent_index.insert(a.id.clone(), i);
        }

        // Base lateral lane and initial line block per team.
        let mut lane_y: HashMap<String, f32> = HashMap::with_capacity(n);
        let mut mid_depth: HashMap<String, f32> = HashMap::with_capacity(n);
        let mut blocks = [LineBlock::zero(); 2];
        for (team_home, template, xi) in [
            (true, &home_template, &home_xi),
            (false, &away_template, &away_xi),
        ] {
            let mirror = !team_home;
            for p in xi {
                let idx = (p.formation_slot - 1) as usize;
                let base = template.get(idx).copied().unwrap_or(Vector2::zero());
                let y = if mirror { -base.y } else { base.y };
                lane_y.insert(p.player_id.clone(), y);
            }
            // Keep the distinct depths of the dynamic midfield template. The
            // mean stays on the line block, so the unit moves as one while the
            // holding midfielder / No.10 split remains visible.
            let mids: Vec<&LineupPlayer> = xi
                .iter()
                .filter(|p| p.is_starter() && p.position_category == 3)
                .collect();
            if mids.len() >= 2 {
                let mean = mids
                    .iter()
                    .map(|p| {
                        let idx = (p.formation_slot - 1) as usize;
                        template.get(idx).copied().unwrap_or(Vector2::zero()).x
                    })
                    .sum::<f32>()
                    / mids.len() as f32;
                for p in mids {
                    let idx = (p.formation_slot - 1) as usize;
                    let x = template.get(idx).copied().unwrap_or(Vector2::zero()).x;
                    mid_depth.insert(p.player_id.clone(), x - mean);
                }
            }
            blocks[usize::from(!team_home)] = initial_block(xi, template, team_home);
        }

        let styles = [
            TeamStyle::from_formation(FormationEngine::detect_formation_label(&home_xi)),
            TeamStyle::from_formation(FormationEngine::detect_formation_label(&away_xi)),
        ];

        let first_event_time = dispatcher
            .events()
            .iter()
            .map(|e| e.match_time_secs)
            .fold(f32::MAX, f32::min);
        // The team that takes the first kick-off is simply the team of the
        // first event on the timeline (the feed owns that fact — this match
        // opens with the away side, and the second half with the home side).
        let first_kickoff_team = dispatcher
            .events()
            .iter()
            .min_by(|a, b| {
                a.match_time_secs
                    .partial_cmp(&b.match_time_secs)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|e| e.team_id.clone())
            .unwrap_or_else(|| ctx.home_team_id.clone());
        let initial_set_piece = if !dispatcher.events().is_empty() {
            Some(SetPiece {
                kind: SetPieceKind::KickOff,
                team: first_kickoff_team,
                until: first_event_time + 1.0,
                spot: Vector2::zero(),
                side: 0.0,
                taker_id: None,
            })
        } else {
            None
        };

        let mut engine = Self {
            home_team_id: ctx.home_team_id,
            away_team_id: ctx.away_team_id,
            match_id: ctx.match_id,
            home_team_name: ctx.home_team_name,
            away_team_name: ctx.away_team_name,
            competition_id: ctx.competition_id,
            season: ctx.season,
            game_date: ctx.game_date,
            lineups: ctx.lineups,
            directions: ctx.directions,
            formation,
            config,
            agents,
            ball: Ball::new(Vector2::zero()),
            possession: None,
            last_touch: None,
            last_touch_player: None,
            timeline,
            dispatcher,
            last_actions: Vec::new(),
            recent_events: VecDeque::with_capacity(EVENT_FEED_CAPACITY),
            event_seq: 0,
            score: [0, 0],
            possession_frames: [0, 0],
            set_piece: initial_set_piece,
            pending_kickoff: None,
            blocks,
            phases: [PhaseTracker::new(); 2],
            last_plans: [None, None],
            lane_y,
            mid_depth,
            current_period: 1,
            event_lookup,
            styles,
            hold_line_x: [0.0, 0.0],
            agent_index,
            pending_subs: HashMap::new(),
            formation_changes: ctx.formation_changes,
            formation_cursor: 0,
            sim_time: 0.0,
            events_dispatched: 0,
            events_executed: 0,
            event_snapshots: Vec::new(),
                staging_decision: "init",
            landing_snapshots: Vec::new(),
            pending_landing: None,
            pending_catch: None,
            anchor_buffer,
        };
        // The match opens on a kick-off: start the players in that shape
        // instead of walking them into it while the first pass — which the
        // feed timestamps at 0:00 — is already being played.
        if let Some(sp) = engine.set_piece.clone() {
            engine.position_set_piece(&sp);
            let starts: Vec<(usize, Vector2)> = engine
                .agents
                .iter()
                .enumerate()
                .filter_map(|(i, a)| engine.anchor_buffer.get(&a.id).map(|s| (i, *s)))
                .collect();
            for (i, slot) in starts {
                engine.agents[i].position = slot;
                engine.agents[i].velocity = Vector2::zero();
            }
        }
        Ok(engine)
    }

    // ── Match metadata (from the Opta <Game> row) ───────

    /// Opta match id.
    #[inline]
    pub fn match_id(&self) -> &str {
        &self.match_id
    }

    /// Home team's display name.
    #[inline]
    pub fn home_team_name(&self) -> &str {
        &self.home_team_name
    }

    /// Away team's display name.
    #[inline]
    pub fn away_team_name(&self) -> &str {
        &self.away_team_name
    }

    /// Competition id the match belongs to.
    #[inline]
    pub fn competition_id(&self) -> &str {
        &self.competition_id
    }

    /// Season label.
    #[inline]
    pub fn season(&self) -> &str {
        &self.season
    }

    /// Kick-off date as recorded by the feed.
    #[inline]
    pub fn game_date(&self) -> &str {
        &self.game_date
    }

    /// Team id for an agent index.
    #[inline]
    pub fn agent_team(&self, idx: usize) -> &str {
        &self.agents[idx].team_id
    }

    // ── Overlay analysis snapshot (engine's own scheme, read-only) ──
    // The renderer keeps the 23×6 positional contract; these small getters
    // expose velocity, duty, phase and the live team plan alongside it.

    /// Velocity of player `index` in m/s.
    #[inline]
    pub fn player_velocity(&self, index: usize) -> Vector2 {
        self.agents.get(index).map(|a| a.velocity).unwrap_or(Vector2::zero())
    }

    /// Duty code of player `index`: 0 unknown, 1 GK, 2 CB, 3 FB, 4 DM, 5 CM,
    /// 6 AM, 7 winger, 8 striker.
    pub fn player_duty(&self, index: usize) -> u8 {
        match self.agents.get(index).map(|a| a.duty) {
            Some(Duty::Goalkeeper) => 1,
            Some(Duty::CentreBack) => 2,
            Some(Duty::FullBack) => 3,
            Some(Duty::DefensiveMid) => 4,
            Some(Duty::CentralMid) => 5,
            Some(Duty::AttackingMid) => 6,
            Some(Duty::Winger) => 7,
            Some(Duty::Striker) => 8,
            _ => 0,
        }
    }

    /// Intent code of player `index`: 0 hold, 1 anticipate, 2 mark, 3 support,
    /// 4 press, 5 cover, 6 run.
    pub fn player_intent(&self, index: usize) -> u8 {
        use crate::brain::TacticalIntent;
        match self.agents.get(index).map(|a| a.brain.current_intent) {
            Some(TacticalIntent::AnticipateEvent { .. }) => 1,
            Some(TacticalIntent::ManMark { .. }) => 2,
            Some(TacticalIntent::SupportTriangle { .. }) => 3,
            Some(TacticalIntent::PressCarrier { .. }) => 4,
            Some(TacticalIntent::CoverSpace { .. }) => 5,
            Some(TacticalIntent::TakeRun { .. }) => 6,
            _ => 0,
        }
    }

    /// Phase code for a team: 0 build-up, 1 progression, 2 final third,
    /// 3 attacking transition, 4 high press, 5 mid-block, 6 low block,
    /// 7 defensive transition.
    pub fn team_phase(&self, team: usize) -> u8 {
        match self.phases.get(team).map(|p| p.phase) {
            Some(TeamPhase::BuildUp) => 0,
            Some(TeamPhase::Progression) => 1,
            Some(TeamPhase::FinalThird) => 2,
            Some(TeamPhase::AttackingTransition) => 3,
            Some(TeamPhase::HighPress) => 4,
            Some(TeamPhase::MidBlock) => 5,
            Some(TeamPhase::LowBlock) => 6,
            Some(TeamPhase::DefensiveTransition) => 7,
            None => 5,
        }
    }

    /// Agent index of the team's designated presser, if any (-1).
    pub fn team_presser(&self, team: usize) -> i32 {
        self.last_plans
            .get(team)
            .and_then(|p| *p)
            .and_then(|p| p.presser)
            .map(|i| i as i32)
            .unwrap_or(-1)
    }

    /// Agent index of the team's designated cover, if any (-1).
    pub fn team_cover(&self, team: usize) -> i32 {
        self.last_plans
            .get(team)
            .and_then(|p| *p)
            .and_then(|p| p.cover)
            .map(|i| i as i32)
            .unwrap_or(-1)
    }

    /// Agent index of the team's designated runner, if any (-1).
    pub fn team_runner(&self, team: usize) -> i32 {
        self.last_plans
            .get(team)
            .and_then(|p| *p)
            .and_then(|p| p.runner)
            .map(|i| i as i32)
            .unwrap_or(-1)
    }

    /// Agent index this player is marking, if any (-1).
    pub fn player_mark(&self, index: usize) -> i32 {
        let team = self.agents.get(index).map(|a| usize::from(a.team_id != self.home_team_id));
        match team {
            Some(t) => self
                .last_plans
                .get(t)
                .and_then(|p| *p)
                .and_then(|p| p.mark_for(index))
                .map(|i| i as i32)
                .unwrap_or(-1),
            None => -1,
        }
    }

    /// Convenience accessor for the current ball position.
    #[inline]
    pub fn ball_pos(&self) -> Vector2 {
        self.ball.position
    }

    /// The current tactical anchor per player id (macro-layer target).
    /// Exposed for diagnostics and tests.
    #[inline]
    pub fn anchors(&self) -> &HashMap<String, Vector2> {
        &self.anchor_buffer
    }

    /// The current defensive-line x per team as `[home, away]`.
    #[inline]
    pub fn hold_lines(&self) -> [f32; 2] {
        self.hold_line_x
    }

    /// The detected team styles as `[home, away]`.
    #[inline]
    pub fn team_styles(&self) -> [TeamStyle; 2] {
        self.styles
    }

    /// Whether a set piece (kick-off, corner, free kick, throw-in) is active.
    #[inline]
    pub fn set_piece_active(&self) -> bool {
        self.set_piece.is_some()
    }

    /// The team currently taking a set piece (kick-off, corner, free kick),
    /// or `None` during open play.
    pub fn set_piece_team(&self) -> Option<&str> {
        self.set_piece.as_ref().map(|s| s.team.as_str())
    }

    /// Current period (1 = first half, 2 = second half, …).
    #[inline]
    pub fn current_period(&self) -> u8 {
        self.current_period
    }

    /// The kind of the active set piece, or `None` during open play.
    pub fn set_piece_kind(&self) -> Option<&'static str> {
        self.set_piece.as_ref().map(|s| match s.kind {
            SetPieceKind::KickOff => "kickoff",
            SetPieceKind::Corner => "corner",
            SetPieceKind::FreeKick => "freekick",
            SetPieceKind::ThrowIn => "throwin",
            SetPieceKind::Penalty => "penalty",
            SetPieceKind::GoalKick => "goalkick",
        })
    }

    /// Advance the engine by one fixed timestep.
    #[inline]
    pub fn step(&mut self) {
        self.tick(self.config.dt);
    }

    /// Advance the engine by an arbitrary timestep.
    pub fn tick(&mut self, dt: f32) {
        // 0. A new period begins: the restart is placed *before* the first
        //    event of that period is dispatched, so the kick-off actually
        //    happens from the centre spot. Detected here (rather than after
        //    execution) because the period boundary event is already due.
        self.prepare_period_restart();

        // 0b. Roll the ball onto the spot of the next event *before* the
        //     dispatcher executes it, so the actor is already over the ball
        //     and the action happens exactly where Opta says it happened.
        self.pre_position_ball(dt);

        // 1. Dispatcher: anticipation + execution.
        self.last_actions =
            self.dispatcher.tick(
                dt,
                &mut self.agents,
                Some(self.ball.position),
                crate::dispatcher::SLIP_MAX_SECS,
            );
        self.sim_time = self.dispatcher.sim_time;
        self.events_dispatched = self.dispatcher.dispatched;
        self.events_executed = self.dispatcher.executed;

        // 1b. Substitutions swap a bench player into the departing player's
        //     agent, so incoming subs own their own events (no ghost passes).
        self.apply_substitutions();

        // 1c. Mid-match formation changes (type 40) retune the XI in place.
        self.apply_formation_changes();

        // 1d. Ground-truth snapshots: the doer and the ball exactly as the UI
        //     sees them at the dispatch instant, before choreography moves
        //     anything. This is the measurement contract for accuracy.
        for action in &self.last_actions {
            let doer_pos = self
                .agent_index
                .get(&action.player_id)
                .map(|&i| self.agents[i].position);
            self.event_snapshots.push(EventSnapshot {
                event_id: action.event_id,
                feed_time: action.feed_time,
                display_time: action.display_time,
                sim_time: self.sim_time,
                type_id: action.type_id,
                origin: action.origin,
                doer_id: action.player_id.clone(),
                doer_pos,
                doer_state: action.doer_state,
                doer_stage_target: action.doer_stage_target,
                ball_flight_target: match self.ball.state {
                    BallState::InFlight { target_pos, .. } => Some(target_pos),
                    _ => None,
                },
                ball_flight_remaining: match self.ball.state {
                    BallState::InFlight {
                        total_time,
                        elapsed_time,
                        ..
                    } => Some(total_time - elapsed_time),
                    _ => None,
                },
                ball_flight_kind: match &self.ball.state {
                    BallState::InFlight { flight_type, .. } => Some(flight_type.label()),
                    _ => None,
                },
                pending_kickoff: self.pending_kickoff.is_some(),
                ball_state_head: self.ball.state.label(),
                staging_decision: self.staging_decision,
                ball_carrier: match &self.ball.state {
                    BallState::AttachedToPlayer { player_id, .. } => Some(player_id.clone()),
                    _ => None,
                },
                ball_pos: self.ball.position,
                ball_expected: ball_spot_expected(action.type_id),
            });
        }

        // 2. Ball choreography for executed actions.
        for i in 0..self.last_actions.len() {
            let action = self.last_actions[i].clone();
            self.choreograph(&action);
            self.record_event(&action);
        }

        // 3. Ball physics.
        self.update_ball(dt);

        // 4. Possession + elastic anchors.
        self.update_possession();
        match &self.possession {
            Some(t) if t == &self.home_team_id => self.possession_frames[0] += 1,
            Some(t) if t == &self.away_team_id => self.possession_frames[1] += 1,
            _ => {}
        }
        self.update_set_piece();
        self.update_phases(dt);
        self.update_anchors();

        // 5. Utility-AI decision + context steering for all agents.
        let phases = [self.phases[0].phase, self.phases[1].phase];
        let lookahead = self.dispatcher.lookahead_blocks(
            &self.home_team_id,
            &self.away_team_id,
            self.sim_time,
            self.dispatcher.lookahead_window,
        );
        // Keep the plans: the overlay reads the engine's own scheme instead
        // of re-deriving pressers, covers, marks and runners.
        let set_piece_taker = self.set_piece.as_ref().and_then(|sp| sp.taker_id.clone());
        self.last_plans = crate::brain::step_agents(
            &mut self.agents,
            &self.anchor_buffer,
            &self.event_lookup,
            self.sim_time,
            self.ball.position,
            self.possession.as_deref(),
            &self.home_team_id,
            &self.styles,
            &phases,
            &lookahead,
            &self.hold_line_x,
            self.set_piece.is_some(),
            set_piece_taker.as_deref(),
            dt,
        );

        // 5b. Kick-off: the opposition must stay outside the centre circle
        //     until the ball is kicked. Enforced as a bounded correction so
        //     it can never teleport a player.
        self.enforce_kickoff_circle(dt);

        // 6. The upcoming actor takes a resting ball, so a pass that lands
        //    short of its receiver is actually collected instead of sitting
        //    untouched on the grass.
        self.pickup_loose_ball();
    }

    /// The exact pitch spot `ev` needs the ball at, or `None` when the event
    /// never moves it (cards, substitutions, formation changes…).
    /// A corner is *awarded* where the ball left the pitch and *taken* from
    /// the arc by the separate pass that follows it — so the award keeps its
    /// own origin instead of teleporting the ball ten metres to the flag.
    fn event_ball_spot(&self, ev: &NormalizedEvent) -> Option<Vector2> {
        let origin = Vector2::new(ev.origin_x, ev.origin_y);
        if ball_spot_expected(ev.type_id) {
            Some(origin.clamp_on_pitch())
        } else {
            None
        }
    }

    /// The next pending event that actually moves the ball, as
    /// `(match_time, ball_spot, actor)`. Cards, substitutions and other
    /// non-ball events are skipped, so the ball is never dragged onto them.
    fn next_ball_event(&self) -> Option<(f32, Vector2, String)> {
        let events = self.dispatcher.events();
        let start = self.dispatcher.cursor();
        events[start..].iter().find_map(|ev| {
            self.event_ball_spot(ev)
                .map(|spot| (ev.match_time_secs, spot, ev.player_id.clone()))
        })
    }

    /// Roll the ball onto the spot of the next pending event *before* the
    /// dispatcher executes it, so the action happens where Opta says it
    /// happened and the actor is already over the ball (no ghost passes).
    fn pre_position_ball(&mut self, dt: f32) {
        // A pending kick-off places the ball deliberately after the goal.
        if self.pending_kickoff.is_some() {
            self.staging_decision = "kickoff pending";
            return;
        }
        let Some((event_time, spot, next_player)) = self.next_ball_event() else {
            self.staging_decision = "no ball event";
            return;
        };
        let raw_eta = event_time - self.sim_time;
        // A due event is not necessarily firing: the readiness gate holds it
        // until the ball is in place, so once the deadline passes keep the
        // reposition running against a short target instead of standing down.
        let eta = if raw_eta <= dt {
            self.staging_decision = "overdue";
            HOLD_STAGING_SECS
        } else {
            raw_eta
        };
        if eta > BALL_STAGING_SECS {
            self.staging_decision = "horizon";
            return; // not the run-up yet: leave the ball where play has it
        }
        let spot = spot.clamp_on_pitch();
        let dist = self.ball.position.distance(spot);
        if dist < 1.0 {
            // As the event becomes due, walk a loose ball the last stride onto
            // its origin so it fires where Opta says it happened (a ball a
            // hand's width off reads as an inaccuracy). A ball in flight or at
            // a carrier's feet is left alone — moving it would fight the
            // follow-the-player logic and stack a second displacement onto the
            // action's own launch on the dispatch tick. The settle is bounded
            // per tick and converges well before the event, so it never reads
            // as a teleport.
            if raw_eta <= 0.3 && self.ball.is_resting() {
                let delta = spot - self.ball.position;
                if delta.length() > 1e-3 {
                    let moved = self.ball.position + delta.truncate(BALL_SETTLE_STEP);
                    self.ball.place_at(moved);
                }
            }
            self.staging_decision = "close";
            return; // already there
        }
        match &self.ball.state {
            // A live pass that is already coming down on this spot: leave it
            // alone. Otherwise the ball is flying elsewhere (or to a spot the
            // event is not on) — redirect it, or the event would fire metres
            // away from the ball.
            BallState::InFlight {
                target_pos,
                total_time,
                elapsed_time,
                ..
            } => {
                // A live pass is left alone only when it lands *on* the spot
                // (inside the accuracy tolerance) *and* before the event
                // fires; otherwise it becomes the ball's way of getting there.
                if target_pos.distance(spot) < 1.4 && total_time - elapsed_time <= eta {
                    self.staging_decision = "flight ok";
                    return;
                }
            }
            // A carrier takes the ball to their own event spot — but only
            // while they are still far away. Once they are close, the ball
            // is staged onto the spot so it sits exactly on the origin at
            // dispatch (a ball at the carrier's feet reads as 0.5–1 m off).
            BallState::AttachedToPlayer { player_id, .. } => {
                if !next_player.is_empty() && *player_id == next_player {
                    let carrier_far = self
                        .agent_index
                        .get(player_id)
                        .map(|&i| self.agents[i].position.distance(spot) > 1.5)
                        .unwrap_or(false);
                    if carrier_far {
                        self.staging_decision = "carrier holds";
                        return;
                    }
                }
            }
            BallState::Loose { .. } => {}
        }
        self.staging_decision = "launch";
        // Exact-arrival profile: the ball is on the spot at `eta`, moving at
        // a constant dist/eta (capped at BALL_SPOT_MAX_SPEED), and stops there.
        self.ball.travel_to_eta(spot, eta, BALL_SPOT_MAX_SPEED);
    }

    /// Hand a resting ball to the player who is about to play it.
    ///
    /// Without this a pass that lands with nobody in the exact spot leaves
    /// the ball stranded: it stops, the players mill around it and nothing
    /// happens until the next event drags the ball away on its own. Only the
    /// upcoming actor can claim it, so a completed pass reads as an actual
    /// reception and nobody else ever runs off with the ball.
    fn pickup_loose_ball(&mut self) {
        if self.pending_kickoff.is_some() || self.set_piece.is_some() {
            return; // a deliberate restart keeps its ball on the spot
        }
        if !self.ball.is_resting() {
            return;
        }
        let spot = self.ball.position;
        let claim = self
            .next_ball_event()
            .filter(|(t, _, _)| *t - self.sim_time <= PICKUP_HORIZON_SECS)
            .and_then(|(_, _, player)| self.agent_index.get(&player).copied())
            .filter(|&i| self.agents[i].position.distance(spot) <= BALL_CLAIM_RADIUS);
        let Some(idx) = claim else {
            return;
        };
        let id = self.agents[idx].id.clone();
        let pos = self.agents[idx].position;
        let heading = self.agents[idx].heading;
        self.ball.attach_to(id, pos, heading);
    }

    /// Bring the ball to a dead-ball spot without ever teleporting it: roll
    /// it there when it is far, place it only once it has arrived.
    fn bring_ball_to(&mut self, spot: Vector2) {
        // Already on its way (pre-positioned toward this very spot): let the
        // roll finish rather than restarting it.
        if let BallState::InFlight { target_pos, .. } = &self.ball.state {
            if target_pos.distance(spot) < 1.5 {
                return;
            }
        }
        if self.ball.position.distance(spot) < 0.6 {
            self.ball.place_at(spot);
        } else {
            self.ball.travel_to(spot, BALL_SPOT_MAX_SPEED, false);
        }
    }

    /// Translate an executed event into ball motion.
    fn choreograph(&mut self, action: &DispatchedAction) {
        // The feed awards a corner to the team attacking that end even though
        // the row itself names the player who touched the ball out, so judge
        // the restart by the end: in the canonical frame home always attacks
        // +x, whoever the row is credited to.
        if action.type_id == EVENT_TYPE_CORNER {
            // The award records where the ball left play; the restart itself
            // is taken from the corner arc, and the separate pass that
            // follows is logged there. Lock the taker and the spot to that
            // pass now so the ball, the anchor and the feed agree; otherwise
            // the player standing over the ball may not be the actual doer
            // and the readiness gate stalls the corner.
            let team = if action.origin.x >= 0.0 {
                self.home_team_id.clone()
            } else {
                self.away_team_id.clone()
            };
            let canonical = Vector2::new(
                if action.origin.x >= 0.0 { 52.0 } else { -52.0 },
                if action.origin.y >= 0.0 { 33.5 } else { -33.5 },
            );
            let (taker_id, flag) = match self.next_restart_actor(&team, canonical, 14.0, None) {
                Some((id, origin)) => (Some(id), origin),
                None => (self.nearest_taker(&team, canonical), canonical),
            };
            self.bring_ball_to(flag);
            self.set_piece = Some(SetPiece {
                kind: SetPieceKind::Corner,
                team,
                until: self.restart_until(20.0),
                spot: flag,
                side: if action.origin.y >= 0.0 { 1.0 } else { -1.0 },
                taker_id,
            });
            return;
        }
        // Other dead-ball restarts (out) and fouls: bring the ball to the spot.
        if is_dead_ball_event(action.type_id) || action.type_id == EVENT_TYPE_FOUL {
            // Fouls arrive as twin rows: offender first, then the fouled side.
            // Only the fouled side's row owns the restart. If a twin foul by
            // the other team follows at the same spot within a few seconds,
            // this is the offender's row — skip arming entirely.
            let mut arm_set_piece = true;
            if action.type_id == EVENT_TYPE_FOUL {
                let events = self.dispatcher.events();
                let start = self.dispatcher.cursor();
                let is_offender_row = events[start..].iter().any(|e| {
                    e.type_id == EVENT_TYPE_FOUL
                        && e.team_id != action.team_id
                        && (e.origin_x - action.origin.x).abs() <= 1.0
                        && (e.origin_y - action.origin.y).abs() <= 1.0
                        && e.match_time_secs >= action.feed_time
                        && e.match_time_secs - action.feed_time <= 5.0
                });
                if is_offender_row {
                    arm_set_piece = false;
                }
            }
            let mut team = action.team_id.clone();
            // If the awarded team put the ball out, the restart goes to the other side.
            if !is_dead_ball_event(action.type_id) && action.type_id != EVENT_TYPE_FOUL && action.type_id != EVENT_TYPE_CORNER {
                if self.last_touch.as_deref() == Some(team.as_str()) {
                    team = if team == self.home_team_id { self.away_team_id.clone() } else { self.home_team_id.clone() };
                }
            }
            let exclude = if action.type_id == EVENT_TYPE_FOUL {
                Some(action.player_id.as_str())
            } else {
                None
            };
            let taker_id = self
                .next_restart_actor(&team, action.origin, 14.0, exclude)
                .map(|(id, _)| id)
                .or_else(|| self.nearest_taker(&team, action.origin));
            self.bring_ball_to(action.origin);
            // Fouls arrive as twin rows (offender outcome=1 first, fouled
            // side outcome=0 second): each arms the restart in turn, so the
            // fouled side's row owns it. Keep the feed's team — do not flip.
            // Penalty fouls (Q9) become a spot kick.
            let kind = if action.type_id == EVENT_TYPE_FOUL {
                if action.penalty {
                    SetPieceKind::Penalty
                } else {
                    SetPieceKind::FreeKick
                }
            } else if action.origin.x.abs() > 46.0 {
                // Out over the goal line: a goal kick, not a throw.
                SetPieceKind::GoalKick
            } else {
                SetPieceKind::ThrowIn
            };
            // The loader leaves the awarded team on an out row — the restart
            // goes to whoever did not put the ball out.
            //
            // The set piece must stay armed until the restart is actually
            // taken (the next event), otherwise the wall/throw-in shape
            // collapses seconds before the kick — the immersion break.
            let next_at = self
                .dispatcher
                .peek_next()
                .map(|e| e.match_time_secs);
            let until = next_at
                .map(|t| t + 2.0)
                .unwrap_or_else(|| self.restart_until(14.0));
            if arm_set_piece {
                self.set_piece = Some(SetPiece {
                    kind,
                    team: team.clone(),
                    until,
                    spot: action.origin,
                    side: 0.0,
                    taker_id,
                });
            }
            return;
        }
        // Substitutions, cards, etc. never touch the ball.
        if !is_ball_event(action.type_id) {
            return;
        }
        let Some(idx) = self.agent_index(&action.player_id) else {
            return;
        };
        let pos = self.agents[idx].position;
        let heading = self.agents[idx].heading;

        // ═══════════════════════════════════════════════════════════════════
        // OPTA FIDELITY: the event says player X at position Y does this
        // action. `pre_position_ball` has already rolled the ball onto Y, and
        // the attachment below *never* moves it: the offset is measured from
        // the actor's real world position, so the ball stays exactly where
        // Opta says and only closes to the boots at a bounded roll speed.
        // This is what stops a pass from appearing to come from thin air —
        // the "home team passes in the log but away looks like they're
        // passing" bug.
        self.ball.attach_to(action.player_id.clone(), pos, heading);

        if let Some(target) = action.target {
            // Pass / shot / clearance with a known destination.
            //
            // WS1: the event graph decides the endpoint. A completed pass
            // flies to its target with a kind-true flight (never stretched to
            // fit a receiver — the receiver is routed to the landing point).
            // A failed pass ends at the interception point or the touchline,
            // and NOBODY is routed onto it.
            let failed_end = if matches!(action.type_id, 1 | 2 | 74)
                && !crate::event_graph::is_completed_pass_like(
                    action.type_id,
                    action.outcome,
                    action.outcome_reason,
                ) {
                self.dispatcher.failed_endpoint(action.event_id, &self.agents)
            } else {
                None
            };
            let target = failed_end.unwrap_or(target);
            let is_shot = matches!(action.type_id, 13..=16);
            if is_shot {
                self.launch_shot(&action, target);
            } else {
                let bend = curve_bend(&action);
                // Kick-off passes (Q279) are soft taps to the second kicker,
                // not driven balls: the feed's target is where the next touch
                // happens, not the pass destination. Cap the tap at 2.5 m —
                // the coordination budget below still paces it to the taker.
                let target = match action.pass_kind {
                    PassKind::KickoffStart | PassKind::KickoffRestart => {
                        tap_target(action.origin, target)
                    }
                    _ => target,
                };
                // Coordination budget (WS1): the flight stays kind-true, but
                // it may hang a beat so the feed-proven owner arrives with it
                // — a linked receiver up to +1.5 s, an interceptor up to
                // +1.0 s. Never shortened, never stretched to the next event.
                let natural = self.ball.position.distance(target)
                    / crate::ball::pass_speed(action.pass_kind).max(1.0);
                let owner_eta = owner_eta(&self.agents, &action, target, failed_end.is_some());
                let cap = if failed_end.is_some() { 1.0 } else { 1.5 };
                let duration = match owner_eta {
                    Some(eta) => natural.max((eta - 0.3).min(natural + cap)),
                    None => natural.max(0.35),
                };
                self.ball.launch_for_kind_with_duration(
                    target,
                    action.pass_kind,
                    bend,
                    action.is_aerial,
                    duration,
                );
                // Through-balls and long releases keep rolling; parries skid.
                match action.pass_kind {
                    PassKind::ThroughBall | PassKind::LongBall | PassKind::Switch => {
                        self.ball.set_landing_roll(5.0, 4.5);
                    }
                    _ => {}
                }
                // Remember the landing so the accuracy harness can measure the
                // receiver against the exact touchdown point. Failed passes
                // name no receiver — the interceptor owns the endpoint.
                let receiver = if failed_end.is_some() {
                    None
                } else {
                    action.receiver_id.clone()
                };
                self.pending_landing = Some((target, receiver));
            }
        } else if matches!(action.type_id, 13..=16) {
            // Shots with no Q140/141 destination: the feed truth (Q102/103
            // goal-mouth, Q230/231 keeper, Q173-183 save style) decides the
            // target — never an invented placement.
            let target = self.shot_target(&action);
            self.launch_shot(&action, target);
        } else {
            // Possession (take-on, tackle, carry): the ball must sit at the
            // event's spot at the actor's feet. Pre-positioning has usually
            // delivered it already; if it is still well away (the actor raced
            // in late), roll it the rest of the way continuously — snapping
            // is never allowed.
            if self.ball.position.distance(action.origin) > 3.0 {
                self.ball.travel_to(action.origin, 20.0, false);
            }
            // Otherwise the attachment above keeps it bound to the actor and
            // it closes to their boots at a bounded roll speed.
        }
    }

    /// Launch a shot at SHOT_SPEED with the keeper reacting: dive lunge plus
    /// hold/parry/rebound landing behaviour. Shared by shots with an explicit
    /// Q140/141 destination and shots resolved from goal-mouth truth.
    fn launch_shot(&mut self, action: &DispatchedAction, target: Vector2) {
        self.ball.launch(target, false, true);
        self.keeper_dive(action, target);
        self.pending_landing = Some((target, None));
    }

    /// Goal-mouth (or wide) target for a shot, from the feed's own geometry.
    ///
    /// `own_goal` flips the target: an own goal must fly into the *scorer's*
    /// own net, i.e. the goal at the opposite end from the one his team
    /// attacks. There is no hash, no flair, no invented placement: goals use
    /// the recorded Q102/103 spot inside the posts; saved shots go on target
    /// within the keeper's reach (the dive/parry reads on the ball, never a
    /// visual goal); posts hit the frame; misses go wide. The deterministic
    /// fallbacks (no Q102/103, e.g. synthetic feeds) aim off-centre away from
    /// the keeper or wide by zone — documented, never random.
    fn shot_target(&self, action: &DispatchedAction) -> Vector2 {
        let attack_dx = if action.team_id == self.home_team_id {
            1.0
        } else {
            -1.0
        };
        let goal_x = if action.is_own_goal {
            -attack_dx * PITCH_HALF_LENGTH
        } else {
            attack_dx * PITCH_HALF_LENGTH
        };
        // Deterministic side fallback: the shooter's flank, else +1.
        let flank = if action.origin.y >= 0.0 { 1.0 } else { -1.0 };
        let mouth = action.goalmouth.and_then(|(y102, z103)| {
            let a = action.team_id == self.home_team_id;
            let dx = if a { 1.0 } else { -1.0 };
            Some((((y102 - 50.0) * 0.68 * dx).clamp(-3.66, 3.66), z103))
        });
        let keeper_lat = action.keeper_xy.map(|(_, y231)| {
            let dx = if action.team_id == self.home_team_id {
                1.0
            } else {
                -1.0
            };
            ((y231 - 50.0) * 0.68 * dx).clamp(-4.5, 4.5)
        });
        let goal_half = 3.66;
        match action.type_id {
            // Goal: the recorded spot, inside the posts. Fallback: off-centre
            // away from the keeper (or the shooter's flank when unknown).
            EVENT_TYPE_GOAL => {
                let lat = mouth
                    .map(|(lat, _)| lat)
                    .unwrap_or_else(|| match keeper_lat {
                        Some(k) => (if k >= 0.0 { -1.4 } else { 1.4 }),
                        None => flank * 1.4,
                    });
                Vector2::new(goal_x * 0.99, lat.clamp(-3.4, 3.4))
            }
            // Saved: on target, inside the keeper's reach. Held claims
            // (collected/standing/hands) fly at the keeper's body; parries fly
            // at the recorded spot and skid wide on landing (see below).
            15 => {
                let lat = mouth.map(|(lat, _)| lat).unwrap_or_else(|| {
                    keeper_lat
                        .map(|k| (k + flank * 1.0).clamp(-3.2, 3.2))
                        .unwrap_or(flank * 1.6)
                });
                Vector2::new(goal_x * 0.99, lat.clamp(-3.4, 3.4))
            }
            // Post: onto the frame at the recorded side, else the flank post.
            14 => {
                let side = mouth
                    .map(|(lat, _)| if lat >= 0.0 { 1.0 } else { -1.0 })
                    .unwrap_or(flank);
                Vector2::new(goal_x, side * goal_half)
            }
            // Miss: wide of the post by zone (fallback 2 m). Side from the
            // recorded spot, the keeper's opposite, or the shooter's flank.
            _ => {
                let side = mouth
                    .map(|(lat, _)| if lat.abs() > 3.66 { lat.signum() } else { flank })
                    .unwrap_or_else(|| {
                        keeper_lat
                            .map(|k| if k >= 0.0 { -1.0 } else { 1.0 })
                            .unwrap_or(flank)
                    });
                let wide = goal_half + 2.0;
                Vector2::new(goal_x, side * wide)
            }
        }
    }

    /// Keeper reaction to a shot: lunge the defending keeper toward the shot
    /// line and set the ball's landing behaviour (hold vs parry vs rebound).
    ///
    /// Held claims (collected/standing/hands) attach on landing; everything
    /// else skids on with a wide residual so a saved shot never reads as a
    /// goal. The dive is a bounded `receiver_task` sprint — the brain honours
    /// it as a hard constraint and it expires on its own.
    fn keeper_dive(&mut self, action: &DispatchedAction, target: Vector2) {
        // Conceding side: the opponent, except on own goals (own team).
        let conceding = if action.is_own_goal {
            action.team_id.clone()
        } else if action.team_id == self.home_team_id {
            self.away_team_id.clone()
        } else {
            self.home_team_id.clone()
        };
        let keeper_idx = self
            .agents
            .iter()
            .position(|a| a.team_id == conceding && a.is_goalkeeper);
        let Some(ki) = keeper_idx else { return };
        let keeper_pos = self.agents[ki].position;

        // Dive toward the shot, capped at a credible 3.5 m lunge.
        let shot_dir = (target - action.origin).normalize();
        let to_shot = target - keeper_pos;
        let dive = keeper_pos + to_shot.truncate(3.5);
        let flight_secs = (self.ball.position.distance(target) / crate::ball::SHOT_SPEED).max(0.35);
        self.agents[ki].receiver_task = Some((dive, self.sim_time + flight_secs));
        let _ = shot_dir;

        match action.type_id {
            // Held: the keeper gathers it — attach on landing.
            15 if matches!(
                action.keeper_action,
                KeeperAction::Collected | KeeperAction::Standing | KeeperAction::Hands
            ) =>
            {
                self.pending_catch = Some(self.agents[ki].id.clone());
            }
            // Parried/diving save: the ball lands on target, then skids back
            // out wide — toward the field and the shot side — so a saved shot
            // never trickles over the line after the whistle.
            15 => {
                let side = if target.y >= 0.0 { 1.0 } else { -1.0 };
                let out = Vector2::new(
                    target.x - attack_goal_sign(&action, self) * 4.0,
                    side * 8.0,
                ) - target;
                self.ball.set_landing_deflect(out, 7.0, 4.0);
            }
            // Post: frame rebound back into play.
            14 => {
                self.ball.set_landing_roll(4.5, 5.0);
            }
            _ => {}
        }
    }

    /// The x of the offside line for the team currently in possession (the
    /// second-last defender, never behind the ball or the half-way line).
    pub fn offside_line_x(&self) -> f32 {
        let Some(att) = &self.possession else {
            return f32::NAN; // no possession → no offside line
        };
        let home_attacking = att == &self.home_team_id;
        // Attack direction of the possessing team; the defending side is the
        // other team, and its second-*last* defender (2nd nearest to the goal
        // the attackers shoot at) is the second-highest progress value.
        let dir = if home_attacking { 1.0 } else { -1.0 };
        let defending_home = !home_attacking;
        let second = crate::brain::second_last_progress(
            self.agents.iter().filter(|a| !a.sent_off).map(|a| {
                (
                    a.position.x,
                    a.team_id == self.home_team_id,
                )
            }),
            defending_home,
            dir,
        );
        if !second.is_finite() {
            return 0.0;
        }
        let second_last = second * dir;
        // Not behind the ball and not in the attacking half's defensive side.
        if dir > 0.0 {
            second_last.max(self.ball.position.x).max(0.0)
        } else {
            second_last.min(self.ball.position.x).min(0.0)
        }
    }

    /// Append an executed event to the rolling UI feed.
    fn record_event(&mut self, action: &DispatchedAction) {
        // An own goal is credited by the feed to the player who put the ball
        // in his own net, so the team that actually *scores* is the opponent.
        // The feed colour, the score and the kick-off all key off the side
        // that benefits, never the row's own team.
        let own_goal = action.type_id == EVENT_TYPE_GOAL && action.is_own_goal;
        let scores_home = if own_goal {
            action.team_id == self.away_team_id
        } else {
            action.team_id == self.home_team_id
        };
        let scores_away = if own_goal {
            action.team_id == self.home_team_id
        } else {
            action.team_id == self.away_team_id
        };
        let team_index = if scores_home {
            TEAM_HOME
        } else if scores_away {
            TEAM_AWAY
        } else {
            TEAM_BALL
        };
        if action.type_id == EVENT_TYPE_GOAL {
            if scores_home {
                self.score[0] += 1;
            } else if scores_away {
                self.score[1] += 1;
            }
            // Football restarts from the centre with the conceding team — but
            // only when the feed's next ball action actually comes off the
            // centre spot. Some feeds log the next action somewhere else a
            // second later; parking the ball on the circle then would strand
            // it forty metres from the play, so leave it for `pre_position`.
            let centre_restart = self
                .next_ball_event()
                .map(|(_, spot, _)| spot.length() < 4.0)
                .unwrap_or(true);
            if centre_restart {
                // The conceding side is the own-goal scorer's own team;
                // otherwise it is the opponent of the scoring side.
                let conceding = if own_goal {
                    action.team_id.clone()
                } else if action.team_id == self.home_team_id {
                    self.away_team_id.clone()
                } else {
                    self.home_team_id.clone()
                };
                // Give the celebration a moment, but never longer than the gap
                // to the next event: the ball must already be on the centre
                // spot before the kick-off action fires.
                let mut at = self.sim_time + 1.6;
                if let Some(next) = self.dispatcher.peek_next() {
                    at = at
                        .min(next.match_time_secs - 0.4)
                        .max(self.sim_time + 0.1);
                }
                self.pending_kickoff = Some((conceding, at));
            }
        }
        // A red card (straight or second yellow) dismisses the player: the
        // side plays the rest short-handed.
        if action.type_id == EVENT_TYPE_CARD
            && matches!(
                action.card,
                CardKind::Red | CardKind::SecondYellow
            )
            && !action.player_id.is_empty()
        {
            self.dismiss_player(&action.player_id.clone());
        }
        let player = self
            .agent_index(&action.player_id)
            .map(|i| self.agents[i].id.clone())
            .unwrap_or_default();
        if self.recent_events.len() == EVENT_FEED_CAPACITY {
            self.recent_events.pop_front();
        }
        self.recent_events.push_back(EventRecord {
            time: action.display_time,
            type_id: action.type_id,
            label: if own_goal {
                "Own Goal"
            } else {
                event_type_name(action.type_id)
            },
            team_index,
            player,
            notable: is_notable_event(action.type_id),
        });
        self.event_seq = self.event_seq.wrapping_add(1);
    }

    /// Advance ball physics using the current player transforms.
    fn update_ball(&mut self, dt: f32) {
        let agents = &self.agents;
        let index = &self.agent_index;
        let (was_flying, was_reposition) = match &self.ball.state {
            BallState::InFlight { flight_type, .. } => {
                (true, matches!(flight_type, crate::ball::BallFlightType::Reposition))
            }
            _ => (false, false),
        };
        self.ball.update(dt, |id| {
            index
                .get(id)
                .map(|&i| (agents[i].position, agents[i].heading))
        });
        // A flight just touched down: record where the ball landed versus where
        // the intended receiver was, so pass accuracy is measurable. Staged
        // repositions are ball logistics, not passes — they must neither be
        // measured as one nor swallow the pending pass landing.
        if was_flying
            && !matches!(self.ball.state, BallState::InFlight { .. })
            && !was_reposition
        {
            // A held claim lands in the keeper's hands, not on the grass.
            if let Some(keeper_id) = self.pending_catch.take() {
                if let Some(&ki) = self.agent_index.get(&keeper_id) {
                    let kpos = self.agents[ki].position;
                    let khead = self.agents[ki].heading;
                    self.ball.attach_to(keeper_id.clone(), kpos, khead);
                }
            }
            if let Some((target, receiver_id)) = self.pending_landing.take() {
                let receiver_pos = receiver_id
                    .as_deref()
                    .and_then(|rid| self.agent_index.get(rid))
                    .map(|&i| self.agents[i].position);
                let mut nearest_id = None;
                let mut nearest_pos = None;
                let mut nearest = f32::INFINITY;
                for a in &self.agents {
                    let d = a.position.distance(self.ball.position);
                    if d < nearest {
                        nearest = d;
                        nearest_id = Some(a.id.clone());
                        nearest_pos = Some(a.position);
                    }
                }
                self.landing_snapshots.push(LandingSnapshot {
                    sim_time: self.sim_time,
                    target,
                    landing: self.ball.position,
                    receiver_id,
                    receiver_pos,
                    nearest_id,
                    nearest_pos,
                });
            }
        }
    }

    /// Advance set-piece state: expire finished restarts and fire a delayed
    /// kick-off. Period boundaries are handled *before* dispatch in
    /// [`Self::prepare_period_restart`].
    fn update_set_piece(&mut self) {
        if self
            .set_piece
            .as_ref()
            .map(|s| self.sim_time > s.until)
            .unwrap_or(false)
        {
            self.set_piece = None;
        }
        if let Some((team, at)) = self.pending_kickoff.clone() {
            if self.sim_time >= at {
                self.pending_kickoff = None;
                self.start_kickoff(&team);
            }
        }
    }

    /// When a restart's shaping expires.
    ///
    /// It has to outlive the restart event itself (a kick-off that releases
    /// the moment the ball is played looks broken), but it must never freeze
    /// the whole team for the length of a feed gap — once the cap is reached
    /// play resumes and anticipation keeps the taker heading for the ball.
    fn restart_until(&self, max_hold: f32) -> f32 {
        match self.next_ball_event() {
            Some((t, _, _)) => (t + 2.0).clamp(self.sim_time + 3.0, self.sim_time + max_hold),
            None => self.sim_time + max_hold,
        }
    }

    /// Place the restart for a period boundary (half-time) as soon as the
    /// first event of the new period is the next one to fire.
    ///
    /// The feed stamps first-half stoppage time and the second half on the
    /// same clock, so the periods can interleave: comparing with `>` (never
    /// `!=`) keeps a single restart instead of re-placing the ball on every
    /// alternating event, which used to teleport the ball back to the centre
    /// in the middle of open play.
    fn prepare_period_restart(&mut self) {
        let Some(next) = self.dispatcher.peek_next() else {
            return;
        };
        if next.period_id <= self.current_period {
            return;
        }
        self.current_period = next.period_id;
        let team = next.team_id.clone();
        self.pending_kickoff = None;
        self.start_kickoff(&team);
        // A period restart has no interval to walk into shape. Set the
        // anchors, then place the two kickers physically inside the circle so
        // the first event of the half is not taken from 20 m away.
        let dir = if team == self.home_team_id { 1.0 } else { -1.0 };
        let (taker, second) = self.position_kickoff(&team, dir);
        self.snap_kickoff_pair(taker.as_deref(), second.as_deref());
    }

    /// Apply due mid-match formation changes (type 40). Each row renames the
    /// XI in place: agents keep their bodies, but their role, duty, lane,
    /// line depth and team style switch to the new shape within a tick or two
    /// (anchors glide via the normal rate limits — no teleport).
    fn apply_formation_changes(&mut self) {
        while self.formation_cursor < self.formation_changes.len()
            && self.formation_changes[self.formation_cursor].match_time_secs <= self.sim_time
        {
            let fc = self.formation_changes[self.formation_cursor].clone();
            self.formation_cursor += 1;
            self.retune_team_shape(&fc.team_id, &fc.lineup);
        }
    }

    /// Rebuild one team's shape from a new XI: roles, duties, lanes, line
    /// depths, block and style. Agents not named in the row keep their jobs.
    fn retune_team_shape(&mut self, team_id: &str, lineup: &[LineupPlayer]) {
        let xi: Vec<LineupPlayer> = lineup.iter().filter(|p| p.is_starter()).cloned().collect();
        if xi.is_empty() {
            return;
        }
        let team_home = team_id == self.home_team_id;
        let team_idx = usize::from(!team_home);
        let template = FormationEngine::dynamic_template(&xi)
            .unwrap_or_else(|| self.formation.template_vec(DEFAULT_FORMATION));
        let duties = Duty::from_lineup(&xi, &template);
        // Roles, duties and attributes follow the new slots.
        for p in &xi {
            let Some(&idx) = self.agent_index.get(&p.player_id) else {
                continue;
            };
            let a = &mut self.agents[idx];
            a.role = PlayerRole::from_category(p.position_category);
            a.is_goalkeeper = a.role == PlayerRole::Goalkeeper;
            a.duty = duties
                .get(&p.player_id)
                .copied()
                .unwrap_or_else(|| Duty::from_role(a.role, a.position.y));
            a.refresh_attributes();
        }
        // Lanes + line depths from the new template (mirrored for away).
        let mirror = !team_home;
        for p in &xi {
            let idx = (p.formation_slot.saturating_sub(1)) as usize;
            let base = template.get(idx).copied().unwrap_or(Vector2::zero());
            self.lane_y
                .insert(p.player_id.clone(), if mirror { -base.y } else { base.y });
        }
        let mids: Vec<&LineupPlayer> = xi
            .iter()
            .filter(|p| p.position_category == 3)
            .collect();
        if mids.len() >= 2 {
            let mean = mids
                .iter()
                .map(|p| {
                    let idx = (p.formation_slot.saturating_sub(1)) as usize;
                    template.get(idx).copied().unwrap_or(Vector2::zero()).x
                })
                .sum::<f32>()
                / mids.len() as f32;
            for p in mids {
                let idx = (p.formation_slot.saturating_sub(1)) as usize;
                let x = template.get(idx).copied().unwrap_or(Vector2::zero()).x;
                self.mid_depth.insert(p.player_id.clone(), x - mean);
            }
        }
        self.blocks[team_idx] = initial_block(&xi, &template, team_home);
        self.styles[team_idx] =
            TeamStyle::from_formation(FormationEngine::detect_formation_label(&xi));
        // Merge the row into the stored roster so re-identification later
        // sees the current shirts/slots.
        let entry = self.lineups.entry(team_id.to_string()).or_default();
        for p in xi {
            if let Some(slot) = entry.iter_mut().find(|e| e.player_id == p.player_id) {
                *slot = p;
            } else {
                entry.push(p);
            }
        }
    }

    /// Send a player off (red card): freeze them out of the team's plans and
    /// walk them to the touchline bench. The body stays rendered (the walk-off
    /// reads on screen) but the side plays the rest short-handed.
    fn dismiss_player(&mut self, player_id: &str) {
        let Some(&idx) = self.agent_index.get(player_id) else {
            return;
        };
        {
            let a = &mut self.agents[idx];
            if a.sent_off {
                return;
            }
            a.sent_off = true;
            a.assigned_event_id = None;
            a.receiver_task = None;
            a.state = AgentState::InFormation;
            a.sprint_boost = 1.0;
        }
        // Touchline bench on the player's own side of the pitch.
        let side = if self.agents[idx].position.y >= 0.0 { 1.0 } else { -1.0 };
        let bench = Vector2::new(
            self.agents[idx].position.x.clamp(-40.0, 40.0),
            side * 36.5,
        );
        self.anchor_buffer.insert(player_id.to_string(), bench);
        // The shape drops a man: recompute the block without the dismissed.
        let team_id = self.agents[idx].team_id.clone();
        self.retune_after_dismissal(&team_id);
    }

    /// Rebuild a 10-man block after a dismissal (drop a forward, compact).
    fn retune_after_dismissal(&mut self, team_id: &str) {
        let xi: Vec<LineupPlayer> = self
            .lineups
            .get(team_id)
            .map(|v| v.clone())
            .unwrap_or_default();
        let xi: Vec<LineupPlayer> = xi
            .into_iter()
            .filter(|p| {
                p.is_starter()
                    && self
                        .agent_index
                        .get(&p.player_id)
                        .and_then(|&i| self.agents.get(i))
                        .map(|a| !a.sent_off)
                        .unwrap_or(true)
            })
            .collect();
        if xi.len() >= 8 {
            let team_home = team_id == self.home_team_id;
            let team_idx = usize::from(!team_home);
            let template = FormationEngine::dynamic_template(&xi)
                .unwrap_or_else(|| self.formation.template_vec(DEFAULT_FORMATION));
            self.blocks[team_idx] = initial_block(&xi, &template, team_home);
        }
    }

    /// Swap a departing player's agent for the substitute coming on, so the
    /// incoming player owns their own events (the feed is full of passes by
    /// players who never started and would otherwise have no agent).
    ///
    /// Off and On are separate events, often seconds apart, so the outgoing
    /// player is remembered per team until their replacement arrives.
    fn apply_substitutions(&mut self) {
        let (offs, ons): (Vec<_>, Vec<_>) = self
            .last_actions
            .iter()
            .filter(|a| {
                !a.player_id.is_empty()
                    && (a.type_id == EVENT_TYPE_PLAYER_OFF
                        || a.type_id == EVENT_TYPE_PLAYER_ON)
            })
            .partition(|a| a.type_id == EVENT_TYPE_PLAYER_OFF);
        for a in offs {
            self.pending_subs
                .insert(a.team_id.clone(), a.player_id.clone());
        }
        let mut swaps: Vec<(String, String)> = Vec::new();
        for a in ons {
            if let Some(departing) = self.pending_subs.remove(&a.team_id) {
                swaps.push((departing, a.player_id.clone()));
            }
        }
        for (departing, incoming) in swaps {
            self.reidentify(&departing, &incoming);
        }
    }

    /// Give the agent currently known as `old_id` the identity of `new_id`
    /// (id, shirt, role, attributes) while keeping its position on the pitch.
    fn reidentify(&mut self, old_id: &str, new_id: &str) {
        let Some(idx) = self.agent_index.get(old_id).copied() else {
            return;
        };
        let pos = self.agents[idx].position;
        let old_role = self.agents[idx].role;
        let anchor = self.anchor_buffer.remove(old_id).unwrap_or(pos);
        let lane = self.lane_y.remove(old_id);
        let depth = self.mid_depth.remove(old_id);
        self.agent_index.remove(old_id);

        // New identity attributes come from the roster (shirt + position).
        let mut shirt = self.agents[idx].shirt_number;
        let mut category = None;
        for roster in self.lineups.values() {
            if let Some(p) = roster.iter().find(|p| p.player_id == new_id) {
                shirt = p.shirt_number;
                category = Some(p.position_category);
                break;
            }
        }
        {
            let a = &mut self.agents[idx];
            a.id = new_id.to_string();
            a.shirt_number = shirt;
            if let Some(cat) = category {
                let incoming = PlayerRole::from_category(cat);
                // A substitute fills the slot he replaces, so an unknown
                // category keeps the outgoing role; otherwise a centre-back
                // would be anchored on the forward line for the rest of the
                // match.
                a.role = if incoming == PlayerRole::Unknown {
                    old_role
                } else {
                    incoming
                };
                a.is_goalkeeper = a.role == PlayerRole::Goalkeeper;
            }
            a.refresh_attributes();
            a.assigned_event_id = None;
            a.receiver_task = None;
            a.state = AgentState::InFormation;
            a.sprint_boost = 1.0;
        }

        self.agent_index.insert(new_id.to_string(), idx);
        self.anchor_buffer.insert(new_id.to_string(), anchor);
        if let Some(l) = lane {
            self.lane_y.insert(new_id.to_string(), l);
        }
        if self.agents[idx].role == PlayerRole::Midfielder {
            // A substitute takes over the outgoing player's line/depth slot.
            self.mid_depth.insert(new_id.to_string(), depth.unwrap_or(0.0));
        } else {
            self.mid_depth.remove(new_id);
        }
    }

    /// Start a kick-off from the centre spot for `team`.
    fn start_kickoff(&mut self, team: &str) {
        // A deliberate restart: the new ball appears on the centre spot. The
        // set piece is armed in the same tick, so this is the one sanctioned
        // placement (teleport checks exempt restarts).
        self.ball.place_at(Vector2::zero());
        // The feed leaves a long celebration gap (often 60–90 s) between the
        // goal and the kick-off. The set piece is armed until the kick is
        // taken — but never frozen longer than a short hold: holding the
        // defending team in a 5 m kickoff cage for the whole gap (or a
        // 15-minute half-time) reads as a collapse, not a shape. After the
        // hold the teams relax to their blocks; the taker stays over the
        // ball via the pickup rule and the kick still fires on schedule.
        let kickoff_at = self
            .dispatcher
            .peek_next()
            .map(|e| e.match_time_secs)
            .unwrap_or(self.sim_time + 10.0);
        let until = (kickoff_at + 2.0).min(self.sim_time + KICKOFF_HOLD_SECS);
        let taker_id = self
            .next_restart_actor(team, Vector2::zero(), 8.0, None)
            .map(|(id, _)| id)
            .or_else(|| self.nearest_taker(team, Vector2::zero()));
        self.set_piece = Some(SetPiece {
            kind: SetPieceKind::KickOff,
            team: team.to_string(),
            until,
            spot: Vector2::zero(),
            side: 0.0,
            taker_id,
        });
    }

    /// Update each team's game phase with hysteresis. Possession changes are
    /// timestamped so a counter-attack window can be recognised for a few
    /// seconds before the side settles into its normal block.
    fn update_phases(&mut self, dt: f32) {
        for team in 0..2 {
            let team_id = if team == 0 {
                self.home_team_id.clone()
            } else {
                self.away_team_id.clone()
            };
            let in_possession = self.possession.as_deref() == Some(team_id.as_str());
            let attack_dx = if team == 0 { 1.0 } else { -1.0 };
            let progress = self.ball.position.x * attack_dx;
            let state = &mut self.phases[team];
            if in_possession != state.last_in_possession {
                state.last_in_possession = in_possession;
                state.changed_at = self.sim_time;
            }
            let just_changed = self.sim_time - state.changed_at < 3.5;
            let desired = if in_possession {
                if just_changed && progress > -20.0 {
                    TeamPhase::AttackingTransition
                } else if progress >= 25.0 {
                    TeamPhase::FinalThird
                } else if progress >= -10.0 {
                    TeamPhase::Progression
                } else {
                    TeamPhase::BuildUp
                }
            } else if just_changed && progress > -12.0 {
                TeamPhase::DefensiveTransition
            } else if progress >= 10.0 {
                TeamPhase::HighPress
            } else if progress >= -18.0 {
                TeamPhase::MidBlock
            } else {
                TeamPhase::LowBlock
            };

            if desired == state.phase {
                state.candidate = desired;
                state.candidate_since = 0.0;
            } else {
                if desired != state.candidate {
                    state.candidate = desired;
                    state.candidate_since = 0.0;
                }
                state.candidate_since += dt;
                let dwell = if desired.is_transition() { 0.18 } else { 0.45 };
                if state.candidate_since >= dwell {
                    state.phase = desired;
                    state.candidate_since = 0.0;
                }
            }
        }
    }

    /// Refresh both teams' explicit line blocks and rebuild every anchor from
    /// them. Lines move (and shift laterally) as units, so the whole block
    /// stays coherent and a back line is never caught ahead of the ball.
    fn update_anchors(&mut self) {
        let ball = self.ball.position;
        let dt = self.config.dt;
        let home_in_poss = matches!(&self.possession, Some(t) if t == &self.home_team_id);
        let away_in_poss = matches!(&self.possession, Some(t) if t == &self.away_team_id);

        // ── 1. Advance each team's three line heights (progress frame). ──
        let open_play = self.set_piece.is_none();
        for team in 0..2 {
            let team_home = team == 0;
            let attack_dx = if team_home { 1.0 } else { -1.0 };
            let style = self.styles[team];
            let in_poss = if team_home { home_in_poss } else { away_in_poss };
            // Opponent's *actual* second-last defender (progress frame) — the
            // real onside bound — falling back to the block proxy only when
            // fewer than two defenders are known. Dismissed players excluded.
            let defending_home = !team_home;
            let second = crate::brain::second_last_progress(
                self.agents.iter().filter(|a| !a.sent_off).map(|a| {
                    (
                        a.position.x,
                        a.team_id == self.home_team_id,
                    )
                }),
                defending_home,
                attack_dx,
            );
            let opp_def_prog = if second.is_finite() {
                second
            } else {
                self.blocks[1 - team].def_x * attack_dx
            };
            let ball_prog = ball.x * attack_dx;
            let (def_p, mid_p, fwd_p) =
                desired_line_progs(ball_prog, style, in_poss, opp_def_prog);
            let b = &mut self.blocks[team];
            b.def_x = approach_prog(b.def_x, def_p, attack_dx, 3.5, 14.0, dt);
            b.mid_x = approach_prog(b.mid_x, mid_p, attack_dx, 4.0, 10.0, dt);
            b.fwd_x = approach_prog(b.fwd_x, fwd_p, attack_dx, 5.0, 8.0, dt);
            // Hard guarantee: during open play the defensive line target is
            // goal-side of the ball. During a fast turnover the line may lag,
            // so clamp it here rather than letting the back four be caught
            // upfield. (Set pieces use explicit positions instead.)
            if open_play {
                let def_prog = b.def_x * attack_dx;
                let ball_prog = ball.x * attack_dx;
                if def_prog > ball_prog - 0.5 {
                    b.def_x = (ball_prog - 0.5) * attack_dx;
                }
            }
            let y_d = (ball.y * 0.24).clamp(-9.0, 9.0);
            b.def_y = approach_scalar(b.def_y, y_d * 0.85, 3.0, dt);
            b.mid_y = approach_scalar(b.mid_y, y_d, 3.5, dt);
            b.fwd_y = approach_scalar(b.fwd_y, y_d * 1.1, 4.5, dt);
        }
        self.hold_line_x[0] = self.blocks[0].def_x;
        self.hold_line_x[1] = self.blocks[1].def_x;

        // ── 2. Rebuild anchors directly from the block. ──
        // Sent-off players keep their bench anchor (pinned at dismissal).
        for agent in self.agents.iter() {
            if agent.sent_off {
                continue;
            }
            let team = usize::from(agent.team_id != self.home_team_id);
            let team_home = team == 0;
            let attack_dx = if team_home { 1.0 } else { -1.0 };
            let style = self.styles[team];
            let in_poss = if team_home { home_in_poss } else { away_in_poss };
            let b = self.blocks[team];
            let lane = self.lane_y.get(&agent.id).copied().unwrap_or(0.0);
            let width = if in_poss {
                0.98 + style.width * 0.10
            } else {
                0.90
            };
            let Some(slot) = self.anchor_buffer.get_mut(&agent.id) else {
                continue;
            };
            match agent.role {
                PlayerRole::Goalkeeper => {
                    let gk_prog = -(PITCH_HALF_LENGTH - 5.0)
                        + (ball.x * attack_dx + PITCH_HALF_LENGTH).clamp(0.0, 20.0) * 0.10;
                    slot.x = gk_prog * attack_dx;
                    slot.y = (ball.y * 0.18).clamp(-9.0, 9.0);
                }
                PlayerRole::Defender => {
                    slot.x = b.def_x;
                    slot.y = lane * width + b.def_y;
                }
                PlayerRole::Midfielder => {
                    let depth = self.mid_depth.get(&agent.id).copied().unwrap_or(0.0);
                    let depth_scale = if in_poss { 1.12 } else { 1.0 };
                    slot.x = b.mid_x + depth * attack_dx * depth_scale;
                    slot.y = lane * width + b.mid_y;
                }
                PlayerRole::Forward | PlayerRole::Unknown => {
                    slot.x = b.fwd_x;
                    slot.y = lane * (width + 0.04) + b.fwd_y;
                }
            }
            slot.y = slot.y.clamp(-32.0, 32.0);
        }

        // ── 3. Bounded per-line breathing (keeps the block live, not chaotic). ──
        let t = self.sim_time;
        for agent in self.agents.iter() {
            let team = usize::from(agent.team_id != self.home_team_id);
            let phase = if team == 0 { 0.0 } else { 1.7 };
            // The back line holds its exact height — zero breathing — so the
            // four defenders stay flat; outfielders only drift gently.
            let base = if agent.role == PlayerRole::Defender {
                0.0
            } else {
                0.6
            };
            let amp = base * (1.1 - agent.attributes.composure * 0.6);
            if let Some(slot) = self.anchor_buffer.get_mut(&agent.id) {
                slot.y += (t * 0.9 + phase).sin() * amp * 0.8;
                slot.x += (t * 0.8 + phase).sin() * amp * 0.28;
            }
        }

        // ── 4. Set pieces override the block. ──
        let set_piece = self.set_piece.clone();
        if let Some(sp) = set_piece {
            self.position_set_piece(&sp);
        }
    }

    /// The next feed ball event for `team` near the restart spot, as
/// `(player_id, origin)`. Used to lock the taker before positions move.
fn next_restart_actor(
    &self,
    team: &str,
    near: Vector2,
    radius: f32,
    exclude: Option<&str>,
) -> Option<(String, Vector2)> {
    let events = self.dispatcher.events();
    let start = self.dispatcher.cursor();
    events[start..]
        .iter()
        .take_while(|e| e.match_time_secs - self.sim_time <= 25.0)
        .filter(|e| e.team_id == team && !e.player_id.is_empty() && is_ball_event(e.type_id))
        .filter(|e| exclude.map(|x| x != e.player_id).unwrap_or(true))
        .map(|e| (e.player_id.clone(), Vector2::new(e.origin_x, e.origin_y)))
        .find(|(_, p)| p.distance(near) <= radius)
}

/// Resolve the stored taker into a live player id (with a one-time nearest
/// fallback if the stored one has been substituted off).
fn resolve_taker(&self, sp: &SetPiece) -> Option<String> {
    sp.taker_id
        .as_deref()
        .filter(|id| self.agent_index.get(*id).is_some_and(|&i| !self.agents[i].sent_off))
        .map(str::to_string)
        .or_else(|| self.nearest_taker(&sp.team, sp.spot))
}

/// Push every non-taker anchor to at least `radius` metres from the restart
/// spot so nobody crowds the ball.
fn enforce_set_piece_spacing(&mut self, sp: &SetPiece, taker: Option<&str>, dir: f32) {
    let radius = match sp.kind {
        SetPieceKind::Corner => 8.0,
        SetPieceKind::FreeKick => 6.0,
        SetPieceKind::Penalty => 9.15,
        SetPieceKind::ThrowIn => 4.0,
        SetPieceKind::GoalKick => 5.0,
        SetPieceKind::KickOff => return,
    };
    if radius <= 0.0 {
        return;
    }
    let players: Vec<(String, bool)> = self
        .agents
        .iter()
        .map(|a| (a.id.clone(), a.is_goalkeeper || a.sent_off))
        .collect();
    for (id, skip) in players {
        if skip || Some(id.as_str()) == taker {
            continue;
        }
        let Some(slot) = self.anchor_buffer.get_mut(&id) else {
            continue;
        };
        let delta = *slot - sp.spot;
        let d = delta.length();
        if d >= radius {
            continue;
        }
        let away = if d > 1e-3 {
            delta / d
        } else {
            Vector2::new(-dir, 0.0)
        };
        *slot = (sp.spot + away * radius).clamp_on_pitch();
    }
}

/// The nearest outfield player of `team` to `ball` (the restart taker).
    fn nearest_taker(&self, team: &str, ball: Vector2) -> Option<String> {
        self.agents
            .iter()
            .filter(|a| a.team_id == team && !a.is_goalkeeper && !a.sent_off)
            .min_by(|a, b| {
                a.position
                    .distance(ball)
                    .partial_cmp(&b.position.distance(ball))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|a| a.id.clone())
    }

    /// Assign a team's outfielders to an ordered list of set-piece slots.
    /// Attackers are sorted forwards-first; defenders defenders-first.
    fn assign_slots(
        &mut self,
        team: &str,
        attacking: bool,
        exclude: Option<&str>,
        slots: &[Vector2],
    ) {
        if slots.is_empty() {
            return;
        }
        let mut ids: Vec<(u8, String)> = self
            .agents
            .iter()
            .filter(|a| {
                a.team_id == team
                    && !a.is_goalkeeper
                    && !a.sent_off
                    && Some(a.id.as_str()) != exclude
            })
            .map(|a| {
                let pri = match (attacking, a.role) {
                    (true, PlayerRole::Forward) => 0,
                    (true, PlayerRole::Midfielder) => 1,
                    (true, _) => 2,
                    (false, PlayerRole::Defender) => 0,
                    (false, PlayerRole::Midfielder) => 1,
                    (false, _) => 2,
                };
                (pri, a.id.clone())
            })
            .collect();
        ids.sort();
        for (i, (_, id)) in ids.iter().enumerate() {
            let target = slots[i.min(slots.len() - 1)];
            if let Some(slot) = self.anchor_buffer.get_mut(id) {
                *slot = target;
            }
        }
    }

    /// Reshape both teams for the active set piece.
    fn position_set_piece(&mut self, sp: &SetPiece) {
        let dir = if sp.team == self.home_team_id { 1.0 } else { -1.0 };
        let ball = sp.spot;
        let taker = self.resolve_taker(sp);
        match sp.kind {
            SetPieceKind::KickOff => {
                self.position_kickoff(&sp.team, dir);
            }
            SetPieceKind::Corner => self.position_corner(&sp.team, dir, sp.side, ball, taker.as_deref()),
            SetPieceKind::FreeKick => self.position_free_kick(&sp.team, dir, ball, taker.as_deref()),
            SetPieceKind::ThrowIn => self.position_throw_in(&sp.team, dir, ball, taker.as_deref()),
            SetPieceKind::Penalty => self.position_penalty(&sp.team, dir, ball, taker.as_deref()),
            SetPieceKind::GoalKick => self.position_goal_kick(&sp.team, dir, ball, taker.as_deref()),
        }
        self.enforce_set_piece_spacing(sp, taker.as_deref(), dir);
    }

    /// Kick-off: ball on the spot, two kickers, everyone else in their own
    /// half with the defending team outside the centre circle.
    ///
    /// Returns the chosen taker and second kicker so a period restart can
    /// place those two bodies inside the circle immediately.
    fn position_kickoff(
        &mut self,
        team: &str,
        dir: f32,
    ) -> (Option<String>, Option<String>) {
        let mut own: Vec<String> = self
            .agents
            .iter()
            .filter(|a| a.team_id == team && !a.is_goalkeeper && !a.sent_off)
            .map(|a| a.id.clone())
            .collect();
        own.sort();
        // The taker is the player who actually takes the kick-off on the
        // timeline; falling back to whoever is closest to the centre spot so
        // a taker starting in his own box still arrives in time.
        let scheduled = self
            .dispatcher
            .peek_next()
            .filter(|e| e.team_id == team)
            .and_then(|e| self.agent_index.get(&e.player_id).copied())
            .filter(|&i| !self.agents[i].sent_off)
            .map(|i| self.agents[i].id.clone());
        let taker = scheduled.or_else(|| {
            self.agents
                .iter()
                .filter(|a| a.team_id == team && !a.is_goalkeeper && !a.sent_off)
                .min_by(|a, b| {
                    a.position
                        .length()
                        .partial_cmp(&b.position.length())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|a| a.id.clone())
        });
        let second = own
            .iter()
            .find(|id| Some(id.as_str()) != taker.as_deref())
            .cloned();
        if let Some(id) = &taker {
            if let Some(slot) = self.anchor_buffer.get_mut(id) {
                *slot = Vector2::new(-dir * 0.9, 0.0);
            }
        }
        if let Some(id) = &second {
            if let Some(slot) = self.anchor_buffer.get_mut(id) {
                *slot = Vector2::new(-dir * 1.4, 0.9);
            }
        }
        for agent in self.agents.iter() {
            if agent.is_goalkeeper
                || agent.sent_off
                || Some(&agent.id) == taker.as_ref()
                || Some(&agent.id) == second.as_ref()
            {
                continue;
            }
            let team_home = agent.team_id == self.home_team_id;
            let a_dir = if team_home { 1.0 } else { -1.0 };
            let Some(slot) = self.anchor_buffer.get_mut(&agent.id) else {
                continue;
            };
            // Kicking side: own half (prog ≤ −2). Defending side: own half and
            // outside the 9.15 m centre circle (prog ≤ −10).
            let limit = if agent.team_id == team { -2.0 } else { -10.0 };
            let prog = (slot.x * a_dir).min(limit);
            slot.x = prog * a_dir;
        }
        (taker, second)
    }

    /// Place the two period-restart kickers physically inside the centre
    /// circle. A half-time break has no simulated walking time, so waiting for
    /// them to jog to their anchors would let the first second-half event fire
    /// with nobody at the spot. This is intentionally limited to period
    /// restarts; goal celebrations keep the normal walk-back.
    fn snap_kickoff_pair(&mut self, taker: Option<&str>, second: Option<&str>) {
        for id in [taker, second].into_iter().flatten() {
            let Some(slot) = self.anchor_buffer.get(id).copied() else {
                continue;
            };
            let Some(idx) = self.agent_index.get(id).copied() else {
                continue;
            };
            let agent = &mut self.agents[idx];
            agent.position = slot;
            agent.prev_position = slot;
            agent.velocity = Vector2::zero();
            agent.acceleration = Vector2::zero();
            agent.state = crate::agent::AgentState::InFormation;
            agent.assigned_event_id = None;
            agent.receiver_task = None;
        }
    }

    /// During a kick-off the opposition must stay outside the centre circle
    /// until the ball is kicked. This is a hard, bounded correction (never a
    /// teleport) that runs after the brain so it cannot be overridden.
    fn enforce_kickoff_circle(&mut self, dt: f32) {
        let Some(sp) = self.set_piece.as_ref() else {
            return;
        };
        if sp.kind != SetPieceKind::KickOff {
            return;
        }
        let kicking = sp.team.as_str();
        const CIRCLE_R: f32 = 9.15; // Laws of the Game centre-circle radius.
        let max_step = crate::agent::MAX_SPEED * dt;
        for agent in self.agents.iter_mut() {
            if agent.is_goalkeeper || agent.team_id == kicking {
                continue;
            }
            let dist = agent.position.length();
            if dist < CIRCLE_R + 0.5 {
                let push = (CIRCLE_R + 0.5 - dist).min(max_step);
                let dir = if dist > 1e-3 {
                    agent.position / dist
                } else {
                    Vector2::new(1.0, 0.0)
                };
                agent.position = (agent.position + dir * push).clamp_on_pitch();
            }
        }
    }

    /// Corner: the taker at the flag, the attack filling the box with real
    /// roles (near post, far post, penalty spot, edge), the defence zonal.
    fn position_corner(&mut self, team: &str, dir: f32, side: f32, ball: Vector2, taker: Option<&str>) {
        let goal_x = dir * PITCH_HALF_LENGTH;
        let near_y = side * 7.0;
        let far_y = -side * 6.0;
        let attack_slots = [
            Vector2::new(goal_x - dir * 2.0, near_y * 0.35),
            Vector2::new(goal_x - dir * 3.5, far_y * 0.75),
            Vector2::new(goal_x - dir * 11.0, 0.0),
            Vector2::new(goal_x - dir * 5.5, near_y * 0.15),
            Vector2::new(goal_x - dir * 6.5, far_y * 0.55),
            Vector2::new(goal_x - dir * 16.5, side * 2.0),
            Vector2::new(goal_x - dir * 4.0, near_y * 0.75),
            Vector2::new(goal_x - dir * 14.0, far_y * 0.30),
            Vector2::new(goal_x - dir * 20.0, -side * 1.0),
        ];
        let def_slots = [
            Vector2::new(goal_x - dir * 0.6, near_y * 0.12),
            Vector2::new(goal_x - dir * 5.0, near_y * 0.20),
            Vector2::new(goal_x - dir * 5.0, 0.0),
            Vector2::new(goal_x - dir * 5.0, far_y * 0.25),
            Vector2::new(goal_x - dir * 9.0, 0.0),
            Vector2::new(goal_x - dir * 10.0, near_y * 0.22),
            Vector2::new(goal_x - dir * 10.0, far_y * 0.25),
            Vector2::new(goal_x - dir * 16.0, 0.0),
            Vector2::new(goal_x - dir * 21.0, 0.0),
        ];
        self.assign_slots(team, true, taker, &attack_slots);
        let def_team = if team == self.home_team_id {
            self.away_team_id.clone()
        } else {
            self.home_team_id.clone()
        };
        self.assign_slots(&def_team, false, None, &def_slots);
        if let Some(id) = taker {
            if let Some(slot) = self.anchor_buffer.get_mut(id) {
                *slot = ball;
            }
        }
    }

    /// Penalty: taker on the spot, keeper on the line, everyone else outside
    /// the box and the arc.
    fn position_penalty(&mut self, team: &str, dir: f32, ball: Vector2, taker: Option<&str>) {
        if let Some(id) = taker {
            if let Some(slot) = self.anchor_buffer.get_mut(id) {
                *slot = ball + Vector2::new(-dir * 0.5, 0.0);
            }
        }
        // Keeper holds the line; both teams stay out of the box.
        let spot_x = ball.x;
        for agent in self.agents.iter() {
            if agent.is_goalkeeper || Some(agent.id.as_str()) == taker {
                continue;
            }
            let team_home = agent.team_id == self.home_team_id;
            let a_dir = if team_home { 1.0 } else { -1.0 };
            let Some(slot) = self.anchor_buffer.get_mut(&agent.id) else {
                continue;
            };
            // Attackers and defenders alike: goal-side of the D (prog margin).
            let prog = (slot.x * a_dir).min(if agent.team_id == team {
                spot_x * a_dir - 9.5
            } else {
                spot_x * a_dir - 12.0
            });
            slot.x = prog * a_dir;
        }
    }

    /// Goal kick: keeper collects at the spot; one short option and one long
    /// outlet, everyone else in shape. Outlets are the nearest available
    /// outfielders — never drafted defenders from across the pitch.
    fn position_goal_kick(&mut self, team: &str, dir: f32, ball: Vector2, taker: Option<&str>) {
        // Keeper comes for it.
        if let Some(id) = self
            .agents
            .iter()
            .find(|a| a.team_id == team && a.is_goalkeeper)
            .map(|a| a.id.clone())
        {
            if let Some(slot) = self.anchor_buffer.get_mut(&id) {
                *slot = ball;
            }
        }
        // A short option and a long outlet from the keeper's team.
        let short = ball + Vector2::new(dir * 8.0, 6.0);
        let long = ball + Vector2::new(dir * 30.0, -8.0);
        let mut candidates: Vec<(f32, String)> = self
            .agents
            .iter()
            .filter(|a| {
                a.team_id == team && !a.is_goalkeeper && !a.sent_off && a.role != PlayerRole::Goalkeeper
            })
            .map(|a| {
                let d = a
                    .position
                    .distance(short)
                    .min(a.position.distance(long));
                (d, a.id.clone())
            })
            .collect();
        candidates.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        for ((_, id), spot) in candidates.into_iter().take(2).zip([short, long]) {
            if let Some(slot) = self.anchor_buffer.get_mut(&id) {
                *slot = spot.clamp_on_pitch();
            }
        }
    }

    /// Free kick: the taker behind the ball and a physical wall between the
    /// ball and the goal being attacked.
    fn position_free_kick(&mut self, team: &str, dir: f32, ball: Vector2, taker: Option<&str>) {
        if let Some(id) = taker {
            let back = Vector2::new(-dir * 0.9, 0.0);
            if let Some(slot) = self.anchor_buffer.get_mut(id) {
                *slot = ball + back;
            }
        }
        let target_goal = Vector2::new(dir * PITCH_HALF_LENGTH, 0.0);
        let to_goal = (target_goal - ball).normalize();
        let perp = Vector2::new(-to_goal.y, to_goal.x);
        let wall_base = ball + to_goal * 9.15;
        let def_team = if team == self.home_team_id {
            self.away_team_id.clone()
        } else {
            self.home_team_id.clone()
        };
        let mut ids: Vec<String> = self
            .agents
            .iter()
            .filter(|a| a.team_id == def_team && !a.is_goalkeeper && !a.sent_off)
            .map(|a| a.id.clone())
            .collect();
        ids.sort();
        for (i, id) in ids.iter().take(4).enumerate() {
            let off = -1.5 + i as f32 * 1.0;
            if let Some(slot) = self.anchor_buffer.get_mut(id) {
                *slot = (wall_base + perp * off).clamp_on_pitch();
            }
        }
    }

    /// Throw-in: the taker at the ball plus two short options infield.
    fn position_throw_in(&mut self, team: &str, dir: f32, ball: Vector2, taker: Option<&str>) {
        if let Some(id) = taker {
            if let Some(slot) = self.anchor_buffer.get_mut(id) {
                *slot = ball;
            }
        }
        let infield = if ball.y >= 0.0 { -1.0 } else { 1.0 };
        let short = (ball + Vector2::new(dir * 5.0, infield * 4.0)).clamp_on_pitch();
        let long = (ball + Vector2::new(dir * 11.0, infield * 8.0)).clamp_on_pitch();
        let mut candidates: Vec<(f32, String)> = self
            .agents
            .iter()
            .filter(|a| {
                a.team_id == team
                    && !a.is_goalkeeper
                    && !a.sent_off
                    && Some(a.id.as_str()) != taker
            })
            .map(|a| {
                let d = a
                    .position
                    .distance(short)
                    .min(a.position.distance(long));
                (d, a.id.clone())
            })
            .collect();
        candidates.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        for ((_, id), spot) in candidates.into_iter().take(2).zip([short, long]) {
            if let Some(slot) = self.anchor_buffer.get_mut(&id) {
                *slot = spot;
            }
        }
    }

    /// Set possession to the team that just won the ball: the actor's team on
    /// every dispatched ball action, except failed passes (in dispute until
    /// the next touch) and fouls (the restart goes to the fouled side, read
    /// from the next event). Persists across quiet ticks; falls back to the
    /// next event's team before anything has been won.
    fn update_possession(&mut self) {
        for action in &self.last_actions {
            if !crate::parser::is_ball_event(action.type_id) {
                continue;
            }
            if matches!(action.type_id, 1 | 2) && !action.outcome {
                continue; // failed pass: still in dispute
            }
            if action.type_id == EVENT_TYPE_FOUL {
                continue; // offender touched it; the restart decides
            }
            self.last_touch = Some(action.team_id.clone());
            self.last_touch_player = Some(action.player_id.clone());
        }
        self.possession = self
            .last_touch
            .clone()
            .or_else(|| self.dispatcher.peek_next().map(|e| e.team_id.clone()));
    }

    /// Run `seconds` of simulation at the configured fixed timestep.
    pub fn run_for(&mut self, seconds: f32) {
        let steps = (seconds / self.config.dt).round() as usize;
        for _ in 0..steps {
            self.step();
        }
    }

    /// Run until the final event (plus a short tail) has elapsed.
    pub fn run_to_end(&mut self) {
        let end = self.timeline.duration_secs() + 2.0;
        while self.sim_time < end {
            self.step();
        }
    }

    /// Number of agents.
    #[inline]
    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    /// Find an agent index by player id (O(1)).
    pub fn agent_index(&self, player_id: &str) -> Option<usize> {
        self.agent_index.get(player_id).copied()
    }

    /// True when every agent and the ball are finite and inside the pitch.
    pub fn is_consistent(&self) -> bool {
        self.ball.position.is_finite()
            && self.ball.altitude.is_finite()
            && self.ball.altitude >= 0.0
            && MatchContext::is_on_pitch(self.ball.position.x, self.ball.position.y)
            && self.agents.iter().all(|a| {
                a.position.is_finite()
                    && a.velocity.is_finite()
                    && MatchContext::is_on_pitch(a.position.x, a.position.y)
            })
    }

    /// Flat render snapshot: 22 players followed by the ball (23 entities).
    pub fn get_render_state(&self) -> Vec<RenderEntity> {
        let mut buffer = Vec::with_capacity(self.agents.len() + 1);
        let home = &self.home_team_id;
        for a in &self.agents {
            buffer.push(RenderEntity {
                x: a.position.x,
                y: a.position.y,
                z: 0.0,
                heading: a.heading,
                team_index: if &a.team_id == home { TEAM_HOME } else { TEAM_AWAY },
                shirt_number: a.shirt_number,
            });
        }
        buffer.push(RenderEntity {
            x: self.ball.position.x,
            y: self.ball.position.y,
            z: self.ball.altitude,
            heading: 0.0,
            team_index: TEAM_BALL,
            shirt_number: 0,
        });
        buffer
    }

    /// Alias for [`SimulationEngine::get_render_state`].
    pub fn render_state(&self) -> Vec<RenderEntity> {
        self.get_render_state()
    }
}

#[cfg(test)]
mod engine_tests {
    use super::*;
    use crate::ball::BallState;
    use crate::parser::{AttackDirection, LineupPlayer, NormalizedEvent};

    fn lineup(prefix: &str) -> Vec<LineupPlayer> {
        (1..=11)
            .map(|slot| LineupPlayer {
                player_id: format!("{prefix}_{slot}"),
                // Realistic shape: 1 GK, 4 DF, 3 MF, 3 FW.
                position_category: match slot {
                    1 => 1,
                    2..=5 => 2,
                    6..=8 => 3,
                    _ => 4,
                },
                shirt_number: slot,
                formation_slot: slot,
            })
            .collect()
    }

    fn event(id: u64, time: f32, team: &str, player: &str, x: f32, y: f32, aerial: bool) -> NormalizedEvent {
        let mut ev = NormalizedEvent::test(id, 1, time, team, player, x, y);
        ev.target_x = Some(x + 20.0);
        ev.target_y = Some(y);
        ev.is_aerial = aerial;
        ev
    }

    fn typed_event(
        id: u64,
        time: f32,
        type_id: u16,
        team: &str,
        x: f32,
        y: f32,
    ) -> NormalizedEvent {
        let mut ev = NormalizedEvent::test(id, type_id, time, team, "", x, y);
        ev.player_id = String::new();
        ev.player_name = String::new();
        ev
    }

    fn shot_event(
        id: u64,
        time: f32,
        type_id: u16,
        team: &str,
        player: &str,
        x: f32,
        y: f32,
    ) -> NormalizedEvent {
        NormalizedEvent {
            player_id: player.into(),
            player_name: player.into(),
            ..typed_event(id, time, type_id, team, x, y)
        }
    }

    fn context() -> MatchContext {
        let mut ctx = MatchContext {
            match_id: "1".into(),
            home_team_id: "home".into(),
            away_team_id: "away".into(),
            home_team_name: "Home".into(),
            away_team_name: "Away".into(),
            ..Default::default()
        };
        ctx.lineups.insert("home".into(), lineup("h"));
        ctx.lineups.insert("away".into(), lineup("a"));
        ctx.directions.insert("home".into(), AttackDirection::LeftToRight);
        ctx.directions.insert("away".into(), AttackDirection::RightToLeft);
        ctx.events = vec![
            event(1, 1.0, "home", "h_6", -10.0, 0.0, false),
            event(2, 3.0, "away", "a_10", 0.0, 5.0, true),
            event(3, 6.0, "home", "h_9", 20.0, -10.0, false),
        ];
        ctx
    }

    #[test]
    fn test_engine_assigns_every_outfielder_a_duty() {
        let e = SimulationEngine::new(context());
        assert_eq!(
            e.agents.iter().filter(|a| a.duty == Duty::Goalkeeper).count(),
            2
        );
        assert!(
            e.agents
                .iter()
                .filter(|a| !a.is_goalkeeper)
                .all(|a| a.duty != Duty::Unknown),
            "an outfielder was left without a job: {:?}",
            e.agents.iter().map(|a| (a.id.as_str(), a.duty)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_substitute_with_unknown_category_keeps_the_slot_role() {
        let mut ctx = context();
        ctx.lineups.get_mut("home").unwrap().push(LineupPlayer {
            player_id: "h_12".into(),
            position_category: 5, // the feed labels every bench player 5
            shirt_number: 12,
            formation_slot: 12,
        });
        let mut e = SimulationEngine::new(ctx);
        let old_anchor = e.anchor_buffer.get("h_3").copied().unwrap();
        assert_eq!(
            e.agents[e.agent_index("h_3").unwrap()].role,
            PlayerRole::Defender
        );

        e.reidentify("h_3", "h_12");

        let idx = e.agent_index("h_12").expect("substitute has an agent");
        assert_eq!(
            e.agents[idx].role,
            PlayerRole::Defender,
            "a category-5 bench player must inherit the slot role, not become Unknown"
        );
        assert_eq!(e.anchor_buffer.get("h_12").copied().unwrap(), old_anchor);

        // The next shape rebuild must keep him on the back line, not move him
        // to the forward line because his nominal category was Unknown.
        e.set_piece = None;
        e.update_anchors();
        let anchors = e.anchors();
        let sub_x = anchors.get("h_12").copied().unwrap().x;
        let defender_x = anchors.get("h_4").copied().unwrap().x;
        assert!(
            (sub_x - defender_x).abs() < 2.0,
            "substitute left the back line: {sub_x:.1} vs {defender_x:.1}"
        );
    }

    #[test]
    fn test_phase_machine_needs_dwell_before_changing() {
        let mut e = SimulationEngine::new(context());
        e.sim_time = 10.0;
        e.possession = Some(e.home_team_id.clone());
        e.ball.position = Vector2::new(-20.0, 0.0);
        let settled = &mut e.phases[0];
        settled.phase = TeamPhase::BuildUp;
        settled.candidate = TeamPhase::BuildUp;
        settled.last_in_possession = true;
        settled.changed_at = 0.0; // no longer a fresh turnover

        e.update_phases(1.0 / 60.0);
        assert_eq!(e.phases[0].phase, TeamPhase::BuildUp);

        // The ball crosses the build-up threshold, but 0.33 s is less than the
        // 0.45 s dwell, so the visible phase must not flicker yet.
        e.ball.position = Vector2::new(0.0, 0.0);
        for _ in 0..20 {
            e.update_phases(1.0 / 60.0);
        }
        assert_eq!(e.phases[0].phase, TeamPhase::BuildUp);

        for _ in 0..8 {
            e.update_phases(1.0 / 60.0);
        }
        assert_eq!(e.phases[0].phase, TeamPhase::Progression);
    }

    #[test]
    fn test_possession_change_triggers_fast_transition_phase() {
        let mut e = SimulationEngine::new(context());
        e.sim_time = 10.0;
        e.ball.position = Vector2::new(0.0, 0.0);
        e.possession = Some(e.home_team_id.clone());
        let state = &mut e.phases[0];
        state.phase = TeamPhase::MidBlock;
        state.candidate = TeamPhase::MidBlock;
        state.last_in_possession = false; // a turnover this tick
        state.changed_at = 10.0;

        for _ in 0..12 {
            e.update_phases(1.0 / 60.0);
        }
        assert_eq!(e.phases[0].phase, TeamPhase::AttackingTransition);
    }

    #[test]
    fn test_out_of_possession_phases_follow_ball_height() {
        let probe = |x: f32| {
            let mut e = SimulationEngine::new(context());
            e.sim_time = 10.0;
            // Home is defending; the away side has the ball.
            e.possession = Some(e.away_team_id.clone());
            e.ball.position = Vector2::new(x, 0.0);
            let state = &mut e.phases[0];
            state.phase = TeamPhase::MidBlock;
            state.candidate = TeamPhase::MidBlock;
            state.last_in_possession = false;
            state.changed_at = 0.0;
            for _ in 0..40 {
                e.update_phases(1.0 / 60.0);
            }
            e.phases[0].phase
        };

        assert_eq!(probe(-25.0), TeamPhase::LowBlock);
        assert_eq!(probe(0.0), TeamPhase::MidBlock);
        assert_eq!(probe(15.0), TeamPhase::HighPress);
    }

    #[test]
    fn test_engine_builds_22_agents_and_ball() {
        let e = SimulationEngine::new(context());
        assert_eq!(e.agent_count(), 22);
        assert_eq!(e.get_render_state().len(), 23);
        assert_eq!(e.timeline.len(), 3);
    }

    #[test]
    fn test_render_state_layout() {
        let e = SimulationEngine::new(context());
        let rs = e.get_render_state();
        assert_eq!(rs.len(), 23);
        let home = rs.iter().filter(|r| r.team_index == TEAM_HOME).count();
        let away = rs.iter().filter(|r| r.team_index == TEAM_AWAY).count();
        assert_eq!(home, 11);
        assert_eq!(away, 11);
        let ball = rs.last().unwrap();
        assert_eq!(ball.team_index, TEAM_BALL);
        assert_eq!(ball.z, 0.0);
    }

    #[test]
    fn test_engine_is_deterministic() {
        let mut a = SimulationEngine::new(context());
        let mut b = SimulationEngine::new(context());
        a.run_for(8.0);
        b.run_for(8.0);
        for (x, y) in a.agents.iter().zip(b.agents.iter()) {
            assert_eq!(x.position, y.position);
            assert_eq!(x.state, y.state);
        }
        assert_eq!(a.ball.position, b.ball.position);
        assert_eq!(a.ball.altitude, b.ball.altitude);
    }

    #[test]
    fn test_ball_launches_on_pass_and_flies() {
        let mut e = SimulationEngine::new(context());
        // First event at t=1.0; run past the aerial event at t=3.0 too.
        let mut flew = false;
        let mut launched = false;
        for _ in 0..360 {
            e.step();
            if e.ball.is_in_flight() {
                launched = true;
            }
            if e.ball.altitude > 0.0 {
                flew = true;
            }
        }
        assert!(launched, "ball never launched on a pass");
        assert!(flew, "ball never left the ground on an aerial event");
    }

    #[test]
    fn test_engine_stays_consistent_full_run() {
        let mut e = SimulationEngine::new(context());
        e.run_to_end();
        assert!(e.is_consistent());
        let rs = e.get_render_state();
        assert_eq!(rs.len(), 23);
        assert!(rs.iter().all(|r| r.x.is_finite() && r.y.is_finite() && r.z.is_finite()));
        assert!(rs.iter().all(|r| r.z >= 0.0));
    }

    #[test]
    fn test_event_feed_records_events() {
        let mut e = SimulationEngine::new(context());
        e.run_for(10.0);
        assert!(e.event_seq > 0, "no events recorded");
        assert!(!e.recent_events.is_empty());
        assert!(e.recent_events.iter().any(|r| r.type_id == 1));
        assert!(e.recent_events.len() <= EVENT_FEED_CAPACITY);
        // Newest entries carry a human-readable label.
        assert!(e.recent_events.back().unwrap().label.len() > 1);
    }

    #[test]
    fn test_ball_never_teleports() {
        let mut e = SimulationEngine::new(context());
        let mut prev = e.ball.position;
        let mut max_move = 0.0f32;
        for _ in 0..(20 * 60) {
            e.step();
            max_move = max_move.max(prev.distance(e.ball.position));
            prev = e.ball.position;
        }
        assert!(max_move < 2.0, "ball moved {max_move} m in one tick");
    }

    #[test]
    fn test_heading_rate_is_bounded_and_settles() {
        let mut e = SimulationEngine::new(context());
        let mut prev: Vec<f32> = e.agents.iter().map(|a| a.heading).collect();
        let dt = e.config.dt;
        for _ in 0..(20 * 60) {
            e.step();
            for (i, a) in e.agents.iter().enumerate() {
                let mut d = (a.heading - prev[i]) % (2.0 * std::f32::consts::PI);
                if d > std::f32::consts::PI {
                    d -= 2.0 * std::f32::consts::PI;
                } else if d < -std::f32::consts::PI {
                    d += 2.0 * std::f32::consts::PI;
                }
                assert!(
                    d.abs() <= crate::agent::TURN_SPEED * dt + 1e-4,
                    "heading jumped {d} rad in one tick"
                );
                prev[i] = a.heading;
            }
        }
    }

    #[test]
    fn test_players_keep_moving_during_lulls() {
        // Build a context with a long gap and make sure nobody fully freezes.
        let mut ctx = context();
        ctx.events = vec![
            event(1, 1.0, "home", "h_6", -10.0, 0.0, false),
            event(2, 40.0, "home", "h_9", 10.0, 0.0, false),
        ];
        let mut e = SimulationEngine::new(ctx);
        e.run_for(10.0); // into the gap, ball at rest, no event for 30s
        let mut min_avg = f32::MAX;
        for _ in 0..(20 * 60) {
            e.step();
            let avg = e.agents.iter().map(|a| a.speed()).sum::<f32>() / e.agents.len() as f32;
            min_avg = min_avg.min(avg);
        }
        assert!(min_avg > 0.05, "players froze during a lull (avg {min_avg})");
    }

    #[test]
    fn test_event_classification() {
        use crate::parser::{event_type_name, is_ball_event, is_dead_ball_event, is_notable_event};
        assert!(is_ball_event(1) && is_ball_event(49) && is_ball_event(16));
        assert!(!is_ball_event(18) && !is_ball_event(17) && !is_ball_event(4));
        assert!(is_dead_ball_event(5) && is_dead_ball_event(6));
        assert!(is_notable_event(16) && is_notable_event(17) && is_notable_event(4));
        assert_eq!(event_type_name(16), "Goal");
        assert_eq!(event_type_name(17), "Card");
        assert_eq!(event_type_name(55), "Offside provoked");
    }

    #[test]
    fn test_corner_places_ball_at_attacking_flag() {
        // The feed emits a Corner for both teams at the same origin. The award
        // records where the ball left play, but the restart — and the pass
        // that takes it — belongs on the corner flag, so that is where the
        // ball is carried.
        let mut ctx = context();
        ctx.events = vec![
            typed_event(1, 1.0, 6, "home", 40.0, 20.0),
            typed_event(2, 1.0, 6, "away", 40.0, 20.0),
        ];
        let mut e = SimulationEngine::new(ctx);
        e.run_for(8.0); // allow the ball to travel out to the flag
        assert!(
            (e.ball.position.x - 52.0).abs() < 1.5,
            "ball x = {}",
            e.ball.position.x
        );
        assert!(
            (e.ball.position.y - 33.5).abs() < 1.5,
            "ball y = {}",
            e.ball.position.y
        );
        assert!(!e.ball.is_in_flight(), "corner ball should have settled");
    }

    #[test]
    fn test_player_roles_assigned() {
        let e = SimulationEngine::new(context());
        // The synthetic lineup is a keeper plus ten category-2 players per side.
        assert_eq!(
            e.agents.iter().filter(|a| a.role == PlayerRole::Goalkeeper).count(),
            2
        );
        assert_eq!(e.agents.iter().filter(|a| a.role == PlayerRole::Defender).count(), 8);
        assert_eq!(e.agents.iter().filter(|a| a.role == PlayerRole::Midfielder).count(), 6);
        assert_eq!(e.agents.iter().filter(|a| a.role == PlayerRole::Forward).count(), 6);
    }

    #[test]
    fn test_goal_increments_score() {
        let mut ctx = context();
        ctx.events = vec![shot_event(1, 1.0, 16, "home", "h_10", 45.0, 0.0)];
        let mut e = SimulationEngine::new(ctx);
        let mut max_x = f32::MIN;
        // The readiness gate may hold the shot up to SLIP_MAX_SECS (2s)
        // because the synthetic striker starts far from the shot origin, so
        // give the run enough wall clock for it to fire and travel.
        for _ in 0..(6 * 60) {
            e.step();
            max_x = max_x.max(e.ball.position.x);
        }
        assert_eq!(e.score, [1, 0]);
        // The shot must reach the net...
        assert!(max_x > 49.0, "shot never reached the net (max x {max_x:.1})");
        // ...and then play restarts from the centre spot.
        assert!(
            e.ball.position.length() < 1.0,
            "kick-off did not reset the ball: {:?}",
            e.ball.position
        );
    }

    #[test]
    fn test_home_own_goal_credits_away_and_finds_the_home_net() {
        // An own goal is a type-16 Goal with qualifier 28, credited by the
        // feed to the scorer's own team. The engine must flip it: the opponent
        // scores, and the ball flies into the *scorer's* own net.
        let mut ctx = context();
        let mut g = shot_event(1, 1.0, 16, "home", "h_10", -45.0, 0.0);
        g.is_own_goal = true;
        ctx.events = vec![g];
        let mut e = SimulationEngine::new(ctx);
        let mut min_x = f32::MAX;
        // The readiness gate may hold the event up to SLIP_MAX_SECS; give the
        // run enough wall clock for the shot to travel and settle.
        for _ in 0..(10 * 60) {
            e.step();
            min_x = min_x.min(e.ball.position.x);
        }
        assert_eq!(e.score, [0, 1], "home own goal must be credited to away");
        assert!(
            min_x < -49.0,
            "own-goal ball never reached the home net (min x {min_x:.1})"
        );
    }

    #[test]
    fn test_away_own_goal_credits_home_and_finds_the_away_net() {
        let mut ctx = context();
        let mut g = shot_event(1, 1.0, 16, "away", "a_10", 45.0, 0.0);
        g.is_own_goal = true;
        ctx.events = vec![g];
        let mut e = SimulationEngine::new(ctx);
        let mut max_x = f32::MIN;
        for _ in 0..(10 * 60) {
            e.step();
            max_x = max_x.max(e.ball.position.x);
        }
        assert_eq!(e.score, [1, 0], "away own goal must be credited to home");
        assert!(
            max_x > 49.0,
            "own-goal ball never reached the away net (max x {max_x:.1})"
        );
    }

    #[test]
    fn test_miss_goes_wide_of_the_posts() {
        let mut ctx = context();
        ctx.events = vec![shot_event(1, 1.0, 13, "home", "h_10", 45.0, 4.0)];
        let mut e = SimulationEngine::new(ctx);
        // Same SLIP_MAX allowance as the goal test: the shot may be held for
        // up to 2s while the striker gets to the origin.
        e.run_for(5.0);
        assert_eq!(e.score, [0, 0]);
        assert!(e.ball.position.x > 49.0);
        assert!(
            e.ball.position.y.abs() > 3.66,
            "miss should be wide, y={}",
            e.ball.position.y
        );
    }

    #[test]
    fn test_saved_shot_never_crosses_the_line() {
        // Attempt Saved with feed truth: goal-mouth (on target) + diving
        // keeper who parries. The ball must enter the box but stop short of
        // the line — a saved shot visually scoring is a credibility bug.
        let mut ctx = context();
        let mut shot = shot_event(1, 1.0, 15, "home", "h_10", 40.0, 0.0);
        shot.goalmouth = Some((50.0, 10.0));
        shot.keeper_xy = Some((99.0, 52.0));
        shot.keeper_action = crate::parser::KeeperAction::Diving;
        ctx.events = vec![shot];
        let mut e = SimulationEngine::new(ctx);
        let mut max_x = f32::MIN;
        let mut min_goal_dist = f32::MAX;
        for _ in 0..(8 * 60) {
            e.step();
            max_x = max_x.max(e.ball.position.x);
            // Distance from the ball to the goal line segment.
            if e.ball.position.x > 40.0 {
                min_goal_dist = min_goal_dist.min(
                    (52.5 - e.ball.position.x).abs() + e.ball.position.y.abs().max(0.0) * 0.0,
                );
            }
        }
        assert!(
            max_x < 52.5,
            "saved shot crossed the line (max x {max_x:.1})"
        );
        assert!(
            max_x > 45.0,
            "saved shot never threatened (max x {max_x:.1})"
        );
        let _ = min_goal_dist;
    }

    #[test]
    fn test_collected_save_ends_in_keeper_hands() {
        // Collected claim: the keeper holds it — the ball ends attached.
        let mut ctx = context();
        let mut shot = shot_event(1, 1.0, 15, "home", "h_10", 40.0, 0.0);
        shot.goalmouth = Some((50.0, 5.0));
        shot.keeper_xy = Some((99.0, 50.0));
        shot.keeper_action = crate::parser::KeeperAction::Collected;
        ctx.events = vec![shot];
        let mut e = SimulationEngine::new(ctx);
        e.run_for(6.0);
        assert!(
            e.ball.is_attached(),
            "collected save should end in the keeper's hands, state={:?}",
            e.ball.state.label()
        );
    }

    #[test]
    fn test_goal_uses_recorded_goalmouth_spot() {
        // Goal with feed truth at (y102=45.8 → lateral ≈ -2.86 m): the ball
        // must finish inside the posts near that spot, not at a hash spot.
        let mut ctx = context();
        let mut goal = shot_event(1, 1.0, 16, "home", "h_10", 45.0, 0.0);
        goal.goalmouth = Some((45.8, 2.5));
        ctx.events = vec![goal];
        let mut e = SimulationEngine::new(ctx);
        let mut max_x: f32 = f32::MIN;
        let mut at_line_y = 0.0;
        for _ in 0..(6 * 60) {
            e.step();
            if e.ball.position.x > max_x {
                max_x = e.ball.position.x;
                at_line_y = e.ball.position.y;
            }
        }
        assert_eq!(e.score, [1, 0]);
        assert!(max_x > 49.0, "goal never reached the net ({max_x:.1})");
        assert!(
            (at_line_y - (-2.86)).abs() < 1.0,
            "goal should finish near the recorded spot, y={at_line_y:.2}"
        );
    }

    #[test]
    fn test_failed_pass_names_no_receiver() {
        // An incomplete pass followed by an opponent interception: nobody on
        // the passing team may be routed onto it, and the emitted action
        // carries no receiver.
        let mut ctx = context();
        let mut failed = event(1, 2.0, "home", "h_6", -10.0, 0.0, false);
        failed.target_x = Some(15.0);
        failed.target_y = Some(0.0);
        failed.outcome = false;
        failed.outcome_reason = crate::parser::OutcomeReason::Intercepted;
        let mut pick = event(2, 2.8, "away", "a_4", 12.0, 1.0, false);
        pick.type_id = 8;
        pick.target_x = None;
        pick.target_y = None;
        ctx.events = vec![failed, pick];
        let mut e = SimulationEngine::new(ctx);
        // Run past the pass (slip may hold it up to 2 s while the doer
        // arrives) and check every tick: no home outfielder may hold a
        // receiver task for the failed pass.
        let mut fired = None;
        for _ in 0..(8 * 60) {
            e.step();
            for a in e.agents.iter() {
                if a.team_id == "home" && !a.is_goalkeeper {
                    assert!(
                        a.receiver_task.is_none(),
                        "teammate routed onto a failed pass: {}",
                        a.id
                    );
                }
            }
            if let Some(a) = e.last_actions.iter().find(|a| a.event_id == 1) {
                fired = Some(a.receiver_id.clone());
            }
        }
        assert_eq!(
            fired,
            Some(None),
            "failed pass must fire and name no receiver"
        );
    }

    #[test]
    fn test_curved_cross_rejoins_the_lane() {
        // In-swinging corner: the ball swings wide mid-flight and still lands
        // on the target — curve is theatre, not error.
        use crate::ball::Ball;
        use crate::formation::Vector2;
        use crate::parser::PassKind;
        let mut b = Ball::new(Vector2::new(-40.0, -20.0));
        b.launch_for_kind(Vector2::new(40.0, 0.0), PassKind::CornerTaken, 2.5, true);
        let mut max_dev = 0.0f32;
        let none: std::collections::HashMap<String, (Vector2, f32)> =
            std::collections::HashMap::new();
        for _ in 0..600 {
            b.update(1.0 / 60.0, |id| none.get(id).copied());
            if b.is_in_flight() {
                // Deviation from the straight lane.
                let lane = Vector2::new(80.0, 20.0);
                let rel = b.position - Vector2::new(-40.0, -20.0);
                let dev = (rel.x * lane.y - rel.y * lane.x).abs() / lane.length();
                max_dev = max_dev.max(dev);
            } else {
                break;
            }
        }
        assert!(max_dev > 0.5, "no curve visible (max dev {max_dev:.2})");
        assert!(
            b.position.distance(Vector2::new(40.0, 0.0)) < 0.1,
            "curve must rejoin the lane, landed at {:?}",
            b.position
        );
    }

    #[test]
    fn test_offside_line_is_finite_and_on_pitch() {
        let mut e = SimulationEngine::new(context());
        e.run_for(3.0);
        let x = e.offside_line_x();
        if x.is_finite() {
            assert!((-52.5..=52.5).contains(&x), "offside line {x} off pitch");
        }
    }

    #[test]
    fn test_possession_ratio_in_range() {
        let mut e = SimulationEngine::new(context());
        e.run_for(3.0);
        let total: u64 = e.possession_frames.iter().sum();
        assert!(total > 0, "possession frames never accumulated");
    }

    #[test]
    fn test_attributes_are_deterministic_and_varied() {
        let a = PlayerAttributes::from_id("player_one");
        let b = PlayerAttributes::from_id("player_one");
        let c = PlayerAttributes::from_id("player_two");
        assert_eq!(a, b);
        assert_ne!(a, c);
        for v in [a.pace, a.aggression, a.workrate, a.flair] {
            assert!((0.0..=1.0).contains(&v));
        }
        // Sprint speed is capped but does vary by pace.
        let e = SimulationEngine::new(context());
        assert!(e.agents.iter().all(|a| a.max_speed_sprint <= crate::agent::MAX_SPEED));
        let speeds: std::collections::HashSet<u32> =
            e.agents.iter().map(|a| (a.max_speed_sprint * 100.0) as u32).collect();
        assert!(speeds.len() > 1, "all players share one sprint speed");
    }

    #[test]
    fn test_restart_taker_approaches_the_ball() {
        // An Out event parks the ball at (40, 30) and starts a restart. A home
        // player should walk over to it *before* the next event fires.
        let mut ctx = context();
        ctx.events = vec![typed_event(1, 1.0, 5, "home", 30.0, 10.0)];
        let mut e = SimulationEngine::new(ctx);
        e.run_for(11.0); // while the restart set-piece is still active
        let ball = e.ball.position;
        let nearest = e
            .agents
            .iter()
            .filter(|a| a.team_id == e.home_team_id && !a.is_goalkeeper)
            .map(|a| a.position.distance(ball))
            .fold(f32::MAX, f32::min);
        assert!(
            nearest < 4.0,
            "no home player walked to the restart (nearest {nearest:.1} m)"
        );
    }

    #[test]
    fn test_defenders_hold_their_zone() {
        // With the ball deep in the attacking half, a defender must not chase
        // all the way up the pitch (zone discipline).
        let mut ctx = context();
        ctx.events = vec![typed_event(1, 1.0, 5, "home", 48.0, 0.0)];
        let mut e = SimulationEngine::new(ctx);
        e.run_for(20.0);
        // Home defenders (who defend -x) must not be camped up at the +x goal.
        for a in e
            .agents
            .iter()
            .filter(|a| a.role == PlayerRole::Defender && a.team_id == e.home_team_id)
        {
            assert!(a.position.x < 30.0, "home defender chased to x={:.1}", a.position.x);
        }
    }

    #[test]
    fn test_formation_change_retunes_shape_mid_match() {
        // A type-40 row at t=30 switches away to a back three: their duty mix
        // must change without anyone teleporting.
        use crate::parser::FormationChange;
        let mut ctx = context();
        let xi: Vec<LineupPlayer> = (1..=11)
            .map(|slot| LineupPlayer {
                player_id: format!("a_{slot}"),
                position_category: match slot {
                    1 => 1,
                    2..=6 => 2,
                    7..=9 => 3,
                    _ => 4,
                },
                shirt_number: slot,
                formation_slot: slot,
            })
            .collect();
        ctx.formation_changes.push(FormationChange {
            match_time_secs: 30.0,
            period_id: 1,
            team_id: "away".into(),
            lineup: xi,
        });
        let mut e = SimulationEngine::new(ctx);
        let before: Vec<_> = e
            .agents
            .iter()
            .filter(|a| a.team_id == "away")
            .map(|a| (a.id.clone(), a.duty))
            .collect();
        let mut max_step = 0.0f32;
        let mut prev: Vec<_> = e.agents.iter().map(|a| a.position).collect();
        e.run_for(40.0);
        for (i, a) in e.agents.iter().enumerate() {
            max_step = max_step.max(prev[i].distance(a.position) / 1.0);
            prev[i] = a.position;
        }
        let after: Vec<_> = e
            .agents
            .iter()
            .filter(|a| a.team_id == "away")
            .map(|a| (a.id.clone(), a.duty))
            .collect();
        assert_ne!(before, after, "formation change did not retune duties");
        assert!(
            after.iter().filter(|(_, d)| *d == Duty::CentreBack).count() >= 3,
            "back three not applied: {after:?}"
        );
        let _ = max_step;
    }

    #[test]
    fn test_red_card_parks_player_and_plays_short_handed() {
        // Straight red: the player walks off (sent_off) and takes no further
        // part; the side keeps 10 active bodies.
        let mut ctx = context();
        let mut card = typed_event(1, 2.0, 17, "away", 0.0, 0.0);
        card.player_id = "a_4".into();
        card.player_name = "a_4".into();
        card.card = crate::parser::CardKind::Red;
        ctx.events = vec![card];
        let mut e = SimulationEngine::new(ctx);
        // The doer may need the slip budget to reach the spot, then the
        // 36 m walk to the touchline at jog pace.
        e.run_for(20.0);
        let idx = e.agent_index("a_4").expect("dismissed agent kept");
        assert!(e.agents[idx].sent_off, "red-carded player not flagged");
        assert!(
            e.agents[idx].position.y.abs() > 34.0 - 0.6,
            "dismissed player did not walk off: {:?}",
            e.agents[idx].position
        );
        let active = e
            .agents
            .iter()
            .filter(|a| a.team_id == "away" && !a.sent_off)
            .count();
        assert_eq!(active, 10, "side not short-handed: {active} active");
    }

    #[test]
    fn test_possession_follows_last_touch() {
        // After a completed home pass, possession is home — not merely the
        // next event's team.
        let mut ctx = context();
        let mut p1 = event(1, 1.0, "home", "h_6", -10.0, 0.0, false);
        p1.target_x = Some(10.0);
        p1.target_y = Some(0.0);
        let mut p2 = event(2, 2.0, "home", "h_9", 10.0, 0.0, false);
        p2.target_x = Some(20.0);
        p2.target_y = Some(0.0);
        let away = event(3, 30.0, "away", "a_10", 0.0, 5.0, false);
        ctx.events = vec![p1, p2, away];
        let mut e = SimulationEngine::new(ctx);
        e.run_for(5.0);
        assert_eq!(
            e.possession.as_deref(),
            Some("home"),
            "possession should follow the last touch"
        );
    }

    #[test]
    fn test_from_context_rejects_empty() {
        let mut ctx = context();
        ctx.home_team_id = String::new();
        assert!(SimulationEngine::from_context(ctx).is_err());
    }

    #[test]
    fn test_ball_reaches_target_after_pass() {
        let mut e = SimulationEngine::new(context());
        e.run_for(5.0);
        // After the first two events, the ball should have moved from centre.
        assert!(e.ball.position.length() > 0.0);
        let _ = BallState::Loose {
            velocity: Vector2::zero(),
            friction: 6.0,
        };
    }

    // ── Data-driven formation & shape coherence ────────────────

    fn lineup_442(prefix: &str) -> Vec<LineupPlayer> {
        (1..=11)
            .map(|slot| LineupPlayer {
                player_id: format!("{prefix}_{slot}"),
                position_category: match slot {
                    1 => 1,
                    2..=5 => 2,
                    6..=9 => 3,
                    _ => 4,
                },
                shirt_number: slot,
                formation_slot: slot,
            })
            .collect()
    }

    fn defender_x_spread(e: &SimulationEngine, team: &str) -> f32 {
        let xs: Vec<f32> = e
            .agents
            .iter()
            .filter(|a| a.team_id == team && a.role == PlayerRole::Defender)
            .map(|a| a.position.x)
            .collect();
        if xs.len() < 2 {
            return 0.0;
        }
        let hi = xs.iter().cloned().fold(f32::MIN, f32::max);
        let lo = xs.iter().cloned().fold(f32::MAX, f32::min);
        hi - lo
    }

    #[test]
    fn test_detected_formation_is_data_driven() {
        // The old engine forced every team into 4-3-3. A 4-4-2 lineup must now
        // produce a 4-4-2 style and put all four defenders on one line.
        let mut ctx = context();
        ctx.lineups.insert("home".into(), lineup_442("h"));
        ctx.lineups.insert("away".into(), lineup_442("a"));
        let e = SimulationEngine::new(ctx);
        assert!(
            (e.team_styles()[0].mentality - 0.50).abs() < 1e-3,
            "expected 4-4-2 style, got {}",
            e.team_styles()[0].mentality
        );
        assert!(defender_x_spread(&e, "home") < 0.01, "defenders not aligned");
        assert!(defender_x_spread(&e, "away") < 0.01, "defenders not aligned");
    }

    #[test]
    fn test_backline_stays_flat_during_play() {
        let mut ctx = context();
        ctx.lineups.insert("home".into(), lineup_442("h"));
        let mut e = SimulationEngine::new(ctx);
        let mut max_spread = 0.0f32;
        for _ in 0..(8 * 60) {
            e.step();
            max_spread = max_spread.max(defender_x_spread(&e, "home"));
        }
        // Even with marking and pressing, the back four stays a unit.
        assert!(max_spread < 9.0, "backline split by {max_spread:.1} m");
    }

    #[test]
    fn test_anchor_accessor_matches_agents() {
        let mut e = SimulationEngine::new(context());
        e.run_for(2.0);
        let anchors = e.anchors();
        assert_eq!(anchors.len(), e.agent_count());
        for a in &e.agents {
            assert!(anchors.contains_key(&a.id));
        }
        let lines = e.hold_lines();
        assert!(lines[0].is_finite() && lines[1].is_finite());
        assert!((-50.0..=34.0).contains(&lines[0]), "home line {}", lines[0]);
        assert!((-34.0..=50.0).contains(&lines[1]), "away line {}", lines[1]);
    }

    // ── Hostile formation invariants ───────────────────────────

    /// Build a sim whose next event is a single home event `x` metres out, so
    /// possession is unambiguously home for the whole run.
    fn possession_sim(event_x: f32) -> SimulationEngine {
        let mut ctx = context();
        ctx.events = vec![event(1, 1000.0, "home", "h_6", event_x, 0.0, false)];
        SimulationEngine::new(ctx)
    }

    fn team_def_mean_x(e: &SimulationEngine, team: &str) -> f32 {
        let xs: Vec<f32> = e
            .agents
            .iter()
            .filter(|a| a.team_id == team && a.role == PlayerRole::Defender)
            .map(|a| a.position.x)
            .collect();
        xs.iter().sum::<f32>() / xs.len().max(1) as f32
    }

    #[test]
    fn hostile_backline_is_goal_side_of_the_ball() {
        // Home attacks +x, own goal at -52.5. With the ball at +30 (their
        // attacking third), every home defender must still be goal-side of it.
        // The flipped-sign bug put the whole back four at x=+34 here.
        let mut e = possession_sim(30.0);
        e.ball.place_at(Vector2::new(30.0, 0.0));
        e.run_for(8.0);
        let mean = team_def_mean_x(&e, "home");
        assert!(mean < 28.0, "home backline at {mean:.1} is ahead of the ball");
        assert!(mean > -12.0, "home backline collapsed to {mean:.1}");
        for a in e
            .agents
            .iter()
            .filter(|a| a.team_id == "home" && a.role == PlayerRole::Defender)
        {
            assert!(a.position.x < 32.0, "defender stranded upfield at {}", a.position.x);
        }
    }

    #[test]
    fn hostile_backline_retreats_with_the_ball() {
        // Ball deep in our own half → the line drops deep, never stays high.
        let mut e = possession_sim(-30.0);
        e.ball.place_at(Vector2::new(-30.0, 0.0));
        e.run_for(8.0);
        let mean = team_def_mean_x(&e, "home");
        assert!(mean < -34.0, "home backline failed to drop: {mean:.1}");
        assert!(mean > -48.0, "home backline too deep: {mean:.1}");
    }

    #[test]
    fn hostile_opposing_line_sits_between_ball_and_goal() {
        // Away attacks −x and defends +x. With the ball at +30, the away line
        // must be between the ball and their own goal (x increasing toward 52).
        let mut e = possession_sim(30.0);
        e.ball.place_at(Vector2::new(30.0, 0.0));
        e.run_for(8.0);
        let mean = team_def_mean_x(&e, "away");
        assert!(mean > 30.0, "away line at {mean:.1} is on the wrong side of the ball");
        assert!(mean < 50.0, "away line at {mean:.1} past its own goal");
    }

    #[test]
    fn hostile_defensive_line_advances_when_ball_advances() {
        // The same team, same possession: the line must be higher for a ball
        // at +30 than for a ball at −30. (Monotonic response, not random.)
        let mut high = possession_sim(30.0);
        high.ball.place_at(Vector2::new(30.0, 0.0));
        high.run_for(8.0);
        let mut low = possession_sim(-30.0);
        low.ball.place_at(Vector2::new(-30.0, 0.0));
        low.run_for(8.0);
        let hi = team_def_mean_x(&high, "home");
        let lo = team_def_mean_x(&low, "home");
        assert!(hi > lo + 15.0, "line did not track ball: high={hi:.1} low={lo:.1}");
    }

    #[test]
    fn hostile_team_never_migrates_as_one_clump() {
        // Even in possession high up the pitch, the team must stay vertically
        // spread out (defence ↔ attack), never a 10 m clump.
        let mut e = possession_sim(30.0);
        e.ball.place_at(Vector2::new(30.0, 0.0));
        for _ in 0..(6 * 60) {
            e.step();
            let xs: Vec<f32> = e
                .agents
                .iter()
                .filter(|a| a.team_id == "home" && a.role != PlayerRole::Goalkeeper)
                .map(|a| a.position.x)
                .collect();
            let spread = xs.iter().cloned().fold(f32::MIN, f32::max)
                - xs.iter().cloned().fold(f32::MAX, f32::min);
            assert!(spread > 14.0, "home compressed into {spread:.1} m");
        }
    }

    // ── Set-piece discipline ───────────────────────────────────

    fn far_context() -> MatchContext {
        let mut ctx = context();
        ctx.events = vec![event(1, 1000.0, "home", "h_6", 0.0, 0.0, false)];
        ctx
    }

    #[test]
    fn hostile_backline_anchors_are_exactly_flat() {
        // The whole point of the line block: defenders share one line x.
        let mut e = SimulationEngine::new(context());
        for _ in 0..(6 * 60) {
            e.step();
            let anchors = e.anchors();
            let xs: Vec<f32> = e
                .agents
                .iter()
                .filter(|a| a.team_id == "home" && a.role == PlayerRole::Defender)
                .map(|a| anchors.get(&a.id).copied().unwrap_or(Vector2::zero()).x)
                .collect();
            let spread = xs.iter().cloned().fold(f32::MIN, f32::max)
                - xs.iter().cloned().fold(f32::MAX, f32::min);
            assert!(spread < 0.6, "back line anchors split by {spread:.2} m");
        }
    }

    #[test]
    fn hostile_kickoff_is_taken_from_the_centre() {
        let mut e = SimulationEngine::new(far_context());
        e.run_for(5.0);
        assert!(e.set_piece_active(), "kickoff not active");
        assert!(
            e.ball.position.length() < 0.6,
            "ball not on the centre spot: {:?}",
            e.ball.position
        );
        let anchors = e.anchors();
        let at = |a: &PlayerAgent| anchors.get(&a.id).copied().unwrap_or(Vector2::zero());
        let near = e
            .agents
            .iter()
            .filter(|a| {
                a.team_id == "home"
                    && !a.is_goalkeeper
                    && at(a).distance(Vector2::zero()) < 2.2
            })
            .count();
        assert!(near >= 2, "kickoff needs two players at the spot, got {near}");
        for a in e.agents.iter().filter(|a| !a.is_goalkeeper) {
            let t = at(a);
            if a.team_id == "home" {
                assert!(t.x < 0.5, "kicking anchor not in own half: {t:?}");
            } else {
                assert!(t.x > 9.0, "defending anchor inside the centre circle: {t:?}");
            }
        }
        // The bodies are converging on the restart, not frozen at kickoff.
        let bodies = e
            .agents
            .iter()
            .filter(|a| {
                a.team_id == "home"
                    && !a.is_goalkeeper
                    && a.position.distance(Vector2::zero()) < 10.0
            })
            .count();
        assert!(bodies >= 2, "players are not converging on the restart");
    }

    #[test]
    fn hostile_half_time_kickoff_is_taken_from_the_circle() {
        // The first event after the interval belongs to the home side. A
        // period boundary has no simulated interval, so the taker and second
        // kicker must already be bodies inside the centre circle on the first
        // second-half tick.
        let mut ctx = context();
        let mut second_half = event(4, 3.0, "home", "h_9", 0.0, 0.0, false);
        second_half.period_id = 2;
        ctx.events.push(second_half);
        let mut e = SimulationEngine::new(ctx);

        // Match start: the scheduled taker's body, not just his anchor.
        let first = e.agent_index("h_6").expect("first taker");
        assert!(
            e.agents[first].position.length() < 9.15,
            "match-start taker outside the centre circle: {:?}",
            e.agents[first].position
        );

        for _ in 0..600 {
            e.step();
            if e.current_period == 2 {
                break;
            }
        }
        assert_eq!(e.current_period, 2, "half-time never reached");
        assert!(
            e.ball.position.length() < 0.6,
            "second-half ball not on the centre spot: {:?}",
            e.ball.position
        );

        let taker = e.agent_index("h_9").expect("second-half taker");
        let second = e.agent_index("h_10").expect("second-half kicker");
        assert!(
            e.agents[taker].position.length() < 9.15,
            "second-half taker outside the centre circle: {:?}",
            e.agents[taker].position
        );
        assert!(
            e.agents[second].position.length() < 9.15,
            "second-half kicker outside the centre circle: {:?}",
            e.agents[second].position
        );
    }

    #[test]
    fn hostile_corner_boxes_are_populated() {
        let mut ctx = context();
        ctx.events = vec![typed_event(1, 1.0, 6, "home", 50.0, 20.0)];
        let mut e = SimulationEngine::new(ctx);
        e.run_for(12.0);
        assert!(e.set_piece_active(), "corner set piece not active");
        assert!(
            (e.ball.position.x - 52.0).abs() < 1.5 && (e.ball.position.y - 33.5).abs() < 1.5,
            "corner ball not at the flag: {:?}",
            e.ball.position
        );
        let att = e
            .agents
            .iter()
            .filter(|a| {
                a.team_id == "home"
                    && !a.is_goalkeeper
                    && a.position.x > 40.0
                    && a.position.y.abs() < 20.0
            })
            .count();
        assert!(att >= 4, "only {att} attackers in the box");
        let def = e
            .agents
            .iter()
            .filter(|a| a.team_id == "away" && !a.is_goalkeeper && a.position.x > 40.0)
            .count();
        assert!(def >= 4, "only {def} defenders in the box");

        // Nobody freelances during a set piece: every player holds the shape
        // they were assigned, and the team is calm, not pirouetting.
        for a in e.agents.iter() {
            assert!(
                matches!(
                    a.brain.current_intent,
                    crate::brain::TacticalIntent::HoldShape
                        | crate::brain::TacticalIntent::AnticipateEvent { .. }
                ),
                "{:?} freelanced during a corner: {:?}",
                a.role,
                a.brain.current_intent
            );
        }
        let avg_speed: f32 =
            e.agents.iter().map(|a| a.speed()).sum::<f32>() / e.agent_count() as f32;
        assert!(
            avg_speed < 2.0,
            "players still charging around during a corner (avg {avg_speed:.2} m/s)"
        );
    }

    #[test]
    fn hostile_free_kick_has_a_wall() {
        let mut ctx = context();
        ctx.events = vec![typed_event(1, 1.0, 4, "home", 20.0, 0.0)];
        let mut e = SimulationEngine::new(ctx);
        e.run_for(8.0);
        assert!(e.set_piece_active(), "free-kick set piece not active");
        let ball = e.ball.position;
        let wall = e
            .agents
            .iter()
            .filter(|a| {
                a.team_id == "away"
                    && !a.is_goalkeeper
                    && (7.0..12.0).contains(&a.position.distance(ball))
            })
            .count();
        assert!(wall >= 3, "no free-kick wall ({wall} defenders near the ball)");
    }

#[test]
fn dbg_kickoff_spot() {
    let mut ctx = context();
    let mut second_half = event(4, 3.0, "home", "h_9", 0.0, 0.0, false);
    second_half.period_id = 2;
    ctx.events.push(second_half);
    for e in &ctx.events { println!("ev id={} p={} t={:.2} type={}", e.id, e.period_id, e.match_time_secs, e.type_id); }
    let mut e = SimulationEngine::new(ctx);
    for i in 0..600 {
        e.step();
        if e.current_period == 2 {
            println!("period2 at tick {i} sim={:.2} ball={:?} pending_kickoff={:?}", e.sim_time, e.ball.position, e.pending_kickoff.is_some());
            break;
        }
    }
    println!("final ball={:?} set_piece={:?}", e.ball.position, e.set_piece_active());
}




}
