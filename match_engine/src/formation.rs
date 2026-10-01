//! Elastic Formation Grid — macro layer of the simulation.
//!
//! Computes a dynamic tactical anchor **Aᵢ(t)** for every player based on:
//! - Ball position → team centroid shifts
//! - Possession phase → formation stretches (attack) or compresses (defend)
//! - Attack direction → mirrors coordinates for RTL teams
//! - Goalkeeper → tethered to the 18-yard box
//!
//! # Math
//!
//! ```text
//! C_team = C_base + K ⊙ (P_ball − C_base)
//! A_i    = C_team + S_phase ⊙ (B_i − C_base)
//! ```
//!
//! `S_phase` is `(1.15, 1.20)` in possession and `(0.85, 0.80)` out of
//! possession.

use crate::parser::{AttackDirection, LineupPlayer};
use std::collections::HashMap;

// ── Public Types ────────────────────────────

/// 2D vector in pitch metres (origin at the centre spot).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vector2 {
    pub x: f32,
    pub y: f32,
}

impl Vector2 {
    #[inline]
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    #[inline]
    pub fn zero() -> Self {
        Self { x: 0.0, y: 0.0 }
    }

    #[inline]
    pub fn distance_to(&self, other: Vector2) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }

    /// Squared length: avoids a `sqrt` when only comparing distances.
    #[inline]
    pub fn length_sq(self) -> f32 {
        self.x * self.x + self.y * self.y
    }

    /// Euclidean length.
    #[inline]
    pub fn length(self) -> f32 {
        self.length_sq().sqrt()
    }

    /// Unit vector, or the zero vector for (near-)zero inputs.
    #[inline]
    pub fn normalize(self) -> Self {
        let l = self.length();
        if l > 0.0001 {
            self / l
        } else {
            Vector2::new(0.0, 0.0)
        }
    }

    /// Clamp the vector's length to `max`, preserving direction.
    #[inline]
    pub fn truncate(self, max: f32) -> Self {
        let l = self.length();
        if l > max && l > 0.0 {
            (self / l) * max
        } else {
            self
        }
    }

    /// Distance to another point (spec alias of [`Vector2::distance_to`]).
    #[inline]
    pub fn distance(self, other: Vector2) -> f32 {
        (self - other).length()
    }

    /// Dot product.
    #[inline]
    pub fn dot(self, other: Vector2) -> f32 {
        self.x * other.x + self.y * other.y
    }

    #[inline]
    pub fn magnitude(&self) -> f32 {
        self.length()
    }

    #[inline]
    pub fn is_finite(&self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    /// Clamp to the pitch rectangle.
    #[inline]
    pub fn clamp_on_pitch(self) -> Self {
        Vector2 {
            x: self.x.clamp(-52.5, 52.5),
            y: self.y.clamp(-34.0, 34.0),
        }
    }
}

impl std::ops::Add for Vector2 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Vector2::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl std::ops::Sub for Vector2 {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Vector2::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl std::ops::Mul<f32> for Vector2 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: f32) -> Self {
        Vector2::new(self.x * rhs, self.y * rhs)
    }
}

impl std::ops::Div<f32> for Vector2 {
    type Output = Self;
    #[inline]
    fn div(self, rhs: f32) -> Self {
        Vector2::new(self.x / rhs, self.y / rhs)
    }
}

/// Possession phase determines formation shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamTacticalPhase {
    /// Attacking: formation widens and pushes upfield.
    InPossession,
    /// Defending: formation compresses and drops back.
    OutOfPossession,
}

impl TeamTacticalPhase {
    /// `(scale_x, scale_y)` applied to the canonical offsets.
    #[inline]
    pub fn scales(self) -> (f32, f32) {
        match self {
            TeamTacticalPhase::InPossession => (1.15, 1.20),
            TeamTacticalPhase::OutOfPossession => (0.85, 0.80),
        }
    }
}

/// A single player's dynamic tactical anchor.
#[derive(Debug, Clone)]
pub struct TacticalAnchor {
    pub player_id: String,
    pub shirt_number: u8,
    pub target_pos: Vector2,
    /// Goalkeepers keep a heavier weight (stay closer to goal).
    pub role_weight: f32,
}

// ── Formation Engine ────────────────────────

/// Elasticity coefficients for the centroid shift.
pub const CENTROID_KX: f32 = 0.45;
pub const CENTROID_KY: f32 = 0.35;

pub struct FormationEngine {
    base_templates: HashMap<String, Vec<Vector2>>,
    /// Precomputed outfield centroid per formation (avoids recomputing every tick).
    base_centroids: HashMap<String, Vector2>,
}

impl Default for FormationEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl FormationEngine {
    pub fn new() -> Self {
        let mut base_templates = HashMap::new();

        // 4-3-3 canonical slots (home half: x ∈ [-52.5, 0])
        let f433 = vec![
            Vector2::new(-47.0, 0.0),   // 1  GK
            Vector2::new(-34.0, -22.0), // 2  RB
            Vector2::new(-36.0, -8.0),  // 3  RCB
            Vector2::new(-36.0, 8.0),   // 4  LCB
            Vector2::new(-34.0, 22.0),  // 5  LB
            Vector2::new(-22.0, 0.0),   // 6  DM
            Vector2::new(-14.0, -14.0), // 7  RCM
            Vector2::new(-14.0, 14.0),  // 8  LCM
            Vector2::new(-5.0, -24.0),  // 9  RW
            Vector2::new(-2.0, 0.0),    // 10 ST
            Vector2::new(-5.0, 24.0),   // 11 LW
        ];

        // 4-2-3-1 canonical slots
        let f4231 = vec![
            Vector2::new(-47.0, 0.0),   // 1  GK
            Vector2::new(-34.0, -22.0), // 2  RB
            Vector2::new(-36.0, -8.0),  // 3  RCB
            Vector2::new(-36.0, 8.0),   // 4  LCB
            Vector2::new(-34.0, 22.0),  // 5  LB
            Vector2::new(-24.0, -9.0),  // 6  RDM
            Vector2::new(-24.0, 9.0),   // 7  LDM
            Vector2::new(-10.0, -22.0), // 8  RAM
            Vector2::new(-12.0, 0.0),   // 9  CAM
            Vector2::new(-10.0, 22.0),  // 10 LAM
            Vector2::new(-3.0, 0.0),    // 11 ST
        ];

        base_templates.insert("4-3-3".to_string(), f433);
        base_templates.insert("4-2-3-1".to_string(), f4231);

        // Precompute the outfield centroid (exclude the GK at slot 0).
        let base_centroids = base_templates
            .iter()
            .map(|(name, t)| {
                let outfield = &t[1..];
                let inv = 1.0 / outfield.len() as f32;
                let cx = outfield.iter().map(|p| p.x).sum::<f32>() * inv;
                let cy = outfield.iter().map(|p| p.y).sum::<f32>() * inv;
                (name.clone(), Vector2::new(cx, cy))
            })
            .collect();

        Self {
            base_templates,
            base_centroids,
        }
    }

    /// Owned copy of a named template (for engines that cache it once).
    pub fn template_vec(&self, formation: &str) -> Vec<Vector2> {
        self.template(formation).clone()
    }

    #[inline]
    fn template(&self, formation: &str) -> &Vec<Vector2> {
        self.base_templates
            .get(formation)
            .or_else(|| self.base_templates.get("4-3-3"))
            .expect("4-3-3 template always exists")
    }

    #[inline]
    fn centroid(&self, formation: &str) -> Vector2 {
        self.base_centroids
            .get(formation)
            .or_else(|| self.base_centroids.get("4-3-3"))
            .copied()
            .unwrap_or(Vector2::zero())
    }

    /// The outfield centroid of an arbitrary template (GK excluded).
    pub fn template_centroid(template: &[Vector2]) -> Vector2 {
        if template.len() <= 1 {
            return Vector2::zero();
        }
        let outfield = &template[1..];
        let inv = 1.0 / outfield.len() as f32;
        let cx = outfield.iter().map(|p| p.x).sum::<f32>() * inv;
        let cy = outfield.iter().map(|p| p.y).sum::<f32>() * inv;
        Vector2::new(cx, cy)
    }

    /// Build a canonical (home-frame) formation template directly from the
    /// starting XI's Opta position categories. This is the key robustness win:
    /// a 4-4-2 side is simulated as a 4-4-2 instead of being forced into a
    /// 4-3-3, which used to place defenders in midfield slots and forwards in
    /// defensive slots.
    pub fn dynamic_template(lineup: &[LineupPlayer]) -> Option<Vec<Vector2>> {
        let mut slots: Vec<(u8, u8)> = lineup
            .iter()
            .filter(|p| p.is_starter())
            .map(|p| (p.formation_slot, p.position_category))
            .collect();
        slots.sort_unstable_by_key(|(s, _)| *s);
        if slots.len() < 11 {
            return None;
        }
        // Every starter must occupy a distinct slot: a duplicate would stack
        // two players into one anchor and silently blank another, breaking
        // the formation (the from_context caller then falls back to the
        // default template).
        if slots.windows(2).any(|w| w[0].0 == w[1].0) {
            return None;
        }
        let mut gk = Vec::new();
        let mut def = Vec::new();
        let mut mid = Vec::new();
        let mut fwd = Vec::new();
        for (slot, cat) in &slots {
            match cat {
                1 => gk.push(*slot),
                2 => def.push(*slot),
                4 => fwd.push(*slot),
                _ => mid.push(*slot),
            }
        }
        let mut t = vec![Vector2::new(-18.0, 0.0); 11];
        for s in gk {
            t[(s - 1) as usize] = Vector2::new(-47.0, 0.0);
        }
        let def_y = spread(def.len(), -22.0, 22.0);
        for (i, s) in def.iter().enumerate() {
            t[(s - 1) as usize] = Vector2::new(-35.0, def_y[i]);
        }
        for (i, s) in mid.iter().enumerate() {
            t[(s - 1) as usize] = mid_slot(mid.len(), i);
        }
        let fwd_y = spread(fwd.len(), -18.0, 18.0);
        for (i, s) in fwd.iter().enumerate() {
            t[(s - 1) as usize] = Vector2::new(-3.0, fwd_y[i]);
        }
        Some(t)
    }

    /// A human-readable label for the detected shape (used to pick a style).
    pub fn detect_formation_label(lineup: &[LineupPlayer]) -> &'static str {
        let (mut d, mut m, mut f) = (0, 0, 0);
        for p in lineup.iter().filter(|p| p.is_starter()) {
            match p.position_category {
                2 => d += 1,
                3 => m += 1,
                4 => f += 1,
                _ => {}
            }
        }
        match (d, m, f) {
            (4, 4, 2) => "4-4-2",
            (4, 3, 3) => "4-3-3",
            (4, 5, 1) => "4-2-3-1",
            (3, 5, 2) => "3-5-2",
            (5, 3, 2) => "5-3-2",
            (4, 1, 1) => "4-1-4-1",
            _ => "4-3-3",
        }
    }

    /// Calculate dynamic anchors for all starting players of a team.
    ///
    /// Only players with formation slot 1–11 are considered; substitutes
    /// (slot 0) are ignored.
    pub fn calculate_team_anchors(
        &self,
        formation_name: &str,
        lineup: &[LineupPlayer],
        ball_pos: Vector2,
        phase: TeamTacticalPhase,
        direction: AttackDirection,
    ) -> Vec<TacticalAnchor> {
        let template = self.template(formation_name);
        let base_centroid = self.centroid(formation_name);
        self.calculate_team_anchors_from_template(
            template, base_centroid, lineup, ball_pos, phase, direction,
        )
    }

    /// Like [`FormationEngine::calculate_team_anchors`] but with an explicit
    /// template and centroid (used by the engine's detected formation).
    pub fn calculate_team_anchors_from_template(
        &self,
        template: &[Vector2],
        base_centroid: Vector2,
        lineup: &[LineupPlayer],
        ball_pos: Vector2,
        phase: TeamTacticalPhase,
        direction: AttackDirection,
    ) -> Vec<TacticalAnchor> {
        let (sx, sy) = phase.scales();

        // Express the ball in the team's own attacking frame before shifting:
        // the template (and its centroid) live in the "attacks toward +x"
        // frame, so an RTL team must have the ball mirrored first, otherwise
        // the shift pulls them the wrong way.
        let mirror = direction == AttackDirection::RightToLeft;
        let ball_local = if mirror {
            Vector2::new(-ball_pos.x, -ball_pos.y)
        } else {
            ball_pos
        };

        // Shifted centroid toward the ball.
        let target_cx = base_centroid.x + CENTROID_KX * (ball_local.x - base_centroid.x);
        let target_cy = base_centroid.y + CENTROID_KY * (ball_local.y - base_centroid.y);

        // Collect and order the starting XI. (Vec of references, 11 items.)
        let mut starting_xi: Vec<&LineupPlayer> =
            lineup.iter().filter(|p| p.is_starter()).collect();
        starting_xi.sort_unstable_by_key(|p| p.formation_slot);

        starting_xi
            .into_iter()
            .map(|player| {
                let slot_idx = (player.formation_slot - 1) as usize;
                let base_pos = template
                    .get(slot_idx)
                    .copied()
                    .unwrap_or(Vector2::new(-20.0, 0.0));

                let pos = if slot_idx == 0 {
                    // Goalkeeper: heavily tethered to the goal area.
                    let gk_x = base_pos.x + 0.12 * (ball_local.x - base_pos.x);
                    let gk_y = base_pos.y + 0.20 * ball_local.y;
                    Vector2::new(gk_x.clamp(-51.0, -40.0), gk_y.clamp(-9.0, 9.0))
                } else {
                    let rel_x = base_pos.x - base_centroid.x;
                    let rel_y = base_pos.y - base_centroid.y;
                    Vector2::new(
                        (target_cx + rel_x * sx).clamp(-51.5, 51.5),
                        (target_cy + rel_y * sy).clamp(-33.0, 33.0),
                    )
                };

                let pos = if mirror {
                    Vector2::new(-pos.x, -pos.y)
                } else {
                    pos
                };

                TacticalAnchor {
                    player_id: player.player_id.clone(),
                    shirt_number: player.shirt_number,
                    target_pos: pos,
                    role_weight: if slot_idx == 0 { 1.5 } else { 1.0 },
                }
            })
            .collect()
    }
}

// ── Dynamic template helpers ─────────────────

/// Evenly spread `n` values across `[lo, hi]`.
fn spread(n: usize, lo: f32, hi: f32) -> Vec<f32> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![(lo + hi) * 0.5];
    }
    (0..n)
        .map(|i| lo + (hi - lo) * i as f32 / (n - 1) as f32)
        .collect()
}

/// Canonical position for the `i`-th midfielder of `n`, in two banks so that
/// four- and five-midfield shapes read as a real midfield rather than a line.
fn mid_slot(n: usize, i: usize) -> Vector2 {
    match n {
        0 => Vector2::new(-18.0, 0.0),
        1 => Vector2::new(-16.0, 0.0),
        2 => Vector2::new(-22.0, if i == 0 { -9.0 } else { 9.0 }),
        3 => match i {
            0 => Vector2::new(-22.0, 0.0),
            1 => Vector2::new(-13.0, -15.0),
            _ => Vector2::new(-13.0, 15.0),
        },
        4 => match i {
            0 => Vector2::new(-22.0, -9.0),
            1 => Vector2::new(-22.0, 9.0),
            2 => Vector2::new(-10.0, -17.0),
            _ => Vector2::new(-10.0, 17.0),
        },
        _ => {
            if i < 2 {
                Vector2::new(-24.0, if i == 0 { -7.0 } else { 7.0 })
            } else {
                let k = i - 2;
                let m = n - 2;
                let y = if m <= 1 {
                    0.0
                } else {
                    -18.0 + 36.0 * k as f32 / (m - 1) as f32
                };
                Vector2::new(-9.0, y)
            }
        }
    }
}

// ── Tests ───────────────────────────────────

#[cfg(test)]
mod formation_tests {
    use super::*;

    fn mock_lineup() -> Vec<LineupPlayer> {
        (1..=11)
            .map(|slot| LineupPlayer {
                player_id: format!("player_{}", slot),
                position_category: if slot == 1 { 1 } else { 2 },
                shirt_number: slot,
                formation_slot: slot,
            })
            .collect()
    }

    #[test]
    fn test_anchor_count() {
        let engine = FormationEngine::new();
        let anchors = engine.calculate_team_anchors(
            "4-3-3",
            &mock_lineup(),
            Vector2::new(0.0, 0.0),
            TeamTacticalPhase::OutOfPossession,
            AttackDirection::LeftToRight,
        );
        assert_eq!(anchors.len(), 11);
    }

    #[test]
    fn test_anchors_within_pitch() {
        let engine = FormationEngine::new();
        let lineup = mock_lineup();
        for phase in [TeamTacticalPhase::InPossession, TeamTacticalPhase::OutOfPossession] {
            for direction in [AttackDirection::LeftToRight, AttackDirection::RightToLeft] {
                for ball in [
                    Vector2::new(-40.0, -20.0),
                    Vector2::new(0.0, 0.0),
                    Vector2::new(40.0, 20.0),
                    Vector2::new(-52.5, -34.0),
                    Vector2::new(52.5, 34.0),
                ] {
                    let anchors = engine.calculate_team_anchors(
                        "4-3-3",
                        &lineup,
                        ball,
                        phase,
                        direction,
                    );
                    for a in &anchors {
                        assert!(
                            a.target_pos.x >= -52.6 && a.target_pos.x <= 52.6,
                            "x {} out of bounds",
                            a.target_pos.x
                        );
                        assert!(
                            a.target_pos.y >= -34.1 && a.target_pos.y <= 34.1,
                            "y {} out of bounds",
                            a.target_pos.y
                        );
                        assert!(a.target_pos.is_finite());
                    }
                }
            }
        }
    }

    #[test]
    fn test_rtl_ball_follow_direction() {
        // Regression: for an RTL team the ball must be mirrored into the
        // team's attacking frame *before* the centroid shift. Previously the
        // shift was applied in world space then mirrored, so the ball deep in
        // a team's attacking half would drag their shape backwards.
        let engine = FormationEngine::new();
        let lineup = mock_lineup();
        let mean_x = |a: &[TacticalAnchor]| {
            a.iter().map(|x| x.target_pos.x).sum::<f32>() / a.len() as f32
        };

        // RTL team attacking toward −x: ball at −40 should push them forward
        // (more negative mean x) than ball at +40.
        let rtl_ball_fwd = engine.calculate_team_anchors(
            "4-3-3",
            &lineup,
            Vector2::new(-40.0, 0.0),
            TeamTacticalPhase::OutOfPossession,
            AttackDirection::RightToLeft,
        );
        let rtl_ball_back = engine.calculate_team_anchors(
            "4-3-3",
            &lineup,
            Vector2::new(40.0, 0.0),
            TeamTacticalPhase::OutOfPossession,
            AttackDirection::RightToLeft,
        );
        assert!(
            mean_x(&rtl_ball_fwd) < mean_x(&rtl_ball_back),
            "RTL shape must move toward an attacking ball: fwd={} back={}",
            mean_x(&rtl_ball_fwd),
            mean_x(&rtl_ball_back)
        );

        // LTR team attacking toward +x: mirror image.
        let ltr_ball_fwd = engine.calculate_team_anchors(
            "4-3-3",
            &lineup,
            Vector2::new(40.0, 0.0),
            TeamTacticalPhase::OutOfPossession,
            AttackDirection::LeftToRight,
        );
        let ltr_ball_back = engine.calculate_team_anchors(
            "4-3-3",
            &lineup,
            Vector2::new(-40.0, 0.0),
            TeamTacticalPhase::OutOfPossession,
            AttackDirection::LeftToRight,
        );
        assert!(mean_x(&ltr_ball_fwd) > mean_x(&ltr_ball_back));
    }

    #[test]
    fn test_rtl_and_ltr_are_mirror_images_for_mirrored_ball() {
        let engine = FormationEngine::new();
        let lineup = mock_lineup();
        let ltr = engine.calculate_team_anchors(
            "4-3-3",
            &lineup,
            Vector2::new(20.0, 10.0),
            TeamTacticalPhase::InPossession,
            AttackDirection::LeftToRight,
        );
        let rtl = engine.calculate_team_anchors(
            "4-3-3",
            &lineup,
            Vector2::new(-20.0, -10.0),
            TeamTacticalPhase::InPossession,
            AttackDirection::RightToLeft,
        );
        for (a, b) in ltr.iter().zip(rtl.iter()) {
            assert!((a.target_pos.x + b.target_pos.x).abs() < 1e-4);
            assert!((a.target_pos.y + b.target_pos.y).abs() < 1e-4);
        }
    }

    #[test]
    fn test_gk_stays_in_goal() {
        let engine = FormationEngine::new();
        let anchors = engine.calculate_team_anchors(
            "4-3-3",
            &mock_lineup(),
            Vector2::new(35.0, 15.0),
            TeamTacticalPhase::InPossession,
            AttackDirection::LeftToRight,
        );
        let gk = anchors.iter().find(|a| a.shirt_number == 1).unwrap();
        assert!(gk.target_pos.x <= -40.0);
        assert!(gk.target_pos.x >= -51.0);
        assert!(gk.target_pos.y >= -9.0 && gk.target_pos.y <= 9.0);
    }

    #[test]
    fn test_centroid_shifts_toward_ball() {
        let engine = FormationEngine::new();
        let lineup = mock_lineup();
        let def = engine.calculate_team_anchors(
            "4-3-3",
            &lineup,
            Vector2::new(-35.0, 0.0),
            TeamTacticalPhase::OutOfPossession,
            AttackDirection::LeftToRight,
        );
        let att = engine.calculate_team_anchors(
            "4-3-3",
            &lineup,
            Vector2::new(35.0, 0.0),
            TeamTacticalPhase::OutOfPossession,
            AttackDirection::LeftToRight,
        );
        let d = def.iter().find(|a| a.shirt_number == 3).unwrap();
        let a = att.iter().find(|a| a.shirt_number == 3).unwrap();
        assert!(a.target_pos.x > d.target_pos.x);
    }

    #[test]
    fn test_in_possession_wider_than_defending() {
        let engine = FormationEngine::new();
        let lineup = mock_lineup();
        let att = engine.calculate_team_anchors(
            "4-3-3",
            &lineup,
            Vector2::zero(),
            TeamTacticalPhase::InPossession,
            AttackDirection::LeftToRight,
        );
        let def = engine.calculate_team_anchors(
            "4-3-3",
            &lineup,
            Vector2::zero(),
            TeamTacticalPhase::OutOfPossession,
            AttackDirection::LeftToRight,
        );
        let att_rw = att.iter().find(|a| a.shirt_number == 9).unwrap();
        let def_rw = def.iter().find(|a| a.shirt_number == 9).unwrap();
        assert!(att_rw.target_pos.x > def_rw.target_pos.x);
        let att_lw = att.iter().find(|a| a.shirt_number == 11).unwrap();
        let def_lw = def.iter().find(|a| a.shirt_number == 11).unwrap();
        assert!(att_lw.target_pos.x > def_lw.target_pos.x);
    }

    #[test]
    fn test_rtl_mirrors_coordinates() {
        // A team's anchors for a ball at `p` must equal the mirrored anchors of
        // the opposite direction for a ball at `-p`.
        let engine = FormationEngine::new();
        let lineup = mock_lineup();
        let ltr = engine.calculate_team_anchors(
            "4-3-3",
            &lineup,
            Vector2::new(20.0, 10.0),
            TeamTacticalPhase::InPossession,
            AttackDirection::LeftToRight,
        );
        let rtl = engine.calculate_team_anchors(
            "4-3-3",
            &lineup,
            Vector2::new(-20.0, -10.0),
            TeamTacticalPhase::InPossession,
            AttackDirection::RightToLeft,
        );
        for (a, b) in ltr.iter().zip(rtl.iter()) {
            assert!((a.target_pos.x + b.target_pos.x).abs() < 1e-4);
            assert!((a.target_pos.y + b.target_pos.y).abs() < 1e-4);
        }
    }

    #[test]
    fn test_fallback_to_433() {
        let engine = FormationEngine::new();
        let anchors = engine.calculate_team_anchors(
            "unknown-formation",
            &mock_lineup(),
            Vector2::zero(),
            TeamTacticalPhase::OutOfPossession,
            AttackDirection::LeftToRight,
        );
        assert_eq!(anchors.len(), 11);
    }

    #[test]
    fn test_rtl_defensive_players_on_right() {
        // With the ball deep in the RTL team's own half (physical +x), the
        // whole shape stays on the right-hand side of the pitch.
        let engine = FormationEngine::new();
        let def = engine.calculate_team_anchors(
            "4-3-3",
            &mock_lineup(),
            Vector2::new(40.0, 0.0),
            TeamTacticalPhase::OutOfPossession,
            AttackDirection::RightToLeft,
        );
        let mean_x: f32 = def.iter().map(|a| a.target_pos.x).sum::<f32>() / def.len() as f32;
        assert!(mean_x > 0.0, "RTL shape not on the right: mean_x={mean_x}");
        // The goalkeeper in particular must remain in the right-hand goal.
        let gk = def.iter().find(|a| a.shirt_number == 1).unwrap();
        assert!(gk.target_pos.x > 0.0);
    }

    #[test]
    fn test_keeper_has_highest_role_weight() {
        let engine = FormationEngine::new();
        let anchors = engine.calculate_team_anchors(
            "4-3-3",
            &mock_lineup(),
            Vector2::zero(),
            TeamTacticalPhase::InPossession,
            AttackDirection::LeftToRight,
        );
        for a in &anchors {
            if a.shirt_number == 1 {
                assert_eq!(a.role_weight, 1.5);
            } else {
                assert_eq!(a.role_weight, 1.0);
            }
        }
    }

    #[test]
    fn test_formation_template_count() {
        let engine = FormationEngine::new();
        assert_eq!(engine.base_templates.get("4-3-3").unwrap().len(), 11);
        assert_eq!(engine.base_templates.get("4-2-3-1").unwrap().len(), 11);
    }

    #[test]
    fn test_precomputed_centroids_match_manual() {
        let engine = FormationEngine::new();
        let t = engine.base_templates.get("4-3-3").unwrap();
        let out = &t[1..];
        let cx = out.iter().map(|p| p.x).sum::<f32>() / out.len() as f32;
        let cy = out.iter().map(|p| p.y).sum::<f32>() / out.len() as f32;
        let c = engine.centroid("4-3-3");
        assert!((c.x - cx).abs() < 1e-4);
        assert!((c.y - cy).abs() < 1e-4);
    }

    #[test]
    fn test_subs_are_ignored() {
        let engine = FormationEngine::new();
        let mut lineup = mock_lineup();
        lineup.push(LineupPlayer {
            player_id: "sub".into(),
            position_category: 5,
            shirt_number: 99,
            formation_slot: 0,
        });
        let anchors = engine.calculate_team_anchors(
            "4-3-3",
            &lineup,
            Vector2::zero(),
            TeamTacticalPhase::OutOfPossession,
            AttackDirection::LeftToRight,
        );
        assert_eq!(anchors.len(), 11);
        assert!(anchors.iter().all(|a| a.player_id != "sub"));
    }

    #[test]
    fn test_empty_lineup() {
        let engine = FormationEngine::new();
        let anchors = engine.calculate_team_anchors(
            "4-3-3",
            &[],
            Vector2::zero(),
            TeamTacticalPhase::OutOfPossession,
            AttackDirection::LeftToRight,
        );
        assert!(anchors.is_empty());
    }

    // ── Dynamic (data-driven) formation detection ──────────────

    fn lineup_with(cats: &[u8]) -> Vec<LineupPlayer> {
        cats.iter()
            .enumerate()
            .map(|(i, c)| LineupPlayer {
                player_id: format!("p{i}"),
                position_category: *c,
                shirt_number: (i + 1) as u8,
                formation_slot: (i + 1) as u8,
            })
            .collect()
    }

    #[test]
    fn test_detect_formation_label() {
        assert_eq!(
            FormationEngine::detect_formation_label(&lineup_with(&[1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4])),
            "4-4-2"
        );
        assert_eq!(
            FormationEngine::detect_formation_label(&lineup_with(&[1, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4])),
            "4-3-3"
        );
        assert_eq!(
            FormationEngine::detect_formation_label(&lineup_with(&[1, 2, 2, 2, 3, 3, 3, 3, 3, 4, 4])),
            "3-5-2"
        );
        assert_eq!(
            FormationEngine::detect_formation_label(&lineup_with(&[1, 2, 2, 2, 2, 3, 3, 3, 3, 3, 4])),
            "4-2-3-1"
        );
    }

    #[test]
    fn test_dynamic_template_truthfully_places_lines() {
        // A real 4-4-2 must not be forced into the 4-3-3 template: four
        // defenders share one line, the forwards share another.
        let xi = lineup_with(&[1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4]);
        let t = FormationEngine::dynamic_template(&xi).expect("template");
        for i in [1usize, 2, 3, 4] {
            assert!((t[i].x - -35.0).abs() < 1e-4, "defender {i} x={}", t[i].x);
        }
        for i in [9usize, 10] {
            assert!((t[i].x - -3.0).abs() < 1e-4, "forward {i} x={}", t[i].x);
        }
        // Defensive line spans the width.
        assert!((t[1].y - -22.0).abs() < 1e-4);
        assert!((t[4].y - 22.0).abs() < 1e-4);
        // Goalkeeper stays on the goal line.
        assert!(t[0].x <= -46.0);
    }

    #[test]
    fn test_dynamic_template_requires_eleven() {
        let short = lineup_with(&[1, 2, 2, 2, 2, 3, 3, 3, 3, 4]);
        assert!(FormationEngine::dynamic_template(&short).is_none());
    }

    #[test]
    fn test_formation_slots_unique() {
        // Two starters sharing one slot would stack both players on a single
        // anchor — refuse to build a template from that lineup.
        let mut xi = lineup_with(&[1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4]);
        xi[10].formation_slot = xi[9].formation_slot;
        assert!(FormationEngine::dynamic_template(&xi).is_none());
        // Distinct slots (the normal case) still produce a template.
        let ok = lineup_with(&[1, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4]);
        assert!(FormationEngine::dynamic_template(&ok).is_some());
    }

    #[test]
    fn test_dynamic_template_preserves_slot_mapping() {
        // Categories are not in template order (defender listed at slot 4).
        let xi = lineup_with(&[1, 3, 3, 2, 2, 2, 2, 3, 3, 4, 4]);
        let t = FormationEngine::dynamic_template(&xi).unwrap();
        // Slot 4 (index 3) is a defender → defender line.
        assert!((t[3].x - -35.0).abs() < 1e-4);
        // Slot 2 (index 1) is a midfielder → not the defender line.
        assert!(t[1].x > -35.0 + 1.0);
    }

    #[test]
    fn test_anchors_from_template_respect_direction() {
        let engine = FormationEngine::new();
        let xi = lineup_with(&[1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4]);
        let t = FormationEngine::dynamic_template(&xi).unwrap();
        let c = FormationEngine::template_centroid(&t);
        let ltr = engine.calculate_team_anchors_from_template(
            &t, c, &xi, Vector2::zero(), TeamTacticalPhase::OutOfPossession, AttackDirection::LeftToRight,
        );
        let rtl = engine.calculate_team_anchors_from_template(
            &t, c, &xi, Vector2::zero(), TeamTacticalPhase::OutOfPossession, AttackDirection::RightToLeft,
        );
        for (a, b) in ltr.iter().zip(rtl.iter()) {
            assert!((a.target_pos.x + b.target_pos.x).abs() < 1e-4);
            assert!((a.target_pos.y + b.target_pos.y).abs() < 1e-4);
        }
    }
}
