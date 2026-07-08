//! Unix socket and Windows named-pipe daemon transports.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

use super::io::{perform_socket_handshake, serve_connection};
use super::types::ServerOptions;
use super::worker::{ActiveStreamGuard, WorkerPool, install_socket_shutdown_handler};

#[cfg(unix)]
use std::io::BufReader;
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(unix)]
use std::sync::Mutex;

#[cfg(windows)]
use std::os::windows::io::FromRawHandle;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{
    CloseHandle, DuplicateHandle, ERROR_NO_DATA, ERROR_PIPE_CONNECTED, FALSE, GetLastError, HANDLE,
    INVALID_HANDLE_VALUE,
};
#[cfg(windows)]
use windows_sys::Win32::Security::{
    CopySid, EqualSid, GetLengthSid, GetTokenInformation, OpenProcessToken, TOKEN_QUERY,
    TOKEN_USER, TokenUser,
};
#[cfg(windows)]
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_ACCESS_DUPLEX,
    PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{
    DUPLICATE_SAME_ACCESS, GetCurrentProcess, GetCurrentProcessId, OpenProcess,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

// ---------------------------------------------------------------------------
// Unix socket server
// ---------------------------------------------------------------------------

#[cfg(unix)]
pub fn run_socket_server_with_options(
    socket_path: &Path,
    repo_root: &str,
    db_path: &str,
    options: ServerOptions,
) -> Result<()> {
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    if socket_path.exists() {
        std::fs::remove_file(socket_path)
            .with_context(|| format!("cannot remove stale {}", socket_path.display()))?;
    }

    let old_mask = unsafe { libc::umask(0o177) };
    let bind_result = UnixListener::bind(socket_path);
    unsafe { libc::umask(old_mask) };
    let listener = bind_result.with_context(|| format!("cannot bind {}", socket_path.display()))?;
    secure_socket_path(socket_path)?;
    listener
        .set_nonblocking(true)
        .with_context(|| format!("cannot set nonblocking {}", socket_path.display()))?;
    crate::tools::health::mark_server_started();
    let worker_pool = Arc::new(WorkerPool::from_env(
        "atlas-mcp:tool-worker",
        options.clone(),
    )?);
    let next_connection = AtomicUsize::new(0);
    let shutdown = Arc::new(AtomicBool::new(false));
    let active_streams = Arc::new(Mutex::new(Vec::<UnixStream>::new()));
    let _shutdown_guard =
        install_socket_shutdown_handler(Arc::clone(&shutdown), Arc::clone(&active_streams))?;

    eprintln!(
        "atlas-mcp: daemon ready (repo={repo_root}, db={db_path}, socket={})",
        socket_path.display()
    );

    while !shutdown.load(Ordering::Relaxed) {
        let stream = match listener.accept() {
            Ok((stream, _addr)) => stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_error) if shutdown.load(Ordering::Relaxed) => break,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("cannot accept connection on {}", socket_path.display())
                });
            }
        };
        if let Err(error) = stream.set_nonblocking(false) {
            tracing::warn!(error = %error, "cannot set accepted socket to blocking; skipping connection");
            continue;
        }
        let repo_root = repo_root.to_owned();
        let db_path = db_path.to_owned();
        let worker_pool = Arc::clone(&worker_pool);
        let server_options = options.clone();
        let active_streams = Arc::clone(&active_streams);
        let connection_id = next_connection.fetch_add(1, Ordering::Relaxed) + 1;
        let thread_name = format!("atlas-mcp:daemon-client-{connection_id}");
        let log_thread_name = thread_name.clone();
        thread::Builder::new()
            .name(thread_name.clone())
            .spawn(move || {
                if let Err(error) = serve_socket_connection(
                    stream,
                    &repo_root,
                    &db_path,
                    worker_pool,
                    server_options,
                    active_streams,
                ) {
                    tracing::warn!(error = %error, client = %log_thread_name, "daemon client failed");
                }
            })
            .with_context(|| format!("cannot spawn '{thread_name}'"))?;
    }

    Ok(())
}

#[cfg(unix)]
fn serve_socket_connection(
    stream: UnixStream,
    repo_root: &str,
    db_path: &str,
    worker_pool: Arc<WorkerPool>,
    server_options: ServerOptions,
    active_streams: Arc<Mutex<Vec<UnixStream>>>,
) -> Result<()> {
    ensure_peer_uid_allowed(&stream)?;
    let registered_stream = stream
        .try_clone()
        .context("cannot clone daemon stream for shutdown")?;
    let registered_fd = registered_stream.as_raw_fd();
    active_streams
        .lock()
        .expect("active daemon streams lock poisoned")
        .push(registered_stream);
    let _stream_guard = ActiveStreamGuard::new(registered_fd, active_streams);
    let reader_stream = stream.try_clone().context("cannot clone daemon stream")?;
    let mut reader = BufReader::new(reader_stream);
    let mut writer = std::io::BufWriter::new(stream);

    perform_socket_handshake(&mut reader, &mut writer, repo_root, db_path)?;
    serve_connection(
        reader,
        &mut writer,
        Some(repo_root),
        Some(db_path),
        false,
        worker_pool,
        server_options,
    )
}

#[cfg(unix)]
fn secure_socket_path(socket_path: &Path) -> Result<()> {
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("cannot secure {}", socket_path.display()))
}

#[cfg(unix)]
fn ensure_peer_uid_allowed(stream: &UnixStream) -> Result<()> {
    let peer_uid = peer_uid(stream)?;
    let process_uid = unsafe { libc::geteuid() };
    if peer_uid == process_uid || peer_uid == 0 {
        return Ok(());
    }

    Err(anyhow::anyhow!(
        "rejecting daemon client from uid {peer_uid}; expected uid {process_uid}"
    ))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn peer_uid(stream: &UnixStream) -> Result<libc::uid_t> {
    let fd = stream.as_raw_fd();
    let mut creds = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut creds as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        )
    };
    if result == 0 {
        Ok(creds.uid)
    } else {
        Err(std::io::Error::last_os_error()).context("cannot read peer credentials")
    }
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "openbsd",
    target_os = "netbsd"
))]
fn peer_uid(stream: &UnixStream) -> Result<libc::uid_t> {
    let fd = stream.as_raw_fd();
    let mut uid = 0;
    let mut gid = 0;
    let result = unsafe { libc::getpeereid(fd, &mut uid, &mut gid) };
    if result == 0 {
        Ok(uid)
    } else {
        Err(std::io::Error::last_os_error()).context("cannot read peer credentials")
    }
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "openbsd",
        target_os = "netbsd",
    ))
))]
fn peer_uid(_stream: &UnixStream) -> Result<libc::uid_t> {
    Err(anyhow::anyhow!(
        "peer credential check not supported on this Unix platform; daemon rejected"
    ))
}

// ---------------------------------------------------------------------------
// Windows named-pipe server
// ---------------------------------------------------------------------------

#[cfg(windows)]
pub fn run_socket_server_with_options(
    pipe_path: &Path,
    repo_root: &str,
    db_path: &str,
    options: ServerOptions,
) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;

    let pipe_name: Vec<u16> = pipe_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let worker_pool = Arc::new(WorkerPool::from_env(
        "atlas-mcp:tool-worker",
        options.clone(),
    )?);
    let next_connection = AtomicUsize::new(0);

    eprintln!(
        "atlas-mcp: daemon ready (repo={repo_root}, db={db_path}, pipe={})",
        pipe_path.display()
    );

    loop {
        let pipe_handle = win_create_named_pipe_server(&pipe_name)?;

        let connected = unsafe {
            ConnectNamedPipe(pipe_handle, std::ptr::null_mut()) != 0
                || GetLastError() == ERROR_PIPE_CONNECTED
        };
        if !connected {
            let err_code = unsafe { GetLastError() };
            unsafe { CloseHandle(pipe_handle) };
            if err_code == ERROR_NO_DATA {
                continue;
            }
            return Err(std::io::Error::from_raw_os_error(err_code as i32))
                .context("ConnectNamedPipe failed");
        }

        let repo_root = repo_root.to_owned();
        let db_path = db_path.to_owned();
        let worker_pool = Arc::clone(&worker_pool);
        let server_options = options.clone();
        let connection_id = next_connection.fetch_add(1, Ordering::Relaxed) + 1;
        let thread_name = format!("atlas-mcp:daemon-client-{connection_id}");
        let log_thread_name = thread_name.clone();

        thread::Builder::new()
            .name(thread_name.clone())
            .spawn(move || {
                if let Err(error) = win_serve_pipe_connection(
                    pipe_handle,
                    &repo_root,
                    &db_path,
                    worker_pool,
                    server_options,
                ) {
                    tracing::warn!(
                        error = %error,
                        client = %log_thread_name,
                        "daemon pipe client failed"
                    );
                }
            })
            .with_context(|| format!("cannot spawn '{thread_name}'"))?;
    }
}

#[cfg(not(any(unix, windows)))]
pub fn run_socket_server_with_options(
    _socket_path: &Path,
    _repo_root: &str,
    _db_path: &str,
    _options: ServerOptions,
) -> Result<()> {
    Err(anyhow::anyhow!(
        "MCP daemon transport is unsupported on this platform"
    ))
}

/// Create a new named-pipe server instance.
#[cfg(windows)]
fn win_create_named_pipe_server(pipe_name: &[u16]) -> Result<HANDLE> {
    let handle = unsafe {
        CreateNamedPipeW(
            pipe_name.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            65536,
            65536,
            0,
            std::ptr::null(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error()).context("cannot create named pipe server");
    }
    Ok(handle)
}

/// Duplicate a Windows HANDLE so reader and writer each own independent handles.
#[cfg(windows)]
fn win_dup_handle(handle: HANDLE) -> Result<HANDLE> {
    let process = unsafe { GetCurrentProcess() };
    let mut new_handle = INVALID_HANDLE_VALUE;
    let ok = unsafe {
        DuplicateHandle(
            process,
            handle,
            process,
            &mut new_handle,
            0,
            FALSE,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error()).context("cannot duplicate pipe handle");
    }
    Ok(new_handle)
}

/// RAII guard that closes a Windows HANDLE on drop.
#[cfg(windows)]
struct WinHandleGuard(HANDLE);

#[cfg(windows)]
impl Drop for WinHandleGuard {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// Retrieve the binary SID of the owner of the given process.
#[cfg(windows)]
fn win_process_owner_sid(pid: u32) -> Result<Vec<u8>> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
    if process == 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("cannot open process {pid}"));
    }
    let _process_guard = WinHandleGuard(process);

    let mut token: HANDLE = 0;
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(std::io::Error::last_os_error()).context("cannot open process token");
    }
    let _token_guard = WinHandleGuard(token);

    let mut len: u32 = 0;
    unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len) };
    if len == 0 {
        return Err(std::io::Error::last_os_error())
            .context("GetTokenInformation size query failed");
    }

    let mut buf = vec![0u8; len as usize];
    if unsafe { GetTokenInformation(token, TokenUser, buf.as_mut_ptr() as *mut _, len, &mut len) }
        == 0
    {
        return Err(std::io::Error::last_os_error()).context("cannot get token user SID");
    }

    let token_user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
    let sid_ptr = token_user.User.Sid;
    let sid_len = unsafe { GetLengthSid(sid_ptr) } as usize;
    let mut sid_bytes = vec![0u8; sid_len];
    if unsafe { CopySid(sid_len as u32, sid_bytes.as_mut_ptr() as *mut _, sid_ptr) } == 0 {
        return Err(std::io::Error::last_os_error()).context("cannot copy SID");
    }
    Ok(sid_bytes)
}

/// Reject daemon connections from a different Windows user account.
#[cfg(windows)]
fn win_ensure_pipe_client_is_same_user(pipe_handle: HANDLE) -> Result<()> {
    let mut client_pid: u32 = 0;
    if unsafe { GetNamedPipeClientProcessId(pipe_handle, &mut client_pid) } == 0 {
        return Err(std::io::Error::last_os_error())
            .context("cannot get named pipe client process id");
    }
    let server_pid = unsafe { GetCurrentProcessId() };

    let client_sid = win_process_owner_sid(client_pid)?;
    let server_sid = win_process_owner_sid(server_pid)?;

    if unsafe { EqualSid(client_sid.as_ptr() as *mut _, server_sid.as_ptr() as *mut _) } == 0 {
        return Err(anyhow::anyhow!(
            "rejecting daemon pipe client from a different user account (client pid {client_pid})"
        ));
    }
    Ok(())
}

/// Serve one named-pipe client connection.
#[cfg(windows)]
fn win_serve_pipe_connection(
    pipe_handle: HANDLE,
    repo_root: &str,
    db_path: &str,
    worker_pool: Arc<WorkerPool>,
    server_options: ServerOptions,
) -> Result<()> {
    win_ensure_pipe_client_is_same_user(pipe_handle)?;

    let reader_handle = win_dup_handle(pipe_handle)?;
    let reader_file = unsafe { std::fs::File::from_raw_handle(reader_handle as _) };
    let writer_file = unsafe { std::fs::File::from_raw_handle(pipe_handle as _) };

    let mut reader = std::io::BufReader::new(reader_file);
    let mut writer = std::io::BufWriter::new(writer_file);

    perform_socket_handshake(&mut reader, &mut writer, repo_root, db_path)?;
    serve_connection(
        reader,
        &mut writer,
        Some(repo_root),
        Some(db_path),
        false,
        worker_pool,
        server_options,
    )
}
