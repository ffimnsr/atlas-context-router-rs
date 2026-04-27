//! Thread-local progress reporter for MCP tool execution.
//!
//! Long-running tools call [`report`] to push `$/progress` notifications back
//! to the transport loop without changing their function signatures.  The
//! transport layer calls [`install`] before dispatching a tool and
//! [`uninstall`] immediately after the tool returns.
//!
//! Tools that support cooperative cancellation call [`is_canceled`]
//! periodically and return early when it returns `true`.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

struct ProgressSink {
    reporter: Box<dyn Fn(&str, Option<u32>) + 'static>,
    cancel_flag: Arc<AtomicBool>,
}

thread_local! {
    static TOOL_PROGRESS: RefCell<Option<ProgressSink>> = const { RefCell::new(None) };
}

/// Install a progress reporter and cancellation flag for the current thread.
///
/// The transport layer calls this once, just before dispatching a tool
/// on a worker thread.  `reporter` is called whenever the tool emits a
/// progress update.  `cancel_flag` is set to `true` by the transport
/// when the client sends `$/cancelRequest`.
pub(crate) fn install(
    reporter: impl Fn(&str, Option<u32>) + 'static,
    cancel_flag: Arc<AtomicBool>,
) {
    TOOL_PROGRESS.with(|p| {
        *p.borrow_mut() = Some(ProgressSink {
            reporter: Box::new(reporter),
            cancel_flag,
        });
    });
}

/// Remove the progress reporter from the current thread.
///
/// The transport layer calls this immediately after the tool returns,
/// whether or not it succeeded.
pub(crate) fn uninstall() {
    TOOL_PROGRESS.with(|p| *p.borrow_mut() = None);
}

/// Emit a progress update from the currently running tool.
///
/// `message` is displayed in agent UIs.  `percentage` is 0–100 if
/// the tool knows how far along it is; pass `None` when unknown.
///
/// Calling this function when no reporter is installed (e.g. in tests
/// or from a synchronous fast tool) is a no-op.
pub fn report(message: &str, percentage: Option<u32>) {
    TOOL_PROGRESS.with(|p| {
        if let Some(sink) = p.borrow().as_ref() {
            (sink.reporter)(message, percentage);
        }
    });
}

/// Returns `true` when the client has requested cancellation of the tool
/// currently running on this thread.
///
/// Long-running tools should call this at natural checkpoints (e.g. between
/// file batches) and return early with an error when it returns `true`.
pub fn is_canceled() -> bool {
    TOOL_PROGRESS.with(|p| {
        p.borrow()
            .as_ref()
            .is_some_and(|sink| sink.cancel_flag.load(Ordering::Relaxed))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn report_noop_without_install() {
        // Should not panic
        report("test", None);
    }

    #[test]
    fn is_canceled_returns_false_without_install() {
        assert!(!is_canceled());
    }

    #[test]
    fn report_calls_reporter() {
        let called = Arc::new(AtomicBool::new(false));
        let called_clone = Arc::clone(&called);
        let cancel = Arc::new(AtomicBool::new(false));
        install(
            move |_msg, _pct| called_clone.store(true, Ordering::Relaxed),
            cancel,
        );
        report("hello", Some(42));
        let was_called = called.load(Ordering::Relaxed);
        uninstall();
        assert!(was_called);
    }

    #[test]
    fn cancel_flag_propagates() {
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = Arc::clone(&cancel);
        install(|_, _| {}, cancel_clone);
        assert!(!is_canceled());
        cancel.store(true, Ordering::Relaxed);
        assert!(is_canceled());
        uninstall();
    }

    #[test]
    fn uninstall_clears_state() {
        let cancel = Arc::new(AtomicBool::new(false));
        install(|_, _| {}, cancel);
        uninstall();
        assert!(!is_canceled());
        // Safe to call report after uninstall
        report("after uninstall", None);
    }
}
