use std::{sync::Arc, time::Duration};

use tokio::{
    sync::{Mutex, watch},
    time::sleep,
};

use crate::DownloadEngineError;

#[derive(Clone, Debug)]
pub struct BandwidthLimiter {
    bytes_per_second: u64,
    state: Arc<Mutex<BucketState>>,
}

#[derive(Debug)]
struct BucketState {
    tokens: u64,
    last_refill: tokio::time::Instant,
}

impl BandwidthLimiter {
    pub fn new(bytes_per_second: u64) -> Result<Self, DownloadEngineError> {
        if bytes_per_second == 0 || bytes_per_second > 1_000_000_000_000 {
            return Err(DownloadEngineError::InvalidConfiguration);
        }
        Ok(Self {
            bytes_per_second,
            state: Arc::new(Mutex::new(BucketState {
                tokens: bytes_per_second,
                last_refill: tokio::time::Instant::now(),
            })),
        })
    }

    pub async fn acquire(
        &self,
        mut bytes: u64,
        mut cancellation: watch::Receiver<bool>,
        mut pause: watch::Receiver<bool>,
    ) -> Result<(), DownloadEngineError> {
        while bytes > 0 {
            if *cancellation.borrow_and_update() {
                return Err(DownloadEngineError::Cancelled);
            }
            if *pause.borrow_and_update() {
                return Err(DownloadEngineError::Paused);
            }

            let wait = {
                let mut state = self.state.lock().await;
                let now = tokio::time::Instant::now();
                let elapsed = now.duration_since(state.last_refill);
                let replenished = elapsed
                    .as_nanos()
                    .saturating_mul(u128::from(self.bytes_per_second))
                    / 1_000_000_000;
                let replenished = u64::try_from(replenished).unwrap_or(u64::MAX);
                if replenished > 0 {
                    state.tokens = state
                        .tokens
                        .saturating_add(replenished)
                        .min(self.bytes_per_second);
                    state.last_refill = now;
                }

                let consumed = bytes.min(state.tokens);
                bytes -= consumed;
                state.tokens -= consumed;
                if bytes == 0 {
                    continue;
                }

                let missing = bytes.min(self.bytes_per_second).max(1);
                let nanos = u128::from(missing)
                    .saturating_mul(1_000_000_000)
                    .div_ceil(u128::from(self.bytes_per_second));
                let nanos = u64::try_from(nanos).unwrap_or(u64::MAX);
                Duration::from_nanos(nanos.max(1))
            };

            tokio::select! {
                _ = sleep(wait) => {}
                changed = cancellation.changed() => {
                    if changed.is_err() || *cancellation.borrow_and_update() {
                        return Err(DownloadEngineError::Cancelled);
                    }
                }
                changed = pause.changed() => {
                    if changed.is_err() || *pause.borrow_and_update() {
                        return Err(DownloadEngineError::Paused);
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct BandwidthControl {
    pub global: Option<Arc<BandwidthLimiter>>,
    pub per_download: Option<Arc<BandwidthLimiter>>,
    pub engine: Option<Arc<BandwidthLimiter>>,
    pub external_download_rate_bytes_per_second: Option<u64>,
}

impl BandwidthControl {
    pub async fn acquire(
        &self,
        bytes: u64,
        cancellation: watch::Receiver<bool>,
        pause: watch::Receiver<bool>,
    ) -> Result<(), DownloadEngineError> {
        let global = async {
            if let Some(limiter) = &self.global {
                limiter
                    .acquire(bytes, cancellation.clone(), pause.clone())
                    .await?;
            }
            Ok::<_, DownloadEngineError>(())
        };
        let per_download = async {
            if let Some(limiter) = &self.per_download {
                limiter
                    .acquire(bytes, cancellation.clone(), pause.clone())
                    .await?;
            }
            Ok::<_, DownloadEngineError>(())
        };
        let engine = async {
            if let Some(limiter) = &self.engine {
                limiter
                    .acquire(bytes, cancellation.clone(), pause.clone())
                    .await?;
            }
            Ok::<_, DownloadEngineError>(())
        };
        tokio::try_join!(global, per_download, engine)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use tokio::sync::watch;

    use super::BandwidthLimiter;
    use crate::DownloadEngineError;

    #[tokio::test]
    async fn shared_limiter_bounds_concurrent_transfers() -> Result<(), DownloadEngineError> {
        let limiter = BandwidthLimiter::new(32 * 1024)?;
        let (_cancel_tx, cancel) = watch::channel(false);
        let (_pause_tx, pause) = watch::channel(false);
        let start = Instant::now();
        let first = limiter.acquire(32 * 1024, cancel.clone(), pause.clone());
        let second = limiter.acquire(32 * 1024, cancel, pause);
        tokio::try_join!(first, second)?;
        assert!(start.elapsed() >= Duration::from_millis(700));
        Ok(())
    }

    #[tokio::test]
    async fn cancellation_interrupts_bandwidth_wait() -> Result<(), DownloadEngineError> {
        let limiter = BandwidthLimiter::new(1024)?;
        let (cancel_tx, cancel) = watch::channel(false);
        let (_pause_tx, pause) = watch::channel(false);
        limiter.acquire(1024, cancel.clone(), pause.clone()).await?;
        let task = tokio::spawn(async move { limiter.acquire(1024, cancel, pause).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancel_tx
            .send(true)
            .map_err(|_| DownloadEngineError::Cancelled)?;
        assert!(matches!(
            task.await.expect("limiter task should finish"),
            Err(DownloadEngineError::Cancelled)
        ));
        Ok(())
    }
}
