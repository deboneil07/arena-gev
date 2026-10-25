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
use crate::parser::LineupPlayer;
use std::collections::HashMap;

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

// ── Duties ────────────────────────────────────────────────────

/// A player's specific job inside the team shape.
///
/// Coarse `PlayerRole` is enough for line logic, but off-ball movement needs
/// to know a centre-back from a full-back, or a holding midfielder from a
/// No.10. Duties are derived once from the lineup and its canonical template,
/// then travel with the agent through substitutions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Duty {
    Unknown,
    Goalkeeper,
    CentreBack,
    FullBack,
    DefensiveMid,
    CentralMid,
    AttackingMid,
    Winger,
    Striker,
}

impl Default for Duty {
    fn default() -> Self {
        Duty::Unknown
    }
}

impl Duty {
    /// Fallback classification when only the coarse role and a lateral
    /// position are known (unit tests, legacy agents).
    pub fn from_role(role: PlayerRole, y: f32) -> Self {
        let wide = y.abs() >= 14.0;
        match role {
            PlayerRole::Goalkeeper => Duty::Goalkeeper,
            PlayerRole::Defender => {
                if wide {
                    Duty::FullBack
                } else {
                    Duty::CentreBack
                }
            }
            PlayerRole::Midfielder => {
                if wide {
                    Duty::Winger
                } else {
                    Duty::CentralMid
                }
            }
            PlayerRole::Forward => {
                if wide {
                    Duty::Winger
                } else {
                    Duty::Striker
                }
            }
            PlayerRole::Unknown => Duty::CentralMid,
        }
    }

    #[inline]
    pub fn line(self) -> Line {
        match self {
            Duty::Goalkeeper => Line::Goalkeeper,
            Duty::CentreBack | Duty::FullBack => Line::Defense,
            Duty::DefensiveMid | Duty::CentralMid | Duty::AttackingMid => Line::Midfield,
            Duty::Winger | Duty::Striker => Line::Attack,
            Duty::Unknown => Line::Midfield,
        }
    }

    #[inline]
    pub fn is_wide(self) -> bool {
        matches!(self, Duty::FullBack | Duty::Winger)
    }

    #[inline]
    pub fn is_central(self) -> bool {
        !self.is_wide()
    }

    /// How comfortable this duty is stepping away from its anchor to press.
    #[inline]
    pub fn press_bias(self) -> f32 {
        match self {
            Duty::Striker => 3.0,
            Duty::Winger | Duty::AttackingMid => 2.0,
            Duty::CentralMid => 0.5,
            Duty::FullBack => -0.5,
            Duty::DefensiveMid => -1.0,
            Duty::CentreBack => -3.0,
            Duty::Goalkeeper => -10.0,
            Duty::Unknown => 0.0,
        }
    }

    /// Minutes-of-football preference for marking, relative to an anchor.
    /// Lower is a better fit for the given opponent duty.
    #[inline]
    pub fn mark_fit(self, opponent: Duty) -> f32 {
        use Duty::*;
        match opponent {
            Striker => match self {
                CentreBack => 0.0,
                DefensiveMid => 2.0,
                FullBack => 5.0,
                CentralMid => 6.0,
                AttackingMid => 9.0,
                Winger => 12.0,
                _ => 18.0,
            },
            Winger => match self {
                FullBack => 0.0,
                Winger => 2.0,
                CentralMid => 5.0,
                DefensiveMid => 7.0,
                CentreBack => 9.0,
                _ => 14.0,
            },
            AttackingMid => match self {
                DefensiveMid => 0.0,
                CentralMid => 1.5,
                CentreBack => 5.0,
                FullBack => 7.0,
                Winger => 9.0,
                _ => 14.0,
            },
            CentralMid => match self {
                CentralMid => 0.0,
                DefensiveMid => 2.0,
                AttackingMid => 3.0,
                FullBack => 5.0,
                Winger => 7.0,
                _ => 12.0,
            },
            DefensiveMid => match self {
                CentralMid => 0.0,
                DefensiveMid => 2.0,
                AttackingMid => 3.0,
                CentreBack => 5.0,
                FullBack => 7.0,
                Winger => 9.0,
                _ => 12.0,
            },
            FullBack => match self {
                Winger => 0.0,
                CentralMid => 2.5,
                AttackingMid => 4.0,
                Striker => 8.0,
                _ => 10.0,
            },
            CentreBack | Goalkeeper => match self {
                Striker => 0.0,
                AttackingMid => 3.0,
                Winger => 5.0,
                CentralMid => 7.0,
                _ => 12.0,
            },
            Unknown => 6.0,
        }
    }

    /// Derive duties for a starting XI from its Opta categories and the
    /// canonical template (home frame, x negative). Positions are classified
    /// by line count and lateral role, so a 4-2-3-1 keeps its double pivot and
    /// a 4-4-2's wide midfielders become wingers rather than central mids.
    pub fn from_lineup(
        lineup: &[LineupPlayer],
        template: &[Vector2],
    ) -> HashMap<String, Duty> {
        let mut def_count = 0usize;
        let mut mid_count = 0usize;
        let mut fwd_count = 0usize;
        let mut mid_min_x = f32::MAX;
        let mut mid_max_x = f32::MIN;
        for p in lineup.iter().filter(|p| p.is_starter()) {
            match p.position_category {
                2 => def_count += 1,
                3 => {
                    mid_count += 1;
                    let idx = (p.formation_slot.saturating_sub(1)) as usize;
                    if let Some(pos) = template.get(idx) {
                        mid_min_x = mid_min_x.min(pos.x);
                        mid_max_x = mid_max_x.max(pos.x);
                    }
                }
                4 => fwd_count += 1,
                _ => {}
            }
        }
        if mid_count == 0 {
            mid_min_x = -18.0;
            mid_max_x = -18.0;
        }

        let mut duties = HashMap::with_capacity(lineup.len());
        for p in lineup.iter().filter(|p| p.is_starter()) {
            let idx = (p.formation_slot.saturating_sub(1)) as usize;
            let pos = template
                .get(idx)
                .copied()
                .unwrap_or(Vector2::new(-18.0, 0.0));
            let duty = classify_duty(
                p.position_category,
                pos,
                def_count,
                mid_count,
                fwd_count,
                mid_min_x,
                mid_max_x,
            );
            duties.insert(p.player_id.clone(), duty);
        }
        duties
    }
}

fn classify_duty(
    category: u8,
    pos: Vector2,
    def_count: usize,
    mid_count: usize,
    fwd_count: usize,
    mid_min_x: f32,
    mid_max_x: f32,
) -> Duty {
    let wide = pos.y.abs();
    match category {
        1 => Duty::Goalkeeper,
        2 => {
            // A back three/five is a central unit; only a true back four (or
            // wing-back system) gives its widest defenders a full-back job.
            if def_count >= 4 && wide >= 14.0 {
                Duty::FullBack
            } else {
                Duty::CentreBack
            }
        }
        3 => {
            if wide >= 16.0 {
                return Duty::Winger;
            }
            if mid_count <= 1 {
                return if pos.x <= -18.0 {
                    Duty::DefensiveMid
                } else {
                    Duty::CentralMid
                };
            }
            if mid_count == 2 {
                return Duty::DefensiveMid;
            }
            if mid_count == 3 {
                return if pos.x <= mid_min_x + 1.0 {
                    Duty::DefensiveMid
                } else {
                    Duty::CentralMid
                };
            }
            if mid_count >= 5 {
                if pos.x <= mid_min_x + 1.0 && wide <= 10.0 {
                    return Duty::DefensiveMid;
                }
                if pos.x >= mid_max_x - 1.0 {
                    return Duty::AttackingMid;
                }
            }
            Duty::CentralMid
        }
        _ => {
            // Forwards: a lone front man or a strike pair are central; a
            // front three uses wide forwards.
            if fwd_count >= 3 && wide >= 14.0 {
                Duty::Winger
            } else {
                Duty::Striker
            }
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

// ── Event lookahead ───────────────────────────────────────────

/// A scheduled on-ball beat that every player can see, whether or not the
/// feed names him as its actor. `time` is when the cue matters: the event
/// timestamp for an action, or the estimated landing time for a pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EventCue {
    pub origin: Vector2,
    pub target: Option<Vector2>,
    pub time: f32,
    pub type_id: u16,
}

impl EventCue {
    /// Where the cue will actually be played: the pass landing/shot target
    /// when known, otherwise the event origin.
    #[inline]
    pub fn spot(self) -> Vector2 {
        self.target.unwrap_or(self.origin)
    }

    #[inline]
    pub fn is_pass(self) -> bool {
        matches!(self.type_id, 1 | 2 | 44)
    }

    #[inline]
    pub fn is_shot(self) -> bool {
        matches!(self.type_id, 13..=16)
    }

    #[inline]
    pub fn is_loose(self) -> bool {
        matches!(self.type_id, 12 | 44 | 49)
    }
}

/// The four things a team needs to know about the near future. Every player
/// reads the same block, so support, cover, box runners and second-ball
/// screeners all react to the same next event instead of guessing.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LookaheadBlock {
    /// Next on-ball event by this team.
    pub team: Option<EventCue>,
    /// Next on-ball event by the opponent.
    pub opponent: Option<EventCue>,
    /// Landing point of this team's next event when that event is a
    /// pass/cross/aerial. `None` for shots, tackles or a pass without a
    /// known target.
    pub team_landing: Option<EventCue>,
    /// Landing point of the opponent's next event when it is a pass/cross/
    /// aerial.
    pub opponent_landing: Option<EventCue>,
}

impl LookaheadBlock {
    #[inline]
    pub fn team_spot(&self) -> Option<Vector2> {
        self.team_landing
            .map(EventCue::spot)
            .or_else(|| self.team.map(EventCue::spot))
    }

    #[inline]
    pub fn opponent_spot(&self) -> Option<Vector2> {
        self.opponent_landing
            .map(EventCue::spot)
            .or_else(|| self.opponent.map(EventCue::spot))
    }

    /// When the team's next meaningful beat happens (landing preferred).
    #[inline]
    pub fn team_time(&self) -> Option<f32> {
        self.team_landing
            .map(|c| c.time)
            .or_else(|| self.team.map(|c| c.time))
    }

    #[inline]
    pub fn opponent_time(&self) -> Option<f32> {
        self.opponent_landing
            .map(|c| c.time)
            .or_else(|| self.opponent.map(|c| c.time))
    }
}

// ── Phase state machine ───────────────────────────────────────

/// The current game phase for one team. Off-ball movement should look
/// different while building from the back, pinning a low block, or pressing
/// after a turnover; this enum is the shared vocabulary for that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamPhase {
    BuildUp,
    Progression,
    FinalThird,
    AttackingTransition,
    HighPress,
    MidBlock,
    LowBlock,
    DefensiveTransition,
}

impl Default for TeamPhase {
    fn default() -> Self {
        TeamPhase::MidBlock
    }
}

impl TeamPhase {
    #[inline]
    pub fn in_possession(self) -> bool {
        matches!(
            self,
            TeamPhase::BuildUp
                | TeamPhase::Progression
                | TeamPhase::FinalThird
                | TeamPhase::AttackingTransition
        )
    }

    #[inline]
    pub fn is_transition(self) -> bool {
        matches!(
            self,
            TeamPhase::AttackingTransition | TeamPhase::DefensiveTransition
        )
    }

    /// Small pace multiplier for phase-appropriate urgency.
    #[inline]
    pub fn tempo(self) -> f32 {
        match self {
            TeamPhase::AttackingTransition => 1.22,
            TeamPhase::DefensiveTransition => 1.18,
            TeamPhase::FinalThird | TeamPhase::HighPress => 1.08,
            TeamPhase::BuildUp | TeamPhase::LowBlock => 0.94,
            _ => 1.0,
        }
    }

    /// How far a player may roam from the shape anchor in this phase.
    #[inline]
    pub fn zone_scale(self) -> f32 {
        match self {
            TeamPhase::AttackingTransition | TeamPhase::DefensiveTransition => 1.15,
            TeamPhase::FinalThird => 1.08,
            TeamPhase::LowBlock => 0.82,
            TeamPhase::BuildUp => 0.92,
            _ => 1.0,
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
    /// Current phase of this team's game (build-up, press, low block…).
    pub phase: TeamPhase,
    /// Shared lookahead block: this team's next event, the opponent's next
    /// event, and both landing points. Every player reads it, not just the
    /// man the feed happens to name.
    pub lookahead: LookaheadBlock,
    /// Simulation clock at the start of this tick (match seconds).
    pub now: f32,
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
    /// One-to-one marking assignments for this team, indexed by the global
    /// agent index of the *marker*. The value is the global agent index of
    /// the opponent he is responsible for. The presser and cover are never
    /// assigned a mark: they are already occupied with the ball.
    pub marks: [Option<usize>; 22],
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

    /// The opponent this player is responsible for, if any.
    #[inline]
    pub fn mark_for(&self, idx: usize) -> Option<usize> {
        self.marks.get(idx).copied().flatten()
    }

    #[inline]
    pub fn is_marked_by(&self, defender: usize, opponent: usize) -> bool {
        self.mark_for(defender) == Some(opponent)
    }
}

#[cfg(test)]
mod tactics_tests {
    use super::*;
    use crate::agent::PlayerRole;

    #[test]
    fn duty_from_role_is_wide_aware() {
        assert_eq!(Duty::from_role(PlayerRole::Goalkeeper, 0.0), Duty::Goalkeeper);
        assert_eq!(Duty::from_role(PlayerRole::Defender, 18.0), Duty::FullBack);
        assert_eq!(Duty::from_role(PlayerRole::Defender, 4.0), Duty::CentreBack);
        assert_eq!(Duty::from_role(PlayerRole::Midfielder, 20.0), Duty::Winger);
        assert_eq!(Duty::from_role(PlayerRole::Midfielder, 2.0), Duty::CentralMid);
        assert_eq!(Duty::from_role(PlayerRole::Forward, 19.0), Duty::Winger);
        assert_eq!(Duty::from_role(PlayerRole::Forward, 0.0), Duty::Striker);
    }

    #[test]
    fn back_three_stays_central_and_back_four_uses_fullbacks() {
        let wide = Vector2::new(-26.0, 18.0);
        assert_eq!(
            classify_duty(2, wide, 3, 3, 3, -18.0, -6.0),
            Duty::CentreBack,
            "a back three's widest defender is still a centre-back"
        );
        assert_eq!(
            classify_duty(2, wide, 4, 3, 3, -18.0, -6.0),
            Duty::FullBack
        );
        assert_eq!(
            classify_duty(2, Vector2::new(-30.0, 4.0), 4, 3, 3, -18.0, -6.0),
            Duty::CentreBack
        );
    }

    #[test]
    fn midfield_triangle_keeps_a_holder_and_two_eights() {
        let holder = classify_duty(3, Vector2::new(-20.0, 0.0), 4, 3, 3, -20.0, -10.0);
        let right = classify_duty(3, Vector2::new(-15.0, -12.0), 4, 3, 3, -20.0, -10.0);
        let left = classify_duty(3, Vector2::new(-15.0, 12.0), 4, 3, 3, -20.0, -10.0);
        assert_eq!(holder, Duty::DefensiveMid);
        assert_eq!(right, Duty::CentralMid);
        assert_eq!(left, Duty::CentralMid);
    }

    #[test]
    fn double_pivot_and_number_ten_are_distinguished() {
        let min_x = -18.0;
        let max_x = -6.0;
        assert_eq!(
            classify_duty(3, Vector2::new(-18.0, -8.0), 4, 5, 1, min_x, max_x),
            Duty::DefensiveMid
        );
        assert_eq!(
            classify_duty(3, Vector2::new(-18.0, 8.0), 4, 5, 1, min_x, max_x),
            Duty::DefensiveMid
        );
        assert_eq!(
            classify_duty(3, Vector2::new(-6.0, 0.0), 4, 5, 1, min_x, max_x),
            Duty::AttackingMid
        );
        assert_eq!(
            classify_duty(3, Vector2::new(-6.0, 20.0), 4, 5, 1, min_x, max_x),
            Duty::Winger
        );
    }

    #[test]
    fn wide_forwards_become_wingers_only_in_a_front_three() {
        assert_eq!(
            classify_duty(4, Vector2::new(20.0, 15.0), 4, 3, 3, -18.0, -6.0),
            Duty::Winger
        );
        assert_eq!(
            classify_duty(4, Vector2::new(20.0, 0.0), 4, 4, 2, -18.0, -6.0),
            Duty::Striker
        );
        assert_eq!(
            classify_duty(4, Vector2::new(20.0, 15.0), 4, 4, 1, -18.0, -6.0),
            Duty::Striker,
            "a lone front man stays central even when listed wide"
        );
    }

    #[test]
    fn mark_fit_prefers_the_natural_matchup() {
        assert!(Duty::CentreBack.mark_fit(Duty::Striker) < Duty::Winger.mark_fit(Duty::Striker));
        assert!(Duty::FullBack.mark_fit(Duty::Winger) < Duty::CentreBack.mark_fit(Duty::Winger));
        assert!(
            Duty::DefensiveMid.mark_fit(Duty::AttackingMid)
                < Duty::Striker.mark_fit(Duty::AttackingMid)
        );
    }

    #[test]
    fn phases_report_possession_and_scale() {
        assert!(TeamPhase::BuildUp.in_possession());
        assert!(TeamPhase::AttackingTransition.in_possession());
        assert!(!TeamPhase::HighPress.in_possession());
        assert!(TeamPhase::AttackingTransition.is_transition());
        assert!(TeamPhase::DefensiveTransition.is_transition());
        assert!(TeamPhase::LowBlock.zone_scale() < 1.0);
        assert!(TeamPhase::AttackingTransition.tempo() > 1.0);
    }
}
