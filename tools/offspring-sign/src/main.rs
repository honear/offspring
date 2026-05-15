//! Headless minisign signer for Offspring CI.
//!
//! rsign2 — the canonical Rust port of minisign — does NOT support
//! reading a password from stdin or env vars. Its `-W` flag means
//! "passwordless key", not "read password from stdin" as I'd hoped.
//! That makes rsign2 fundamentally unsuitable for CI without spawning
//! a pseudo-TTY, which is a whole separate pile of complexity per
//! platform. The `minisign` Rust crate, on the other hand, exposes
//! `SecretKey::from_file(path, password: Option<String>)` directly —
//! so we just read the password from MINISIGN_PASSWORD and call it.
//!
//! Output is byte-identical to what `minisign -Sm` produces locally
//! (same format, same Ed25519 signature), so .minisig files signed
//! here verify against the same pinned public key in updates.rs.
//!
//! ## Usage
//!
//!     offspring-sign <key-file> <input-file> <sig-file> \
//!                    [<trusted-comment> [<untrusted-comment>]]
//!
//! With MINISIGN_PASSWORD set in the environment.
//!
//! Exit codes:
//!   0  success — sig file written
//!   2  bad arguments
//!   3  missing MINISIGN_PASSWORD
//!   4  key load failed (typically wrong password)
//!   5  signing or I/O error

use std::env;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::process::ExitCode;

use minisign::{sign, SecretKey, SignatureBox};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 4 || args.len() > 6 {
        eprintln!(
            "Usage: {} <key-file> <input-file> <sig-file> [<trusted> [<untrusted>]]",
            args.first().map(String::as_str).unwrap_or("offspring-sign")
        );
        return ExitCode::from(2);
    }

    let key_path = PathBuf::from(&args[1]);
    let input_path = PathBuf::from(&args[2]);
    let sig_path = PathBuf::from(&args[3]);
    let trusted = args.get(4).cloned();
    let untrusted = args.get(5).cloned();

    let password = match env::var("MINISIGN_PASSWORD") {
        Ok(s) => s,
        Err(_) => {
            eprintln!("error: MINISIGN_PASSWORD environment variable is required");
            return ExitCode::from(3);
        }
    };

    eprintln!("offspring-sign: key={}", key_path.display());
    eprintln!("offspring-sign: input={}", input_path.display());
    eprintln!("offspring-sign: sig={}", sig_path.display());
    eprintln!("offspring-sign: password length={}", password.chars().count());

    // Load + decrypt the secret key. SecretKey::from_file takes the
    // password as an owned String; mismatched password manifests as
    // an explicit error here, so we surface it with a distinct exit
    // code (4) instead of conflating it with later I/O failures.
    let sk = match SecretKey::from_file(&key_path, Some(password)) {
        Ok(sk) => sk,
        Err(e) => {
            eprintln!("error: failed to load secret key: {e}");
            return ExitCode::from(4);
        }
    };

    // Read the file to sign into memory. minisign signs over the
    // file's full bytes; for our installer payloads (~10-13 MB) this
    // is fine. If we ever sign files large enough to OOM a CI runner
    // we'd switch to streaming, but that's not today.
    let data = match fs::read(&input_path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: failed to read input file: {e}");
            return ExitCode::from(5);
        }
    };

    let signature: SignatureBox = match sign(
        None,
        &sk,
        Cursor::new(&data),
        trusted.as_deref(),
        untrusted.as_deref(),
    ) {
        Ok(sig) => sig,
        Err(e) => {
            eprintln!("error: failed to sign: {e}");
            return ExitCode::from(5);
        }
    };

    if let Err(e) = fs::write(&sig_path, signature.into_string()) {
        eprintln!("error: failed to write signature file: {e}");
        return ExitCode::from(5);
    }

    eprintln!("offspring-sign: wrote {}", sig_path.display());
    ExitCode::SUCCESS
}
