//! The capture hotkey, detected by polling the keyboard state. `RegisterHotKey` doesn't
//! fire while Star Citizen has focus: games that read the keyboard through Raw Input with
//! `RIDEV_NOHOTKEYS` make Windows drop application hotkeys, and an elevated game would block
//! them too. `GetAsyncKeyState` (through `device_query`) reads the key state itself, so
//! neither applies. A thread polls it, captures the screen on the rising edge and queues the
//! shot for OCR, so a capture works while the app is in the background or minimized. The
//! key is not swallowed: the game sees it too.

// Outside Windows nothing starts the thread, so most of this is unused there.
#![cfg_attr(not(windows), allow(dead_code))]

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

use eframe::egui;

use super::worker::Submitter;

/// The combination, as shown to the player.
pub const LABEL: &str = "Ctrl+Shift+C";

/// What a press of the hotkey did.
#[derive(Debug)]
pub enum Capture {
    /// Saved there and queued for OCR.
    Saved(PathBuf),
    Failed(String),
}

/// Turns "held down" samples into presses: true once per press, however long it is held.
#[derive(Debug, Default)]
struct Edge {
    was: bool,
}

impl Edge {
    fn rising(&mut self, down: bool) -> bool {
        let rising = down && !self.was;
        self.was = down;
        rising
    }
}

#[cfg(windows)]
fn down(keys: &[device_query::Keycode]) -> bool {
    use device_query::Keycode as K;
    let held = |a: K, b: K| keys.contains(&a) || keys.contains(&b);
    held(K::LControl, K::RControl) && held(K::LShift, K::RShift) && keys.contains(&K::C)
}

/// The polling thread's end; dropping it stops the thread at its next press.
#[derive(Debug)]
pub struct Hotkey {
    events: Receiver<Capture>,
}

impl Hotkey {
    /// Starts polling; shots go to `shots_dir` and then to `submitter`. `None` where the
    /// hotkey isn't supported (outside Windows).
    #[cfg(windows)]
    pub fn start(ctx: &egui::Context, shots_dir: PathBuf, submitter: Submitter) -> Option<Self> {
        use device_query::{DeviceQuery, DeviceState};
        use std::time::Duration;

        use crate::contracts::capture;

        const POLL: Duration = Duration::from_millis(30);

        let (tx, events) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let keyboard = DeviceState::new();
            let mut edge = Edge::default();
            loop {
                std::thread::sleep(POLL);
                if !edge.rising(down(&keyboard.get_keys())) {
                    continue;
                }
                let event = match capture::grab(&shots_dir) {
                    Ok(path) => {
                        submitter.submit(path.clone());
                        Capture::Saved(path)
                    }
                    Err(e) => Capture::Failed(format!("{e:#}")),
                };
                if tx.send(event).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        });
        Some(Self { events })
    }

    #[cfg(not(windows))]
    pub fn start(_ctx: &egui::Context, _shots_dir: PathBuf, _submitter: Submitter) -> Option<Self> {
        None
    }

    /// Presses handled since the last call.
    pub fn poll(&self) -> Vec<Capture> {
        let mut out = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(c) => out.push(c),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return out,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holding_fires_once_and_release_rearms() {
        let mut edge = Edge::default();
        let fired: Vec<bool> = [false, true, true, true, false, true]
            .into_iter()
            .map(|d| edge.rising(d))
            .collect();
        assert_eq!(fired, [false, true, false, false, false, true]);
    }

    #[cfg(windows)]
    #[test]
    fn needs_ctrl_shift_and_c() {
        use device_query::Keycode as K;
        assert!(down(&[K::LControl, K::RShift, K::C]));
        assert!(down(&[K::C, K::RControl, K::LShift, K::A]));
        assert!(!down(&[K::LControl, K::C]));
        assert!(!down(&[K::LControl, K::LShift]));
        assert!(!down(&[K::LAlt, K::LShift, K::C]));
    }
}
