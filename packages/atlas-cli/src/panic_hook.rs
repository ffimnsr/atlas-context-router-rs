use std::io::Write;
use std::panic::PanicHookInfo;
use std::sync::Once;

use atlas_core::user_facing_error_message;

static INSTALL_HOOK: Once = Once::new();

pub(crate) fn install() {
    INSTALL_HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|panic_info| {
            emit_panic_report(panic_info, wants_json_output());
        }));
    });
}

fn wants_json_output() -> bool {
    std::env::args().any(|arg| arg == "--json")
}

fn emit_panic_report(panic_info: &PanicHookInfo<'_>, json_output: bool) {
    let detail = panic_detail(panic_info);
    let message = user_facing_error_message(&detail, &detail);
    let stderr = std::io::stderr();
    let mut stderr = stderr.lock();

    if json_output {
        let payload = crate::commands::json_envelope(
            "panic",
            serde_json::json!({
                "error": message,
                "detail": detail,
            }),
        );
        match serde_json::to_string_pretty(&payload) {
            Ok(text) => {
                let _ = writeln!(stderr, "{text}");
            }
            Err(error) => {
                let _ = writeln!(stderr, "error: internal panic");
                let _ = writeln!(stderr, "detail: failed to encode panic envelope: {error}");
            }
        }
        return;
    }

    let _ = writeln!(stderr, "error: {message}");
}

fn panic_detail(panic_info: &PanicHookInfo<'_>) -> String {
    let payload = panic_payload_message(panic_info.payload());

    match panic_info.location() {
        Some(location) => format!(
            "internal panic at {}:{}:{}: {payload}",
            location.file(),
            location.line(),
            location.column()
        ),
        None => format!("internal panic: {payload}"),
    }
}

fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::panic_payload_message;

    #[test]
    fn panic_payload_message_supports_static_str() {
        let panic = std::panic::catch_unwind(|| panic!("panic hook smoke test"));
        let payload = panic.expect_err("panic payload");
        assert_eq!(
            panic_payload_message(payload.as_ref()),
            "panic hook smoke test"
        );
    }

    #[test]
    fn panic_payload_message_supports_string() {
        let panic = std::panic::catch_unwind(|| std::panic::panic_any("owned payload".to_owned()));
        let payload = panic.expect_err("panic payload");
        assert_eq!(panic_payload_message(payload.as_ref()), "owned payload");
    }
}
