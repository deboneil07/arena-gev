//! Generate `web/matches.json` — a manifest of every Opta F24 match
//! bundled in `data/`, so the web UI can list and browse them.
//!
//! Reads each `.xml` fixture, pulls the `<Game>` metadata out of the
//! parsed context, and writes a sorted JSON array to `web/matches.json`.
//!
//! ```text
//! cargo run --release --example gen_manifest
//! ```

use match_engine::parse_opta_f24;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

/// One row in the generated match manifest.
#[derive(Serialize)]
struct MatchEntry {
    /// File name inside `web/matches/` (the browser fetches this).
    file: String,
    match_id: String,
    home_team_name: String,
    away_team_name: String,
    competition_id: String,
    season: String,
    game_date: String,
}

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let data_dir = manifest_dir.join("data");
    let web_dir = manifest_dir.join("web");

    // Collect and sort the bundled fixtures so the list is stable.
    let mut paths: Vec<PathBuf> = Vec::new();
    if let Ok(read) = fs::read_dir(&data_dir) {
        for entry in read.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("xml") {
                paths.push(path);
            }
        }
    }
    paths.sort();

    let mut entries: Vec<MatchEntry> = Vec::new();
    for path in &paths {
        let xml = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("warning: skipping {}: {e}", path.display());
                continue;
            }
        };
        let ctx = match parse_opta_f24(&xml) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("warning: skipping {}: {e}", path.display());
                continue;
            }
        };
        entries.push(MatchEntry {
            file: path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            match_id: ctx.match_id,
            home_team_name: ctx.home_team_name,
            away_team_name: ctx.away_team_name,
            competition_id: ctx.competition_id,
            season: ctx.season,
            game_date: ctx.game_date,
        });
    }

    fs::create_dir_all(&web_dir).expect("create web/");
    let out = serde_json::to_string_pretty(&entries).expect("serialize manifest");
    let dest = web_dir.join("matches.json");
    fs::write(&dest, out).expect("write matches.json");
    println!(
        "wrote {} match(es) to {}",
        entries.len(),
        dest.display()
    );
}
