use std::time::{SystemTime, UNIX_EPOCH};

pub(super) trait RelayerClock: Send + Sync {
    fn now_unix(&self) -> u64;
}

pub(super) struct SystemClock;

impl RelayerClock for SystemClock {
    fn now_unix(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs())
    }
}
