//! Whole-match analysis log for the browser.
//!
//! Builds one compact JSON snapshot at load time containing every gameplay
//! event (type, clocks, player, coordinates, outcome) plus both rosters with
//! resolved display names. The front end parses it once and derives dribble
//! zones, per-player stats and timeline scrubbers from it client-side, so
//! the analysis is complete from kick-off and unaffected by seeks.
//!
//! Row shapes (JSON arrays, field order fixed):
//!
//! ```text
//! event: [type, display_secs, match_secs, period, team, player_id,
//!         player_name, x, y, end_x|null, end_y|null, outcome]
//! roster: [player_id, name, shirt, position_category]
//! ```

use crate::engine::SimulationEngine;
use serde::Serialize;

/// Compact event row. `team` is `0` home, `1` away, `2` unknown.
type EventRow = (
    u16,
    f32,
    f32,
    u8,
    u8,
    String,
    String,
    f32,
    f32,
    Option<f32>,
    Option<f32>,
    u8,
);

/// Roster row: `[player_id, name, shirt, position_category]`.
type RosterRow = (String, String, u8, u8);

#[derive(Serialize)]
struct AnalysisLog {
    /// Schema version, so the JS side can reject a stale package.
    v: u8,
    events: Vec<EventRow>,
    /// Indexed by team: `0` home, `1` away.
    roster: [Vec<RosterRow>; 2],
}

/// Round to one decimal — sub-decimetre precision adds bytes without
/// improving any zone assignment or metric.
#[inline]
fn r1(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

/// Map a feed team id onto `0` home, `1` away, `2` unknown.
fn team_index(engine: &SimulationEngine, team_id: &str) -> u8 {
    if team_id == engine.home_team_id.as_str() {
        0
    } else if team_id == engine.away_team_id.as_str() {
        1
    } else {
        2
    }
}

/// Build the JSON analysis log for `engine`.
pub fn build(engine: &SimulationEngine) -> String {
    // Display names come from the event rows themselves: `LineupPlayer`
    // carries only ids, shirts and position categories.
    let mut names: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for ev in engine.dispatcher.events() {
        if !ev.player_id.is_empty() && !ev.player_name.is_empty() {
            names.entry(ev.player_id.as_str()).or_insert(ev.player_name.as_str());
        }
    }

    let events: Vec<EventRow> = engine
        .dispatcher
        .events()
        .iter()
        .map(|ev| {
            (
                ev.type_id,
                r1(ev.display_secs),
                r1(ev.match_time_secs),
                ev.period_id,
                team_index(engine, ev.team_id.as_str()),
                ev.player_id.clone(),
                ev.player_name.clone(),
                r1(ev.origin_x),
                r1(ev.origin_y),
                ev.target_x.map(r1),
                ev.target_y.map(r1),
                u8::from(ev.outcome),
            )
        })
        .collect();

    let mut roster = [Vec::new(), Vec::new()];
    for (slot, team_id) in [engine.home_team_id.as_str(), engine.away_team_id.as_str()].iter().enumerate() {
        let Some(players) = engine.lineups.get(*team_id) else {
            continue;
        };
        // Starting XI first (formation slot 1-11), subs (slot 0) last.
        let mut ordered: Vec<&crate::parser::LineupPlayer> = players.iter().collect();
        ordered.sort_by_key(|p| (p.formation_slot == 0, p.formation_slot));
        let mut rows: Vec<RosterRow> = ordered
            .into_iter()
            .map(|p| {
                let name = names.get(p.player_id.as_str()).copied().unwrap_or("").to_string();
                (p.player_id.clone(), name, p.shirt_number, p.position_category)
            })
            .collect();
        // Drop duplicate ids a mid-match roster merge may have re-inserted.
        rows.dedup_by(|a, b| a.0 == b.0);
        roster[slot] = rows;
    }

    // Events whose player never appears in a roster (defensive feeds) still
    // need a name: append one synthetic entry so name lookup stays total.
    // The slot is the player's *majority* event team, and a player already
    // filed in either lineup is never re-filed: dead-ball twin folding
    // (`loader::drop_twin_dead_ball_rows`) hands a surviving row the
    // restart-awarding team while keeping the doer's identity, so a per-row
    // assignment would place the same player under both teams.
    {
        use std::collections::{HashMap, HashSet};

        let known: HashSet<String> = roster.iter().flatten().map(|r| r.0.clone()).collect();
        let mut counts: HashMap<&str, [usize; 3]> = HashMap::new();
        for ev in events.iter() {
            if ev.5.is_empty() || ev.6.is_empty() || known.contains(ev.5.as_str()) {
                continue;
            }
            counts.entry(ev.5.as_str()).or_insert([0; 3])[ev.4 as usize] += 1;
        }
        // First-event order keeps the appended order deterministic.
        let mut filed: HashSet<&str> = HashSet::new();
        for ev in events.iter() {
            let pid = ev.5.as_str();
            if pid.is_empty() || ev.6.is_empty() || known.contains(pid) || filed.contains(pid) {
                continue;
            }
            let Some(c) = counts.get(pid) else { continue };
            if c[0] == 0 && c[1] == 0 {
                continue; // only neutral rows — nowhere to file the player
            }
            let team = if c[1] > c[0] {
                1
            } else if c[0] > c[1] {
                0
            } else {
                // Symmetric split: side of the first row that named a team.
                events
                    .iter()
                    .find(|e| e.5 == ev.5 && e.4 < 2)
                    .map_or(0, |e| e.4 as usize)
            };
            filed.insert(pid);
            roster[team].push((ev.5.clone(), ev.6.clone(), 0, 0));
        }
    }

    let log = AnalysisLog { v: 1, events, roster };
    serde_json::to_string(&log).unwrap_or_else(|_| "{\"v\":1,\"events\":[],\"roster\":[[],[]]}".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wasm::MatchRenderer;

    /// Minimal match: lineups for both sides, one take-on, one carry, one
    /// pass and a goal, with names only on the event rows.
    const XML: &str = r#"<Games>
      <Game id="42" home_team_id="home" away_team_id="away"
            home_team_name="Home" away_team_name="Away"
            competition_id="10" season="2026" game_date="2026-01-01">
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
        <Event id="10" type_id="3" period_id="1" min="5" sec="10" team_id="home"
               player_id="h10" player_name="J. Smith" x="70" y="40" outcome="1"/>
        <Event id="11" type_id="83" period_id="1" min="5" sec="13" team_id="home"
               player_id="h10" player_name="J. Smith" x="74" y="42" outcome="1">
          <Q qualifier_id="140" value="80"/>
          <Q qualifier_id="141" value="44"/>
        </Event>
        <Event id="12" type_id="1" period_id="1" min="5" sec="20" team_id="home"
               player_id="h10" player_name="J. Smith" x="80" y="44" outcome="1"/>
        <Event id="13" type_id="16" period_id="1" min="6" sec="0" team_id="home"
               player_id="h9" player_name="A. Striker" x="94" y="50" outcome="1"/>
      </Game></Games>"#;

    fn renderer() -> MatchRenderer {
        MatchRenderer::from_xml(XML).unwrap()
    }

    fn parsed() -> serde_json::Value {
        serde_json::from_str(&build(renderer().engine())).unwrap()
    }

    #[test]
    fn log_carries_every_gameplay_event_in_order() {
        let log = parsed();
        assert_eq!(log["v"], 1);
        let rows = log["events"].as_array().unwrap();
        // Lineup rows (type 34) are meta; only the four gameplay rows land.
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0][0], 3, "first row is the take-on");
        assert_eq!(rows[1][0], 83, "then the carry");
        assert_eq!(rows[3][0], 16, "goal last");
        for row in rows {
            assert_eq!(row.as_array().unwrap().len(), 12, "every row has 12 fields");
        }
    }

    #[test]
    fn event_rows_carry_clocks_teams_and_outcomes() {
        let rows = parsed()["events"].as_array().unwrap().clone();
        let take_on = &rows[0];
        assert_eq!(take_on[1], 310.0, "display clock 5:10");
        assert_eq!(take_on[3], 1, "period 1");
        assert_eq!(take_on[4], 0, "home team index");
        assert_eq!(take_on[5], "h10");
        assert_eq!(take_on[6], "J. Smith");
        assert_eq!(take_on[11], 1, "successful take-on");
        // Feed x=70 in the home attacking direction → 0.7·105 − 52.5 = 21 m.
        let x = take_on[7].as_f64().unwrap();
        assert!((x - 21.0).abs() < 0.11, "normalised x was {x}");
        // The carry carries an end point from Q140/Q141.
        let carry = &rows[1];
        assert!(carry[9].is_number() && carry[10].is_number(), "carry end point kept");
        assert!(carry[2].as_f64().unwrap() >= carry[1].as_f64().unwrap(),
            "match clock never trails the display clock");
    }

    #[test]
    fn roster_resolves_names_from_event_rows() {
        let log = parsed();
        let home = log["roster"][0].as_array().unwrap();
        assert_eq!(home.len(), 11, "one row per starter");
        let ten = home.iter().find(|r| r[0] == "h10").expect("h10 in the XI");
        assert_eq!(ten[1], "J. Smith", "name resolved from the event rows");
        assert_eq!(ten[2], 10, "shirt");
        assert_eq!(ten[3], 4, "position category (FW)");
        let away = log["roster"][1].as_array().unwrap();
        assert_eq!(away.len(), 11);
        assert_eq!(away[0][1], "", "away players never named in the feed stay empty");
    }

    #[test]
    fn unknown_team_events_are_marked_neutral() {
        let json = build(renderer().engine());
        assert!(json.contains("\"v\":1"));
        let log: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(log["roster"].as_array().unwrap().len(), 2);
        for row in log["events"].as_array().unwrap() {
            assert!(row[4].as_u64().unwrap() <= 2, "team index stays in 0..=2");
            assert!(row[7].as_f64().unwrap().abs() <= 52.5, "x on the pitch");
            assert!(row[8].as_f64().unwrap().abs() <= 34.0, "y on the pitch");
        }
    }

    /// Twin dead-ball rows (same type, same second, a metre apart) hand the
    /// surviving row the restart-awarding team while keeping the doer's
    /// identity — so a home player can carry one away-team event. The roster
    /// must not follow that single row: he stays in his own XI, and a player
    /// in neither XI is filed under his majority team only.
    const TWIN_XML: &str = r#"<Games>
      <Game id="43" home_team_id="home" away_team_id="away"
            home_team_name="Home" away_team_name="Away"
            competition_id="10" season="2026" game_date="2026-01-01">
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
        <Event id="10" type_id="3" period_id="1" min="6" sec="10" team_id="home"
               player_id="h5" player_name="H. Five" x="40" y="10" outcome="1"/>
        <Event id="11" type_id="3" period_id="1" min="6" sec="12" team_id="home"
               player_id="z9" player_name="Z. Nine" x="41" y="11" outcome="1"/>
        <Event id="12" type_id="83" period_id="1" min="6" sec="14" team_id="home"
               player_id="z9" player_name="Z. Nine" x="42" y="12" outcome="1">
          <Q qualifier_id="140" value="45"/>
          <Q qualifier_id="141" value="15"/>
        </Event>
        <Event id="13" type_id="5" period_id="1" min="6" sec="20" team_id="home"
               player_id="z9" player_name="Z. Nine" x="50" y="20" outcome="1"/>
        <Event id="14" type_id="5" period_id="1" min="6" sec="20" team_id="away"
               player_id="a3" player_name="A. Three" x="50.5" y="80" outcome="0"/>
        <Event id="15" type_id="1" period_id="1" min="6" sec="50" team_id="home"
               player_id="h5" player_name="H. Five" x="44" y="14" outcome="1"/>
        <Event id="16" type_id="6" period_id="1" min="7" sec="0" team_id="home"
               player_id="h5" player_name="H. Five" x="60" y="20" outcome="1"/>
        <Event id="17" type_id="6" period_id="1" min="7" sec="0" team_id="away"
               player_id="a4" player_name="A. Four" x="40" y="80" outcome="0"/>
      </Game></Games>"#;

    #[test]
    fn flipped_dead_ball_rows_never_re_file_a_player() {
        let log: serde_json::Value =
            serde_json::from_str(&build(MatchRenderer::from_xml(TWIN_XML).unwrap().engine()))
                .unwrap();
        let rows = log["events"].as_array().unwrap();

        // The twin fold really happened: both doers each carry one row under
        // the restart-awarding (away) team.
        let flipped = |pid: &str| {
            rows.iter()
                .filter(|r| r[5] == pid && r[4].as_u64() == Some(1))
                .count()
        };
        assert_eq!(flipped("h5"), 1, "h5's dead-ball row was re-teamed");
        assert_eq!(flipped("z9"), 1, "z9's dead-ball row was re-teamed");

        let home = log["roster"][0].as_array().unwrap();
        let away = log["roster"][1].as_array().unwrap();
        let member = |side: &[serde_json::Value], pid: &str| {
            side.iter().any(|r| r[0] == pid)
        };

        // h5 belongs to the home XI: one away-team row must not add him to
        // the opposition roster, nor duplicate him in his own.
        assert!(member(home, "h5"), "h5 stays in his own XI");
        assert!(!member(away, "h5"), "h5 is not filed under the flip");
        assert_eq!(home.iter().filter(|r| r[0] == "h5").count(), 1, "no duplicate");
        assert_eq!(away.len(), 11, "away XI only — no fallback additions");

        // z9 never had a lineup row: majority team (2 home vs 1 away) wins,
        // exactly once, and never under both sides.
        assert!(member(home, "z9"), "z9 filed under his majority team");
        assert!(!member(away, "z9"), "z9 is not filed under the minority team");
        assert_eq!(home.iter().filter(|r| r[0] == "z9").count(), 1, "filed once");
        let z9 = home.iter().find(|r| r[0] == "z9").unwrap();
        assert_eq!(z9[1], "Z. Nine", "synthetic row still carries the name");
        assert_eq!(z9[2], 0, "unknown shirt");
    }
}
