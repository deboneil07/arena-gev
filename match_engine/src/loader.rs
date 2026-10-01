//! Fast XML ingestion using quick-xml streaming reader.
//!
//! Parses Opta F24 XML into a [`MatchContext`] with normalized coordinates,
//! attack directions, lineups, and gameplay events only.
//!
//! ## Design notes
//! * Attributes are parsed in a **single pass** per element, so attribute
//!   order in the XML never matters and no intermediate `String`s are
//!   allocated for numeric fields.
//! * Attribute values are XML-unescaped (`&apos;` → `'`, …) so player names
//!   such as `C. O'Hare` round-trip correctly.
//! * Event ids are 64-bit — real Opta ids are ~10 digits and overflow `u16`.

use crate::parser::*;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::HashMap;

// ── Public API ────────────────────────────────────────────

/// Parse an Opta F24 XML string into a fully normalized [`MatchContext`].
pub fn parse_opta_f24(xml_content: &str) -> Result<MatchContext, String> {
    let mut reader = Reader::from_str(xml_content);
    reader.config_mut().trim_text(true);

    let mut ctx = ParseContext::default();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => handle_element(&e, &mut ctx),
            Ok(Event::Empty(e)) => {
                handle_element(&e, &mut ctx);
                // Self-closing <Event .../> has no matching End event.
                if e.name().as_ref() == b"Event" {
                    finalize_event(&mut ctx);
                }
            }
            Ok(Event::End(e)) => {
                if e.name().as_ref() == b"Event" {
                    finalize_event(&mut ctx);
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(format!(
                    "XML parse error at byte {}: {}",
                    reader.buffer_position(),
                    e
                ))
            }
            _ => {}
        }
        buf.clear();
    }

    drop_twin_dead_ball_rows(&mut ctx.events);
    stagger_same_second_events(&mut ctx.events);

    Ok(ctx.build())
}

/// Opta records every ball-out row twice: once per team, each expressed in
/// that team's attacking frame, so both copies canonicalize onto the same
/// spot at the same second. Only one copy names the player who actually
/// touched the ball out — the man standing on the spot — while the twin
/// credits a player who may be half a pitch away and can never get there in
/// time. Dispatching both also drags the ball back to the exit point for a
/// restart that has already been placed, wiping out the roll to the corner
/// flag.
///
/// Keep the toucher's row (the feasible copy) and drop the twin, but hand the
/// survivor the twin's team: the restart belongs to the opponents of whoever
/// put the ball out, even though the doer is the man who touched it.
fn drop_twin_dead_ball_rows(events: &mut Vec<NormalizedEvent>) {
    /// Two rows in the same second describe the same incident.
    const PAIR_WINDOW_SECS: f32 = 0.5;
    /// …and this close in space (canonical frame).
    const PAIR_SPOT_TOL: f32 = 1.5;
    /// A touch older than this never belonged to this stoppage.
    const TOUCH_LOOKBACK_SECS: f32 = 30.0;

    // Index of the most recent ball touch before each row.
    let mut last_touch: Vec<Option<usize>> = Vec::with_capacity(events.len());
    let mut touch: Option<usize> = None;
    for (i, ev) in events.iter().enumerate() {
        if is_ball_event(ev.type_id) && !ev.player_id.is_empty() {
            touch = Some(i);
        }
        last_touch.push(touch);
    }

    let mut drop = vec![false; events.len()];
    for i in 0..events.len() {
        if drop[i] || !is_dead_ball_event(events[i].type_id) {
            continue;
        }
        let Some(j) = (i + 1..events.len()).find(|&j| {
            !drop[j]
                && events[j].type_id == events[i].type_id
                && (events[j].match_time_secs - events[i].match_time_secs).abs() <= PAIR_WINDOW_SECS
                && (events[j].origin_x - events[i].origin_x).abs() <= PAIR_SPOT_TOL
                && (events[j].origin_y - events[i].origin_y).abs() <= PAIR_SPOT_TOL
        }) else {
            continue;
        };
        let Some(t) = last_touch[i].filter(|&t| {
            events[t].match_time_secs + TOUCH_LOOKBACK_SECS >= events[i].match_time_secs
        }) else {
            continue;
        };
        // The twin's team is the side awarded the restart, so hand it to the
        // surviving row: the event keeps the toucher's identity (he is the
        // one standing on the spot) but belongs to the team that takes it.
        if events[t].player_id == events[i].player_id {
            events[i].team_id = events[j].team_id.clone();
            drop[j] = true;
        } else if events[t].player_id == events[j].player_id {
            events[j].team_id = events[i].team_id.clone();
            drop[i] = true;
        }
    }
    if drop.iter().any(|&d| d) {
        let mut row = 0;
        events.retain(|_| {
            let keep = !drop[row];
            row += 1;
            keep
        });
    }
}

/// Opta stamps events with whole seconds, so anything recorded inside one
/// second — a duel, then the clearance that follows it, then the reception
/// several metres away — claims the very same instant. The ball cannot be in
/// two places at once, so those events would all have to fire on the same
/// tick and every one but the first would see the ball (and often the doer)
/// somewhere else entirely.
///
/// Give each event inside a group its own slot within the second, keeping
/// feed order. The offsets stay below 1s, so the displayed mm:ss timestamp
/// is unchanged and the feed remains chronological.
fn stagger_same_second_events(events: &mut [NormalizedEvent]) {
    /// Spread of a group inside its second; never reaches the next second.
    const SPAN: f32 = 0.9;
    let mut i = 0;
    while i < events.len() {
        let t = events[i].match_time_secs;
        let mut j = i;
        while j < events.len() && events[j].match_time_secs == t {
            j += 1;
        }
        let n = j - i;
        if n > 1 {
            let step = SPAN / n as f32;
            for (k, ev) in events[i..j].iter_mut().enumerate() {
                ev.match_time_secs = t + step * (k as f32 + 0.5);
            }
        }
        i = j;
    }
}

// ── Parse Context (accumulator) ────────────────────────────

#[derive(Default)]
struct ParseContext {
    // Match metadata
    match_id: String,
    home_team_id: String,
    away_team_id: String,
    home_team_name: String,
    away_team_name: String,
    competition_id: String,
    season: String,
    game_date: String,

    physical_directions: HashMap<String, AttackDirection>,
    lineups: HashMap<String, Vec<LineupPlayer>>,
    events: Vec<NormalizedEvent>,

    // Current event accumulator
    event_id: u64,
    event_type_id: u16,
    event_period_id: u8,
    event_min: f32,
    event_sec: f32,
    event_team_id: String,
    event_player_id: String,
    event_player_name: String,
    event_x: f32,
    event_y: f32,
    event_outcome: bool,

    // Per-event qualifier accumulators
    q_target_x: Option<f32>,
    q_target_y: Option<f32>,
    q_chipped: bool,
    q_own_goal: bool,
    q30_players: Vec<String>,
    q44_positions: Vec<u8>,
    q59_shirts: Vec<u8>,
    q131_slots: Vec<u8>,
}

impl ParseContext {
    /// Reset all per-event fields before an `<Event>` tag is read.
    fn reset_event(&mut self) {
        self.event_id = 0;
        self.event_type_id = 0;
        self.event_period_id = 0;
        self.event_min = 0.0;
        self.event_sec = 0.0;
        self.event_team_id.clear();
        self.event_player_id.clear();
        self.event_player_name.clear();
        self.event_x = 0.0;
        self.event_y = 0.0;
        self.event_outcome = false;

        self.q_target_x = None;
        self.q_target_y = None;
        self.q_chipped = false;
        self.q_own_goal = false;
        self.q30_players.clear();
        self.q44_positions.clear();
        self.q59_shirts.clear();
        self.q131_slots.clear();
    }

    fn build(self) -> MatchContext {
        // Canonical frame: home attacks +x, away attacks −x. Stable across
        // half-time swaps (raw feed directions are kept separately).
        let mut directions = HashMap::new();
        if !self.home_team_id.is_empty() {
            directions.insert(self.home_team_id.clone(), AttackDirection::LeftToRight);
        }
        if !self.away_team_id.is_empty() {
            directions.insert(self.away_team_id.clone(), AttackDirection::RightToLeft);
        }
        MatchContext {
            match_id: self.match_id,
            home_team_id: self.home_team_id,
            away_team_id: self.away_team_id,
            home_team_name: self.home_team_name,
            away_team_name: self.away_team_name,
            competition_id: self.competition_id,
            season: self.season,
            game_date: self.game_date,
            lineups: self.lineups,
            directions,
            physical_directions: self.physical_directions,
            events: self.events,
        }
    }
}

// ── Attribute parsing helpers ───────────────────────────────

/// Unescaped attribute value as an owned `String`.
///
/// Falls back to a lossy UTF-8 decode if the entity references are malformed,
/// so a single bad attribute never silently empties a field (or aborts the
/// whole parse).
#[inline]
fn attr_str(attr: &quick_xml::events::attributes::Attribute<'_>) -> String {
    match attr.unescape_value() {
        Ok(cow) => cow.into_owned(),
        Err(_) => String::from_utf8_lossy(&attr.value).into_owned(),
    }
}

#[inline]
fn parse_u64(s: &str) -> u64 {
    s.trim().parse().unwrap_or(0)
}
#[inline]
fn parse_u16(s: &str) -> u16 {
    s.trim().parse().unwrap_or(0)
}
#[inline]
fn parse_u8(s: &str) -> u8 {
    s.trim().parse().unwrap_or(0)
}
#[inline]
fn parse_f32(s: &str) -> f32 {
    s.trim().parse().unwrap_or(0.0)
}

// ── Element dispatch ────────────────────────────────────────

fn handle_element(e: &BytesStart<'_>, ctx: &mut ParseContext) {
    match e.name().as_ref() {
        b"Game" => handle_game(e, ctx),
        b"Event" => handle_event_start(e, ctx),
        b"Q" => handle_qualifier(e, ctx),
        _ => {}
    }
}

fn handle_game(e: &BytesStart<'_>, ctx: &mut ParseContext) {
    for attr in e.attributes().flatten() {
        let v = attr_str(&attr);
        match attr.key.as_ref() {
            b"id" | b"game_id" => {
                if ctx.match_id.is_empty() {
                    ctx.match_id = v;
                }
            }
            b"home_team_id" => ctx.home_team_id = v,
            b"away_team_id" => ctx.away_team_id = v,
            b"home_team_name" => ctx.home_team_name = v,
            b"away_team_name" => ctx.away_team_name = v,
            b"competition_id" => ctx.competition_id = v,
            b"season" => ctx.season = v,
            b"game_date" => ctx.game_date = v,
            _ => {}
        }
    }
}

fn handle_event_start(e: &BytesStart<'_>, ctx: &mut ParseContext) {
    ctx.reset_event();
    for attr in e.attributes().flatten() {
        let v = attr_str(&attr);
        match attr.key.as_ref() {
            b"id" => ctx.event_id = parse_u64(&v),
            b"type_id" => ctx.event_type_id = parse_u16(&v),
            b"period_id" => ctx.event_period_id = parse_u8(&v),
            b"min" => ctx.event_min = parse_f32(&v),
            b"sec" => ctx.event_sec = parse_f32(&v),
            b"team_id" => ctx.event_team_id = v,
            b"player_id" => ctx.event_player_id = v,
            b"player_name" => ctx.event_player_name = v,
            b"x" => ctx.event_x = parse_f32(&v),
            b"y" => ctx.event_y = parse_f32(&v),
            b"outcome" => ctx.event_outcome = v == "1",
            _ => {}
        }
    }
}

fn handle_qualifier(e: &BytesStart<'_>, ctx: &mut ParseContext) {
    let mut q_id: u16 = 0;
    let mut q_val = String::new();
    for attr in e.attributes().flatten() {
        let v = attr_str(&attr);
        match attr.key.as_ref() {
            b"qualifier_id" => q_id = parse_u16(&v),
            b"value" => q_val = v,
            _ => {}
        }
    }

    match q_id {
        QUAL_TARGET_X => ctx.q_target_x = q_val.trim().parse::<f32>().ok(),
        QUAL_TARGET_Y => ctx.q_target_y = q_val.trim().parse::<f32>().ok(),
        QUAL_CHIPPED => ctx.q_chipped = true,
        QUAL_OWN_GOAL => ctx.q_own_goal = true,
        QUAL_OWN_GOAL_FLAG => {
            if q_val.trim().eq_ignore_ascii_case("OWN_GOAL") {
                ctx.q_own_goal = true;
            }
        }
        QUAL_DIR => {
            if let Some(dir) = AttackDirection::from_opta(&q_val) {
                // Raw/physical side, overwritten at half-time by the feed. Kept
                // for reference only; coordinates use the canonical frame.
                ctx.physical_directions.insert(ctx.event_team_id.clone(), dir);
            }
        }
        QUAL_LINEUP_PLAYERS => {
            ctx.q30_players = q_val.split(',').map(|s| s.trim().to_string()).collect();
        }
        QUAL_LINEUP_POSITIONS => {
            ctx.q44_positions = q_val.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        }
        QUAL_LINEUP_SHIRTS => {
            ctx.q59_shirts = q_val.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        }
        QUAL_LINEUP_SLOTS => {
            ctx.q131_slots = q_val.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        }
        _ => {}
    }
}

fn finalize_event(ctx: &mut ParseContext) {
    // 1. Lineups (team setup) are captured *before* meta filtering so the
    //    roster survives. Previously this was unreachable dead code.
    if ctx.event_type_id == EVENT_TYPE_TEAM_SETUP && !ctx.q30_players.is_empty() {
        let roster = build_roster(
            &ctx.q30_players,
            &ctx.q44_positions,
            &ctx.q59_shirts,
            &ctx.q131_slots,
        );
        ctx.lineups.insert(ctx.event_team_id.clone(), roster);
    }

    // 2. Drop pre-match and meta / non-positional events.
    if ctx.event_period_id == PERIOD_PREMATCH || !is_gameplay_event(ctx.event_type_id) {
        return;
    }

    // 3. Normalize coordinates into the canonical frame (home attacks +x).
    //    The feed defines x in the *attacking direction* of the active team,
    //    so mapping home→LTR and away→RTL yields one stable frame for the
    //    whole match, unaffected by the half-time side swap.
    let dir = canonical_direction(&ctx.event_team_id, &ctx.home_team_id, &ctx.away_team_id);
    let (origin_x, origin_y) = MatchContext::normalize_coord(ctx.event_x, ctx.event_y, dir);

    let (target_x, target_y) = match (ctx.q_target_x, ctx.q_target_y) {
        (Some(qx), Some(qy)) => {
            let (tx, ty) = MatchContext::normalize_coord(qx, qy, dir);
            (Some(tx), Some(ty))
        }
        _ => (None, None),
    };

    ctx.events.push(NormalizedEvent {
        id: ctx.event_id,
        type_id: ctx.event_type_id,
        period_id: ctx.event_period_id,
        match_time_secs: ctx.event_min * 60.0 + ctx.event_sec,
        team_id: ctx.event_team_id.clone(),
        player_id: ctx.event_player_id.clone(),
        player_name: ctx.event_player_name.clone(),
        origin_x,
        origin_y,
        target_x,
        target_y,
        outcome: ctx.event_outcome,
        // Aerial = aerial duel (type 44) or a chipped (in-air) pass.
        is_aerial: ctx.event_type_id == EVENT_TYPE_AERIAL || ctx.q_chipped,
        // Own goal: only meaningful on a Goal row.
        is_own_goal: ctx.q_own_goal && ctx.event_type_id == EVENT_TYPE_GOAL,
    });
}

/// Canonical frame direction: the away team attacks −x, everyone else +x.
#[inline]
fn canonical_direction(team_id: &str, _home_id: &str, away_id: &str) -> AttackDirection {
    if !team_id.is_empty() && team_id == away_id {
        AttackDirection::RightToLeft
    } else {
        AttackDirection::LeftToRight
    }
}

fn build_roster(
    players: &[String],
    positions: &[u8],
    shirts: &[u8],
    slots: &[u8],
) -> Vec<LineupPlayer> {
    players
        .iter()
        .enumerate()
        .map(|(i, pid)| LineupPlayer {
            player_id: pid.clone(),
            position_category: positions.get(i).copied().unwrap_or(0),
            shirt_number: shirts.get(i).copied().unwrap_or(0),
            formation_slot: slots.get(i).copied().unwrap_or(0),
        })
        .collect()
}

// ── Unit Tests ──────────────────────────────────────────────

#[cfg(test)]
mod loader_tests {
    use super::*;

    fn sample_xml() -> &'static str {
        r#"<?xml version="1.0"?>
<Games>
  <Game id="1" game_id="1" home_team_id="home1" away_team_id="away1"
        home_team_name="Home &amp; Away" away_team_name="Away" competition_id="10" season="2026" game_date="2026-01-01">
    <Event id="1234567890" type_id="34" period_id="16" min="0" sec="0" team_id="home1" player_id="" player_name="" x="0" y="0" outcome="1">
      <Q qualifier_id="30" value="p1, p2, p3, p4, p5, p6, p7, p8, p9, p10, p11, sub1"/>
      <Q qualifier_id="44" value="1, 2, 2, 3, 2, 2, 3, 3, 4, 4, 3, 5"/>
      <Q qualifier_id="59" value="1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12"/>
      <Q qualifier_id="131" value="1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 0"/>
    </Event>
    <Event id="1234567891" type_id="32" period_id="1" min="0" sec="0" team_id="home1" player_id="" player_name="" x="0" y="0" outcome="1">
      <Q qualifier_id="127" value="Left to Right"/>
    </Event>
    <Event id="1234567892" type_id="32" period_id="1" min="0" sec="0" team_id="away1" player_id="" player_name="" x="0" y="0" outcome="1">
      <Q qualifier_id="127" value="Right to Left"/>
    </Event>
    <Event id="1234567900" type_id="1" period_id="1" min="5" sec="30" team_id="home1" player_id="p1" player_name="Player 1" x="50" y="50" outcome="1">
      <Q qualifier_id="140" value="70"/>
      <Q qualifier_id="141" value="60"/>
    </Event>
    <Event id="1234567901" type_id="1" period_id="1" min="6" sec="0" team_id="away1" player_id="p2" player_name="C. O&apos;Hare" x="80" y="80" outcome="1">
      <Q qualifier_id="155" value=""/>
      <Q qualifier_id="140" value="20"/>
      <Q qualifier_id="141" value="20"/>
    </Event>
    <Event id="1234567902" type_id="16" period_id="1" min="10" sec="15" team_id="home1" player_id="p1" player_name="Player 1" x="90" y="50" outcome="1">
      <Q qualifier_id="140" value="95"/>
      <Q qualifier_id="141" value="50"/>
    </Event>
    <Event id="1234567903" type_id="2" period_id="1" min="12" sec="0" team_id="home1" player_id="p1" player_name="Player 1" x="85" y="45" outcome="0"/>
    <Event id="1234567904" type_id="43" period_id="1" min="13" sec="0" team_id="home1" player_id="p1" player_name="Player 1" x="0" y="0" outcome="1"/>
    <Event id="1234567905" type_id="30" period_id="1" min="45" sec="0" team_id="home1" player_id="" player_name="" x="0" y="0" outcome="1"/>
  </Game>
</Games>"#
    }

    #[test]
    fn test_parse_has_gameplay_events() {
        let ctx = parse_opta_f24(sample_xml()).expect("parse ok");
        assert!(!ctx.events.is_empty());
    }

    #[test]
    fn test_parse_skips_meta_events() {
        let ctx = parse_opta_f24(sample_xml()).expect("parse ok");
        for ev in &ctx.events {
            assert!(
                is_gameplay_event(ev.type_id),
                "meta event leaked: type_id={}",
                ev.type_id
            );
            assert_ne!(ev.period_id, PERIOD_PREMATCH);
        }
    }

    #[test]
    fn test_parse_skips_deleted_events() {
        let ctx = parse_opta_f24(sample_xml()).expect("parse ok");
        assert!(
            ctx.events.iter().all(|e| e.type_id != EVENT_TYPE_DELETED),
            "deleted-event marker (type 43) must be excluded"
        );
    }

    #[test]
    fn test_goal_event_is_retained() {
        // Regression: type 16 is Goal and must NOT be filtered as "match end".
        let ctx = parse_opta_f24(sample_xml()).expect("parse ok");
        let goal = ctx
            .events
            .iter()
            .find(|e| e.id == 1234567902)
            .expect("goal event must be present");
        assert_eq!(goal.type_id, EVENT_TYPE_GOAL);
        assert!(goal.outcome);
        assert!((goal.match_time_secs - 615.0).abs() < 0.01);
    }

    #[test]
    fn test_event_ids_are_not_truncated() {
        let ctx = parse_opta_f24(sample_xml()).expect("parse ok");
        let ids: Vec<u64> = ctx.events.iter().map(|e| e.id).collect();
        assert!(ids.contains(&1234567900), "64-bit id lost: {:?}", ids);
        assert!(ids.contains(&1234567902));
        assert!(ids.iter().all(|&id| id != 0), "ids must be non-zero");
    }

    #[test]
    fn test_lineups_are_built() {
        let ctx = parse_opta_f24(sample_xml()).expect("parse ok");
        let home = ctx.lineups.get("home1").expect("home lineup missing");
        assert_eq!(home.len(), 12, "expected 11 starters + 1 sub");
        assert_eq!(home[0].player_id, "p1");
        assert_eq!(home[0].formation_slot, 1);
        assert_eq!(home[11].formation_slot, 0);
        assert_eq!(home[11].position_category, 5);
    }

    #[test]
    fn test_starting_xi_helper() {
        let ctx = parse_opta_f24(sample_xml()).expect("parse ok");
        let xi = ctx.starting_xi("home1");
        assert_eq!(xi.len(), 11);
        assert!(xi.iter().all(|p| p.is_starter()));
        assert_eq!(xi[0].player_id, "p1");
    }

    #[test]
    fn test_xml_entities_are_unescaped() {
        let ctx = parse_opta_f24(sample_xml()).expect("parse ok");
        let ev = ctx.events.iter().find(|e| e.id == 1234567901).unwrap();
        assert_eq!(ev.player_name, "C. O'Hare", "entities must be decoded");
        assert_eq!(ctx.home_team_name, "Home & Away");
    }

    #[test]
    fn test_half_time_direction_swap_is_canonicalised() {
        // The feed flips direction at half-time. A home event with the same
        // attacking-frame x in period 1 and period 2 must land at the *same*
        // canonical coordinate, and the stored direction must stay canonical.
        let xml = r#"<Games><Game id="1" home_team_id="h" away_team_id="a" home_team_name="H" away_team_name="A" competition_id="10" season="2026" game_date="2026-01-01">
            <Event id="1" type_id="32" period_id="1" min="0" sec="0" team_id="h"><Q qualifier_id="127" value="Left to Right"/></Event>
            <Event id="2" type_id="32" period_id="1" min="0" sec="0" team_id="a"><Q qualifier_id="127" value="Right to Left"/></Event>
            <Event id="10" type_id="1" period_id="1" min="1" sec="0" team_id="h" player_id="p" player_name="P" x="20" y="50" outcome="1"/>
            <Event id="3" type_id="32" period_id="2" min="45" sec="0" team_id="h"><Q qualifier_id="127" value="Right to Left"/></Event>
            <Event id="4" type_id="32" period_id="2" min="45" sec="0" team_id="a"><Q qualifier_id="127" value="Left to Right"/></Event>
            <Event id="11" type_id="1" period_id="2" min="46" sec="0" team_id="h" player_id="p" player_name="P" x="20" y="50" outcome="1"/>
        </Game></Games>"#;
        let ctx = parse_opta_f24(xml).expect("parse ok");

        // Gameplay events only (period-start events are filtered).
        assert_eq!(ctx.events.len(), 2);
        let p1 = ctx.events.iter().find(|e| e.id == 10).unwrap();
        let p2 = ctx.events.iter().find(|e| e.id == 11).unwrap();
        // Same canonical origin for both halves (x=20 -> -31.5 m).
        assert!((p1.origin_x - (-31.5)).abs() < 0.01, "p1={}", p1.origin_x);
        assert!((p2.origin_x - (-31.5)).abs() < 0.01, "p2={}", p2.origin_x);

        // Stored directions are canonical (home LTR, away RTL) ...
        assert_eq!(ctx.directions.get("h"), Some(&AttackDirection::LeftToRight));
        assert_eq!(ctx.directions.get("a"), Some(&AttackDirection::RightToLeft));
        // ... while the raw feed directions reflect the period-2 swap.
        assert_eq!(ctx.physical_directions.get("h"), Some(&AttackDirection::RightToLeft));
        assert_eq!(ctx.physical_directions.get("a"), Some(&AttackDirection::LeftToRight));
    }

    #[test]
    fn test_parse_direction() {
        let ctx = parse_opta_f24(sample_xml()).expect("parse ok");
        assert_eq!(
            ctx.directions.get("home1"),
            Some(&AttackDirection::LeftToRight)
        );
        assert_eq!(
            ctx.directions.get("away1"),
            Some(&AttackDirection::RightToLeft)
        );
    }

    #[test]
    fn test_parse_ground_pass_coords() {
        let ctx = parse_opta_f24(sample_xml()).expect("parse ok");
        let pass = ctx.events.iter().find(|e| e.id == 1234567900).expect("pass");
        assert!((pass.origin_x - 0.0).abs() < 0.01);
        assert!((pass.origin_y - 0.0).abs() < 0.01);
        assert!((pass.target_x.unwrap() - 21.0).abs() < 0.01);
        assert!((pass.target_y.unwrap() - 6.8).abs() < 0.01);
    }

    #[test]
    fn test_parse_chipped_pass_is_aerial() {
        let ctx = parse_opta_f24(sample_xml()).expect("parse ok");
        let pass = ctx
            .events
            .iter()
            .find(|e| e.id == 1234567901)
            .expect("chipped pass");
        assert!(pass.is_aerial, "chipped pass (qual 155) should be aerial");
        // RTL flip: x=80 -> -31.5, target x=20 -> 31.5
        assert!((pass.origin_x - (-31.5)).abs() < 0.01);
        assert!((pass.target_x.unwrap() - 31.5).abs() < 0.01);
    }

    #[test]
    fn test_long_ball_is_not_aerial() {
        // qualifier 1 = "Long ball", must NOT set is_aerial.
        let xml = r#"<Games><Game id="1" home_team_id="h" away_team_id="a" home_team_name="H" away_team_name="A" competition_id="10" season="2026" game_date="2026-01-01">
            <Event id="500" type_id="1" period_id="1" min="1" sec="0" team_id="h" player_id="p" player_name="P" x="50" y="50" outcome="1">
              <Q qualifier_id="1" value=""/>
            </Event></Game></Games>"#;
        let ctx = parse_opta_f24(xml).expect("parse ok");
        assert!(!ctx.events[0].is_aerial, "long ball must not be aerial");
    }

    #[test]
    fn test_aerial_duel_type_is_aerial() {
        let xml = r#"<Games><Game id="1" home_team_id="h" away_team_id="a" home_team_name="H" away_team_name="A" competition_id="10" season="2026" game_date="2026-01-01">
            <Event id="501" type_id="44" period_id="1" min="1" sec="0" team_id="h" player_id="p" player_name="P" x="50" y="50" outcome="1"/>
            </Game></Games>"#;
        let ctx = parse_opta_f24(xml).expect("parse ok");
        assert!(ctx.events[0].is_aerial);
    }

    #[test]
    fn test_own_goal_is_flagged() {
        // Own goals arrive as a type-16 Goal carrying qualifier 28 (older
        // feeds) or qualifier 280 = "OWN_GOAL" (newer feeds), still credited
        // to the scorer's own team at parse time.
        let xml = r#"<Games><Game id="1" home_team_id="h" away_team_id="a" home_team_name="H" away_team_name="A" competition_id="10" season="2026" game_date="2026-01-01">
            <Event id="600" type_id="16" period_id="1" min="10" sec="0" team_id="h" player_id="p" player_name="P" x="4" y="50" outcome="1">
              <Q qualifier_id="28"/>
            </Event>
            <Event id="601" type_id="16" period_id="1" min="20" sec="0" team_id="a" player_id="q" player_name="Q" x="95" y="50" outcome="1">
              <Q qualifier_id="280" value="OWN_GOAL"/>
            </Event>
            <Event id="602" type_id="16" period_id="1" min="30" sec="0" team_id="h" player_id="p" player_name="P" x="95" y="50" outcome="1"/>
            <Event id="603" type_id="1" period_id="1" min="31" sec="0" team_id="h" player_id="p" player_name="P" x="50" y="50" outcome="1">
              <Q qualifier_id="28"/>
            </Event>
            </Game></Games>"#;
        let ctx = parse_opta_f24(xml).expect("parse ok");
        let g1 = ctx.events.iter().find(|e| e.id == 600).unwrap();
        let g2 = ctx.events.iter().find(|e| e.id == 601).unwrap();
        let g3 = ctx.events.iter().find(|e| e.id == 602).unwrap();
        // The pass (603) also carries q28 but must never be flagged.
        let pass = ctx.events.iter().find(|e| e.id == 603).unwrap();
        assert!(g1.is_own_goal, "qualifier 28 must flag an own goal");
        assert!(g2.is_own_goal, "qualifier 280=OWN_GOAL must flag an own goal");
        assert!(!g3.is_own_goal, "a plain goal must not be flagged");
        assert!(!pass.is_own_goal, "qualifier 28 only means own goal on a Goal row");
        // The row stays on the scorer's team until the engine flips it.
        assert_eq!(g1.team_id, "h");
        assert_eq!(g2.team_id, "a");
    }

    #[test]
    fn test_parse_empty_xml() {
        let ctx = parse_opta_f24(
            r#"<Games><Game id="1" home_team_id="h" away_team_id="a" home_team_name="H" away_team_name="A" competition_id="10" season="2026" game_date="2026-01-01"></Game></Games>"#,
        )
        .expect("parse ok");
        assert!(ctx.events.is_empty());
        assert_eq!(ctx.home_team_id, "h");
        assert_eq!(ctx.away_team_id, "a");
    }

    #[test]
    fn test_parse_malformed_returns_error() {
        assert!(parse_opta_f24("<not valid xml{{{").is_err());
    }

    #[test]
    fn test_all_coords_on_pitch() {
        let ctx = parse_opta_f24(sample_xml()).expect("parse ok");
        for ev in &ctx.events {
            assert!(
                MatchContext::is_on_pitch(ev.origin_x, ev.origin_y),
                "event {} origin ({}, {}) off pitch",
                ev.id, ev.origin_x, ev.origin_y
            );
            if let (Some(tx), Some(ty)) = (ev.target_x, ev.target_y) {
                assert!(MatchContext::is_on_pitch(tx, ty));
            }
        }
    }

    #[test]
    fn test_parse_player_info() {
        let ctx = parse_opta_f24(sample_xml()).expect("parse ok");
        let ev = ctx.events.iter().find(|e| e.id == 1234567900).unwrap();
        assert_eq!(ev.player_id, "p1");
        assert_eq!(ev.player_name, "Player 1");
        assert_eq!(ev.team_id, "home1");
    }

    #[test]
    fn test_parse_self_closing_event() {
        let xml = r#"<Games><Game id="1" home_team_id="h" away_team_id="a" home_team_name="H" away_team_name="A" competition_id="10" season="2026" game_date="2026-01-01">
            <Event id="500" type_id="1" period_id="1" min="3" sec="20" team_id="h" player_id="p1" player_name="Test" x="50" y="50" outcome="1"/>
        </Game></Games>"#;
        let ctx = parse_opta_f24(xml).expect("parse ok");
        assert_eq!(ctx.events.len(), 1);
        assert_eq!(ctx.events[0].id, 500);
        assert!((ctx.events[0].match_time_secs - 200.0).abs() < 0.001);
    }

    #[test]
    fn test_attribute_order_independence() {
        // Attributes deliberately reversed relative to the common order.
        let xml = r#"<Games><Game away_team_name="A" home_team_name="H" away_team_id="a" home_team_id="h" competition_id="10" season="2026" game_date="2026-01-01" id="1">
            <Event outcome="1" y="50" x="50" player_name="P" player_id="p" team_id="h" sec="5" min="1" period_id="1" type_id="1" id="999"/>
        </Game></Games>"#;
        let ctx = parse_opta_f24(xml).expect("parse ok");
        assert_eq!(ctx.events.len(), 1);
        assert_eq!(ctx.events[0].id, 999);
        assert_eq!(ctx.events[0].match_time_secs, 65.0);
        assert_eq!(ctx.events[0].team_id, "h");
        assert_eq!(ctx.events[0].player_name, "P");
    }
}
