//! Early shutdown-signal handling for the Unix stdio broker.
//!
//! The broker must not die from the default signal disposition while it is
//! still acquiring its coordination lock or waiting for a daemon. Installing
//! the handler before that work starts lets a shutdown signal interrupt
//! startup cleanly, and [`BrokerShutdown::attach`] lets the relay register its
//! socket so a signal can also interrupt an in-flight session.

use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

/// Installed broker shutdown state.
///
/// Dropping the value closes the signal handler registration; the handler
/// thread then exits on its next iteration.
pub(crate) struct BrokerShutdown {
    requested: Arc<AtomicBool>,
    active_stream: Arc<Mutex<Option<UnixStream>>>,
    handle: signal_hook::iterator::Handle,
}

impl BrokerShutdown {
    /// Install SIGINT/SIGTERM handling for the lifetime of the broker.
    pub(crate) fn install() -> Result<Self> {
        let requested = Arc::new(AtomicBool::new(false));
        let active_stream = Arc::new(Mutex::new(None::<UnixStream>));
        let mut signals = signal_hook::iterator::Signals::new([
            signal_hook::consts::SIGINT,
            signal_hook::consts::SIGTERM,
        ])
        .context("cannot install broker shutdown signals")?;
        let handle = signals.handle();
        let signal_requested = Arc::clone(&requested);
        let signal_stream = Arc::clone(&active_stream);
        std::thread::Builder::new()
            .name("atlas-cli:broker-signal-handler".to_owned())
            .spawn(move || {
                if signals.forever().next().is_some() {
                    signal_requested.store(true, Ordering::Relaxed);
                    if let Some(stream) = signal_stream
                        .lock()
                        .expect("broker active stream lock poisoned")
                        .as_ref()
                    {
                        let _ = stream.shutdown(Shutdown::Both);
                    }
                    unsafe {
                        // Interrupt a blocked stdin relay, matching the
                        // pre-install behaviour of the relay-local handler.
                        let _ = libc::close(libc::STDIN_FILENO);
                    }
                }
            })
            .context("cannot spawn broker shutdown signal handler")?;
        Ok(Self {
            requested,
            active_stream,
            handle,
        })
    }

    /// Whether a shutdown signal has been observed.
    pub(crate) fn is_requested(&self) -> bool {
        self.requested.load(Ordering::Relaxed)
    }

    /// Register the socket of the active relay so a signal can interrupt it.
    pub(crate) fn attach(&self, stream: UnixStream) {
        *self
            .active_stream
            .lock()
            .expect("broker active stream lock poisoned") = Some(stream);
    }

    /// Unregister the active relay socket once the relay has finished.
    pub(crate) fn detach(&self) {
        *self
            .active_stream
            .lock()
            .expect("broker active stream lock poisoned") = None;
    }
}

impl Drop for BrokerShutdown {
    fn drop(&mut self) {
        self.handle.close();
    }
}
