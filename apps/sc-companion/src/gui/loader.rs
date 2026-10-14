//! Loads the universe on a background thread so the window opens immediately.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;

use eframe::egui;
use sc_data::{LoadedUniverse, UniverseSource};

use crate::SourceArgs;

#[derive(Debug)]
pub enum LoadState {
    Loading(Receiver<Result<LoadedUniverse, String>>),
    Ready(Arc<LoadedUniverse>),
    Failed(String),
}

impl LoadState {
    /// Starts loading on a new thread; `ctx` is repainted when the result arrives.
    pub fn start(args: &SourceArgs, ctx: &egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        let (args, ctx) = (args.clone(), ctx.clone());
        std::thread::spawn(move || {
            let started = Instant::now();
            let result = args.build().load().map_err(|e| format!("{e:#}"));
            if let Ok(loaded) = &result {
                tracing::info!(
                    "loaded {} locations in {:.0?}",
                    loaded.universe.len(),
                    started.elapsed()
                );
            }
            // The receiver is gone only if the app closed meanwhile.
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        Self::Loading(rx)
    }

    /// Picks up the loader result, if it arrived. Returns true when the state became `Ready`.
    pub fn poll(&mut self) -> bool {
        let Self::Loading(rx) = self else {
            return false;
        };
        match rx.try_recv() {
            Ok(Ok(loaded)) => {
                *self = Self::Ready(Arc::new(loaded));
                true
            }
            Ok(Err(e)) => {
                *self = Self::Failed(e);
                false
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                *self = Self::Failed("the loader thread stopped unexpectedly".into());
                false
            }
        }
    }
}
