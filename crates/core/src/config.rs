//! Persisted settings (JSON in the user's config directory).

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::columns::{Col, View};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrokerConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

impl Default for BrokerConfig {
    fn default() -> Self {
        Self { host: String::new(), port: 1883, username: String::new(), password: String::new() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ColumnSetting {
    pub col: Col,
    pub visible: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub broker: BrokerConfig,
    pub show_offline: bool,
    pub view: View,
    pub columns: BTreeMap<View, Vec<ColumnSetting>>,
}

impl Default for Config {
    fn default() -> Self {
        let mut c = Self {
            broker: BrokerConfig::default(),
            show_offline: true,
            view: View::Home,
            columns: BTreeMap::new(),
        };
        c.normalize();
        c
    }
}

impl Config {
    fn path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "tasmota-derust")
            .map(|d| d.config_dir().join("config.json"))
    }

    pub fn load() -> Self {
        let mut cfg: Config = Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        cfg.normalize();
        cfg
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, s);
        }
    }

    /// Makes sure every view lists every column exactly once.
    fn normalize(&mut self) {
        for view in View::ALL {
            let list = self.columns.entry(view).or_insert_with(|| default_columns(view));
            let mut seen = Vec::new();
            list.retain(|c| {
                let fresh = !seen.contains(&c.col);
                seen.push(c.col);
                fresh
            });
            for col in Col::ALL {
                if !list.iter().any(|c| c.col == col) {
                    list.push(ColumnSetting { col, visible: false });
                }
            }
        }
    }

    pub fn reset_columns(&mut self, view: View) {
        self.columns.insert(view, default_columns(view));
        self.normalize();
    }

    pub fn visible_columns(&self, view: View) -> Vec<Col> {
        self.columns
            .get(&view)
            .map(|l| l.iter().filter(|c| c.visible).map(|c| c.col).collect())
            .unwrap_or_default()
    }
}

fn default_columns(view: View) -> Vec<ColumnSetting> {
    view.default_columns().iter().map(|&col| ColumnSetting { col, visible: true }).collect()
}
