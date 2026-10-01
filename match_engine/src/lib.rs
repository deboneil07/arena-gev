//! # Match Engine — Steps 1–5
//!
//! A deterministic, high-performance football simulation core that turns
//! sparse Opta F24 XML into smooth, render-ready 22-player movement and a
//! continuous 3D ball.
//!
//! | Module | Responsibility |
//! |---|---|
//! | [`parser`]    | Event schema, pitch constants, coordinate normalisation |
//! | [`loader`]    | Streaming XML ingestion → [`MatchContext`] |
//! | [`timeline`]  | Time-indexed lookahead playback buffer |
//! | [`formation`] | Elastic tactical anchor grid + data-driven shape detection |
//! | [`tactics`]   | Team style, lines, and per-tick team plans (presser/cover/runner) |
//! | [`agent`]     | Reynolds steering + 60 Hz state machine (micro layer) |
//! | [`brain`]     | Utility-AI tactical decisions + context steering |
//! | [`dispatcher`]| Lookahead event dispatcher (anticipation + execution) |
//! | [`ball`]      | 3D ballistic ball trajectory & choreography |
//! | [`engine`]    | Master 60 Hz orchestrator + render snapshots |
//! | [`wasm`]      | WASM bridge + zero-copy flat render buffer |
//!
//! ## Quick start
//!
//! ```no_run
//! use match_engine::{parse_opta_f24, SimulationEngine};
//!
//! let xml = std::fs::read_to_string("match.xml").unwrap();
//! let ctx = parse_opta_f24(&xml).expect("parse failed");
//! let mut engine = SimulationEngine::from_context(ctx).expect("build failed");
//!
//! // Advance 10 seconds of match time in 1/60 s steps.
//! engine.run_for(10.0);
//! let frame = engine.get_render_state(); // 22 players + 1 ball
//! assert!(engine.is_consistent());
//! ```

pub mod parser;
pub mod loader;
pub mod timeline;
pub mod formation;
pub mod tactics;
pub mod agent;
pub mod brain;
pub mod dispatcher;
pub mod ball;
pub mod engine;
pub mod simulation;
pub mod wasm;

// ── Re-exports ─────────────────────────────────────────────

pub use loader::parse_opta_f24;
pub use parser::{
    event_type_name, is_ball_event, is_dead_ball_event, is_gameplay_event, is_notable_event,
    AttackDirection, LineupPlayer, MatchContext, NormalizedEvent, EVENT_TYPE_AERIAL,
    EVENT_TYPE_CORNER, EVENT_TYPE_FOUL, EVENT_TYPE_GOAL, EVENT_TYPE_OUT, EVENT_TYPE_PASS,
    PERIOD_PREMATCH, PITCH_HALF_LENGTH, PITCH_HALF_WIDTH, PITCH_LENGTH, PITCH_WIDTH,
};
pub use timeline::Timeline;
pub use formation::{FormationEngine, TacticalAnchor, TeamTacticalPhase, Vector2};
pub use tactics::{Line, TeamContext, TeamPlan, TeamStyle};
pub use agent::{
    braking_speed, init_agents, simulate_step, AgentState, PlayerAgent, PlayerAttributes,
    PlayerRole, DEFAULT_JOG_SPEED, DEFAULT_MAX_ACCEL, DEFAULT_SPRINT_SPEED, DT, MAX_SPEED,
    RECOVER_SECS, SEPARATION_RADIUS, TURN_SPEED,
};
pub use brain::{step_agents, TacticalBrain, TacticalIntent, RAY_COUNT};
pub use dispatcher::{
    action_duration, arrival_time, travel_time, DispatchedAction, LookaheadDispatcher,
    DEFAULT_LOOKAHEAD_SECS,
};
pub use ball::{Ball, BallFlightType, BallState};
pub use engine::{
    ball_spot_expected, EventRecord, EventSnapshot, LandingSnapshot, SimulationConfig,
    SimulationEngine, RenderEntity, EVENT_FEED_CAPACITY, TEAM_AWAY, TEAM_BALL, TEAM_HOME,
};
pub use wasm::{MatchRenderer, RenderBuffer, WasmMatchEngine, BUFFER_LEN, ENTITY_STRIDE, TOTAL_ENTITIES};
// Backwards-compatible alias.
pub use engine::SimulationEngine as MatchSimulation;
