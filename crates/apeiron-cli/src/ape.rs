//! `ape`: a short name for `apeiron` (docs/CLI.md §1). It runs the `apeiron`
//! beside it, or the one on `PATH`, with the same arguments.
#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::{Command, ExitCode};

fn apeiron() -> PathBuf {
    let name = format!("apeiron{}", std::env::consts::EXE_SUFFIX);
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(&name)))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

#[cfg(unix)]
fn main() -> ExitCode {
    use std::os::unix::process::CommandExt;
    // `exec` only returns when it failed.
    let error = Command::new(apeiron())
        .args(std::env::args_os().skip(1))
        .exec();
    eprintln!("ape: cannot run apeiron: {error}");
    ExitCode::from(127)
}

#[cfg(not(unix))]
fn main() -> ExitCode {
    // Ctrl+C reaches both programs; `apeiron` decides what it means.
    let _ = ctrlc::set_handler(|| {});
    match Command::new(apeiron())
        .args(std::env::args_os().skip(1))
        .status()
    {
        Ok(status) => match status.code() {
            Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
            None => ExitCode::FAILURE,
        },
        Err(error) => {
            eprintln!("ape: cannot run apeiron: {error}");
            ExitCode::from(127)
        }
    }
}
