//! This module bounds the asset files held open while their bytes stream to peers.
//!
//! A streamed response keeps its file descriptor open while its peer reads slowly. A peer that
//! stops reading altogether is cut between the egress stall window and that window plus one
//! sampler tick after its last progress. The descriptor budget (`crate::fd_budget`) counts
//! [`STREAMED_FILE_PERMITS`] descriptors for this purpose, so the server must never hold more:
//! every handler that opens a file to stream it takes a permit from one process-wide semaphore
//! first and keeps it for as long as it holds the file.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::fd_budget::STREAMED_FILE_PERMITS;
use crate::http::error::AppError;

/// How long a request waits for a permit before it is refused. Bounded so the queue of waiting
/// requests is bounded too: only requests that arrive inside one wait can queue.
pub const PERMIT_WAIT: Duration = Duration::from_secs(2);

/// Seconds a refused client is told to wait (`Retry-After`). Chosen to be shorter than
/// [`PERMIT_WAIT`], since a permit that frees within the wait was already offered to a queued
/// request.
const RETRY_AFTER_SECS: u64 = 1;

/// A permit to hold one asset file open for streaming; dropping it returns the permit.
pub type FilePermit = OwnedSemaphorePermit;

/// The process-wide pool of streamed-file permits.
///
/// # Examples
///
/// ```
/// # #[tokio::main] async fn main() {
/// use std::sync::Arc;
/// use std::time::Duration;
/// use shadowcat::http::assets::permits::StreamedFilePermits;
///
/// let permits = Arc::new(StreamedFilePermits::new(1, Duration::from_millis(10)));
/// let first = permits.acquire().await.unwrap();
/// assert!(permits.acquire().await.is_err());
/// drop(first);
/// assert!(permits.acquire().await.is_ok());
/// # }
/// ```
#[derive(Debug)]
pub struct StreamedFilePermits {
    /// The permits; one unit is one open streamed file.
    semaphore: Arc<Semaphore>,
    /// Total permits, which `held` subtracts the free ones from.
    capacity: usize,
    /// How long `acquire` waits before refusing.
    wait: Duration,
}

impl StreamedFilePermits {
    /// A pool of `capacity` permits whose `acquire` waits at most `wait`.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use shadowcat::http::assets::permits::StreamedFilePermits;
    ///
    /// let permits = StreamedFilePermits::new(4, Duration::from_secs(1));
    /// assert_eq!(permits.capacity(), 4);
    /// assert_eq!(permits.held(), 0);
    /// ```
    pub fn new(capacity: usize, wait: Duration) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(capacity)),
            capacity,
            wait,
        }
    }

    /// The pool the server runs with: [`STREAMED_FILE_PERMITS`] permits and a [`PERMIT_WAIT`]
    /// wait, the same constant the descriptor budget counts.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::fd_budget::STREAMED_FILE_PERMITS;
    /// use shadowcat::http::assets::permits::StreamedFilePermits;
    ///
    /// assert_eq!(StreamedFilePermits::production().capacity(), STREAMED_FILE_PERMITS);
    /// ```
    pub fn production() -> Self {
        Self::new(STREAMED_FILE_PERMITS, PERMIT_WAIT)
    }

    /// Takes one permit, waiting at most the pool's wait. Expiry is a 503 with `Retry-After`,
    /// returned before any response header is written.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main] async fn main() {
    /// use std::sync::Arc;
    /// use std::time::Duration;
    /// use shadowcat::http::assets::permits::StreamedFilePermits;
    ///
    /// let permits = Arc::new(StreamedFilePermits::new(2, Duration::from_millis(10)));
    /// let permit = permits.acquire().await.unwrap();
    /// assert_eq!(permits.held(), 1);
    /// drop(permit);
    /// assert_eq!(permits.held(), 0);
    /// # }
    /// ```
    pub async fn acquire(&self) -> Result<FilePermit, AppError> {
        let acquired = tokio::time::timeout(self.wait, self.semaphore.clone().acquire_owned())
            .await
            .map_err(|_| AppError::Unavailable {
                retry_after_secs: RETRY_AFTER_SECS,
            })?;
        acquired.map_err(|e| {
            tracing::error!(?e, "streamed-file permit pool closed");
            AppError::Internal
        })
    }

    /// Permits currently held.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use shadowcat::http::assets::permits::StreamedFilePermits;
    ///
    /// assert_eq!(StreamedFilePermits::new(3, Duration::ZERO).held(), 0);
    /// ```
    pub fn held(&self) -> usize {
        self.capacity - self.semaphore.available_permits()
    }

    /// Total permits in the pool.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use shadowcat::http::assets::permits::StreamedFilePermits;
    ///
    /// assert_eq!(StreamedFilePermits::new(3, Duration::ZERO).capacity(), 3);
    /// ```
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

#[cfg(test)]
mod tests;
