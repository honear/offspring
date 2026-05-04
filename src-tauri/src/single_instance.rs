//! Manual single-instance + argv-forwarding IPC for offspring.exe.
//!
//! Replaces `tauri-plugin-single-instance` because the plugin's "is
//! there a primary?" check happens INSIDE `Builder::run()` — by then
//! tao has already created its hidden message-pump window on Windows,
//! which briefly flashes on screen before the secondary exits.
//! Performing the IPC ourselves at the top of `run()`, BEFORE any
//! Tauri/tao initialization, eliminates that flash entirely.
//!
//! Mechanism (Windows-only — Offspring is a Windows app):
//!
//!   1. A named mutex (`Local\Offspring-Singleton-...-Mutex`) gates
//!      the "I am the primary" decision. `CreateMutexW` returns
//!      ERROR_ALREADY_EXISTS when another holder exists, so the
//!      check is a single Win32 call with no race window.
//!   2. A named pipe (`\\.\pipe\Offspring-Singleton-...-Pipe`) carries
//!      argv from secondaries to the primary. Each forward is one
//!      connection: length-prefixed JSON, then disconnect. The primary
//!      spawns a long-lived listener thread that loops
//!      CreateNamedPipeW + ConnectNamedPipe forever.
//!   3. Secondaries open the pipe via `std::fs::OpenOptions` (Windows
//!      treats named pipes as filesystem paths) and write the
//!      length-prefixed payload, then call `std::process::exit(0)`
//!      WITHOUT touching Tauri. No window is ever created in the
//!      secondary — that's the whole point.
//!
//! All names are scoped with a fixed app-tag so we don't collide with
//! other applications, and live in the `Local\` namespace (not
//! `Global\`) so each Windows user session gets its own primary.
//!
//! ## Threat model for the pipe
//!
//!   * **Network attacker.** Blocked by `PIPE_REJECT_REMOTE_CLIENTS` —
//!     CreateNamedPipeW refuses connections that originate from a
//!     remote machine. The pipe is local-only by construction.
//!
//!   * **Cross-user local attacker.** Blocked by Windows' default
//!     named-pipe DACL. With `lpSecurityAttributes = NULL`, Windows
//!     applies a default that grants Full Control only to the creator
//!     owner (us, the primary process), LocalSystem, and built-in
//!     Administrators; everyone else gets Read at most, which is
//!     useless for forwarding argv (forwarding requires Write).
//!
//!   * **Same-user local attacker (malware running as the user that
//!     started Offspring).** Can write to the pipe and inject argv
//!     that Offspring will then dispatch to FFmpeg. Out of scope per
//!     `THREAT_MODEL.md` — at that point the attacker already owns
//!     the user's session and can directly delete/encode files,
//!     overwrite our binary, and modify our settings without going
//!     through the pipe at all. We deliberately don't try to defend
//!     this surface because doing so would be theatre.
//!
//! See also `THREAT_MODEL.md` in the repo root.
//!
//! We use raw FFI instead of the `windows` crate to keep the dep
//! footprint small and avoid the feature-gating maze that crate's
//! moved through over versions. The surface area we need is six
//! functions and a handful of constants — declaring them here is
//! shorter than enumerating the right `features = [...]` and rebuilds
//! cleanly across `windows`/`windows-sys` version drift.

use anyhow::{anyhow, Context, Result};
use std::ffi::c_void;
use std::io::Write;
use std::time::Duration;

// Win32 typedefs we touch. Keeping them here (rather than pulling in
// `windows-sys`) means this file is self-contained.
#[allow(non_camel_case_types)]
type BOOL = i32;
#[allow(non_camel_case_types)]
type DWORD = u32;
type HANDLE = *mut c_void;
type LPCWSTR = *const u16;

const FALSE: BOOL = 0;
// CreateNamedPipeW returns this when something goes wrong.
const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;

// Errors. Win32 names + values from winerror.h.
const ERROR_ALREADY_EXISTS: DWORD = 183;
const ERROR_PIPE_CONNECTED: DWORD = 535;

// CreateNamedPipeW open-mode flags (one for direction, one ORed for behavior).
const PIPE_ACCESS_DUPLEX: DWORD = 0x0000_0003;

// CreateNamedPipeW pipe-mode flags. Message-mode means each WriteFile
// is one logical message that ReadFile delivers atomically — matches
// our "send the whole argv as one shot" semantics.
const PIPE_TYPE_MESSAGE: DWORD = 0x0000_0004;
const PIPE_READMODE_MESSAGE: DWORD = 0x0000_0002;
const PIPE_WAIT: DWORD = 0x0000_0000;
// Defense in depth: refuse to accept connections from outside the
// machine. The pipe is local-only by design.
const PIPE_REJECT_REMOTE_CLIENTS: DWORD = 0x0000_0008;

#[link(name = "kernel32")]
extern "system" {
    fn CreateMutexW(
        attributes: *const c_void,
        initial_owner: BOOL,
        name: LPCWSTR,
    ) -> HANDLE;

    fn CreateNamedPipeW(
        name: LPCWSTR,
        open_mode: DWORD,
        pipe_mode: DWORD,
        max_instances: DWORD,
        out_buffer_size: DWORD,
        in_buffer_size: DWORD,
        default_timeout: DWORD,
        security: *const c_void,
    ) -> HANDLE;

    fn ConnectNamedPipe(pipe: HANDLE, overlapped: *mut c_void) -> BOOL;
    fn DisconnectNamedPipe(pipe: HANDLE) -> BOOL;

    fn ReadFile(
        file: HANDLE,
        buffer: *mut c_void,
        bytes_to_read: DWORD,
        bytes_read: *mut DWORD,
        overlapped: *mut c_void,
    ) -> BOOL;

    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetLastError() -> DWORD;
}

// App-specific tag baked into the mutex/pipe names. The hex tail is a
// piece of our Inno AppId — unique enough that we won't collide with
// anything else even if some other app happened to pick "Offspring".
const APP_TAG: &str = "Offspring-Singleton-d8e5c6bc";

fn to_wide_z(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn mutex_name_w() -> Vec<u16> {
    // Local\ namespace = per-Windows-session. Two users on the same
    // machine each have their own primary, which matches everything
    // else in the app being per-user (presets, settings, FFmpeg).
    to_wide_z(&format!("Local\\{APP_TAG}-Mutex"))
}

fn pipe_name_str() -> String {
    format!(r"\\.\pipe\{APP_TAG}-Pipe")
}

fn pipe_name_w() -> Vec<u16> {
    to_wide_z(&pipe_name_str())
}

/// Owns the singleton mutex handle. Holding this value alive for the
/// process's lifetime is what keeps us "the primary" — drop it and the
/// next instance to start can claim the role. We never drop it
/// explicitly: process exit cleans up.
pub struct PrimaryGuard {
    _mutex: HANDLE,
}

// HANDLE is a raw pointer-sized integer; the underlying kernel object
// is shareable across threads. Wrapping it in PrimaryGuard for the
// borrow checker's benefit doesn't change that.
unsafe impl Send for PrimaryGuard {}
unsafe impl Sync for PrimaryGuard {}

/// Try to claim the singleton role.
///
///   * `Ok(Some(guard))` — we acquired the mutex; we are the primary.
///     Hold the guard for the lifetime of the process.
///   * `Ok(None)` — another instance owns the mutex; we are a secondary.
///     Caller should forward argv and exit.
///   * `Err(_)` — Win32 call failed unexpectedly. Caller should treat
///     as "act as primary" so the feature still works in a degraded
///     state rather than wedging on a kernel error.
pub fn try_become_primary() -> Result<Option<PrimaryGuard>> {
    let name = mutex_name_w();
    unsafe {
        let h = CreateMutexW(std::ptr::null(), 1, name.as_ptr());
        if h.is_null() {
            return Err(anyhow!(
                "CreateMutexW returned null; GetLastError={}",
                GetLastError()
            ));
        }
        // CreateMutexW returns a handle even when the mutex already
        // existed; GetLastError differentiates "we created it" from
        // "we opened an existing one". This is the canonical Windows
        // singleton check.
        let last = GetLastError();
        if last == ERROR_ALREADY_EXISTS {
            CloseHandle(h);
            return Ok(None);
        }
        Ok(Some(PrimaryGuard { _mutex: h }))
    }
}

/// Send `argv` to the running primary via the named pipe.
/// Length-prefixed JSON so the listener can read the whole message in
/// a couple of `ReadFile` calls.
///
/// Retries briefly to tolerate the "primary just acquired the mutex
/// but hasn't bound the pipe listener yet" race window. 10 attempts
/// at 50ms each = at most ~0.5s, which the user perceives as
/// indistinguishable from instant.
pub fn forward_argv_to_primary(argv: &[String]) -> Result<()> {
    let json = serde_json::to_vec(argv).context("serializing argv")?;
    let len_bytes = (json.len() as u32).to_le_bytes();
    let pipe_path = pipe_name_str();

    let mut last_err: Option<std::io::Error> = None;
    for _ in 0..10 {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&pipe_path)
        {
            Ok(mut pipe) => {
                pipe.write_all(&len_bytes)?;
                pipe.write_all(&json)?;
                pipe.flush()?;
                return Ok(());
            }
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    Err(anyhow!(
        "could not connect to primary's argv pipe after 10 attempts: {:?}",
        last_err
    ))
}

/// Spawn a long-lived listener thread on the primary side. Each pipe
/// connection delivers one argv vector — we hand it to `callback`
/// before disconnecting and waiting for the next secondary.
///
/// The thread runs forever; there's no clean shutdown signal because
/// the process exits when the primary's main event loop terminates,
/// which kills the thread along with it.
///
/// `callback` only needs `Send + 'static` (not `Sync`) because every
/// invocation runs on the same listener thread — there's no
/// cross-thread sharing of the closure itself. This matters because
/// the natural way to bridge to setup() is via an `mpsc::Sender`,
/// which is `Send` but not `Sync`.
pub fn start_listener<F>(callback: F)
where
    F: Fn(Vec<String>) + Send + 'static,
{
    std::thread::spawn(move || {
        let name = pipe_name_w();
        loop {
            unsafe {
                // One pipe instance per iteration. We close it at the
                // end of each loop body so the next CreateNamedPipeW
                // succeeds without colliding with the previous handle.
                let pipe = CreateNamedPipeW(
                    name.as_ptr(),
                    PIPE_ACCESS_DUPLEX,
                    PIPE_TYPE_MESSAGE
                        | PIPE_READMODE_MESSAGE
                        | PIPE_WAIT
                        | PIPE_REJECT_REMOTE_CLIENTS,
                    16,        // max instances — overkill, secondaries are short-lived
                    65_536,    // out buffer
                    65_536,    // in buffer (argv won't realistically exceed a few KB)
                    0,         // default timeout
                    std::ptr::null(),
                );
                if pipe == INVALID_HANDLE_VALUE || pipe.is_null() {
                    // Don't burn CPU on a tight failure loop if
                    // CreateNamedPipeW is somehow broken.
                    std::thread::sleep(Duration::from_secs(1));
                    continue;
                }

                // Block until a secondary connects. ERROR_PIPE_CONNECTED
                // means a connection landed BETWEEN CreateNamedPipeW and
                // ConnectNamedPipe (race) — still a valid connection.
                let connected = ConnectNamedPipe(pipe, std::ptr::null_mut()) != FALSE
                    || GetLastError() == ERROR_PIPE_CONNECTED;
                if !connected {
                    CloseHandle(pipe);
                    continue;
                }

                if let Some(argv) = read_message(pipe) {
                    callback(argv);
                }

                DisconnectNamedPipe(pipe);
                CloseHandle(pipe);
            }
        }
    });
}

/// Read one length-prefixed JSON message off `pipe` and decode it as
/// a `Vec<String>` argv. `None` on any framing/parsing failure;
/// callers treat that as "ignore this client".
unsafe fn read_message(pipe: HANDLE) -> Option<Vec<String>> {
    // Length prefix: 4 little-endian bytes.
    let mut len_buf = [0u8; 4];
    let mut bytes_read: DWORD = 0;
    if ReadFile(
        pipe,
        len_buf.as_mut_ptr() as *mut c_void,
        4,
        &mut bytes_read,
        std::ptr::null_mut(),
    ) == FALSE
        || bytes_read != 4
    {
        return None;
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    // 1 MiB cap on argv. A real CLI invocation is well under a few KB;
    // anything beyond that is malformed or hostile and we don't want
    // to allocate megabytes for it.
    if len == 0 || len > 1_048_576 {
        return None;
    }

    let mut buf = vec![0u8; len];
    let mut total = 0usize;
    while total < len {
        let mut got: DWORD = 0;
        let remaining = (len - total) as DWORD;
        if ReadFile(
            pipe,
            buf[total..].as_mut_ptr() as *mut c_void,
            remaining,
            &mut got,
            std::ptr::null_mut(),
        ) == FALSE
            || got == 0
        {
            return None;
        }
        total += got as usize;
    }

    serde_json::from_slice::<Vec<String>>(&buf).ok()
}
