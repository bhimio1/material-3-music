//! Thin wrappers around `std::process::Command`.

use std::path::Path;
use std::process::{Command, Stdio};

/// Start a program without waiting for it. It gets its own process group
/// so it survives this app closing.
pub fn spawn_detached(program: &str, args: &[&str]) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("{program}: {e}"))
}

/// Is `program` on `$PATH` (or an existing absolute path)?
pub fn exists(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_lookup() {
        assert!(exists("sh"));
        assert!(!exists("definitely-not-a-real-program-m3s"));
    }
}
