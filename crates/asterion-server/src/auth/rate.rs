use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::AuthError;

const WINDOW: Duration = Duration::from_secs(60);
const PER_PEER: u8 = 8;
const TOTAL: u8 = 40;

pub(super) struct LoginThrottle {
    window: Mutex<LoginWindow>,
    hash_slots: Arc<Semaphore>,
}

struct LoginWindow {
    started_at: Instant,
    attempts: u8,
    by_peer: HashMap<Option<IpAddr>, u8>,
}

impl LoginWindow {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            attempts: 0,
            by_peer: HashMap::new(),
        }
    }
}

impl LoginThrottle {
    pub(super) fn new() -> Self {
        Self {
            window: Mutex::new(LoginWindow::new()),
            hash_slots: Arc::new(Semaphore::new(2)),
        }
    }

    pub(super) fn acquire(&self, peer: Option<IpAddr>) -> Result<OwnedSemaphorePermit, AuthError> {
        let mut window = self.window.lock().map_err(|_| AuthError::Internal)?;
        let elapsed = window.started_at.elapsed();
        if elapsed >= WINDOW {
            *window = LoginWindow::new();
        }

        if window.attempts >= TOTAL || window.by_peer.get(&peer).copied().unwrap_or(0) >= PER_PEER {
            let retry_after = WINDOW.as_secs().saturating_sub(elapsed.as_secs()).max(1);
            return Err(AuthError::RateLimited(retry_after));
        }

        let permit = Arc::clone(&self.hash_slots)
            .try_acquire_owned()
            .map_err(|_| AuthError::RateLimited(1))?;
        window.attempts += 1;
        *window.by_peer.entry(peer).or_default() += 1;
        Ok(permit)
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::*;

    #[test]
    fn restricts_concurrency_peer_and_total_attempts() {
        let throttle = LoginThrottle::new();
        let permits = [
            throttle.acquire(None).unwrap(),
            throttle.acquire(None).unwrap(),
        ];
        assert!(matches!(
            throttle.acquire(None),
            Err(AuthError::RateLimited(1))
        ));
        drop(permits);

        for _ in 2..PER_PEER {
            drop(throttle.acquire(None).unwrap());
        }
        assert!(matches!(
            throttle.acquire(None),
            Err(AuthError::RateLimited(_))
        ));

        for host in 1..=TOTAL - PER_PEER {
            let peer = IpAddr::V4(Ipv4Addr::new(192, 0, 2, host));
            drop(throttle.acquire(Some(peer)).unwrap());
        }
        assert!(matches!(
            throttle.acquire(Some(IpAddr::V4(Ipv4Addr::LOCALHOST))),
            Err(AuthError::RateLimited(_))
        ));

        throttle.window.lock().unwrap().started_at -= WINDOW;
        assert!(throttle.acquire(None).is_ok());
    }
}
