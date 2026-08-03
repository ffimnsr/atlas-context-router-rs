//! Signal-handling and socket-lifetime helpers for rmcp transports.

#[cfg(unix)]
use std::net::Shutdown;
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use anyhow::{Context, Result};

// ---------------------------------------------------------------------------
// Signal handlers (Unix)
// ---------------------------------------------------------------------------

#[cfg(unix)]
pub(crate) struct ShutdownHandlerGuard(signal_hook::iterator::Handle);

#[cfg(unix)]
impl Drop for ShutdownHandlerGuard {
    fn drop(&mut self) {
        self.0.close();
    }
}

#[cfg(unix)]
pub(crate) fn install_stdio_shutdown_handler() -> Result<ShutdownHandlerGuard> {
    spawn_signal_handler(move || unsafe {
        let _ = libc::close(libc::STDIN_FILENO);
    })
}

#[cfg(unix)]
pub(crate) fn install_socket_shutdown_handler(
    shutdown: Arc<AtomicBool>,
    active_streams: Arc<std::sync::Mutex<Vec<UnixStream>>>,
) -> Result<ShutdownHandlerGuard> {
    spawn_signal_handler(move || {
        shutdown.store(true, Ordering::Relaxed);
        let streams = active_streams
            .lock()
            .expect("active daemon streams lock poisoned");
        for stream in streams.iter() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    })
}

#[cfg(unix)]
fn spawn_signal_handler(action: impl Fn() + Send + 'static) -> Result<ShutdownHandlerGuard> {
    let mut signals = signal_hook::iterator::Signals::new([
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
    ])
    .context("cannot register shutdown signals")?;
    let handle = signals.handle();
    thread::Builder::new()
        .name("atlas-mcp:signal-handler".to_owned())
        .spawn(move || {
            if signals.forever().next().is_some() {
                action();
            }
        })
        .context("cannot spawn shutdown signal handler")?;
    Ok(ShutdownHandlerGuard(handle))
}

#[cfg(unix)]
pub(crate) struct ActiveStreamGuard {
    raw_fd: i32,
    active_streams: Arc<std::sync::Mutex<Vec<UnixStream>>>,
}

#[cfg(unix)]
impl ActiveStreamGuard {
    pub(crate) fn new(raw_fd: i32, active_streams: Arc<std::sync::Mutex<Vec<UnixStream>>>) -> Self {
        Self {
            raw_fd,
            active_streams,
        }
    }
}

#[cfg(unix)]
impl Drop for ActiveStreamGuard {
    fn drop(&mut self) {
        let mut streams = self
            .active_streams
            .lock()
            .expect("active daemon streams lock poisoned");
        if let Some(index) = streams
            .iter()
            .position(|stream| stream.as_raw_fd() == self.raw_fd)
        {
            streams.swap_remove(index);
        }
    }
}
