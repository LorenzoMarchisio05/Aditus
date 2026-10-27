//! Tiny file log at `<data_dir>/aditus.log` for failures that would otherwise be dropped silently.

use std::{fs, io::Write, path::PathBuf, sync::Mutex};

const MAX: u64 = 512 * 1024;
static LOCK: Mutex<()> = Mutex::new(());

fn path() -> PathBuf {
    crate::data_dir().join("aditus.log")
}

/// Append a line; past `MAX` bytes the file is rotated once to `aditus.log.1`.
pub fn log(msg: &str) {
    let _g = LOCK.lock();
    let p = path();
    if fs::metadata(&p).is_ok_and(|m| m.len() > MAX) {
        let _ = fs::rename(&p, p.with_extension("log.1"));
    }
    let _ = fs::create_dir_all(p.parent().unwrap_or(&p));
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{t} {msg}");
    }
}

/// Log `ctx: error` for a failed result and hand the result back.
pub fn check<T, E: std::fmt::Display>(ctx: &str, r: Result<T, E>) -> Result<T, E> {
    if let Err(e) = &r {
        log(&format!("{ctx}: {e}"));
    }
    r
}
