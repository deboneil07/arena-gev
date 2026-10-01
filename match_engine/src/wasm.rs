//! WASM bridge — exposes the engine to JavaScript and streams a flat `f32`
//! render buffer into WASM linear memory for zero-copy Canvas rendering.
//!
//! # Buffer layout
//!
//! `ENTITY_STRIDE` floats per entity, `TOTAL_ENTITIES` entities:
//!
//! ```text
//! [0] x (metres)  [1] y (metres)  [2] z (altitude, m)
//! [3] heading (radians)  [4] team_index (0 home, 1 away, 2 ball)  [5] shirt_number
//! ```
//!
//! `23 entities × 6 floats = 138 floats` (22 players followed by the ball).
//!
//! The pure [`MatchRenderer`] core is testable on the native target; the
//! `#[wasm_bindgen]` [`WasmMatchEngine`] is a thin JS-facing wrapper compiled
//! only for `wasm32`.

use crate::engine::{EventRecord, RenderEntity, SimulationEngine};
use crate::loader::parse_opta_f24;

/// Floats per entity in the flat buffer.
pub const ENTITY_STRIDE: usize = 6;
/// Players (22) plus the ball (1).
pub const TOTAL_ENTITIES: usize = 23;
/// Total number of floats in the flat buffer.
pub const BUFFER_LEN: usize = TOTAL_ENTITIES * ENTITY_STRIDE;

/// A flat `f32` buffer laid out for direct `Float32Array` viewing in JS.
#[derive(Debug, Clone)]
pub struct RenderBuffer {
    data: Vec<f32>,
}

impl RenderBuffer {
    /// Allocate a zeroed buffer of exactly [`BUFFER_LEN`] floats.
    pub fn new() -> Self {
        Self {
            data: vec![0.0; BUFFER_LEN],
        }
    }

    /// Read-only view of the flat data.
    #[inline]
    pub fn as_slice(&self) -> &[f32] {
        &self.data
    }

    /// Raw pointer into WASM linear memory (stable for the buffer's lifetime).
    #[inline]
    pub fn as_ptr(&self) -> *const f32 {
        self.data.as_ptr()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Floats per entity.
    #[inline]
    pub fn stride(&self) -> usize {
        ENTITY_STRIDE
    }

    /// Number of entity slots.
    #[inline]
    pub fn entity_count(&self) -> usize {
        TOTAL_ENTITIES
    }

    /// Overwrite the buffer from a render snapshot.
    ///
    /// Player entities fill slots `0..TOTAL_ENTITIES-1` in order and the ball
    /// is always pinned to the final slot, so the layout stays valid even if a
    /// team is short-handed. Missing slots are zeroed; extras are ignored.
    /// This never changes the allocation, so a `Float32Array` view into the
    /// buffer stays valid across ticks.
    pub fn write(&mut self, entities: &[RenderEntity]) {
        use crate::engine::TEAM_BALL;
        self.data.fill(0.0);
        let mut slot = 0usize;
        let mut ball = None;
        for e in entities {
            let encoded = [
                e.x,
                e.y,
                e.z,
                e.heading,
                e.team_index as f32,
                e.shirt_number as f32,
            ];
            if e.team_index == TEAM_BALL {
                ball = Some(encoded);
            } else if slot < TOTAL_ENTITIES - 1 {
                let o = slot * ENTITY_STRIDE;
                self.data[o..o + ENTITY_STRIDE].copy_from_slice(&encoded);
                slot += 1;
            }
        }
        if let Some(b) = ball {
            let o = (TOTAL_ENTITIES - 1) * ENTITY_STRIDE;
            self.data[o..o + ENTITY_STRIDE].copy_from_slice(&b);
        }
    }

    /// Decode a single entity slot.
    #[inline]
    pub fn entity(&self, index: usize) -> Option<[f32; ENTITY_STRIDE]> {
        if index >= TOTAL_ENTITIES {
            return None;
        }
        let o = index * ENTITY_STRIDE;
        Some([
            self.data[o],
            self.data[o + 1],
            self.data[o + 2],
            self.data[o + 3],
            self.data[o + 4],
            self.data[o + 5],
        ])
    }
}

impl Default for RenderBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Native-testable bridge core: owns the engine and the flat render buffer.
pub struct MatchRenderer {
    engine: SimulationEngine,
    buffer: RenderBuffer,
    /// Original XML, retained so [`MatchRenderer::seek_to`] can rebuild state.
    xml: String,
}

impl MatchRenderer {
    /// Parse Opta F24 XML and build a renderer.
    pub fn from_xml(xml: &str) -> Result<Self, String> {
        let ctx = parse_opta_f24(xml)?;
        let engine = SimulationEngine::from_context(ctx)?;
        Ok(Self {
            engine,
            buffer: RenderBuffer::new(),
            xml: xml.to_string(),
        })
    }

    /// Jump to `time` seconds by rebuilding from the source XML and fast-
    /// forwarding. Used by the UI's ±10 minute playback buttons.
    pub fn seek_to(&mut self, time: f32) -> bool {
        let Ok(ctx) = parse_opta_f24(&self.xml) else {
            return false;
        };
        let Ok(mut engine) = SimulationEngine::with_config(ctx, self.engine.config.clone()) else {
            return false;
        };
        engine.run_for(time.max(0.0));
        self.engine = engine;
        self.buffer.write(&self.engine.get_render_state());
        true
    }

    /// Advance the simulation and refresh the flat buffer.
    pub fn tick(&mut self, dt: f32) {
        self.engine.tick(dt);
        self.buffer.write(&self.engine.get_render_state());
    }

    /// Advance by the engine's fixed timestep.
    pub fn step(&mut self) {
        self.tick(self.engine.config.dt);
    }

    /// Run `seconds` at the fixed timestep.
    pub fn run_for(&mut self, seconds: f32) {
        let steps = (seconds / self.engine.config.dt).round() as usize;
        for _ in 0..steps {
            self.step();
        }
    }

    /// The flat render buffer.
    #[inline]
    pub fn buffer(&self) -> &[f32] {
        self.buffer.as_slice()
    }

    /// Raw pointer to the flat render buffer.
    #[inline]
    pub fn buffer_ptr(&self) -> *const f32 {
        self.buffer.as_ptr()
    }

    #[inline]
    pub fn entity_stride(&self) -> usize {
        ENTITY_STRIDE
    }

    #[inline]
    pub fn total_entities(&self) -> usize {
        TOTAL_ENTITIES
    }

    /// Number of actual player entities (normally 22).
    #[inline]
    pub fn player_count(&self) -> usize {
        self.engine.agent_count()
    }

    /// A copy of the full flat buffer.
    #[inline]
    pub fn buffer_vec(&self) -> Vec<f32> {
        self.buffer.as_slice().to_vec()
    }

    /// Goals: `[home, away]`.
    #[inline]
    pub fn score(&self) -> [u32; 2] {
        self.engine.score
    }

    /// Share of possession (0..1) held by the home team.
    pub fn possession_home(&self) -> f32 {
        let h = self.engine.possession_frames[0] as f32;
        let a = self.engine.possession_frames[1] as f32;
        if h + a > 0.0 {
            h / (h + a)
        } else {
            0.5
        }
    }

    /// Offside line x (metres) for the team currently in possession.
    #[inline]
    pub fn offside_line_x(&self) -> f32 {
        self.engine.offside_line_x()
    }

    /// Monotonic sequence number of the newest executed event.
    #[inline]
    pub fn event_seq(&self) -> u64 {
        self.engine.event_seq
    }

    /// Number of entries currently in the recent-event feed.
    #[inline]
    pub fn recent_event_count(&self) -> usize {
        self.engine.recent_events.len()
    }

    /// Access a recent event by index (0 = oldest retained).
    #[inline]
    pub fn recent_event(&self, index: usize) -> Option<&EventRecord> {
        self.engine.recent_events.get(index)
    }

    /// Current simulation clock (seconds).
    #[inline]
    pub fn sim_time(&self) -> f32 {
        self.engine.sim_time
    }

    /// True once every event has been executed.
    #[inline]
    pub fn is_finished(&self) -> bool {
        self.engine.dispatcher.is_finished()
    }

    /// Read-only access to the underlying engine.
    #[inline]
    pub fn engine(&self) -> &SimulationEngine {
        &self.engine
    }

    /// Mutable access to the underlying engine (e.g. custom config in tests).
    #[inline]
    pub fn engine_mut(&mut self) -> &mut SimulationEngine {
        &mut self.engine
    }
}

// ── WASM bindings (wasm32 only) ────────────────────────────────

#[cfg(target_arch = "wasm32")]
mod glue {
    use super::{MatchRenderer, ENTITY_STRIDE, TOTAL_ENTITIES};
    use wasm_bindgen::prelude::*;

    /// JS-facing engine handle. Construct with `new WasmMatchEngine(xmlString)`.
    #[wasm_bindgen]
    pub struct WasmMatchEngine {
        inner: MatchRenderer,
    }

    #[wasm_bindgen]
    impl WasmMatchEngine {
        /// Build from an Opta F24 XML string.
        #[wasm_bindgen(constructor)]
        pub fn new(xml_content: &str) -> Result<WasmMatchEngine, JsValue> {
            MatchRenderer::from_xml(xml_content)
                .map(|inner| Self { inner })
                .map_err(|e| JsValue::from_str(&format!("Failed to parse Opta F24 XML: {e}")))
        }

        /// Advance physics and choreography by `dt` seconds (0.01667 for 60 Hz).
        pub fn tick(&mut self, dt: f32) {
            self.inner.tick(dt);
        }

        /// Advance by the engine's fixed 60 Hz timestep.
        pub fn step(&mut self) {
            self.inner.step();
        }

        /// Pointer to the flat `f32` render buffer in WASM linear memory.
        pub fn get_render_buffer_ptr(&self) -> *const f32 {
            self.inner.buffer_ptr()
        }

        /// A *copy* of the flat render buffer (handy for tests/debugging where
        /// raw linear-memory access is unavailable).
        pub fn get_render_buffer_copy(&self) -> Vec<f32> {
            self.inner.buffer().to_vec()
        }

        /// Number of floats written per entity (6).
        pub fn get_entity_stride(&self) -> usize {
            ENTITY_STRIDE
        }

        /// Number of entity slots (23).
        pub fn get_total_entities(&self) -> usize {
            TOTAL_ENTITIES
        }

        /// Number of actual player entities (normally 22).
        pub fn get_player_count(&self) -> usize {
            self.inner.player_count()
        }

        /// Tournament clock in seconds.
        pub fn get_sim_time(&self) -> f32 {
            self.inner.sim_time()
        }

        /// True once the match has played out.
        pub fn get_is_finished(&self) -> bool {
            self.inner.is_finished()
        }

        /// Total match duration in seconds.
        pub fn get_duration_secs(&self) -> f32 {
            self.inner.engine().timeline.duration_secs()
        }

        // ── Match metadata (from the Opta <Game> row) ──

        /// Opta match id.
        pub fn get_match_id(&self) -> String {
            self.inner.engine().match_id().to_string()
        }

        /// Home team's display name.
        pub fn get_home_team_name(&self) -> String {
            self.inner.engine().home_team_name().to_string()
        }

        /// Away team's display name.
        pub fn get_away_team_name(&self) -> String {
            self.inner.engine().away_team_name().to_string()
        }

        /// Competition id the match belongs to.
        pub fn get_competition_id(&self) -> String {
            self.inner.engine().competition_id().to_string()
        }

        /// Season label.
        pub fn get_season(&self) -> String {
            self.inner.engine().season().to_string()
        }

        /// Kick-off date as recorded by the feed.
        pub fn get_game_date(&self) -> String {
            self.inner.engine().game_date().to_string()
        }

        // ── Event feed (for splash banners + the running commentary) ──

        /// Monotonic sequence number of the newest executed event.
        pub fn get_event_seq(&self) -> u64 {
            self.inner.event_seq()
        }

        /// Number of entries in the recent-event feed (newest last).
        pub fn get_recent_event_count(&self) -> usize {
            self.inner.recent_event_count()
        }

        /// Match clock of feed entry `i` (seconds).
        pub fn get_recent_event_time(&self, i: usize) -> f32 {
            self.inner.recent_event(i).map(|e| e.time).unwrap_or(0.0)
        }

        /// Event-type name of feed entry `i` (e.g. "Goal", "Card").
        pub fn get_recent_event_label(&self, i: usize) -> String {
            self.inner.recent_event(i).map(|e| e.label.to_string()).unwrap_or_default()
        }

        /// Event type id of feed entry `i`.
        pub fn get_recent_event_type(&self, i: usize) -> u16 {
            self.inner.recent_event(i).map(|e| e.type_id).unwrap_or(0)
        }

        /// Team index of feed entry `i` (0 home, 1 away, 2 neutral).
        pub fn get_recent_event_team(&self, i: usize) -> u8 {
            self.inner.recent_event(i).map(|e| e.team_index).unwrap_or(2)
        }

        /// Whether feed entry `i` deserves a large splash.
        pub fn get_recent_event_notable(&self, i: usize) -> bool {
            self.inner.recent_event(i).map(|e| e.notable).unwrap_or(false)
        }

        // ── Match stats ──

        /// Home goals.
        pub fn get_score_home(&self) -> u32 {
            self.inner.score()[0]
        }

        /// Away goals.
        pub fn get_score_away(&self) -> u32 {
            self.inner.score()[1]
        }

        /// Home possession share, 0..1.
        pub fn get_possession_home(&self) -> f32 {
            self.inner.possession_home()
        }

        /// Offside line x (metres) for the team in possession.
        pub fn get_offside_line(&self) -> f32 {
            self.inner.offside_line_x()
        }

        /// Jump to `secs` seconds of match time (0 = kick-off).
        pub fn seek_to(&mut self, secs: f32) -> bool {
            self.inner.seek_to(secs)
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use glue::WasmMatchEngine;

/// On non-WASM targets the JS wrapper is unavailable, so alias the native core
/// so callers and tests can use the same name.
#[cfg(not(target_arch = "wasm32"))]
pub type WasmMatchEngine = MatchRenderer;

#[cfg(test)]
mod wasm_tests {
    use super::*;
    use crate::engine::{TEAM_AWAY, TEAM_BALL, TEAM_HOME};

    const SAMPLE_XML: &str = r#"<Games>
      <Game id="1" home_team_id="home" away_team_id="away" home_team_name="Home" away_team_name="Away" competition_id="10" season="2026" game_date="2026-01-01">
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
        <Event id="3" type_id="1" period_id="1" min="0" sec="2" team_id="home" player_id="h10" x="50" y="50">
          <Q qualifier_id="140" value="70"/>
          <Q qualifier_id="141" value="80"/>
        </Event>
      </Game></Games>"#;

    #[test]
    fn test_render_buffer_size_and_defaults() {
        let b = RenderBuffer::new();
        assert_eq!(b.len(), BUFFER_LEN);
        assert_eq!(b.len(), 138);
        assert_eq!(b.stride(), 6);
        assert_eq!(b.entity_count(), 23);
        assert!(b.as_slice().iter().all(|v| *v == 0.0));
        assert!(!b.as_ptr().is_null());
    }

    #[test]
    fn test_buffer_write_roundtrip() {
        let mut b = RenderBuffer::new();
        let entities = vec![
            RenderEntity { x: 1.0, y: 2.0, z: 3.0, heading: 0.5, team_index: TEAM_HOME, shirt_number: 9 },
            RenderEntity { x: -4.0, y: 5.0, z: 0.0, heading: -1.0, team_index: TEAM_AWAY, shirt_number: 22 },
            RenderEntity { x: 0.0, y: 0.0, z: 2.5, heading: 0.0, team_index: TEAM_BALL, shirt_number: 0 },
        ];
        b.write(&entities);
        assert_eq!(b.entity(0), Some([1.0, 2.0, 3.0, 0.5, 0.0, 9.0]));
        assert_eq!(b.entity(1), Some([-4.0, 5.0, 0.0, -1.0, 1.0, 22.0]));
        // The ball is always pinned to the final slot.
        assert_eq!(b.entity(TOTAL_ENTITIES - 1), Some([0.0, 0.0, 2.5, 0.0, 2.0, 0.0]));
        // Untouched player slots stay zeroed.
        assert_eq!(b.entity(2), Some([0.0; 6]));
        assert_eq!(b.entity(TOTAL_ENTITIES), None);
    }

    #[test]
    fn test_buffer_ball_last_with_partial_team() {
        // A short-handed team must not displace the ball from the last slot.
        let mut b = RenderBuffer::new();
        let mut entities: Vec<RenderEntity> = (0..5)
            .map(|i| RenderEntity {
                x: i as f32,
                y: 0.0,
                z: 0.0,
                heading: 0.0,
                team_index: TEAM_HOME,
                shirt_number: i as u8,
            })
            .collect();
        entities.push(RenderEntity {
            x: 7.0,
            y: 7.0,
            z: 1.5,
            heading: 0.0,
            team_index: TEAM_BALL,
            shirt_number: 0,
        });
        b.write(&entities);
        assert_eq!(b.entity(0), Some([0.0, 0.0, 0.0, 0.0, 0.0, 0.0]));
        assert_eq!(b.entity(4), Some([4.0, 0.0, 0.0, 0.0, 0.0, 4.0]));
        assert_eq!(b.entity(TOTAL_ENTITIES - 1), Some([7.0, 7.0, 1.5, 0.0, 2.0, 0.0]));
    }

    #[test]
    fn test_buffer_zeroes_previous_values() {
        let mut b = RenderBuffer::new();
        b.write(&[RenderEntity { x: 9.0, y: 9.0, z: 9.0, heading: 9.0, team_index: 0, shirt_number: 9 }]);
        // Second write with fewer entities must clear the rest.
        b.write(&[RenderEntity { x: 1.0, y: 0.0, z: 0.0, heading: 0.0, team_index: 0, shirt_number: 1 }]);
        assert_eq!(b.entity(1), Some([0.0; 6]));
    }

    #[test]
    fn test_renderer_parses_and_streams() {
        let mut r = MatchRenderer::from_xml(SAMPLE_XML).unwrap();
        assert_eq!(r.entity_stride(), 6);
        assert_eq!(r.total_entities(), 23);
        assert_eq!(r.buffer().len(), 138);
        r.run_for(5.0);
        assert!(r.sim_time() >= 4.9);
        // 11 home + 11 away + 1 ball.
        let teams: Vec<f32> = (0..23).map(|i| r.buffer()[i * 6 + 4]).collect();
        assert_eq!(teams.iter().filter(|t| **t == 0.0).count(), 11);
        assert_eq!(teams.iter().filter(|t| **t == 1.0).count(), 11);
        assert_eq!(teams.iter().filter(|t| **t == 2.0).count(), 1);
    }

    #[test]
    fn test_renderer_first_tick_matches_engine_snapshot() {
        let mut r = MatchRenderer::from_xml(SAMPLE_XML).unwrap();
        r.step();
        let snapshot = r.engine().get_render_state();
        for (i, e) in snapshot.iter().enumerate().take(TOTAL_ENTITIES) {
            let v = r.buffer()[i * ENTITY_STRIDE..(i + 1) * ENTITY_STRIDE].to_vec();
            assert_eq!(v, vec![e.x, e.y, e.z, e.heading, e.team_index as f32, e.shirt_number as f32]);
        }
    }

    #[test]
    fn test_buffer_pointer_is_stable_across_ticks() {
        let mut r = MatchRenderer::from_xml(SAMPLE_XML).unwrap();
        let p0 = r.buffer_ptr();
        r.run_for(30.0);
        let p1 = r.buffer_ptr();
        assert_eq!(p0, p1, "buffer reallocated; Float32Array view would detach");
    }

    #[test]
    fn test_buffer_values_are_finite_and_bounded() {
        let mut r = MatchRenderer::from_xml(SAMPLE_XML).unwrap();
        for _ in 0..600 {
            r.step();
            for v in r.buffer() {
                assert!(v.is_finite(), "non-finite value in render buffer");
            }
            // Altitudes are non-negative.
            for i in 0..TOTAL_ENTITIES {
                assert!(r.buffer()[i * ENTITY_STRIDE + 2] >= 0.0);
            }
        }
    }

    #[test]
    fn test_renderer_rejects_bad_xml() {
        assert!(MatchRenderer::from_xml("<not valid").is_err());
    }

    #[test]
    fn test_renderer_is_deterministic() {
        let mut a = MatchRenderer::from_xml(SAMPLE_XML).unwrap();
        let mut b = MatchRenderer::from_xml(SAMPLE_XML).unwrap();
        a.run_for(10.0);
        b.run_for(10.0);
        assert_eq!(a.buffer(), b.buffer());
    }
}
