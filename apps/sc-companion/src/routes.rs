//! Route planner setup kept between sessions: the pilot's ships and the planning
//! settings, in `planner.json` next to the saved contracts. No egui here.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sc_planner::{Settings, Ship};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RouteSetup {
    /// Never empty once loaded.
    pub ships: Vec<Ship>,
    /// Index into `ships` of the ship in use.
    pub ship: usize,
    pub settings: Settings,
}

impl Default for RouteSetup {
    fn default() -> Self {
        Self {
            ships: vec![Ship::default()],
            ship: 0,
            settings: Settings::default(),
        }
    }
}

impl RouteSetup {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("planner.json")
    }

    /// The setup saved in `dir`, or the default one (with a warning when the file exists
    /// but can't be read).
    pub fn load(dir: &Path) -> (Self, Option<String>) {
        let path = Self::path(dir);
        let (mut setup, warning) = match std::fs::read_to_string(&path) {
            Ok(json) => match serde_json::from_str(&json) {
                Ok(s) => (s, None),
                Err(e) => (Self::default(), Some(format!("{}: {e}", path.display()))),
            },
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                (Self::default(), Some(format!("{}: {e}", path.display())))
            }
            Err(_) => (Self::default(), None),
        };
        if setup.ships.is_empty() {
            setup.ships.push(Ship::default());
        }
        setup.ship = setup.ship.min(setup.ships.len() - 1);
        (setup, warning)
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = Self::path(dir);
        std::fs::write(&path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// The ship in use.
    pub fn ship(&self) -> &Ship {
        &self.ships[self.ship]
    }

    pub fn ship_mut(&mut self) -> &mut Ship {
        &mut self.ships[self.ship]
    }

    /// Adds a ship and selects it.
    pub fn add_ship(&mut self) {
        self.ships.push(Ship {
            name: format!("Ship {}", self.ships.len() + 1),
            ..Ship::default()
        });
        self.ship = self.ships.len() - 1;
    }

    /// Removes the ship in use, unless it is the last one.
    pub fn remove_ship(&mut self) {
        if self.ships.len() > 1 {
            self.ships.remove(self.ship);
            self.ship = self.ship.saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use sc_planner::Goal;

    use super::*;
    use crate::contracts::ContractStore;

    #[test]
    fn saves_and_loads() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let (mut setup, warning) = RouteSetup::load(dir.path());
        assert_eq!((setup.clone(), warning), (RouteSetup::default(), None));
        setup.add_ship();
        setup.ship_mut().hard_max = Some(66.0);
        setup.settings.start = "Lorville".into();
        setup.settings.goal = Goal::MaxReward {
            budget_minutes: 45.0,
        };
        setup.save(dir.path()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(RouteSetup::load(dir.path()).0, setup);
        // The contract store keeps ignoring the file.
        let (store, warnings) = ContractStore::open(dir.path());
        assert!(
            store.entries().is_empty() && warnings.is_empty(),
            "{warnings:?}"
        );

        setup.remove_ship();
        setup.remove_ship();
        assert_eq!((setup.ships.len(), setup.ship), (1, 0));
    }

    #[test]
    fn unreadable_file_falls_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(RouteSetup::path(dir.path()), r#"{"ships": [], "ship": 4}"#)
            .unwrap_or_else(|e| panic!("{e}"));
        let (setup, warning) = RouteSetup::load(dir.path());
        assert_eq!((setup.ships.len(), setup.ship, warning), (1, 0, None));
        std::fs::write(RouteSetup::path(dir.path()), "not json").unwrap_or_else(|e| panic!("{e}"));
        let (setup, warning) = RouteSetup::load(dir.path());
        assert_eq!(setup, RouteSetup::default());
        assert!(warning.is_some());
    }
}
