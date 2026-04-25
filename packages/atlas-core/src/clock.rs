use time::OffsetDateTime;

/// Shared clock boundary for Atlas code that needs "now".
///
/// Library code should depend on this trait instead of calling
/// `OffsetDateTime::now_utc()` directly so tests can inject deterministic time.
pub trait Clock {
    fn now_utc(&self) -> OffsetDateTime;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_utc(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FixedClock {
    now: OffsetDateTime,
}

impl FixedClock {
    pub const fn new(now: OffsetDateTime) -> Self {
        Self { now }
    }
}

impl Clock for FixedClock {
    fn now_utc(&self) -> OffsetDateTime {
        self.now
    }
}

pub fn now_utc() -> OffsetDateTime {
    SystemClock.now_utc()
}

pub fn format_rfc3339(ts: OffsetDateTime) -> String {
    ts.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_clock_returns_injected_time() {
        let ts = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let clock = FixedClock::new(ts);
        assert_eq!(clock.now_utc(), ts);
    }

    #[test]
    fn format_rfc3339_formats_stable_timestamp() {
        let ts = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        assert_eq!(format_rfc3339(ts), "2023-11-14T22:13:20Z");
    }
}
