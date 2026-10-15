//! Background thread that runs `git status` on every repo, so the UI never blocks
//! on a slow repo. It rescans on a timer and whenever asked.

use crate::git::{self, RepoStatus};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

const INTERVAL: Duration = Duration::from_secs(3);

pub type ScanResult = Vec<(PathBuf, Result<RepoStatus, String>)>;

pub struct Scanner {
    wake: Sender<()>,
    pub results: Receiver<ScanResult>,
}

impl Scanner {
    pub fn start(repos: Vec<PathBuf>) -> Self {
        let (wake, wake_rx) = mpsc::channel::<()>();
        let (tx, results) = mpsc::channel();
        // `move` hands ownership of `repos`, `wake_rx` and `tx` to the thread.
        thread::spawn(move || loop {
            let scan: ScanResult = repos
                .iter()
                .map(|r| (r.clone(), git::status(r).map_err(|e| e.to_string())))
                .collect();
            if tx.send(scan).is_err() {
                return; // UI is gone
            }
            match wake_rx.recv_timeout(INTERVAL) {
                Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        });
        Scanner { wake, results }
    }

    pub fn request(&self) {
        let _ = self.wake.send(());
    }
}
