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
    rebase_period_clocks(&mut ctx.events, &mut ctx.formation_changes);

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

/// Seconds of simulated half-time between two periods. The feed reuses the
/// 45:00 clock in P2, so without a gap P2's opening would interleave P1's
/// stoppage tail and the kick-off would be simulated before P1 had ended.
/// 15 real minutes keeps true chronology without a 45-minute dead sim.
pub const HALFTIME_GAP_SECS: f32 = 2.0 * 60.0;

/// Opta stamps events with whole seconds, so anything recorded inside one
/// second — a duel, then the clearance that follows it, then the reception
/// several metres away — claims the very same instant. The ball cannot be in
/// two places at once, so those events would all have to fire on the same
/// tick and every one but the first would see the ball (and often the doer)
/// somewhere else entirely.
///
/// Give each event inside a group its own slot within the second, keeping
/// feed order. The feed authors sequence simultaneous rows deliberately (a
/// duel winner before the duel loser, a shot before its save), so document
/// order wins — `time_order` stays as provenance for diagnostics. The offsets
/// stay below 1s, so the displayed mm:ss timestamp is unchanged and the feed
/// remains chronological.
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
                let base = ev.match_time_secs;
                ev.match_time_secs = base + step * (k as f32 + 0.5);
            }
        }
        i = j;
    }
}

/// Rebase later periods past the previous period's last event plus a
/// half-time gap, in first-seen period order. Within a period nothing moves
/// relative to anything else — only whole-period blocks shift, so the
/// football stays in true chronological order (P1 stoppage ends before the
/// P2 kick-off) while the mm:ss display clock is untouched.
fn rebase_period_clocks(
    events: &mut [NormalizedEvent],
    formation_changes: &mut [FormationChange],
) {
    let mut order: Vec<u8> = Vec::new();
    for ev in events.iter() {
        if !order.contains(&ev.period_id) {
            order.push(ev.period_id);
        }
    }
    // Cumulative end of the sim clock so far; each later period starts a
    // half-time gap after the previous period's last event.
    let mut sim_end = 0.0f32;
    let mut first = true;
    // Period → absolute shift applied, so formation changes (parsed with raw
    // clocks) can be moved by the same amount.
    let mut shifts: Vec<(u8, f32)> = Vec::new();
    for period in order {
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        for ev in events.iter() {
            if ev.period_id == period {
                min = min.min(ev.match_time_secs);
                max = max.max(ev.match_time_secs);
            }
        }
        let shift = if first {
            first = false;
            0.0
        } else {
            sim_end + HALFTIME_GAP_SECS - min
        };
        if shift != 0.0 {
            for ev in events.iter_mut() {
                if ev.period_id == period {
                    ev.match_time_secs += shift;
                }
            }
            max += shift;
        }
        shifts.push((period, shift));
        sim_end = max;
    }
    // Formation rows carry the raw period clock; move them with their
    // period so mid-match shape switches fire on the sim clock.
    for fc in formation_changes.iter_mut() {
        for (period, shift) in &shifts {
            if fc.period_id == *period {
                fc.match_time_secs += *shift;
                break;
            }
        }
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
    formation_changes: Vec<FormationChange>,
    events: Vec<NormalizedEvent>,

    // Current event accumulator
    event_id: u64,
    event_type_id: u16,
    event_period_id: u8,
    event_min: f32,
    event_sec: f32,
    event_timestamp_ms: Option<f32>,
    event_team_id: String,
    event_player_id: String,
    event_player_name: String,
    event_x: f32,
    event_y: f32,
    event_outcome: bool,
    /// Whether the row carried an explicit `outcome` attribute. Real F24 rows
    /// always do; synthetic fixtures often omit it — a missing outcome means
    /// "the logged action happened", never a failed pass.
    event_outcome_present: bool,

    // Per-event qualifier accumulators (WS0: the whole event, not just x/y).
    // Presence of valueless qualifiers is tracked in `q_present`; valued
    // qualifiers accumulate in `q_values`. Anything unmapped lands in
    // `raw_qualifiers` on the event itself.
    q_target_x: Option<f32>,
    q_target_y: Option<f32>,
    q_present: Vec<u16>,
    q_values: Vec<(u16, String)>,
    q30_players: Vec<String>,
    q44_positions: Vec<String>,
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
        self.event_timestamp_ms = None;
        self.event_team_id.clear();
        self.event_player_id.clear();
        self.event_player_name.clear();
        self.event_x = 0.0;
        self.event_y = 0.0;
        self.event_outcome = false;
        self.event_outcome_present = false;

        self.q_target_x = None;
        self.q_target_y = None;
        self.q_present.clear();
        self.q_values.clear();
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
            formation_changes: self.formation_changes,
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
            b"outcome" => {
                ctx.event_outcome = v == "1";
                ctx.event_outcome_present = true;
            }
            // Wall-clock ms for sub-second ordering inside one min:sec tick.
            b"timestamp" => ctx.event_timestamp_ms = parse_wall_ms(&v),
            _ => {}
        }
    }
}

/// Parse an ISO-8601 `timestamp` wall clock to milliseconds within its second
/// (`…T19:05:50.709Z` → `709.0`). Returns `None` when the value has no
/// fractional part — the caller then falls back to feed order.
fn parse_wall_ms(s: &str) -> Option<f32> {
    let t = s.split('T').next_back()?;
    let dot = t.find('.')?;
    let frac: String = t[dot + 1..].chars().take_while(|c| c.is_ascii_digit()).collect();
    if frac.is_empty() {
        return None;
    }
    let scale = 10f32.powi(3 - frac.len().min(3) as i32);
    frac.parse::<f32>().ok().map(|v| v * scale)
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
        QUAL_OWN_GOAL_FLAG => {
            if q_val.trim().eq_ignore_ascii_case("OWN_GOAL") {
                ctx.q_present.push(QUAL_OWN_GOAL);
            } else {
                // Fantasy-assist type (PASS_LOST / …): provenance, not a flag.
                ctx.q_values.push((QUAL_FANTASY_ASSIST_TYPE, q_val));
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
            ctx.q44_positions = q_val.split(',').map(|s| s.trim().to_string()).collect();
        }
        QUAL_LINEUP_SHIRTS => {
            ctx.q59_shirts = q_val.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        }
        QUAL_LINEUP_SLOTS => {
            ctx.q131_slots = q_val.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        }
        // Valueless presence qualifiers: remember the id, keep the value too
        // (usually empty) for provenance.
        QUAL_LONG_BALL | QUAL_CROSS | QUAL_HEAD_PASS | QUAL_THROUGH_BALL | QUAL_FK_TAKEN
        | QUAL_CORNER_TAKEN | QUAL_OWN_GOAL | QUAL_CHIPPED | QUAL_LAYOFF | QUAL_LAUNCH
        | QUAL_SWITCH | QUAL_THROW_IN | QUAL_GOAL_KICK | QUAL_KEEPER_HOOF | QUAL_FK_DIRECT
        | QUAL_FK_INDIRECT | QUAL_FK_LOW | QUAL_INSWINGER | QUAL_OUTSWINGER | QUAL_RIGHT_FOOT
        | QUAL_LEFT_FOOT | QUAL_HEAD | QUAL_OTHER_BODY_PART | QUAL_VOLLEY | QUAL_FLICK_ON
        | QUAL_FIRST_TOUCH | QUAL_OLIMPICO | QUAL_KEEPER_TOUCHED | QUAL_BLOCKED
        | QUAL_PARRIED_SAFE | QUAL_COLLECTED | QUAL_STANDING | QUAL_DIVING | QUAL_HANDS
        | QUAL_FEET | QUAL_SAVED_OFF_LINE | QUAL_HIT_WOODWORK | QUAL_DUEL_OFF | QUAL_DUEL_DEF
        | QUAL_AERIAL_FOUL | QUAL_YELLOW | QUAL_SECOND_YELLOW | QUAL_RED | QUAL_SUB_INJURY
        | QUAL_SUB_TACTICAL | QUAL_GOAL_DISALLOWED | QUAL_PENALTY | QUAL_LEADING_TO_ATTEMPT
        | QUAL_INTENTIONAL_ASSIST | QUAL_ASSIST | QUAL_SECOND_ASSIST | QUAL_SECOND_ASSIST_INTENT
        | QUAL_BIG_CHANCE | QUAL_INDIVIDUAL_PLAY | QUAL_REGULAR_PLAY | QUAL_FAST_BREAK
        | QUAL_SET_PIECE_CTX | QUAL_FROM_CORNER | QUAL_FK_DIRECT_SHOT | QUAL_ASSISTED
        | QUAL_HIGH | QUAL_DEF_BLOCK | QUAL_SIX_YARD_BLOCK | QUAL_BLOCKED_CROSS
        | QUAL_BLOCKED_PASS | QUAL_OUT_OF_PLAY | QUAL_TAKE_ON | QUAL_VAR_DELAY => {
            ctx.q_present.push(q_id);
            ctx.q_values.push((q_id, q_val));
        }
        // Valued qualifiers: links, geometry, slots, codes.
        QUAL_RELATED_ID | QUAL_RELATED_ID_2 | QUAL_OPPOSITE_ID | QUAL_SUB_LINK
        | QUAL_LENGTH_M | QUAL_ANGLE_RAD | QUAL_KEEPER_X | QUAL_KEEPER_Y
        | QUAL_KEEPER_X_AT_GOAL | QUAL_KEEPER_Y_AT_GOAL | QUAL_BLOCKED_X | QUAL_BLOCKED_Y
        | QUAL_OFFSIDE_PLAYER | QUAL_FOUL | QUAL_FORMATION_ID | QUAL_FORMATION_SLOT
        | QUAL_DETAILED_POSITION | QUAL_SUB_EXTRA | QUAL_CAPTAIN | QUAL_ZONE | QUAL_DELETED_TYPE
        | QUAL_PRE_REVIEW_TYPE | QUAL_TWIN_A | QUAL_TWIN_B | QUAL_KICK_OFF => {
            ctx.q_values.push((q_id, q_val));
        }
        _ => {
            // Unknown / future qualifiers are kept verbatim — never dropped.
            ctx.q_values.push((q_id, q_val));
        }
    }
}

fn finalize_event(ctx: &mut ParseContext) {
    // 1. Lineups (team setup) are captured *before* meta filtering so the
    //    roster survives. Formation changes (type 40) are captured as timed
    //    snapshots — never merged into the opening XI — and the engine
    //    applies them when the clock reaches them.
    if ctx.event_type_id == EVENT_TYPE_TEAM_SETUP && !ctx.q30_players.is_empty() {
        let roster = build_roster(
            &ctx.q30_players,
            &ctx.q44_positions,
            &ctx.q59_shirts,
            &ctx.q131_slots,
        );
        ctx.lineups.insert(ctx.event_team_id.clone(), roster);
        return;
    }
    if ctx.event_type_id == EVENT_TYPE_FORMATION_CHANGE {
        if !ctx.q30_players.is_empty() {
            let roster = build_roster(
                &ctx.q30_players,
                &ctx.q44_positions,
                &ctx.q59_shirts,
                &ctx.q131_slots,
            );
            if !roster.is_empty() {
                // Raw period clock; `rebase_period_clocks` moves it with the
                // rest of the period (sub-second stagger excluded — a shape
                // switch does not need tick precision).
                ctx.formation_changes.push(FormationChange {
                    match_time_secs: ctx.event_min * 60.0 + ctx.event_sec,
                    period_id: ctx.event_period_id,
                    team_id: ctx.event_team_id.clone(),
                    lineup: roster,
                });
            }
        }
        return;
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

    // Period offset: P1 stoppage (45:xx) and P2 (45:00) share the same clock.
    // `match_time_secs` keeps the raw period clock here; `rebase_period_clocks`
    // later shifts whole periods into true chronology. `display_secs` stays
    // raw forever — it is what the mm:ss UI rows show.
    let raw_secs = ctx.event_min * 60.0 + ctx.event_sec;
    let present = |q: u16| ctx.q_present.contains(&q);
    let value = |q: u16| {
        ctx.q_values
            .iter()
            .find(|(id, _)| *id == q)
            .map(|(_, v)| v.clone())
    };
    let value_f32 = |q: u16| value(q).and_then(|v| v.trim().parse::<f32>().ok());
    let value_u64 = |q: u16| value(q).and_then(|v| v.trim().parse::<u64>().ok());
    let value_u8 = |q: u16| value(q).and_then(|v| v.trim().parse::<u8>().ok());

    // Pass taxonomy: cross > through > switch > launch > long > layoff >
    // set-piece delivery > ground. (A cross can also carry Q155 chipped.)
    let pass_kind = if ctx.event_type_id == EVENT_TYPE_PASS {
        if present(QUAL_CORNER_TAKEN) {
            PassKind::CornerTaken
        } else if present(QUAL_FK_TAKEN) {
            PassKind::FreeKickTaken
        } else if present(QUAL_THROW_IN) {
            PassKind::Throw
        } else if present(QUAL_GOAL_KICK) {
            PassKind::GoalKick
        } else if present(QUAL_KEEPER_HOOF) {
            PassKind::KeeperHoof
        } else if let Some(v) = value(QUAL_KICK_OFF) {
            if v.trim() == "S" {
                PassKind::KickoffStart
            } else {
                PassKind::KickoffRestart
            }
        } else if present(QUAL_CROSS) {
            PassKind::Cross
        } else if present(QUAL_THROUGH_BALL) {
            PassKind::ThroughBall
        } else if present(QUAL_SWITCH) {
            PassKind::Switch
        } else if present(QUAL_LAUNCH) {
            PassKind::Launch
        } else if present(QUAL_LONG_BALL) {
            PassKind::LongBall
        } else if present(QUAL_LAYOFF) {
            PassKind::Layoff
        } else {
            PassKind::Ground
        }
    } else if ctx.event_type_id == EVENT_TYPE_OFFSIDE_PASS {
        // Offside passes carry the same delivery qualifiers; keep them.
        if present(QUAL_CROSS) {
            PassKind::Cross
        } else if present(QUAL_THROUGH_BALL) {
            PassKind::ThroughBall
        } else {
            PassKind::Ground
        }
    } else {
        PassKind::Unknown
    };

    // Body part: footedness > head > volley/flick > pass-header > other.
    let body_part = if present(QUAL_RIGHT_FOOT) {
        BodyPart::Right
    } else if present(QUAL_LEFT_FOOT) {
        BodyPart::Left
    } else if present(QUAL_HEAD) || present(QUAL_HEAD_PASS) {
        BodyPart::Head
    } else if present(QUAL_VOLLEY) {
        BodyPart::Volley
    } else if present(QUAL_OTHER_BODY_PART) {
        BodyPart::Other
    } else {
        BodyPart::Unknown
    };

    // Outcome reason: feed outcome + reason qualifiers + type truth. A row
    // without an explicit outcome attribute counts as completed (synthetic
    // fixtures omit it; the logged action happened).
    let outcome = ctx.event_outcome || !ctx.event_outcome_present;
    let outcome_reason = if ctx.event_type_id == EVENT_TYPE_OFFSIDE_PASS {
        OutcomeReason::Offside
    } else if ctx.event_type_id == EVENT_TYPE_ATTEMPT_SAVED {
        OutcomeReason::Saved
    } else if ctx.event_type_id == EVENT_TYPE_POST {
        OutcomeReason::Post
    } else if ctx.event_type_id == EVENT_TYPE_MISS {
        OutcomeReason::Miss
    } else if ctx.event_type_id == EVENT_TYPE_GOAL {
        OutcomeReason::Completed
    } else if ctx.event_type_id == EVENT_TYPE_INTERCEPTION {
        OutcomeReason::Intercepted
    } else if ctx.event_type_id == EVENT_TYPE_BLOCKED_PASS
        || present(QUAL_BLOCKED)
        || present(QUAL_DEF_BLOCK)
        || present(QUAL_SIX_YARD_BLOCK)
        || present(QUAL_BLOCKED_CROSS)
        || present(QUAL_BLOCKED_PASS)
    {
        OutcomeReason::Blocked
    } else if present(QUAL_OUT_OF_PLAY) || ctx.event_type_id == EVENT_TYPE_OUT {
        OutcomeReason::Out
    } else if outcome {
        OutcomeReason::Completed
    } else if matches!(ctx.event_type_id, 1 | 74 | 83) {
        OutcomeReason::Intercepted
    } else {
        OutcomeReason::Unknown
    };

    // Shot zone + goal-mouth / block / keeper geometry (feed 0–100 units).
    let shot_zone = if ctx.event_type_id >= EVENT_TYPE_MISS as u16
        && ctx.event_type_id <= EVENT_TYPE_GOAL as u16
    {
        if present(QUAL_SHOT_ZONE_MIN) {
            ShotZone::SmallBoxCentre
        } else if present(QUAL_SHOT_ZONE_MIN + 1) {
            ShotZone::BoxCentre
        } else if present(QUAL_SHOT_ZONE_MIN + 2) {
            ShotZone::OutOfBoxCentre
        } else if present(QUAL_SHOT_ZONE_MIN + 3) {
            ShotZone::FarOut
        } else {
            ShotZone::Unknown
        }
    } else {
        ShotZone::Unknown
    };
    let goalmouth = match (value_f32(QUAL_GOAL_MOUTH_Y), value_f32(QUAL_GOAL_MOUTH_Z)) {
        (Some(y), Some(z)) => Some((y, z)),
        _ => None,
    };
    let blocked_at = match (value_f32(QUAL_BLOCKED_X), value_f32(QUAL_BLOCKED_Y)) {
        (Some(x), Some(y)) => {
            let (bx, by) = MatchContext::normalize_coord(x, y, dir);
            Some((bx, by))
        }
        _ => None,
    };
    let keeper_xy = match (value_f32(QUAL_KEEPER_X), value_f32(QUAL_KEEPER_Y)) {
        (Some(x), Some(y)) => Some((x, y)),
        _ => None,
    };
    let keeper_at_goal = match (
        value_f32(QUAL_KEEPER_X_AT_GOAL),
        value_f32(QUAL_KEEPER_Y_AT_GOAL),
    ) {
        (Some(x), Some(y)) => Some((x, y)),
        _ => None,
    };

    let setpiece = if present(QUAL_FK_DIRECT) || present(QUAL_FK_DIRECT_SHOT) {
        SetPieceTag::Direct
    } else if present(QUAL_FK_INDIRECT) {
        SetPieceTag::Indirect
    } else if present(QUAL_FK_LOW) {
        SetPieceTag::Low
    } else if present(QUAL_FROM_CORNER) {
        SetPieceTag::FromCorner
    } else if present(QUAL_THROW_SET_PIECE) {
        SetPieceTag::FromThrow
    } else if present(QUAL_FAST_BREAK) {
        SetPieceTag::FastBreak
    } else if present(QUAL_REGULAR_PLAY) || present(QUAL_SET_PIECE_CTX) {
        SetPieceTag::OpenPlay
    } else {
        SetPieceTag::Unknown
    };
    let setpiece_swing = if present(QUAL_INSWINGER) {
        SetPieceTag::Inswinger
    } else if present(QUAL_OUTSWINGER) {
        SetPieceTag::Outswinger
    } else {
        SetPieceTag::Unknown
    };
    let zone = match value(QUAL_ZONE).as_deref().map(|s| s.trim()) {
        Some("Back") => PitchZone::Back,
        Some("Center") => PitchZone::Center,
        Some("Left") => PitchZone::Left,
        Some("Right") => PitchZone::Right,
        _ => PitchZone::Unknown,
    };
    let duel = if present(QUAL_DUEL_OFF) {
        DuelPolarity::Offensive
    } else if present(QUAL_DUEL_DEF) {
        DuelPolarity::Defensive
    } else if present(QUAL_AERIAL_FOUL) {
        DuelPolarity::AerialFoul
    } else {
        DuelPolarity::Unknown
    };
    let keeper_action = if present(QUAL_PARRIED_SAFE) {
        KeeperAction::ParriedSafe
    } else if present(QUAL_COLLECTED) {
        KeeperAction::Collected
    } else if present(QUAL_STANDING) {
        KeeperAction::Standing
    } else if present(QUAL_DIVING) {
        KeeperAction::Diving
    } else if present(QUAL_HANDS) {
        KeeperAction::Hands
    } else if present(QUAL_FEET) {
        KeeperAction::Feet
    } else if present(QUAL_SAVED_OFF_LINE) {
        KeeperAction::SavedOffLine
    } else if present(QUAL_KEEPER_TOUCHED) {
        KeeperAction::Touched
    } else {
        KeeperAction::Unknown
    };
    let card = if present(QUAL_RED) {
        CardKind::Red
    } else if present(QUAL_SECOND_YELLOW) {
        CardKind::SecondYellow
    } else if present(QUAL_YELLOW) {
        CardKind::Yellow
    } else {
        CardKind::None
    };
    let sub_reason = if present(QUAL_SUB_INJURY) {
        SubReason::Injury
    } else if present(QUAL_SUB_TACTICAL) {
        SubReason::Tactical
    } else {
        SubReason::Unknown
    };
    // Sub-on rows in this feed carry Q399 = the player coming on… no: Q399
    // carries a player id related to the carry/touch row (type 83). On type-19
    // rows the link qualifier is Q55; keep both.
    let sub_link = value_u64(QUAL_RELATED_ID).or_else(|| value_u64(QUAL_SUB_LINK));
    let carry_of = if ctx.event_type_id == EVENT_TYPE_CARRY {
        value(QUAL_SUB_LINK)
    } else {
        None
    };
    let offside_players: Vec<String> = ctx
        .q_values
        .iter()
        .filter(|(id, _)| *id == QUAL_OFFSIDE_PLAYER)
        .map(|(_, v)| v.clone())
        .collect();
    // Raw: everything valued that was NOT consumed above stays verbatim.
    const CONSUMED: &[u16] = &[
        QUAL_TARGET_X,
        QUAL_TARGET_Y,
        QUAL_OWN_GOAL,
        QUAL_OWN_GOAL_FLAG,
        QUAL_DIR,
        QUAL_LINEUP_PLAYERS,
        QUAL_LINEUP_POSITIONS,
        QUAL_LINEUP_SHIRTS,
        QUAL_LINEUP_SLOTS,
        QUAL_FORMATION_ID,
        QUAL_CAPTAIN,
        QUAL_TWIN_A,
        QUAL_TWIN_B,
        QUAL_RELATED_ID,
        QUAL_RELATED_ID_2,
        QUAL_OPPOSITE_ID,
        QUAL_SUB_LINK,
        QUAL_LENGTH_M,
        QUAL_ANGLE_RAD,
        QUAL_KEEPER_X,
        QUAL_KEEPER_Y,
        QUAL_KEEPER_X_AT_GOAL,
        QUAL_KEEPER_Y_AT_GOAL,
        QUAL_BLOCKED_X,
        QUAL_BLOCKED_Y,
        QUAL_OFFSIDE_PLAYER,
        QUAL_FOUL,
        QUAL_FORMATION_SLOT,
        QUAL_DETAILED_POSITION,
        QUAL_SUB_EXTRA,
        QUAL_ZONE,
        QUAL_DELETED_TYPE,
        QUAL_PRE_REVIEW_TYPE,
        QUAL_KICK_OFF,
    ];
    let consumed_present: &[u16] = &[
        QUAL_LONG_BALL,
        QUAL_CROSS,
        QUAL_HEAD_PASS,
        QUAL_THROUGH_BALL,
        QUAL_FK_TAKEN,
        QUAL_CORNER_TAKEN,
        QUAL_OWN_GOAL,
        QUAL_CHIPPED,
        QUAL_LAYOFF,
        QUAL_LAUNCH,
        QUAL_SWITCH,
        QUAL_THROW_IN,
        QUAL_GOAL_KICK,
        QUAL_KEEPER_HOOF,
        QUAL_FK_DIRECT,
        QUAL_FK_INDIRECT,
        QUAL_FK_LOW,
        QUAL_INSWINGER,
        QUAL_OUTSWINGER,
        QUAL_RIGHT_FOOT,
        QUAL_LEFT_FOOT,
        QUAL_HEAD,
        QUAL_OTHER_BODY_PART,
        QUAL_VOLLEY,
        QUAL_FLICK_ON,
        QUAL_FIRST_TOUCH,
        QUAL_OLIMPICO,
        QUAL_KEEPER_TOUCHED,
        QUAL_BLOCKED,
        QUAL_PARRIED_SAFE,
        QUAL_COLLECTED,
        QUAL_STANDING,
        QUAL_DIVING,
        QUAL_HANDS,
        QUAL_FEET,
        QUAL_SAVED_OFF_LINE,
        QUAL_HIT_WOODWORK,
        QUAL_DUEL_OFF,
        QUAL_DUEL_DEF,
        QUAL_AERIAL_FOUL,
        QUAL_YELLOW,
        QUAL_SECOND_YELLOW,
        QUAL_RED,
        QUAL_SUB_INJURY,
        QUAL_SUB_TACTICAL,
        QUAL_GOAL_DISALLOWED,
        QUAL_PENALTY,
        QUAL_LEADING_TO_ATTEMPT,
        QUAL_INTENTIONAL_ASSIST,
        QUAL_ASSIST,
        QUAL_SECOND_ASSIST,
        QUAL_SECOND_ASSIST_INTENT,
        QUAL_BIG_CHANCE,
        QUAL_INDIVIDUAL_PLAY,
        QUAL_REGULAR_PLAY,
        QUAL_FAST_BREAK,
        QUAL_SET_PIECE_CTX,
        QUAL_FROM_CORNER,
        QUAL_FK_DIRECT_SHOT,
        QUAL_ASSISTED,
        QUAL_HIGH,
        QUAL_DEF_BLOCK,
        QUAL_SIX_YARD_BLOCK,
        QUAL_BLOCKED_CROSS,
        QUAL_BLOCKED_PASS,
        QUAL_OUT_OF_PLAY,
        QUAL_TAKE_ON,
        QUAL_VAR_DELAY,
    ];
    let raw_qualifiers: Vec<(u16, String)> = ctx
        .q_values
        .iter()
        .filter(|(id, _)| !CONSUMED.contains(id) && !consumed_present.contains(id))
        .cloned()
        .collect();

    ctx.events.push(NormalizedEvent {
        id: ctx.event_id,
        type_id: ctx.event_type_id,
        period_id: ctx.event_period_id,
        match_time_secs: raw_secs,
        display_secs: raw_secs,
        time_order: ctx.event_timestamp_ms.unwrap_or(0.0) / 1000.0,
        team_id: ctx.event_team_id.clone(),
        player_id: ctx.event_player_id.clone(),
        player_name: ctx.event_player_name.clone(),
        origin_x,
        origin_y,
        target_x,
        target_y,
        // `outcome` fills in a missing attribute: synthetic rows without one
        // count the logged action as completed.
        outcome,
        // Aerial = aerial duel (type 44) or a chipped (in-air) pass.
        is_aerial: ctx.event_type_id == EVENT_TYPE_AERIAL || present(QUAL_CHIPPED),
        // Own goal: only meaningful on a Goal row.
        is_own_goal: present(QUAL_OWN_GOAL) && ctx.event_type_id == EVENT_TYPE_GOAL,
        pass_kind,
        body_part,
        outcome_reason,
        shot_zone,
        goalmouth,
        blocked_at,
        keeper_xy,
        keeper_at_goal,
        related_id: value_u64(QUAL_RELATED_ID),
        related_id_2: value_u64(QUAL_RELATED_ID_2),
        opposite_id: value_u64(QUAL_OPPOSITE_ID),
        assist: present(QUAL_ASSIST),
        intentional_assist: present(QUAL_INTENTIONAL_ASSIST),
        second_assist: present(QUAL_SECOND_ASSIST) || present(QUAL_SECOND_ASSIST_INTENT),
        big_chance: present(QUAL_BIG_CHANCE),
        individual_play: present(QUAL_INDIVIDUAL_PLAY),
        first_time: present(QUAL_FIRST_TOUCH),
        olimpico: present(QUAL_OLIMPICO),
        keeper_touched: present(QUAL_KEEPER_TOUCHED),
        setpiece,
        setpiece_swing,
        zone,
        duel,
        keeper_action,
        hit_woodwork: present(QUAL_HIT_WOODWORK),
        length_m: value_f32(QUAL_LENGTH_M),
        angle_rad: value_f32(QUAL_ANGLE_RAD),
        card,
        foul_code: value(QUAL_FOUL),
        foul_victim_of: value_u64(QUAL_RELATED_ID).filter(|_| ctx.event_type_id == 17),
        sub_reason,
        formation_slot: value_u8(QUAL_FORMATION_SLOT),
        sub_link,
        shirt_number: value_u8(QUAL_LINEUP_SHIRTS)
            // Sub rows carry the shirt as a single Q59 value, which the
            // roster branch parses into `q59_shirts` instead of `q_values`.
            .or_else(|| ctx.q59_shirts.first().copied()),
        offside_players,
        goal_disallowed: present(QUAL_GOAL_DISALLOWED),
        penalty: present(QUAL_PENALTY),
        leading_to_attempt: present(QUAL_LEADING_TO_ATTEMPT),
        carry_of,
        raw_qualifiers,
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
    positions: &[String],
    shirts: &[u8],
    slots: &[u8],
) -> Vec<LineupPlayer> {
    players
        .iter()
        .enumerate()
        .map(|(i, pid)| LineupPlayer {
            player_id: pid.clone(),
            position_category: positions.get(i).map(|s| parse_category(s)).unwrap_or(0),
            shirt_number: shirts.get(i).copied().unwrap_or(0),
            formation_slot: slots.get(i).copied().unwrap_or(0),
        })
        .collect()
}

/// Position category from a Q44 value: numeric (`1`–`5`) on lineup rows,
/// text (`Defender`/`Midfielder`/…) on substitution rows.
fn parse_category(s: &str) -> u8 {
    if let Ok(n) = s.trim().parse::<u8>() {
        return n;
    }
    match s.trim().to_ascii_lowercase().as_str() {
        "goalkeeper" | "goal keeper" | "gk" => 1,
        "defender" | "defence" | "defense" => 2,
        "midfielder" | "midfield" => 3,
        "forward" | "striker" | "attacker" => 4,
        _ => 0,
    }
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
    fn test_wall_ms_parsing() {
        assert_eq!(parse_wall_ms("2026-09-11T19:05:50.709Z"), Some(709.0));
        assert_eq!(parse_wall_ms("2026-09-11T19:05:50.7Z"), Some(700.0));
        assert_eq!(parse_wall_ms("2026-09-11T20:54:23Z"), None);
        assert_eq!(parse_wall_ms(""), None);
    }

    #[test]
    fn test_whole_event_semantics_are_preserved() {
        // Cross → goal chain with assist graph, shot placement, keeper xy.
        let xml = r#"<Games><Game id="1" home_team_id="h" away_team_id="a" home_team_name="H" away_team_name="A" competition_id="10" season="2026" game_date="2026-01-01">
            <Event id="300" type_id="1" period_id="1" min="27" sec="12" team_id="h" player_id="p1" player_name="Winger" x="93" y="83.9" outcome="1">
              <Q qualifier_id="2"/><Q qualifier_id="22"/><Q qualifier_id="72"/>
              <Q qualifier_id="140" value="96.2"/><Q qualifier_id="141" value="52.4"/>
              <Q qualifier_id="154"/><Q qualifier_id="210" value="16"/>
              <Q qualifier_id="212" value="21.7"/><Q qualifier_id="213" value="4.87"/>
              <Q qualifier_id="224"/>
            </Event>
            <Event id="301" type_id="16" period_id="1" min="27" sec="13" team_id="h" player_id="p9" player_name="Striker" x="96.2" y="52.4" outcome="1">
              <Q qualifier_id="20"/><Q qualifier_id="22"/><Q qualifier_id="29"/>
              <Q qualifier_id="55" value="300"/><Q qualifier_id="56" value="Center"/>
              <Q qualifier_id="80"/><Q qualifier_id="102" value="45.8"/><Q qualifier_id="103" value="2.5"/>
              <Q qualifier_id="230" value="99.0"/><Q qualifier_id="231" value="52.2"/>
              <Q qualifier_id="214"/><Q qualifier_id="328"/>
            </Event>
            <Event id="302" type_id="1" period_id="1" min="30" sec="0" team_id="a" player_id="q1" player_name="Mid" x="50" y="50" outcome="0">
              <Q qualifier_id="140" value="70"/><Q qualifier_id="141" value="60"/>
            </Event>
            <Event id="303" type_id="8" period_id="1" min="30" sec="1" team_id="h" player_id="p4" player_name="DM" x="65" y="55" outcome="1">
              <Q qualifier_id="233" value="302"/><Q qualifier_id="285"/>
            </Event>
        </Game></Games>"#;
        let ctx = parse_opta_f24(xml).expect("parse ok");
        assert_eq!(ctx.events.len(), 4);

        let cross = &ctx.events[0];
        assert_eq!(cross.pass_kind, crate::parser::PassKind::Cross);
        assert_eq!(cross.body_part, crate::parser::BodyPart::Left);
        assert!(cross.intentional_assist && cross.assist);
        assert_eq!(cross.related_id, None);
        assert!((cross.length_m.unwrap() - 21.7).abs() < 1e-3);
        assert!((cross.angle_rad.unwrap() - 4.87).abs() < 1e-3);
        assert_eq!(cross.outcome_reason, crate::parser::OutcomeReason::Completed);
        assert_eq!(cross.setpiece_swing, crate::parser::SetPieceTag::Outswinger);

        let goal = &ctx.events[1];
        assert_eq!(goal.related_id, Some(300));
        assert_eq!(goal.goalmouth, Some((45.8, 2.5)));
        assert_eq!(goal.keeper_xy, Some((99.0, 52.2)));
        assert!(goal.big_chance && goal.first_time);
        let (lat, z) = goal.goalmouth_target(1.0).unwrap();
        assert!((lat - ((45.8 - 50.0) * 0.68)).abs() < 1e-3, "lat {lat}");
        assert!((z - (2.5 * 2.44 / 38.0)).abs() < 1e-3, "z {z}");
        assert!(goal.estimate_xg().unwrap() > 0.3);

        let failed = &ctx.events[2];
        assert_eq!(failed.outcome_reason, crate::parser::OutcomeReason::Intercepted);
        assert!(failed.is_failed_pass());

        let pick = &ctx.events[3];
        assert_eq!(pick.opposite_id, Some(302));
        assert_eq!(pick.duel, crate::parser::DuelPolarity::Defensive);
    }

    #[test]
    fn test_cards_subs_and_formation_changes() {
        let xml = r#"<Games><Game id="1" home_team_id="h" away_team_id="a" home_team_name="H" away_team_name="A" competition_id="10" season="2026" game_date="2026-01-01">
            <Event id="400" type_id="17" period_id="2" min="93" sec="45" team_id="h" player_id="p3" player_name="LB" x="0" y="0" outcome="1">
              <Q qualifier_id="13" value="243"/><Q qualifier_id="31"/><Q qualifier_id="55" value="399"/>
            </Event>
            <Event id="401" type_id="40" period_id="2" min="66" sec="33" team_id="a" player_id="" player_name="" x="0" y="0" outcome="1">
              <Q qualifier_id="30" value="a1,a2,a3,a4,a5,a6,a7,a8,a9,a10,a11"/>
              <Q qualifier_id="44" value="1,3,3,2,2,2,3,3,4,4,4"/>
              <Q qualifier_id="59" value="22,12,3,2,5,26,37,8,19,17,27"/>
              <Q qualifier_id="131" value="1,2,3,4,5,6,7,8,9,10,11"/>
            </Event>
            <Event id="402" type_id="19" period_id="2" min="66" sec="32" team_id="a" player_id="sub1" player_name="Sub" x="0" y="0" outcome="1">
              <Q qualifier_id="42"/><Q qualifier_id="44" value="Defender"/><Q qualifier_id="59" value="2"/>
              <Q qualifier_id="145" value="2"/><Q qualifier_id="292" value="5"/>
            </Event>
        </Game></Games>"#;
        let ctx = parse_opta_f24(xml).expect("parse ok");
        // Type 40 never enters the timeline; the card + sub do.
        assert!(ctx.events.iter().all(|e| e.type_id != 40));
        let card = ctx.events.iter().find(|e| e.id == 400).unwrap();
        assert_eq!(card.card, crate::parser::CardKind::Yellow);
        assert_eq!(card.foul_code.as_deref(), Some("243"));
        assert_eq!(card.foul_victim_of, Some(399));
        let sub = ctx.events.iter().find(|e| e.id == 402).unwrap();
        assert_eq!(sub.sub_reason, crate::parser::SubReason::Tactical);
        assert_eq!(sub.formation_slot, Some(2));
        assert_eq!(sub.shirt_number, Some(2));
        assert_eq!(ctx.formation_changes.len(), 1);
        let fc = &ctx.formation_changes[0];
        assert_eq!(fc.team_id, "a");
        assert_eq!(fc.lineup.iter().filter(|p| p.is_starter()).count(), 11);
        assert!(fc.match_time_secs > 2700.0, "FC rebased past P1: {}", fc.match_time_secs);
    }

    #[test]
    fn test_period_rebase_keeps_chronology_and_display() {
        // P1 tail at 47:16, P2 head at 45:00: sim clock must order P1 first,
        // display clock must still read 45:00 for the P2 rows.
        let xml = r#"<Games><Game id="1" home_team_id="h" away_team_id="a" home_team_name="H" away_team_name="A" competition_id="10" season="2026" game_date="2026-01-01">
            <Event id="500" type_id="1" period_id="1" min="47" sec="16" team_id="h" player_id="p" player_name="P" x="50" y="50" outcome="1"/>
            <Event id="501" type_id="1" period_id="2" min="45" sec="0" team_id="h" player_id="p" player_name="P" x="50" y="50" outcome="1"/>
        </Game></Games>"#;
        let ctx = parse_opta_f24(xml).expect("parse ok");
        let p1 = ctx.events.iter().find(|e| e.id == 500).unwrap();
        let p2 = ctx.events.iter().find(|e| e.id == 501).unwrap();
        assert!(p1.match_time_secs < p2.match_time_secs, "P1 tail must precede P2 head");
        assert!((p2.match_time_secs - p1.match_time_secs - HALFTIME_GAP_SECS).abs() < 1.0);
        assert!((p2.display_secs - 2700.0).abs() < 1e-3);
        assert_eq!(p2.time_string(), "45:00");
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
