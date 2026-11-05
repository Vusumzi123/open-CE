//! Minimal engine-side smoke test:
//! loads a game directory, its archives and a master plugin, then counts cells
//! and worldspaces from the plugin's own records. No game assets are bundled.

use std::error::Error;

use oce_data::Game;
use oce_plugin::{Record, WalkEntry};

fn main() -> Result<(), Box<dyn Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: game_summary <game-directory>")?;
    let mut game = Game::open(root)?;
    println!("data dir: {}", game.data.path().display());
    println!("load order entries: {}", game.load_order.entries.len());

    let report = game.load_archives()?;
    println!(
        "archives: {} loaded, {} failed",
        report.loaded,
        report.failed.len()
    );

    let plugin = game
        .plugin("Skyrim.esm")?
        .ok_or("Skyrim.esm was not found in the data directory")?;
    let mut cells = 0usize;
    let mut worldspaces = 0usize;
    let mut sample = Vec::new();
    plugin.walk(|entry| {
        if let WalkEntry::Record {
            header,
            body,
            offset,
            ..
        } = entry
        {
            if &header.signature == b"CELL" {
                cells += 1;
            } else if &header.signature == b"WRLD" {
                worldspaces += 1;
            } else {
                return Ok(());
            }
            if sample.len() >= 8 {
                return Ok(());
            }
            let record = Record {
                header,
                body,
                offset,
            };
            if let Ok(decoded) = record.decode() {
                if let Ok(subrecords) = decoded.subrecords() {
                    if let Some(edid) = subrecords
                        .iter()
                        .find(|subrecord| &subrecord.signature == b"EDID")
                        .and_then(|subrecord| subrecord.as_cstring())
                    {
                        sample.push(format!(
                            "{} {}",
                            header.type_name(),
                            String::from_utf8_lossy(edid)
                        ));
                    }
                }
            }
        }
        Ok(())
    })?;

    println!("cells: {cells}, worldspaces: {worldspaces}");
    for line in sample {
        println!("  {line}");
    }
    Ok(())
}
