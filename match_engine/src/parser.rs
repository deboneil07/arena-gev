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
// See `data/mappings/qualifiers.csv` for the full reference. IDs below are
// grouped by the meaning the engine actually consumes; anything else is kept
// verbatim in `NormalizedEvent::raw_qualifiers` for future use.

/// Long-ball intent (no length threshold; NOT an aerial indicator).
pub const QUAL_LONG_BALL: u16 = 1;
/// Cross (open play; +Q6 corner delivery, +Q5 free-kick delivery).
pub const QUAL_CROSS: u16 = 2;
/// Headed pass (distinct from Q15 headed shot).
pub const QUAL_HEAD_PASS: u16 = 3;
/// Through ball.
pub const QUAL_THROUGH_BALL: u16 = 4;
/// Free-kick delivery (direct or indirect).
pub const QUAL_FK_TAKEN: u16 = 5;
/// Corner delivery.
pub const QUAL_CORNER_TAKEN: u16 = 6;
/// Player id caught offside (on an offside pass, type 2).
pub const QUAL_OFFSIDE_PLAYER: u16 = 7;
/// Goal disallowed (on an offside pass leading to a ruled-out goal).
pub const QUAL_GOAL_DISALLOWED: u16 = 8;
/// Penalty context (taken, or awarded on a foul).
pub const QUAL_PENALTY: u16 = 9;
/// Foul qualifier code (value = offence code).
pub const QUAL_FOUL: u16 = 13;
/// Headed shot/goal (body part; cf. Q3 headed *pass*).
pub const QUAL_HEAD: u16 = 15;
/// Shot pitch zones 16–19 / 60–71 (small box centre … 35+).
pub const QUAL_SHOT_ZONE_MIN: u16 = 16;
pub const QUAL_SHOT_ZONE_MAX: u16 = 19;
/// Throw-in set-piece context (shot/pass from a throw-in).
pub const QUAL_THROW_SET_PIECE: u16 = 160;
/// Goal-mouth target Y (feed 0–100 units, posts at 45.2/54.8).
pub const QUAL_GOAL_MOUTH_Y: u16 = 102;
/// Goal-mouth target Z (feed units, 0 = ground, 38 = crossbar).
pub const QUAL_GOAL_MOUTH_Z: u16 = 103;
/// Right-footed action.
pub const QUAL_RIGHT_FOOT: u16 = 20;
/// Other body part (knee, chest, …).
pub const QUAL_OTHER_BODY_PART: u16 = 21;
/// Regular open play.
pub const QUAL_REGULAR_PLAY: u16 = 22;
/// Fast break.
pub const QUAL_FAST_BREAK: u16 = 23;
/// Set piece / fast-break context flag.
pub const QUAL_SET_PIECE_CTX: u16 = 24;
/// Shot following a corner.
pub const QUAL_FROM_CORNER: u16 = 25;
/// Shot struck directly from a free kick.
pub const QUAL_FK_DIRECT_SHOT: u16 = 26;
/// Assisted chance (a team-mate's pass set up the shot).
pub const QUAL_ASSISTED: u16 = 29;
/// Own goal, on a `type_id=16` (Goal) row. The row is credited to the
/// team of the player who put the ball in their own net, and its coordinates
/// sit at that player's *own* goal end (low x), so the team that actually
/// scores is the **opponent** and the ball must fly into the scorer's net.
pub const QUAL_OWN_GOAL: u16 = 28;
/// Newer feeds repeat the own-goal marker as qualifier 280 = "OWN_GOAL".
pub const QUAL_OWN_GOAL_FLAG: u16 = 280;
/// Fantasy assist type (PASS_LOST / BLOCKED_SHOT / …). NOT an own-goal flag.
pub const QUAL_FANTASY_ASSIST_TYPE: u16 = 280;
/// Yellow card.
pub const QUAL_YELLOW: u16 = 31;
/// Second yellow card.
pub const QUAL_SECOND_YELLOW: u16 = 32;
/// Straight red card.
pub const QUAL_RED: u16 = 33;
/// Injury substitution reason.
pub const QUAL_SUB_INJURY: u16 = 41;
/// Tactical substitution reason.
pub const QUAL_SUB_TACTICAL: u16 = 42;
/// Chipped pass (ball played through the air) — genuine aerial indicator.
pub const QUAL_CHIPPED: u16 = 155;
/// Lay-off pass.
pub const QUAL_LAYOFF: u16 = 156;
/// Launch (own-half punt upfield).
pub const QUAL_LAUNCH: u16 = 157;
/// Pitch direction of play for the active team ("Left to Right" / "Right to Left").
pub const QUAL_DIR: u16 = 127;
/// Throw-in.
pub const QUAL_THROW_IN: u16 = 107;
/// Goal kick.
pub const QUAL_GOAL_KICK: u16 = 124;
/// Kick-off pass: value `S` starts a period, `G` follows a goal.
pub const QUAL_KICK_OFF: u16 = 279;
/// Switch of play (pass crossing the centre zone, y-distance > 60).
pub const QUAL_SWITCH: u16 = 196;
/// Keeper punt upfield.
pub const QUAL_KEEPER_HOOF: u16 = 198;
/// In-swinging delivery.
pub const QUAL_INSWINGER: u16 = 223;
/// Out-swinging delivery.
pub const QUAL_OUTSWINGER: u16 = 224;
/// Direct free kick.
pub const QUAL_FK_DIRECT: u16 = 152;
/// Indirect free kick.
pub const QUAL_FK_INDIRECT: u16 = 241;
/// Low free-kick delivery.
pub const QUAL_FK_LOW: u16 = 237;
/// Left-footed action.
pub const QUAL_LEFT_FOOT: u16 = 72;
/// Volley (ball did not bounce before the shot).
pub const QUAL_VOLLEY: u16 = 108;
/// Flick-on header.
pub const QUAL_FLICK_ON: u16 = 168;
/// First-time shot (no controlling touch).
pub const QUAL_FIRST_TOUCH: u16 = 328;
/// Olimpico (shot/goal directly from a corner).
pub const QUAL_OLIMPICO: u16 = 263;
/// Goalkeeper got a touch on the ball.
pub const QUAL_KEEPER_TOUCHED: u16 = 136;
/// The shot was blocked.
pub const QUAL_BLOCKED: u16 = 82;
/// Blocked X: where the shot was blocked / an opponent touched it.
pub const QUAL_BLOCKED_X: u16 = 146;
/// Blocked Y.
pub const QUAL_BLOCKED_Y: u16 = 147;
/// Pass destination X (0-100).
pub const QUAL_TARGET_X: u16 = 140;
/// Pass destination Y (0-100).
pub const QUAL_TARGET_Y: u16 = 141;
/// Related event id (assist pass, delete pair, card↔foul link …).
pub const QUAL_RELATED_ID: u16 = 55;
/// Second related event id.
pub const QUAL_RELATED_ID_2: u16 = 216;
/// Opposite event id (shot↔save pair, out/corner twins …).
pub const QUAL_OPPOSITE_ID: u16 = 233;
/// Intentional assist (the passer's assist, also on the shot).
pub const QUAL_INTENTIONAL_ASSIST: u16 = 154;
/// The pass set up a shot/goal/missed chance.
pub const QUAL_ASSIST: u16 = 210;
/// Second assist (hockey assist).
pub const QUAL_SECOND_ASSIST: u16 = 218;
/// Intentional second assist.
pub const QUAL_SECOND_ASSIST_INTENT: u16 = 217;
/// Big chance (a clear-cut chance the player should score).
pub const QUAL_BIG_CHANCE: u16 = 214;
/// Individual play (dribble-led chance).
pub const QUAL_INDIVIDUAL_PLAY: u16 = 215;
/// Estimated pass/clearance length, metres travelled.
pub const QUAL_LENGTH_M: u16 = 212;
/// Pass/clearance direction, radians 0–2π relative to direction of play.
pub const QUAL_ANGLE_RAD: u16 = 213;
/// Offensive duel (player attacks the duel).
pub const QUAL_DUEL_OFF: u16 = 286;
/// Defensive duel (player defends the duel).
pub const QUAL_DUEL_DEF: u16 = 285;
/// Aerial foul.
pub const QUAL_AERIAL_FOUL: u16 = 264;
/// Take-on / skill qualifier on dribbles.
pub const QUAL_TAKE_ON: u16 = 465;
/// Leading to an attempt (error chains into a shot).
pub const QUAL_LEADING_TO_ATTEMPT: u16 = 169;
/// Parried safe (keeper).
pub const QUAL_PARRIED_SAFE: u16 = 173;
/// Keeper collected the ball.
pub const QUAL_COLLECTED: u16 = 177;
/// Standing (unsighted) save.
pub const QUAL_STANDING: u16 = 178;
/// Diving save.
pub const QUAL_DIVING: u16 = 179;
/// Save with hands.
pub const QUAL_HANDS: u16 = 182;
/// Save with feet.
pub const QUAL_FEET: u16 = 183;
/// Saved off the line.
pub const QUAL_SAVED_OFF_LINE: u16 = 101;
/// Shot hit the woodwork.
pub const QUAL_HIT_WOODWORK: u16 = 275;
/// Keeper X position for the shot (attacking-frame 0–100).
pub const QUAL_KEEPER_X: u16 = 230;
/// Keeper Y position for the shot.
pub const QUAL_KEEPER_Y: u16 = 231;
/// Keeper X when the ball crossed the line (post-match, goals only).
pub const QUAL_KEEPER_X_AT_GOAL: u16 = 395;
/// Keeper Y when the ball crossed the line.
pub const QUAL_KEEPER_Y_AT_GOAL: u16 = 396;
/// Player IDs in the lineup (comma separated).
pub const QUAL_LINEUP_PLAYERS: u16 = 30;
/// Playing position category per lineup player (comma separated).
pub const QUAL_LINEUP_POSITIONS: u16 = 44;
/// Shirt numbers per lineup player (comma separated).
pub const QUAL_LINEUP_SHIRTS: u16 = 59;
/// Formation slot per lineup player (1-11 = starting XI, 0 = bench).
pub const QUAL_LINEUP_SLOTS: u16 = 131;
/// Team formation id (on lineup events).
pub const QUAL_FORMATION_ID: u16 = 130;
/// Captain's player id.
pub const QUAL_CAPTAIN: u16 = 194;
/// Formation slot of the player coming on (1-11).
pub const QUAL_FORMATION_SLOT: u16 = 145;
/// Granular position of the sub coming on (1-10).
pub const QUAL_DETAILED_POSITION: u16 = 292;
/// Unknown companion to Q292 carried on sub-on rows.
pub const QUAL_SUB_EXTRA: u16 = 293;
/// Sub-on link qualifier (carried on type-19 rows in this feed).
pub const QUAL_SUB_LINK: u16 = 399;
/// Pitch zone: Back / Center / Left / Right.
pub const QUAL_ZONE: u16 = 56;
/// VAR delay marker (start delay, type 27).
pub const QUAL_VAR_DELAY: u16 = 364;
/// Deleted-event marker: the type the twin/backup row had before.
pub const QUAL_DELETED_TYPE: u16 = 144;
/// Pre-review type on a VAR-deleted event (type 84): a VAR-ruled-out goal
/// carries the goal's type id here.
pub const QUAL_PRE_REVIEW_TYPE: u16 = 436;
/// Twin-row marker (out/corner duplicates).
pub const QUAL_TWIN_A: u16 = 346;
/// Twin-row marker (out/corner duplicates).
pub const QUAL_TWIN_B: u16 = 347;
/// Out of play (pass went out).
pub const QUAL_OUT_OF_PLAY: u16 = 167;
/// Blocked cross.
pub const QUAL_BLOCKED_CROSS: u16 = 185;
/// Blocked-pass reason qualifier.
pub const QUAL_BLOCKED_PASS: u16 = 236;
/// High ball qualifier.
pub const QUAL_HIGH: u16 = 74;
/// Deflected block qualifier.
pub const QUAL_DEF_BLOCK: u16 = 94;
/// Six-yard blocked qualifier.
pub const QUAL_SIX_YARD_BLOCK: u16 = 100;

// ── Opta Event Type IDs ──────────────────────────────────────────────────────
//
// See `data/mappings/event_types.csv`. Only genuinely meta / non-positional
// events are excluded from gameplay; everything else is retained.

// Gameplay types referenced directly by the engine:
pub const EVENT_TYPE_PASS: u16 = 1;
pub const EVENT_TYPE_OFFSIDE_PASS: u16 = 2;
pub const EVENT_TYPE_TAKE_ON: u16 = 3;
pub const EVENT_TYPE_FOUL: u16 = 4;
pub const EVENT_TYPE_OUT: u16 = 5;
pub const EVENT_TYPE_CORNER: u16 = 6;
pub const EVENT_TYPE_TACKLE: u16 = 7;
pub const EVENT_TYPE_INTERCEPTION: u16 = 8;
pub const EVENT_TYPE_SAVE: u16 = 10;
pub const EVENT_TYPE_CLEARANCE: u16 = 12;
pub const EVENT_TYPE_MISS: u16 = 13;
pub const EVENT_TYPE_POST: u16 = 14;
pub const EVENT_TYPE_ATTEMPT_SAVED: u16 = 15;
pub const EVENT_TYPE_GOAL: u16 = 16;         // ⚠ was mislabelled "match end"
pub const EVENT_TYPE_AERIAL: u16 = 44;
pub const EVENT_TYPE_CHALLENGE: u16 = 45;
pub const EVENT_TYPE_CARD: u16 = 17;
pub const EVENT_TYPE_PLAYER_OFF: u16 = 18;
pub const EVENT_TYPE_PLAYER_ON: u16 = 19;
pub const EVENT_TYPE_BLOCKED_PASS: u16 = 74;
pub const EVENT_TYPE_CARRY: u16 = 83;
pub const EVENT_TYPE_VAR_DELETED: u16 = 84;

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
/// that should be simulated. Meta events (period start/end, condition
/// changes, deleted-event markers, formation changes, …) return `false`.
///
/// Formation changes (type 40) are captured as lineup state *before* this
/// filter — see `loader.rs` — so the mid-match shape switch is applied even
/// though the row itself never enters the movement timeline.
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
            | EVENT_TYPE_VAR_DELETED
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
        83 => "Carry",
        84 => "VAR review",
        _ => "Unknown",
    }
}

/// Events that move the ball as *live possession* (launch/attach). Events not
/// in this list must never drag the ball (substitutions, cards, carries …).
///
/// Type 83 (carry) is deliberately excluded: it is a soft touch in stride,
/// owned by whoever already has the ball. Staging the ball onto its origin
/// would drag play backward; the doer-position gate alone pins it.
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

/// How a pass was played (qualifiers 1–7, 107, 124, 152, 156, 157, 196, 198,
/// 223, 224, 237, 241, 279).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PassKind {
    #[default]
    Unknown,
    Ground,
    Cross,
    ThroughBall,
    LongBall,
    Switch,
    Cutback,
    Layoff,
    Launch,
    Throw,
    CornerTaken,
    FreeKickTaken,
    GoalKick,
    KickoffStart,
    KickoffRestart,
    KeeperHoof,
}

/// Which body part played the ball (Q15/20/21/72/108/168, pass Q3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BodyPart {
    #[default]
    Unknown,
    Right,
    Left,
    Head,
    Volley,
    Other,
}

/// Why an action failed, when it did (feed `outcome` + reason qualifiers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OutcomeReason {
    #[default]
    Unknown,
    Completed,
    Intercepted,
    Blocked,
    Out,
    Offside,
    Saved,
    Post,
    Miss,
}

/// Shot pitch zone (qualifiers 16–19 / 60–71): small box centre … 35+.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ShotZone {
    #[default]
    Unknown,
    SmallBoxCentre,
    BoxCentre,
    OutOfBoxCentre,
    FarOut,
}

/// Pitch zone (qualifier 56): Back / Center / Left / Right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PitchZone {
    #[default]
    Unknown,
    Back,
    Center,
    Left,
    Right,
}

/// Set-piece delivery tag (Q223/224 swing, Q152/241/237 direct/indirect/low,
/// Q22/23/24/25/160 context).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SetPieceTag {
    #[default]
    Unknown,
    OpenPlay,
    FastBreak,
    FromCorner,
    FromThrow,
    Direct,
    Indirect,
    Low,
    Inswinger,
    Outswinger,
}

/// Duel polarity (Q286 offensive / Q285 defensive / Q264 aerial foul).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DuelPolarity {
    #[default]
    Unknown,
    Offensive,
    Defensive,
    AerialFoul,
}

/// Keeper action on a shot (Q173/177/178/179/182/183 + Q101/Q136/Q275).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum KeeperAction {
    #[default]
    Unknown,
    ParriedSafe,
    Collected,
    Standing,
    Diving,
    Hands,
    Feet,
    Touched,
    SavedOffLine,
}

/// Which card was shown (Q31 yellow / Q32 second yellow / Q33 red).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CardKind {
    #[default]
    None,
    Yellow,
    SecondYellow,
    Red,
}

/// Why a substitution was made (Q41 injury / Q42 tactical).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SubReason {
    #[default]
    Unknown,
    Injury,
    Tactical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedEvent {
    /// Unique Opta event id (64-bit: real ids are ~10 digits).
    pub id: u64,
    pub type_id: u16,
    pub period_id: u8,
    /// Simulation clock: chronological across periods (later periods are
    /// offset past the previous period's last event + a half-time gap).
    pub match_time_secs: f32,
    /// Display clock: raw `min*60+sec` within the period, for mm:ss UI rows.
    /// Never offset — P2 45:00 displays as 45:00, not 107:00.
    #[serde(default)]
    pub display_secs: f32,
    /// Sub-second ordering within the same `min:sec` tick, from the feed's
    /// `timestamp` wall clock when present (0.0 = feed order only).
    #[serde(default)]
    pub time_order: f32,
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
    // ── WS0: whole-event semantics (all `#[serde(default)]` → additive) ──
    /// How the pass was played (type 1 rows).
    #[serde(default)]
    pub pass_kind: PassKind,
    /// Body part used.
    #[serde(default)]
    pub body_part: BodyPart,
    /// Why the action succeeded / failed.
    #[serde(default)]
    pub outcome_reason: OutcomeReason,
    /// Shot pitch zone (types 13–16).
    #[serde(default)]
    pub shot_zone: ShotZone,
    /// Goal-mouth target `(y102, z103)` in feed 0–100 units, when known.
    #[serde(default)]
    pub goalmouth: Option<(f32, f32)>,
    /// Block point `(x146, y147)` where a shot was blocked / touched.
    #[serde(default)]
    pub blocked_at: Option<(f32, f32)>,
    /// Keeper position `(x230, y231)` for the shot, attacking-frame 0–100.
    #[serde(default)]
    pub keeper_xy: Option<(f32, f32)>,
    /// Keeper position at goal time `(Q395, Q396)` for goals.
    #[serde(default)]
    pub keeper_at_goal: Option<(f32, f32)>,
    /// Explicit link graph: related (Q55) / second (Q216) / opposite (Q233).
    #[serde(default)]
    pub related_id: Option<u64>,
    #[serde(default)]
    pub related_id_2: Option<u64>,
    #[serde(default)]
    pub opposite_id: Option<u64>,
    /// Assist graph: intentional (Q154), assist (Q210), 2nd (Q218/217).
    #[serde(default)]
    pub assist: bool,
    #[serde(default)]
    pub intentional_assist: bool,
    #[serde(default)]
    pub second_assist: bool,
    /// Big chance (Q214), individual play (Q215), first-time shot (Q328),
    /// Olimpico (Q263), keeper touched (Q136).
    #[serde(default)]
    pub big_chance: bool,
    #[serde(default)]
    pub individual_play: bool,
    #[serde(default)]
    pub first_time: bool,
    #[serde(default)]
    pub olimpico: bool,
    #[serde(default)]
    pub keeper_touched: bool,
    /// Set-piece delivery + zone context.
    #[serde(default)]
    pub setpiece: SetPieceTag,
    #[serde(default)]
    pub setpiece_swing: SetPieceTag,
    #[serde(default)]
    pub zone: PitchZone,
    /// Duel polarity + aerial won/lost (from `outcome`).
    #[serde(default)]
    pub duel: DuelPolarity,
    /// Keeper action on shots/saves.
    #[serde(default)]
    pub keeper_action: KeeperAction,
    /// Hit the woodwork (Q275).
    #[serde(default)]
    pub hit_woodwork: bool,
    /// Measured ball travel, metres (Q212) + direction, radians (Q213).
    #[serde(default)]
    pub length_m: Option<f32>,
    #[serde(default)]
    pub angle_rad: Option<f32>,
    /// Card shown + foul code (Q13 value).
    #[serde(default)]
    pub card: CardKind,
    #[serde(default)]
    pub foul_code: Option<String>,
    #[serde(default)]
    pub foul_victim_of: Option<u64>,
    /// Substitution semantics: reason, slot, links, shirts.
    #[serde(default)]
    pub sub_reason: SubReason,
    #[serde(default)]
    pub formation_slot: Option<u8>,
    #[serde(default)]
    pub sub_link: Option<u64>,
    #[serde(default)]
    pub shirt_number: Option<u8>,
    /// Offside: player ids caught offside (Q7), disallowed goal (Q8).
    #[serde(default)]
    pub offside_players: Vec<String>,
    #[serde(default)]
    pub goal_disallowed: bool,
    /// Penalty context (Q9).
    #[serde(default)]
    pub penalty: bool,
    /// Error leading to an attempt (Q169).
    #[serde(default)]
    pub leading_to_attempt: bool,
    /// Carousel rows: type-83 carry toucher (Q399 player id).
    #[serde(default)]
    pub carry_of: Option<String>,
    /// Every qualifier not mapped above, verbatim (id → raw value).
    #[serde(default)]
    pub raw_qualifiers: Vec<(u16, String)>,
}

impl NormalizedEvent {
    /// Minimal event for tests: everything semantic defaults to unknown.
    pub fn test(
        id: u64,
        type_id: u16,
        time: f32,
        team: &str,
        player: &str,
        x: f32,
        y: f32,
    ) -> Self {
        Self {
            id,
            type_id,
            period_id: 1,
            match_time_secs: time,
            display_secs: time,
            time_order: 0.0,
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
            pass_kind: PassKind::default(),
            body_part: BodyPart::default(),
            outcome_reason: OutcomeReason::default(),
            shot_zone: ShotZone::default(),
            goalmouth: None,
            blocked_at: None,
            keeper_xy: None,
            keeper_at_goal: None,
            related_id: None,
            related_id_2: None,
            opposite_id: None,
            assist: false,
            intentional_assist: false,
            second_assist: false,
            big_chance: false,
            individual_play: false,
            first_time: false,
            olimpico: false,
            keeper_touched: false,
            setpiece: SetPieceTag::default(),
            setpiece_swing: SetPieceTag::default(),
            zone: PitchZone::default(),
            duel: DuelPolarity::default(),
            keeper_action: KeeperAction::default(),
            hit_woodwork: false,
            length_m: None,
            angle_rad: None,
            card: CardKind::default(),
            foul_code: None,
            foul_victim_of: None,
            sub_reason: SubReason::default(),
            formation_slot: None,
            sub_link: None,
            shirt_number: None,
            offside_players: Vec::new(),
            goal_disallowed: false,
            penalty: false,
            leading_to_attempt: false,
            carry_of: None,
            raw_qualifiers: Vec::new(),
        }
    }

    /// Time in minutes:seconds formatted string (display clock).
    pub fn time_string(&self) -> String {
        let total = self.display_secs.max(0.0);
        let mins = (total / 60.0) as u32;
        let secs = (total % 60.0) as u32;
        format!("{}:{:02}", mins, secs)
    }

    /// Whether this event carries a pass destination.
    pub fn has_target(&self) -> bool {
        self.target_x.is_some() && self.target_y.is_some()
    }

    /// True when this pass was completed to a team-mate.
    pub fn is_completed_pass(&self) -> bool {
        self.type_id == EVENT_TYPE_PASS
            && self.outcome
            && self.outcome_reason != OutcomeReason::Offside
    }

    /// True when this pass failed (intercepted / blocked / out / offside).
    pub fn is_failed_pass(&self) -> bool {
        matches!(self.type_id, 1 | 2 | 74)
            && (!self.outcome || self.outcome_reason != OutcomeReason::Completed)
    }

    /// True when this is a shot (13–16).
    pub fn is_shot(&self) -> bool {
        matches!(self.type_id, 13..=16)
    }

    /// Goal-mouth target in canonical metres `(lateral_y, height_z)`.
    ///
    /// The feed's `Q102` spans 45.2–54.8 across the mouth in the *attacking*
    /// frame: canonical `y = (q/100·68 − 34)`, mirrored for the away side, so
    /// `lateral = (q − 50)·0.68·attack_dx`. `Q103` is 0 (ground) to 38 (bar)
    /// → 0–2.44 m, unaffected by direction.
    pub fn goalmouth_target(&self, attack_dx: f32) -> Option<(f32, f32)> {
        let (y102, z103) = self.goalmouth?;
        Some((
            ((y102 - 50.0) * 0.68 * attack_dx).clamp(-3.66, 3.66),
            (z103 * 2.44 / 38.0).clamp(0.0, 2.44),
        ))
    }

    /// Estimated xG bucket from the feed's own chance flags + zone/body.
    /// Transparent heuristic (0.02–0.65), NOT a trained model: label any UI
    /// readout as a model estimate.
    pub fn estimate_xg(&self) -> Option<f32> {
        if !self.is_shot() {
            return None;
        }
        let mut xg: f32 = match self.shot_zone {
            ShotZone::SmallBoxCentre => 0.38,
            ShotZone::BoxCentre => 0.14,
            ShotZone::OutOfBoxCentre => 0.05,
            ShotZone::FarOut => 0.03,
            ShotZone::Unknown => 0.08,
        };
        if self.penalty {
            xg = 0.76;
        }
        if self.big_chance {
            xg = (xg + 0.30).min(0.65);
        }
        if self.first_time {
            xg *= 1.1;
        }
        if self.body_part == BodyPart::Head {
            xg *= 0.8;
        }
        Some(xg.clamp(0.02, 0.76))
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

// ── Formation Change ─────────────────────────────────────────────────────────

/// A mid-match shape switch (type-40 row): when it happened, for which team,
/// and the XI it named.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormationChange {
    pub match_time_secs: f32,
    pub period_id: u8,
    pub team_id: String,
    pub lineup: Vec<LineupPlayer>,
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
    /// Mid-match formation changes (type 40), chronological: (time, team,
    /// XI named on the row). Applied by the engine when the clock reaches
    /// them — never merged into the opening lineup.
    #[serde(default)]
    pub formation_changes: Vec<FormationChange>,
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
        let ev = NormalizedEvent::test(1, 1, 75.5, "t1", "Test", 0.0, 0.0);
        assert_eq!(ev.time_string(), "1:15");
        assert!(!ev.has_target());
        assert!(!ev.is_completed_pass() || ev.outcome);
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
        assert!(!is_gameplay_event(EVENT_TYPE_VAR_DELETED));
        assert!(!is_gameplay_event(EVENT_TYPE_INJURY_TIME));
        // Carry rows (type 83) are soft touches: simulated for the doer beat
        // (and their Q399 link) but never ball-staged.
        assert!(is_gameplay_event(EVENT_TYPE_CARRY));
        assert!(!is_ball_event(EVENT_TYPE_CARRY));
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
