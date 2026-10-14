//! OCR on a background thread: screenshots are read one at a time, in the order they were
//! submitted, so the UI never blocks (the first read also downloads and loads the models).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use eframe::egui;

use crate::contracts::{ContractReader, ShotText};

/// A finished screenshot.
#[derive(Debug)]
pub enum Outcome {
    Read(Box<ShotText>),
    Failed { path: PathBuf, error: String },
}

#[derive(Debug)]
enum Event {
    Status(String),
    Done(Outcome),
}

/// Queues screenshots from any thread (the capture hotkey's).
#[derive(Debug, Clone)]
pub struct Submitter {
    jobs: Sender<PathBuf>,
    /// Submitted and not finished yet.
    pending: Arc<AtomicUsize>,
}

impl Submitter {
    pub fn submit(&self, path: PathBuf) {
        self.pending.fetch_add(1, Ordering::SeqCst);
        // A send only fails if the thread died; `poll` then reports the pending reads.
        let _ = self.jobs.send(path);
    }
}

#[derive(Debug)]
pub struct Worker {
    submitter: Submitter,
    events: Receiver<Event>,
    status: Option<String>,
}

impl Worker {
    /// Starts the thread; `ctx` is repainted whenever something happens.
    pub fn start(ctx: &egui::Context) -> Self {
        let (jobs, job_rx) = mpsc::channel::<PathBuf>();
        let (event_tx, events) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut reader = ContractReader::new();
            let send = |event| {
                // The receiver is gone only if the app closed meanwhile.
                let _ = event_tx.send(event);
                ctx.request_repaint();
            };
            for path in job_rx {
                let result = reader.read(&path, &mut |s| send(Event::Status(s)));
                let outcome = match result {
                    Ok(shot) => Outcome::Read(Box::new(shot)),
                    Err(e) => Outcome::Failed {
                        path,
                        error: format!("{e:#}"),
                    },
                };
                send(Event::Done(outcome));
            }
        });
        Self {
            submitter: Submitter {
                jobs,
                pending: Arc::new(AtomicUsize::new(0)),
            },
            events,
            status: None,
        }
    }

    pub fn submit(&self, path: PathBuf) {
        self.submitter.submit(path);
    }

    /// A handle to queue screenshots from another thread.
    pub fn submitter(&self) -> Submitter {
        self.submitter.clone()
    }

    /// Screenshots finished since the last call.
    pub fn poll(&mut self) -> Vec<Outcome> {
        let mut done = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(Event::Status(s)) => self.status = Some(s),
                Ok(Event::Done(outcome)) => {
                    // Never below zero: a job is counted before it is sent.
                    let _ = self.submitter.pending.fetch_update(
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                        |n| Some(n.saturating_sub(1)),
                    );
                    done.push(outcome);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.submitter.pending.swap(0, Ordering::SeqCst) > 0 {
                        done.push(Outcome::Failed {
                            path: PathBuf::new(),
                            error: "the OCR thread stopped unexpectedly".into(),
                        });
                    }
                    break;
                }
            }
        }
        if self.pending() == 0 {
            self.status = None;
        }
        done
    }

    /// Screenshots waiting or being read.
    pub fn pending(&self) -> usize {
        self.submitter.pending.load(Ordering::SeqCst)
    }

    /// What the thread is doing now (while busy).
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }
}
