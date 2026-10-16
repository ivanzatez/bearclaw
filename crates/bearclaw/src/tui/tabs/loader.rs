//! One background fetch at a time, read back without blocking the loop.

use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

/// A tab's data source: [`start`](Loader::start) runs a fetch on its own thread, the loop
/// calls [`poll`](Loader::poll) every frame. A failed fetch keeps the last good data, so a
/// tab can show it alongside the error.
pub struct Loader<T> {
    pub data: Option<T>,
    pub error: Option<String>,
    pub loading: bool,
    fetched_at: Option<Instant>,
    rx: Option<Receiver<Result<T, String>>>,
}

impl<T: Send + 'static> Default for Loader<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send + 'static> Loader<T> {
    pub fn new() -> Self {
        Loader {
            data: None,
            error: None,
            loading: false,
            fetched_at: None,
            rx: None,
        }
    }

    /// Run `f` on a background thread. A no-op while a fetch is in flight.
    pub fn start(&mut self, f: impl FnOnce() -> Result<T, String> + Send + 'static) {
        if self.loading {
            return;
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        self.rx = Some(rx);
        self.loading = true;
    }

    /// Take the fetch's result if it has arrived. True when something changed.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        let got = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err("the fetch thread died".into()),
        };
        match got {
            Ok(v) => {
                self.data = Some(v);
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
        self.rx = None;
        self.loading = false;
        self.fetched_at = Some(Instant::now());
        true
    }

    /// Never fetched, or the last fetch is older than `ttl`.
    pub fn stale(&self, ttl: Duration) -> bool {
        self.fetched_at.is_none_or(|t| t.elapsed() >= ttl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle<T: Send + 'static>(l: &mut Loader<T>) {
        let until = Instant::now() + Duration::from_secs(2);
        while l.loading && Instant::now() < until {
            l.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn loader_delivers_and_keeps_data_on_error() {
        let mut l: Loader<u32> = Loader::new();
        assert!(l.stale(Duration::from_secs(60)));
        l.start(|| Ok(5));
        assert!(l.loading);
        settle(&mut l);
        assert_eq!(l.data, Some(5));
        assert!(!l.loading);
        assert!(!l.stale(Duration::from_secs(60)));

        l.start(|| Err("x".into()));
        settle(&mut l);
        assert_eq!(l.data, Some(5));
        assert_eq!(l.error.as_deref(), Some("x"));
    }
}
