//! Integration tests for the WASM bridge core against the real match file.
//!
//! The pure `MatchRenderer` is the exact code path the browser/WASM wrapper
//! uses, so these tests exercise the flat render buffer end to end.

use match_engine::wasm::{MatchRenderer, ENTITY_STRIDE, TOTAL_ENTITIES};
use std::path::PathBuf;

fn real_xml() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join("2647319.xml");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

#[test]
fn renderer_seek_jumps_to_requested_time() {
    let mut r = MatchRenderer::from_xml(&real_xml()).expect("parse");
    assert!(r.seek_to(1200.0));
    assert!(
        r.sim_time() >= 1199.0 && r.sim_time() < 1290.0,
        "seek landed at {}",
        r.sim_time()
    );
    // Seeking backwards is a rebuild + fast-forward, and must stay valid.
    assert!(r.seek_to(300.0));
    assert!(r.sim_time() >= 299.0 && r.sim_time() < 400.0);
    r.step();
    assert!(r.buffer().iter().all(|v| v.is_finite()));
}

#[test]
fn renderer_tracks_score_and_possession() {
    let mut r = MatchRenderer::from_xml(&real_xml()).expect("parse");
    let end = r.engine().timeline.duration_secs() + 1.0;
    while r.sim_time() < end {
        r.step();
    }
    let score = r.score();
    assert_eq!(score[0] + score[1], 6, "fixture has 6 goals");
    let poss = r.possession_home();
    assert!((0.0..=1.0).contains(&poss));
}

#[test]
fn renderer_builds_from_real_match() {
    let r = MatchRenderer::from_xml(&real_xml()).expect("parse");
    assert_eq!(r.entity_stride(), ENTITY_STRIDE);
    assert_eq!(r.total_entities(), TOTAL_ENTITIES);
    assert_eq!(r.player_count(), 22);
    assert_eq!(r.buffer().len(), TOTAL_ENTITIES * ENTITY_STRIDE);
}

#[test]
fn renderer_streams_valid_frames_with_ball_pinned_last() {
    let mut r = MatchRenderer::from_xml(&real_xml()).expect("parse");
    r.run_for(30.0);
    let buf = r.buffer();
    assert!(buf.iter().all(|v| v.is_finite()));

    // Exactly 11 home + 11 away across the player slots, ball in the last slot.
    let teams: Vec<f32> = (0..TOTAL_ENTITIES).map(|i| buf[i * ENTITY_STRIDE + 4]).collect();
    assert_eq!(teams.iter().filter(|t| **t == 0.0).count(), 11);
    assert_eq!(teams.iter().filter(|t| **t == 1.0).count(), 11);
    assert_eq!(teams.iter().filter(|t| **t == 2.0).count(), 1);
    assert_eq!(buf[(TOTAL_ENTITIES - 1) * ENTITY_STRIDE + 4], 2.0);
}

#[test]
fn renderer_pointer_is_stable_across_the_whole_match() {
    let mut r = MatchRenderer::from_xml(&real_xml()).expect("parse");
    let p0 = r.buffer_ptr();
    // Advance far enough that lots of Rust allocations happen.
    r.run_for(600.0);
    assert_eq!(p0, r.buffer_ptr(), "render buffer moved; JS view would detach");
}

#[test]
fn renderer_full_match_is_consistent_and_lands_the_ball() {
    let mut r = MatchRenderer::from_xml(&real_xml()).expect("parse");
    let mut max_alt = 0.0f32;
    let end = r.engine().timeline.duration_secs() + 2.0;
    while r.sim_time() < end {
        r.step();
        max_alt = max_alt.max(r.buffer()[(TOTAL_ENTITIES - 1) * ENTITY_STRIDE + 2]);
    }
    assert!(r.is_finished(), "match did not finish");
    assert!(max_alt > 2.0, "no meaningful aerial play (max {max_alt})");
    assert!(r.buffer().iter().all(|v| v.is_finite()));
    for i in 0..TOTAL_ENTITIES {
        assert!(r.buffer()[i * ENTITY_STRIDE + 2] >= 0.0);
    }
}
