//! Data structures and coordinate normalization for Opta F24 events.
//!
//! All coordinates are unified to a standard FIFA pitch in meters:
//!   X ∈ [-52.5, +52.5]  (left-to-right from home perspective)
//!   Y ∈ [-34.0, +34.0]  (bottom-to-top from home perspective)
//!
//! The origin (0,0) is at the center spot.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Pitch Constants ──────────────────────────────────────────────────────────

pub const PITCH_LENGTH: f32 = 105.0;
pub const PITCH_WIDTH: f32 = 68.0;
pub const PITCH_HALF_LENGTH: f32 = PITCH_LENGTH / 2.0; // 52.5
pub const PITCH_HALF_WIDTH: f32 = PITCH_WIDTH / 2.0;   // 34.0

// ── Opta Qualifier IDs ───────────────────────────────────────────────────────
//
// See `data/mappings/qualifiers.csv` for the full reference.

/// Long ball (>35 yard pass). NOTE: *not* an aerial indicator.
pub const QUAL_LONG_BALL: u16 = 1;
/// Chipped pass (ball played through the air) — genuine aerial indicator.
pub const QUAL_CHIPPED: u16 = 155;
/// Own goal, given on a `type_id=16` (Goal) row. The row is credited to the
/// team of the player who put the ball in their own net, and its coordinates
/// sit at that player's *own* goal end (low x), so the team that actually
/// scores is the **opponent** and the ball must fly into the scorer's net.
pub const QUAL_OWN_GOAL: u16 = 28;
/// Newer feeds repeat the own-goal marker as qualifier 280 with value
/// `OWN_GOAL`.
pub const QUAL_OWN_GOAL_FLAG: u16 = 280;
/// Pitch direction of play for the active team ("Left to Right" / "Right to Left").
pub const QUAL_DIR: u16 = 127;
/// Pass destination X (0-100).
pub const QUAL_TARGET_X: u16 = 140;
/// Pass destination Y (0-100).
pub const QUAL_TARGET_Y: u16 = 141;
/// Player IDs in the lineup (comma separated).
pub const QUAL_LINEUP_PLAYERS: u16 = 30;
/// Playing position category per lineup player (comma separated).
pub const QUAL_LINEUP_POSITIONS: u16 = 44;
/// Shirt numbers per lineup player (comma separated).
pub const QUAL_LINEUP_SHIRTS: u16 = 59;
/// Formation slot per lineup player (1-11 = starting XI, 0 = bench).
pub const QUAL_LINEUP_SLOTS: u16 = 131;

// ── Opta Event Type IDs ──────────────────────────────────────────────────────
//
// See `data/mappings/event_types.csv`. Only genuinely meta / non-positional
// events are excluded from gameplay; everything else is retained.

// Gameplay types referenced directly by the engine:
pub const EVENT_TYPE_PASS: u16 = 1;
pub const EVENT_TYPE_FOUL: u16 = 4;
pub const EVENT_TYPE_OUT: u16 = 5;
pub const EVENT_TYPE_CORNER: u16 = 6;
pub const EVENT_TYPE_GOAL: u16 = 16;         // ⚠ was mislabelled "match end"
pub const EVENT_TYPE_AERIAL: u16 = 44;
pub const EVENT_TYPE_CARD: u16 = 17;
pub const EVENT_TYPE_PLAYER_OFF: u16 = 18;
pub const EVENT_TYPE_PLAYER_ON: u16 = 19;

// Meta / non-positional types (excluded from the movement timeline):
pub const EVENT_TYPE_CONDITION_CHANGE: u16 = 24;
pub const EVENT_TYPE_PERIOD_END: u16 = 30;   // actual "End" event
pub const EVENT_TYPE_PERIOD_START: u16 = 32;
pub const EVENT_TYPE_TEAM_SETUP: u16 = 34;   // lineup; parsed then excluded
pub const EVENT_TYPE_COLLECTION_END: u16 = 37;
pub const EVENT_TYPE_FORMATION_CHANGE: u16 = 40;
pub const EVENT_TYPE_DELETED: u16 = 43;      // deleted-event marker
pub const EVENT_TYPE_INJURY_TIME: u16 = 70;

/// Period ID used for pre-match setup events (lineups, etc.).
pub const PERIOD_PREMATCH: u8 = 16;

/// Returns `true` when an event type represents an on-pitch player action
/// that should be simulated. Meta events (period start/end, lineups,
/// condition changes, deleted-event markers, …) return `false`.
#[inline]
pub fn is_gameplay_event(type_id: u16) -> bool {
    !matches!(
        type_id,
        EVENT_TYPE_CONDITION_CHANGE
            | EVENT_TYPE_PERIOD_END
            | EVENT_TYPE_PERIOD_START
            | EVENT_TYPE_TEAM_SETUP
            | EVENT_TYPE_COLLECTION_END
            | EVENT_TYPE_FORMATION_CHANGE
            | EVENT_TYPE_DELETED
            | EVENT_TYPE_INJURY_TIME
    )
}

/// Human-readable name for an Opta F24 event type (mirrors
/// `data/mappings/event_types.csv`).
#[inline]
pub fn event_type_name(type_id: u16) -> &'static str {
    match type_id {
        1 => "Pass",
        2 => "Offside Pass",
        3 => "Take On",
        4 => "Foul",
        5 => "Out",
        6 => "Corner Awarded",
        7 => "Tackle",
        8 => "Interception",
        10 => "Save",
        12 => "Clearance",
        13 => "Miss",
        14 => "Post",
        15 => "Attempt Saved",
        16 => "Goal",
        17 => "Card",
        18 => "Player off",
        19 => "Player on",
        24 => "Condition change",
        30 => "End",
        32 => "Start",
        34 => "Team set up",
        37 => "Collection End",
        40 => "Formation change",
        41 => "Punch",
        43 => "Deleted event",
        44 => "Aerial",
        45 => "Challenge",
        49 => "Ball recovery",
        50 => "Dispossessed",
        51 => "Error",
        52 => "Keeper pick-up",
        55 => "Offside provoked",
        56 => "Shield ball opp",
        59 => "Keeper Sweeper",
        61 => "Ball touch",
        70 => "Injury Time Announcement",
        74 => "Blocked Pass",
        _ => "Unknown",
    }
}

/// Events that move the ball as *live possession* (launch/attach). Events not
/// in this list must never drag the ball (substitutions, cards, …).
#[inline]
pub fn is_ball_event(type_id: u16) -> bool {
    matches!(
        type_id,
        1 | 2 | 3 | 7 | 8 | 10 | 12 | 13 | 14 | 15 | 16 | 41 | 44 | 45 | 49 | 50 | 51 | 52 | 59 | 61 | 74
    )
}

/// Dead-ball restarts: the ball is placed at the event origin and held still
/// rather than attached to the (off-pitch) actor.
#[inline]
pub fn is_dead_ball_event(type_id: u16) -> bool {
    matches!(type_id, 5 | 6)
}

/// Significant events worth surfacing as an on-screen splash.
#[inline]
pub fn is_notable_event(type_id: u16) -> bool {
    matches!(type_id, 2 | 4 | 6 | 13 | 14 | 15 | 16 | 17 | 18 | 19 | 55)
}

// ── Attack Direction ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AttackDirection {
    LeftToRight,
    RightToLeft,
}

impl AttackDirection {
    /// Parse from Opta qualifier string value.
    pub fn from_opta(s: &str) -> Option<Self> {
        match s.trim() {
            "Left to Right" => Some(AttackDirection::LeftToRight),
            "Right to Left" => Some(AttackDirection::RightToLeft),
            _ => None,
        }
    }

    /// Flip the direction (used for the opposing team's perspective).
    pub fn flip(self) -> Self {
        match self {
            AttackDirection::LeftToRight => AttackDirection::RightToLeft,
            AttackDirection::RightToLeft => AttackDirection::LeftToRight,
        }
    }
}

// ── Normalized Event ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedEvent {
    /// Unique Opta event id (64-bit: real ids are ~10 digits).
    pub id: u64,
    pub type_id: u16,
    pub period_id: u8,
    pub match_time_secs: f32,
    pub team_id: String,
    pub player_id: String,
    pub player_name: String,
    pub origin_x: f32,
    pub origin_y: f32,
    pub target_x: Option<f32>,
    pub target_y: Option<f32>,
    pub outcome: bool,
    pub is_aerial: bool,
    /// True for an own goal (qualifier 28 on a Goal row). `team_id` is then
    /// the *conceding* side and the goal is credited to its opponent.
    #[serde(default)]
    pub is_own_goal: bool,
}

impl NormalizedEvent {
    /// Time in minutes:seconds formatted string.
    pub fn time_string(&self) -> String {
        let total = self.match_time_secs.max(0.0);
        let mins = (total / 60.0) as u32;
        let secs = (total % 60.0) as u32;
        format!("{}:{:02}", mins, secs)
    }

    /// Whether this event carries a pass destination.
    pub fn has_target(&self) -> bool {
        self.target_x.is_some() && self.target_y.is_some()
    }
}

// ── Lineup Player ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineupPlayer {
    pub player_id: String,
    pub position_category: u8, // 1: GK, 2: DF, 3: MF, 4: FW, 5: Sub
    pub shirt_number: u8,
    pub formation_slot: u8, // 1-11 starting, 0 = sub
}

impl LineupPlayer {
    /// True when the player is in the starting XI.
    pub fn is_starter(&self) -> bool {
        (1..=11).contains(&self.formation_slot)
    }
}

// ── Match Context ────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct MatchContext {
    pub match_id: String,
    pub home_team_id: String,
    pub away_team_id: String,
    pub home_team_name: String,
    pub away_team_name: String,
    pub competition_id: String,
    pub season: String,
    pub game_date: String,
    pub lineups: HashMap<String, Vec<LineupPlayer>>,
    /// **Canonical** match frame: the home team always attacks toward +x and
    /// the away team toward −x. This is stable across half-time swaps, so it
    /// is the frame used by the formation and agent layers.
    pub directions: HashMap<String, AttackDirection>,
    /// Raw `qualifier_id=127` values as reported by the feed (physical side
    /// relative to the camera). These change at half-time and are kept for
    /// reference/display only — never use them for coordinates.
    pub physical_directions: HashMap<String, AttackDirection>,
    pub events: Vec<NormalizedEvent>,
}

impl MatchContext {
    /// Normalize raw Opta (0-100) coordinates to unified pitch meters.
    ///
    /// # Formula
    /// - LeftToRight:  `x_m = (x_opta / 100) * 105 - 52.5`
    /// - RightToLeft:  `x_m = ((100 - x_opta) / 100) * 105 - 52.5`
    ///
    /// The same mapping is applied to Y with the pitch width (68).
    ///
    /// Values are clamped to the pitch bounds to handle real-world data
    /// where Opta occasionally emits coordinates slightly outside [0, 100].
    #[inline]
    pub fn normalize_coord(x_opta: f32, y_opta: f32, dir: AttackDirection) -> (f32, f32) {
        let (nx, ny) = match dir {
            AttackDirection::LeftToRight => (x_opta / 100.0, y_opta / 100.0),
            AttackDirection::RightToLeft => ((100.0 - x_opta) / 100.0, (100.0 - y_opta) / 100.0),
        };
        let x = ((nx * PITCH_LENGTH) - PITCH_HALF_LENGTH).clamp(-PITCH_HALF_LENGTH, PITCH_HALF_LENGTH);
        let y = ((ny * PITCH_WIDTH) - PITCH_HALF_WIDTH).clamp(-PITCH_HALF_WIDTH, PITCH_HALF_WIDTH);
        (x, y)
    }

    /// Validate that a normalized coordinate lies on the pitch.
    #[inline]
    pub fn is_on_pitch(x: f32, y: f32) -> bool {
        (-PITCH_HALF_LENGTH - 0.01..=PITCH_HALF_LENGTH + 0.01).contains(&x)
            && (-PITCH_HALF_WIDTH - 0.01..=PITCH_HALF_WIDTH + 0.01).contains(&y)
    }

    /// The canonical frame direction for a team: home attacks +x, away −x.
    ///
    /// Opta F24 coordinates are expressed in the *attacking direction* of the
    /// active team, so this mapping produces one stable frame for the whole
    /// match regardless of half-time side swaps.
    #[inline]
    pub fn canonical_direction(&self, team_id: &str) -> AttackDirection {
        if team_id == self.away_team_id && !team_id.is_empty() {
            AttackDirection::RightToLeft
        } else {
            AttackDirection::LeftToRight
        }
    }

    /// Starting XI for a team (formation slot 1-11), sorted by slot.
    pub fn starting_xi(&self, team_id: &str) -> Vec<&LineupPlayer> {
        let mut players: Vec<&LineupPlayer> = self
            .lineups
            .get(team_id)
            .map(|v| v.iter().filter(|p| p.is_starter()).collect())
            .unwrap_or_default();
        players.sort_by_key(|p| p.formation_slot);
        players
    }
}

// ── Unit Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attack_direction_from_opta() {
        assert_eq!(
            AttackDirection::from_opta("Left to Right"),
            Some(AttackDirection::LeftToRight)
        );
        assert_eq!(
            AttackDirection::from_opta("Right to Left"),
            Some(AttackDirection::RightToLeft)
        );
        assert_eq!(AttackDirection::from_opta("unknown"), None);
    }

    #[test]
    fn test_attack_direction_flip() {
        assert_eq!(
            AttackDirection::LeftToRight.flip(),
            AttackDirection::RightToLeft
        );
        assert_eq!(
            AttackDirection::RightToLeft.flip(),
            AttackDirection::LeftToRight
        );
    }

    #[test]
    fn test_normalize_left_to_right() {
        let (x, y) = MatchContext::normalize_coord(0.0, 0.0, AttackDirection::LeftToRight);
        assert!((x - (-52.5)).abs() < 0.001);
        assert!((y - (-34.0)).abs() < 0.001);

        let (x, y) = MatchContext::normalize_coord(100.0, 100.0, AttackDirection::LeftToRight);
        assert!((x - 52.5).abs() < 0.001);
        assert!((y - 34.0).abs() < 0.001);

        let (x, y) = MatchContext::normalize_coord(50.0, 50.0, AttackDirection::LeftToRight);
        assert!((x - 0.0).abs() < 0.001);
        assert!((y - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_normalize_right_to_left() {
        let (x, y) = MatchContext::normalize_coord(0.0, 0.0, AttackDirection::RightToLeft);
        assert!((x - 52.5).abs() < 0.001);
        assert!((y - 34.0).abs() < 0.001);

        let (x, y) = MatchContext::normalize_coord(100.0, 100.0, AttackDirection::RightToLeft);
        assert!((x - (-52.5)).abs() < 0.001);
        assert!((y - (-34.0)).abs() < 0.001);
    }

    #[test]
    fn test_normalize_pass_destination() {
        let (tx, ty) = MatchContext::normalize_coord(95.2, 75.4, AttackDirection::LeftToRight);
        assert!((tx - (95.2 / 100.0 * 105.0 - 52.5)).abs() < 0.001);
        assert!((ty - (75.4 / 100.0 * 68.0 - 34.0)).abs() < 0.001);
    }

    #[test]
    fn test_is_on_pitch() {
        assert!(MatchContext::is_on_pitch(0.0, 0.0));
        assert!(MatchContext::is_on_pitch(52.5, 34.0));
        assert!(MatchContext::is_on_pitch(-52.5, -34.0));
        assert!(!MatchContext::is_on_pitch(60.0, 0.0));
        assert!(!MatchContext::is_on_pitch(0.0, 40.0));
    }

    #[test]
    fn test_event_time_string() {
        let ev = NormalizedEvent {
            id: 1,
            type_id: 1,
            period_id: 1,
            match_time_secs: 75.5,
            team_id: "t1".into(),
            player_id: "p1".into(),
            player_name: "Test".into(),
            origin_x: 0.0,
            origin_y: 0.0,
            target_x: None,
            target_y: None,
            outcome: true,
            is_aerial: false,
            is_own_goal: false,
        };
        assert_eq!(ev.time_string(), "1:15");
        assert!(!ev.has_target());
    }

    #[test]
    fn test_is_gameplay_event() {
        // Gameplay events must be retained
        assert!(is_gameplay_event(EVENT_TYPE_PASS));
        assert!(is_gameplay_event(EVENT_TYPE_GOAL)); // type 16 = Goal
        assert!(is_gameplay_event(EVENT_TYPE_AERIAL)); // type 44
        assert!(is_gameplay_event(7)); // Tackle
        assert!(is_gameplay_event(49)); // Ball recovery

        // Meta events must be excluded
        assert!(!is_gameplay_event(EVENT_TYPE_CONDITION_CHANGE));
        assert!(!is_gameplay_event(EVENT_TYPE_PERIOD_END));
        assert!(!is_gameplay_event(EVENT_TYPE_PERIOD_START));
        assert!(!is_gameplay_event(EVENT_TYPE_TEAM_SETUP));
        assert!(!is_gameplay_event(EVENT_TYPE_COLLECTION_END));
        assert!(!is_gameplay_event(EVENT_TYPE_FORMATION_CHANGE));
        assert!(!is_gameplay_event(EVENT_TYPE_DELETED));
        assert!(!is_gameplay_event(EVENT_TYPE_INJURY_TIME));
    }

    #[test]
    fn test_goal_is_not_treated_as_match_end() {
        // Regression: type 16 was previously (incorrectly) skipped as "match end".
        assert_eq!(EVENT_TYPE_GOAL, 16);
        assert_ne!(EVENT_TYPE_PERIOD_END, 16);
        assert!(is_gameplay_event(16));
    }

    #[test]
    fn test_lineup_player_is_starter() {
        let starter = LineupPlayer {
            player_id: "p".into(),
            position_category: 2,
            shirt_number: 5,
            formation_slot: 11,
        };
        let sub = LineupPlayer {
            formation_slot: 0,
            ..starter.clone()
        };
        assert!(starter.is_starter());
        assert!(!sub.is_starter());
    }
}
