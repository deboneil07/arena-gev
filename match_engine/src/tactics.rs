//! Tactical context — the "brain stem" shared by every player.
//!
//! Derives, once per team per tick, the macro facts a player needs:
//! attacking direction, own **back line**, opponent back line (the **offside
//! limit**), team **style**, and which teammates are the designated
//! presser / cover / support / runner.
//!
//! Keeping these in one place means the micro layer ([`crate::brain`]) never
//! has to guess at team shape, and every player reads a *consistent, coherent*
//! view of the world rather than one corrupted by iteration order.

use crate::agent::PlayerRole;
use crate::formation::Vector2;

// ── Lines ──────────────────────────────────────────────────────

/// A player's broad line on the pitch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    Goalkeeper,
    Defense,
    Midfield,
    Attack,
}

impl Line {
    /// Map a coarse role to its line.
    #[inline]
    pub fn from_role(role: PlayerRole) -> Self {
        match role {
            PlayerRole::Goalkeeper => Line::Goalkeeper,
            PlayerRole::Defender => Line::Defense,
            PlayerRole::Midfielder => Line::Midfield,
            PlayerRole::Forward | PlayerRole::Unknown => Line::Attack,
        }
    }

    /// How strongly this line prioritises holding shape over chasing the ball.
    /// Defenders are the most disciplined; forwards the most liberated.
    #[inline]
    pub fn shape_weight(self) -> f32 {
        match self {
            Line::Goalkeeper => 1.8,
            Line::Defense => 1.45,
            Line::Midfield => 1.20,
            Line::Attack => 0.95,
        }
    }
}

// ── Team style ────────────────────────────────────────────────

/// A deterministic tactical identity derived from the formation (and, later,
/// from squad attributes). All values are in `0..1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TeamStyle {
    /// 0 = parked bus, 1 = all-out attack.
    pub mentality: f32,
    /// 0 = low block, 1 = high press.
    pub pressing: f32,
    /// Attacking width (how far wide players hold the touchline).
    pub width: f32,
    /// Speed of circulation / willingness to play forward.
    pub tempo: f32,
    /// 0 = deep defensive line, 1 = high line.
    pub defensive_line: f32,
    /// How tightly the block stays together (vertical compactness).
    pub compactness: f32,
    /// 0 = patient build-up, 1 = direct into the channels.
    pub directness: f32,
}

impl Default for TeamStyle {
    fn default() -> Self {
        Self::from_formation("4-3-3")
    }
}

impl TeamStyle {
    /// Derive a style from a formation name. Unknown formations fall back to
    /// the balanced 4-3-3 identity.
    pub fn from_formation(name: &str) -> Self {
        match name {
            "4-3-3" => Self {
                mentality: 0.62,
                pressing: 0.62,
                width: 0.80,
                tempo: 0.62,
                defensive_line: 0.60,
                compactness: 0.52,
                directness: 0.48,
            },
            "4-2-3-1" => Self {
                mentality: 0.58,
                pressing: 0.66,
                width: 0.70,
                tempo: 0.58,
                defensive_line: 0.58,
                compactness: 0.60,
                directness: 0.46,
            },
            "4-4-2" => Self {
                mentality: 0.50,
                pressing: 0.55,
                width: 0.74,
                tempo: 0.55,
                defensive_line: 0.50,
                compactness: 0.58,
                directness: 0.55,
            },
            "3-5-2" => Self {
                mentality: 0.58,
                pressing: 0.60,
                width: 0.64,
                tempo: 0.56,
                defensive_line: 0.55,
                compactness: 0.62,
                directness: 0.52,
            },
            "5-3-2" => Self {
                mentality: 0.40,
                pressing: 0.44,
                width: 0.58,
                tempo: 0.48,
                defensive_line: 0.38,
                compactness: 0.70,
                directness: 0.48,
            },
            "4-1-4-1" => Self {
                mentality: 0.52,
                pressing: 0.58,
                width: 0.72,
                tempo: 0.55,
                defensive_line: 0.52,
                compactness: 0.64,
                directness: 0.44,
            },
            _ => Self::from_formation("4-3-3"),
        }
    }
}

// ── Per-team context (computed once per tick) ─────────────────

/// The macro facts about one team at one instant.
#[derive(Debug, Clone, Copy)]
pub struct TeamContext {
    /// True for the canonical home side (attacks +x).
    pub is_home: bool,
    /// +1 when the team attacks toward +x, −1 otherwise.
    pub attack_dx: f32,
    /// This team's own back-line x (the second-deepest outfielder).
    pub own_back_line_x: f32,
    /// The opponent's back-line x (where our runners must stay onside).
    pub opp_back_line_x: f32,
    /// The offside limit: opponent's line, never behind the ball/halfway.
    pub offside_limit_x: f32,
    pub ball_x: f32,
    pub ball_y: f32,
    /// Whether this team is currently in possession.
    pub in_possession: bool,
    /// True while a set piece is being taken: players hold their assigned
    /// positions and do not freelance.
    pub set_piece: bool,
    /// Team tactical identity.
    pub style: TeamStyle,
}

impl TeamContext {
    /// Position of this team's own goal centre.
    #[inline]
    pub fn own_goal(&self) -> Vector2 {
        Vector2::new(-self.attack_dx * crate::parser::PITCH_HALF_LENGTH, 0.0)
    }

    /// Position of the goal this team attacks.
    #[inline]
    pub fn target_goal(&self) -> Vector2 {
        Vector2::new(self.attack_dx * crate::parser::PITCH_HALF_LENGTH, 0.0)
    }

    /// Signed progress into the attacking half, roughly `-1..1`.
    #[inline]
    pub fn attack_progress(&self) -> f32 {
        (self.ball_x * self.attack_dx) / crate::parser::PITCH_HALF_LENGTH
    }
}

/// Which players are currently "activated" in a team's plan. Everyone else
/// holds shape, which is what keeps the unit cohesive instead of a swarm.
#[derive(Debug, Clone, Copy)]
pub struct TeamPlan {
    pub ctx: TeamContext,
    /// The one player closing down the ball (out of possession).
    pub presser: Option<usize>,
    /// The second presser, covering the space behind the first.
    pub cover: Option<usize>,
    /// Up to two passing options around the ball (in possession).
    pub support: [Option<usize>; 2],
    /// The one forward making a run in behind (in possession).
    pub runner: Option<usize>,
}

impl TeamPlan {
    #[inline]
    pub fn is_presser(&self, idx: usize) -> bool {
        self.presser == Some(idx)
    }
    #[inline]
    pub fn is_cover(&self, idx: usize) -> bool {
        self.cover == Some(idx)
    }
    #[inline]
    pub fn is_runner(&self, idx: usize) -> bool {
        self.runner == Some(idx)
    }
    #[inline]
    pub fn support_rank(&self, idx: usize) -> Option<usize> {
        self.support.iter().position(|s| *s == Some(idx))
    }
}
