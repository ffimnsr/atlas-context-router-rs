use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::Notify;

const DEFAULT_EVENT_RETENTION_MS: u64 = 60_000;
const DEFAULT_EVENT_QUEUE_SIZE: usize = 256;
const DEFAULT_SESSION_TTL_MS: u64 = 900_000;
const DEFAULT_POLL_WAIT_MS: u64 = 250;

const HTTP_EVENT_RETENTION_MS_ENV: &str = "ATLAS_HTTP_EVENT_RETENTION_MS";
const HTTP_EVENT_QUEUE_SIZE_ENV: &str = "ATLAS_HTTP_EVENT_QUEUE_SIZE";
const HTTP_SESSION_TTL_MS_ENV: &str = "ATLAS_HTTP_SESSION_TTL_MS";
const HTTP_POLL_WAIT_MS_ENV: &str = "ATLAS_HTTP_POLL_WAIT_MS";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RetainedEvent {
    pub(crate) id: String,
    pub(crate) payload_json: String,
    pub(crate) sequence: u64,
}

#[derive(Clone)]
pub(crate) struct SessionManager {
    retention_window: Duration,
    max_retained_events: usize,
    session_ttl: Duration,
    poll_wait: Duration,
    next_session_counter: Arc<AtomicU64>,
    sessions: Arc<Mutex<HashMap<String, Arc<HttpSession>>>>,
}

impl SessionManager {
    pub(crate) fn from_env() -> Self {
        Self {
            retention_window: Duration::from_millis(read_env_u64(
                HTTP_EVENT_RETENTION_MS_ENV,
                DEFAULT_EVENT_RETENTION_MS,
            )),
            max_retained_events: read_env_usize(
                HTTP_EVENT_QUEUE_SIZE_ENV,
                DEFAULT_EVENT_QUEUE_SIZE,
            )
            .max(1),
            session_ttl: Duration::from_millis(read_env_u64(
                HTTP_SESSION_TTL_MS_ENV,
                DEFAULT_SESSION_TTL_MS,
            )),
            poll_wait: Duration::from_millis(read_env_u64(
                HTTP_POLL_WAIT_MS_ENV,
                DEFAULT_POLL_WAIT_MS,
            )),
            next_session_counter: Arc::new(AtomicU64::new(0)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[doc(hidden)]
    pub(crate) fn for_tests(
        retention_window: Duration,
        max_retained_events: usize,
        session_ttl: Duration,
        poll_wait: Duration,
    ) -> Self {
        Self {
            retention_window,
            max_retained_events: max_retained_events.max(1),
            session_ttl,
            poll_wait,
            next_session_counter: Arc::new(AtomicU64::new(0)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) fn poll_wait(&self) -> Duration {
        self.poll_wait
    }

    pub(crate) fn create_session(
        &self,
        protocol_version: &str,
        client_info: Value,
        client_capabilities: Value,
    ) -> Arc<HttpSession> {
        self.purge_expired_sessions();
        let counter = self.next_session_counter.fetch_add(1, Ordering::Relaxed) + 1;
        let session_id = generate_session_id(protocol_version, &client_info, counter);
        let session = Arc::new(HttpSession::new(
            session_id.clone(),
            protocol_version.to_owned(),
            client_info,
            client_capabilities,
            self.retention_window,
            self.max_retained_events,
            self.session_ttl,
        ));
        self.sessions
            .lock()
            .expect("http session map lock poisoned")
            .insert(session_id, Arc::clone(&session));
        session
    }

    pub(crate) fn require_session(
        &self,
        session_id: &str,
    ) -> Result<Arc<HttpSession>, SessionLookupError> {
        self.purge_expired_sessions();
        let Some(session) = self
            .sessions
            .lock()
            .expect("http session map lock poisoned")
            .get(session_id)
            .cloned()
        else {
            return Err(SessionLookupError::Missing);
        };
        if session.is_closed_or_expired() {
            self.sessions
                .lock()
                .expect("http session map lock poisoned")
                .remove(session_id);
            return Err(SessionLookupError::Missing);
        }
        session.touch();
        Ok(session)
    }

    pub(crate) fn delete_session(&self, session_id: &str) -> bool {
        let removed = self
            .sessions
            .lock()
            .expect("http session map lock poisoned")
            .remove(session_id);
        if let Some(session) = removed {
            session.close();
            true
        } else {
            false
        }
    }

    fn purge_expired_sessions(&self) {
        let mut sessions = self
            .sessions
            .lock()
            .expect("http session map lock poisoned");
        sessions.retain(|_, session| !session.is_closed_or_expired());
    }
}

pub(crate) struct HttpSession {
    id: String,
    protocol_version: String,
    client_info: Value,
    client_capabilities: Value,
    state: Mutex<HttpSessionState>,
    notify: Notify,
    retention_window: Duration,
    max_retained_events: usize,
    session_ttl: Duration,
}

impl HttpSession {
    fn new(
        id: String,
        protocol_version: String,
        client_info: Value,
        client_capabilities: Value,
        retention_window: Duration,
        max_retained_events: usize,
        session_ttl: Duration,
    ) -> Self {
        Self {
            id,
            protocol_version,
            client_info,
            client_capabilities,
            state: Mutex::new(HttpSessionState {
                events: VecDeque::new(),
                next_sequence: 0,
                stream_identity: 0,
                last_event_id: None,
                expires_at: Instant::now() + session_ttl,
                initialized: false,
                log_level: None,
                closed: false,
            }),
            notify: Notify::new(),
            retention_window,
            max_retained_events,
            session_ttl,
        }
    }

    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn protocol_version(&self) -> &str {
        &self.protocol_version
    }

    pub(crate) fn client_info(&self) -> &Value {
        &self.client_info
    }

    pub(crate) fn client_capabilities(&self) -> &Value {
        &self.client_capabilities
    }

    pub(crate) fn touch(&self) {
        let mut state = self.state.lock().expect("http session lock poisoned");
        state.expires_at = Instant::now() + self.session_ttl;
    }

    pub(crate) fn enqueue_event(&self, payload_json: String) -> String {
        let event = {
            let mut state = self.state.lock().expect("http session lock poisoned");
            state.expires_at = Instant::now() + self.session_ttl;
            state.next_sequence += 1;
            let sequence = state.next_sequence;
            let id = format!("{}:{sequence}", self.id);
            let event = StoredEvent {
                retained: RetainedEvent {
                    id: id.clone(),
                    payload_json,
                    sequence,
                },
                recorded_at: Instant::now(),
            };
            state.last_event_id = Some(id.clone());
            state.events.push_back(event);
            trim_events(
                &mut state.events,
                self.retention_window,
                self.max_retained_events,
            );
            id
        };
        self.notify.notify_waiters();
        event
    }

    pub(crate) fn mark_stream_open(&self) {
        let mut state = self.state.lock().expect("http session lock poisoned");
        state.stream_identity += 1;
        state.expires_at = Instant::now() + self.session_ttl;
    }

    pub(crate) fn mark_initialized(&self) {
        let mut state = self.state.lock().expect("http session lock poisoned");
        state.initialized = true;
        state.expires_at = Instant::now() + self.session_ttl;
    }

    pub(crate) fn set_log_level(&self, level: crate::logging::LogLevel) {
        let mut state = self.state.lock().expect("http session lock poisoned");
        state.log_level = Some(level);
        state.expires_at = Instant::now() + self.session_ttl;
    }

    pub(crate) fn should_emit_log(&self, level: crate::logging::LogLevel) -> bool {
        let state = self.state.lock().expect("http session lock poisoned");
        state.initialized && crate::logging::should_emit(state.log_level, level)
    }

    pub(crate) async fn wait_for_events(
        &self,
        last_event_id: Option<&str>,
        poll_wait: Duration,
    ) -> Result<Vec<RetainedEvent>, PollEventsError> {
        match self.events_after(last_event_id)? {
            events if !events.is_empty() => return Ok(events),
            _ => {}
        }

        tokio::time::timeout(poll_wait, self.notify.notified())
            .await
            .ok();
        self.events_after(last_event_id)
    }

    pub(crate) fn events_after(
        &self,
        last_event_id: Option<&str>,
    ) -> Result<Vec<RetainedEvent>, PollEventsError> {
        let mut state = self.state.lock().expect("http session lock poisoned");
        if state.closed {
            return Err(PollEventsError::MissingSession);
        }
        if Instant::now() > state.expires_at {
            state.closed = true;
            return Err(PollEventsError::MissingSession);
        }
        state.expires_at = Instant::now() + self.session_ttl;
        trim_events(
            &mut state.events,
            self.retention_window,
            self.max_retained_events,
        );

        let requested_sequence = match last_event_id {
            Some(id) if !id.trim().is_empty() => parse_event_id(&self.id, id)?,
            _ => 0,
        };
        let earliest_sequence = state
            .events
            .front()
            .map(|event| event.retained.sequence)
            .unwrap_or(state.next_sequence.saturating_add(1));
        if requested_sequence.saturating_add(1) < earliest_sequence {
            return Err(PollEventsError::ResumeWindowExpired);
        }
        Ok(state
            .events
            .iter()
            .filter(|event| event.retained.sequence > requested_sequence)
            .map(|event| event.retained.clone())
            .collect())
    }

    pub(crate) fn close(&self) {
        let mut state = self.state.lock().expect("http session lock poisoned");
        state.closed = true;
        state.events.clear();
        self.notify.notify_waiters();
    }

    fn is_closed_or_expired(&self) -> bool {
        let state = self.state.lock().expect("http session lock poisoned");
        state.closed || Instant::now() > state.expires_at
    }
}

struct HttpSessionState {
    events: VecDeque<StoredEvent>,
    next_sequence: u64,
    stream_identity: u64,
    last_event_id: Option<String>,
    expires_at: Instant,
    initialized: bool,
    log_level: Option<crate::logging::LogLevel>,
    closed: bool,
}

struct StoredEvent {
    retained: RetainedEvent,
    recorded_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionLookupError {
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PollEventsError {
    InvalidEventId,
    MissingSession,
    ResumeWindowExpired,
}

fn trim_events(events: &mut VecDeque<StoredEvent>, retention_window: Duration, max_events: usize) {
    let now = Instant::now();
    while let Some(front) = events.front() {
        if now.duration_since(front.recorded_at) <= retention_window {
            break;
        }
        events.pop_front();
    }
    while events.len() > max_events {
        events.pop_front();
    }
}

fn parse_event_id(expected_session_id: &str, event_id: &str) -> Result<u64, PollEventsError> {
    let Some((session_id, sequence)) = event_id.rsplit_once(':') else {
        return Err(PollEventsError::InvalidEventId);
    };
    if session_id != expected_session_id {
        return Err(PollEventsError::InvalidEventId);
    }
    sequence
        .parse::<u64>()
        .map_err(|_| PollEventsError::InvalidEventId)
}

fn generate_session_id(protocol_version: &str, client_info: &Value, counter: u64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(protocol_version.as_bytes());
    hasher.update(client_info.to_string().as_bytes());
    hasher.update(counter.to_le_bytes());
    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    hasher.update(now_nanos.to_le_bytes());
    let digest = hasher.finalize();
    let mut session_id = String::with_capacity(32);
    for byte in digest.iter().take(16) {
        use std::fmt::Write as _;
        let _ = write!(&mut session_id, "{byte:02x}");
    }
    session_id
}

fn read_env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn read_env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn session_resume_returns_only_missed_events() {
        let manager = SessionManager::for_tests(
            Duration::from_secs(60),
            8,
            Duration::from_secs(60),
            Duration::from_millis(1),
        );
        let session = manager.create_session(
            "2025-11-25",
            serde_json::json!({"name": "zed", "version": "1.0.0"}),
            serde_json::json!({}),
        );
        let first = session.enqueue_event("{\"step\":1}".to_owned());
        session.enqueue_event("{\"step\":2}".to_owned());

        let resumed = session
            .wait_for_events(Some(&first), manager.poll_wait())
            .await
            .unwrap();
        assert_eq!(resumed.len(), 1);
        assert_eq!(resumed[0].sequence, 2);
    }

    #[test]
    fn session_resume_window_expiry_is_detected() {
        let manager = SessionManager::for_tests(
            Duration::from_secs(60),
            1,
            Duration::from_secs(60),
            Duration::from_millis(1),
        );
        let session = manager.create_session(
            "2025-11-25",
            serde_json::json!({"name": "zed", "version": "1.0.0"}),
            serde_json::json!({}),
        );
        let first = session.enqueue_event("{\"step\":1}".to_owned());
        session.enqueue_event("{\"step\":2}".to_owned());
        session.enqueue_event("{\"step\":3}".to_owned());

        let result = session.events_after(Some(&first));
        assert_eq!(result, Err(PollEventsError::ResumeWindowExpired));
    }

    #[test]
    fn wrong_session_event_id_is_rejected() {
        let manager = SessionManager::for_tests(
            Duration::from_secs(60),
            8,
            Duration::from_secs(60),
            Duration::from_millis(1),
        );
        let first = manager.create_session(
            "2025-11-25",
            serde_json::json!({"name": "zed", "version": "1.0.0"}),
            serde_json::json!({}),
        );
        let second = manager.create_session(
            "2025-11-25",
            serde_json::json!({"name": "zed", "version": "1.0.0"}),
            serde_json::json!({}),
        );
        let event_id = first.enqueue_event("{\"step\":1}".to_owned());

        let result = second.events_after(Some(&event_id));
        assert_eq!(result, Err(PollEventsError::InvalidEventId));
    }
}
