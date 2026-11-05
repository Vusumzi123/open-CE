use std::collections::HashMap;

use oce_core::Result;
use oce_plugin::{Plugin, Record, WalkEntry};

pub struct GameSettings {
    values: HashMap<String, f32>,
}

impl GameSettings {
    pub fn load(plugin: &Plugin) -> Result<Self> {
        let mut values = HashMap::new();
        plugin.walk(|entry| {
            let WalkEntry::Record {
                header,
                body,
                offset,
                ..
            } = entry
            else {
                return Ok(());
            };
            if &header.signature != b"GMST" {
                return Ok(());
            }
            let record = Record {
                header,
                body,
                offset,
            };
            let decoded = record.decode()?;
            let subrecords = decoded.subrecords()?;
            let name = subrecords
                .iter()
                .find(|subrecord| &subrecord.signature == b"EDID")
                .and_then(|subrecord| subrecord.as_cstring())
                .and_then(|name| std::str::from_utf8(name).ok());
            let value = subrecords
                .iter()
                .find(|subrecord| &subrecord.signature == b"DATA")
                .and_then(|subrecord| subrecord.as_f32());
            if let (Some(name), Some(value)) = (name, value) {
                values.insert(name.to_ascii_lowercase(), value);
            }
            Ok(())
        })?;
        Ok(Self { values })
    }

    pub fn f32(&self, name: &str, fallback: f32) -> f32 {
        self.values
            .get(&name.to_ascii_lowercase())
            .copied()
            .unwrap_or(fallback)
    }
}
