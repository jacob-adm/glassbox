//! Opening files that a hostile process may have planted.
//!
//! Opening a FIFO blocks until a writer appears, and a symlink to /dev/zero never ends.
//! Every file glassbox opens goes through `open_regular`, which opens without blocking
//! and refuses anything that is not a regular file once open (so a swap after a check
//! cannot get through).
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

#[cfg(target_os = "macos")]
const O_NONBLOCK: i32 = 0x0004;
#[cfg(not(target_os = "macos"))]
const O_NONBLOCK: i32 = 0o4000;

/// Open `path` read-only without blocking, whatever kind of file it is.
pub fn open_nonblocking(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).custom_flags(O_NONBLOCK).open(path)
}

/// Open `path` read-only, never blocking, and only if it is a regular file.
pub fn open_regular(path: &Path) -> io::Result<File> {
    let file = open_nonblocking(path)?;
    if file.metadata()?.is_file() {
        Ok(file)
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidInput, "not a regular file"))
    }
}

/// At most `cap` bytes of a regular file, and its full size.
pub fn read_capped(path: &Path, cap: u64) -> io::Result<(Vec<u8>, u64)> {
    let file = open_regular(path)?;
    let size = file.metadata()?.len();
    let mut buf = Vec::new();
    file.take(cap).read_to_end(&mut buf)?;
    Ok((buf, size))
}

/// Text with control characters made visible, so names cannot rewrite the terminal.
pub fn visible(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
