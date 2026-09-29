//! Background jobs (export today): cancellation and progress, shared by every front end.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Cooperative cancellation flag. Clones share the flag: the UI keeps one and calls
/// [`Self::cancel`], the job checks [`Self::is_cancelled`] between units of work.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the job to stop. Idempotent.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// How far a job has gone, in units of its own choosing (e.g. rows).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_the_cancellation() {
        let token = CancelToken::new();
        let job = token.clone();
        assert!(!job.is_cancelled());
        token.cancel();
        assert!(job.is_cancelled());
        token.cancel();
        assert!(job.is_cancelled());
        // A fresh token is independent.
        assert!(!CancelToken::default().is_cancelled());
    }

    #[test]
    fn tokens_cross_threads() {
        let token = CancelToken::new();
        let job = token.clone();
        std::thread::spawn(move || token.cancel())
            .join()
            .expect("the cancelling thread does not panic");
        assert!(job.is_cancelled());
    }
}
