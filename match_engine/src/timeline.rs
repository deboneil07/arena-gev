//! Time-indexed priority buffer for match events.
//!
//! This is the "Lookahead Playback Buffer" from the engine architecture.
//! Events are indexed by [`NormalizedEvent::match_time_secs`] and can be
//! queried in chronological order or within a time window, enabling the
//! simulation to pre-signal upcoming player intents 3–5 seconds before
//! execution.
//!
//! All range queries return **borrowed slices** (no allocation) and use
//! partition-point search, so they are correct even when many events share
//! an identical timestamp.

use crate::parser::NormalizedEvent;
use std::ops::Range;

// ── Timeline ────────────────────────────────────────────────

/// A time-indexed, sorted buffer of normalized match events.
///
/// Events are kept in ascending order by `match_time_secs`. Events with equal
/// timestamps preserve their insertion order.
#[derive(Debug, Clone, Default)]
pub struct Timeline {
    events: Vec<NormalizedEvent>,
}

impl Timeline {
    // ── Construction ───────────────────────────────────────

    /// Create an empty timeline.
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Create a timeline from a pre-sorted event list.
    ///
    /// # Panics
    /// Panics if the input is not sorted by `match_time_secs` (checked in
    /// every build, not just debug, because this runs once per match).
    pub fn from_sorted(events: Vec<NormalizedEvent>) -> Self {
        assert!(
            events
                .windows(2)
                .all(|w| w[0].match_time_secs <= w[1].match_time_secs),
            "events must be sorted by match_time_secs"
        );
        Self { events }
    }

    /// Build a timeline from raw events (sorts internally, stable).
    pub fn from_events(mut events: Vec<NormalizedEvent>) -> Self {
        events.sort_by(|a, b| {
            a.match_time_secs
                .partial_cmp(&b.match_time_secs)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Self { events }
    }

    // ── Insertion ──────────────────────────────────────────

    /// Insert a single event while maintaining time order.
    ///
    /// Duplicate timestamps append after existing ones, preserving order.
    /// `Vec::insert` is O(n); prefer [`Timeline::extend`] for bulk loading.
    pub fn insert(&mut self, event: NormalizedEvent) {
        let idx = self.partition_start(event.match_time_secs, true);
        self.events.insert(idx, event);
    }

    /// Extend the timeline with many events, sorting once at the end.
    pub fn extend<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = NormalizedEvent>,
    {
        self.events.extend(iter);
        self.events.sort_by(|a, b| {
            a.match_time_secs
                .partial_cmp(&b.match_time_secs)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    // ── Binary-search helpers ──────────────────────────────

    /// First index whose timestamp is `>= time` (`inclusive == false`) or
    /// `> time` (`inclusive == true`).
    #[inline]
    fn partition_start(&self, time: f32, inclusive: bool) -> usize {
        if inclusive {
            self.events
                .partition_point(|e| e.match_time_secs <= time)
        } else {
            self.events
                .partition_point(|e| e.match_time_secs < time)
        }
    }

    // ── Queries ────────────────────────────────────────────

    /// Total number of events in the timeline.
    #[inline]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// All events (immutable slice).
    #[inline]
    pub fn events(&self) -> &[NormalizedEvent] {
        &self.events
    }

    /// First event with `time >= t`. O(log n).
    #[inline]
    pub fn next_at_or_after(&self, time: f32) -> Option<&NormalizedEvent> {
        self.events.get(self.partition_start(time, false))
    }

    /// All events within the half-open interval `[start, end)`. O(log n).
    ///
    /// Returns a borrowed slice — no allocation. Correct for duplicate
    /// timestamps at either boundary.
    pub fn range(&self, range: Range<f32>) -> &[NormalizedEvent] {
        let start = self.partition_start(range.start, false);
        let end = self.partition_start(range.end, false);
        &self.events[start..end.max(start)]
    }

    /// Lookahead: all events in `[from_time, from_time + window_secs)`.
    ///
    /// This is the core lookahead query used by the simulation engine.
    #[inline]
    pub fn lookahead(&self, from_time: f32, window_secs: f32) -> &[NormalizedEvent] {
        self.range(from_time..from_time + window_secs)
    }

    /// All events exactly at `time`. O(log n) via partition search.
    pub fn at(&self, time: f32) -> &[NormalizedEvent] {
        let start = self.partition_start(time, false);
        let end = self.partition_start(time, true);
        &self.events[start..end.max(start)]
    }

    // ── Stats ──────────────────────────────────────────────

    /// Total match duration in seconds (last event time).
    pub fn duration_secs(&self) -> f32 {
        self.events.last().map(|e| e.match_time_secs).unwrap_or(0.0)
    }
}

// ── Unit Tests ──────────────────────────────────────────────

#[cfg(test)]
mod timeline_tests {
    use super::*;
    use crate::parser::{AttackDirection, MatchContext, NormalizedEvent};

    fn make_event(id: u64, time_secs: f32, type_id: u16) -> NormalizedEvent {
        NormalizedEvent {
            id,
            type_id,
            period_id: 1,
            match_time_secs: time_secs,
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
        }
    }

    #[test]
    fn test_timeline_empty() {
        let t = Timeline::new();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
        assert!(t.next_at_or_after(0.0).is_none());
        assert!(t.range(0.0..10.0).is_empty());
        assert!(t.lookahead(0.0, 5.0).is_empty());
    }

    #[test]
    fn test_timeline_insert_sorted() {
        let mut t = Timeline::new();
        t.insert(make_event(1, 50.0, 1));
        t.insert(make_event(2, 10.0, 1));
        t.insert(make_event(3, 30.0, 1));
        t.insert(make_event(4, 70.0, 1));
        t.insert(make_event(5, 20.0, 1));

        let times: Vec<f32> = t.events().iter().map(|e| e.match_time_secs).collect();
        assert_eq!(times, vec![10.0, 20.0, 30.0, 50.0, 70.0]);
    }

    #[test]
    fn test_timeline_extend() {
        let mut t = Timeline::new();
        t.extend(vec![
            make_event(1, 50.0, 1),
            make_event(2, 10.0, 1),
            make_event(3, 30.0, 1),
        ]);
        let times: Vec<f32> = t.events().iter().map(|e| e.match_time_secs).collect();
        assert_eq!(times, vec![10.0, 30.0, 50.0]);
    }

    #[test]
    fn test_timeline_from_sorted() {
        let t = Timeline::from_sorted(vec![
            make_event(1, 10.0, 1),
            make_event(2, 20.0, 1),
            make_event(3, 30.0, 1),
        ]);
        assert_eq!(t.len(), 3);
    }

    #[test]
    #[should_panic(expected = "events must be sorted")]
    fn test_from_sorted_panics_on_unsorted() {
        // debug_assert compiles to a panic in test (debug) builds.
        Timeline::from_sorted(vec![make_event(1, 30.0, 1), make_event(2, 10.0, 1)]);
    }

    #[test]
    fn test_next_at_or_after() {
        let mut t = Timeline::new();
        t.insert(make_event(1, 10.0, 1));
        t.insert(make_event(2, 30.0, 1));
        t.insert(make_event(3, 50.0, 1));

        assert_eq!(t.next_at_or_after(30.0).unwrap().id, 2);
        assert_eq!(t.next_at_or_after(25.0).unwrap().id, 2);
        assert_eq!(t.next_at_or_after(5.0).unwrap().id, 1);
        assert!(t.next_at_or_after(100.0).is_none());
    }

    #[test]
    fn test_range_query() {
        let mut t = Timeline::new();
        for i in 0..10 {
            t.insert(make_event(i, i as f32 * 10.0, 1));
        }
        let ids: Vec<u64> = t.range(25.0..55.0).iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![3, 4, 5]);
    }

    #[test]
    fn test_range_half_open_boundary_with_duplicates() {
        // Regression: duplicate timestamps on the boundary must be handled.
        let mut t = Timeline::new();
        t.extend(vec![
            make_event(1, 20.0, 1),
            make_event(2, 20.0, 1),
            make_event(3, 20.0, 1),
            make_event(4, 30.0, 1),
            make_event(5, 30.0, 1),
        ]);

        // [20, 30) must include all three 20s, and none of the 30s.
        let ids: Vec<u64> = t.range(20.0..30.0).iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![1, 2, 3], "half-open range must exclude end");

        // [20, 31) includes everything.
        let ids: Vec<u64> = t.range(20.0..31.0).iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![1, 2, 3, 4, 5]);

        // [21, 30) includes only the 30s (none, exclusive end).
        assert!(t.range(21.0..30.0).is_empty());
    }

    #[test]
    fn test_exact_boundary_duplicates_start() {
        // Start boundary sitting in the middle of a run of duplicates must
        // include the whole run.
        let mut t = Timeline::new();
        t.extend((1..=5).map(|i| make_event(i, 50.0, 1)));
        let ids: Vec<u64> = t.range(50.0..60.0).iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_lookahead() {
        let mut t = Timeline::new();
        for i in 0..20 {
            t.insert(make_event(i, i as f32 * 5.0, 1));
        }
        // [50, 65) → events at 50, 55, 60
        let ids: Vec<u64> = t.lookahead(50.0, 15.0).iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![10, 11, 12]);
    }

    #[test]
    fn test_at_exact_time() {
        let mut t = Timeline::new();
        t.insert(make_event(1, 30.0, 1));
        t.insert(make_event(2, 30.0, 2));
        t.insert(make_event(3, 30.0, 3));
        t.insert(make_event(4, 40.0, 1));

        let ids: Vec<u64> = t.at(30.0).iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![1, 2, 3]);

        assert_eq!(t.at(40.0).len(), 1);
        assert!(t.at(35.0).is_empty());
    }

    #[test]
    fn test_duration() {
        let mut t = Timeline::new();
        t.insert(make_event(1, 0.0, 1));
        t.insert(make_event(2, 900.0, 1));
        assert!((t.duration_secs() - 900.0).abs() < 0.001);
    }

    #[test]
    fn test_timeline_from_match_context() {
        let mut ctx = MatchContext::default();
        ctx.directions
            .insert("t1".into(), AttackDirection::LeftToRight);
        ctx.events.push(make_event(1, 60.0, 1));
        ctx.events.push(make_event(2, 120.0, 2));

        let t = Timeline::from_events(ctx.events);
        assert_eq!(t.len(), 2);
        assert_eq!(t.events().first().unwrap().id, 1);
        assert_eq!(t.events().last().unwrap().id, 2);
    }

    #[test]
    fn test_lookahead_3_to_5_seconds() {
        let mut t = Timeline::new();
        for i in 0..24 {
            t.insert(make_event(i, i as f32 * 5.0, 1));
        }
        let w3 = t.lookahead(50.0, 3.0);
        let w5 = t.lookahead(50.0, 5.0);
        assert!(w3.len() <= w5.len());
        for ev in w3 {
            assert!(w5.iter().any(|e| e.id == ev.id));
        }
    }
}
