//! Opening and printing things that a hostile process may have planted.
//!
//! Opening a FIFO blocks until a writer appears, opening a device can have side effects,
//! and a symlink to /dev/zero never ends. Files are checked to be regular before they are
//! opened, opened non-blocking and without acquiring a controlling terminal, and checked
//! again once open (so a swap between the check and the open cannot get through).
use crate::sha256;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

#[cfg(target_os = "macos")]
const FLAGS: i32 = 0x0004 /* O_NONBLOCK */ | 0x20000 /* O_NOCTTY */;
#[cfg(all(
    target_os = "linux",
    any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64",
        target_arch = "loongarch64",
        target_arch = "powerpc64",
        target_arch = "s390x"
    )
))]
const FLAGS: i32 = 0o4000 /* O_NONBLOCK */ | 0o400 /* O_NOCTTY */;
#[cfg(not(any(
    target_os = "macos",
    all(
        target_os = "linux",
        any(
            target_arch = "x86_64",
            target_arch = "aarch64",
            target_arch = "riscv64",
            target_arch = "loongarch64",
            target_arch = "powerpc64",
            target_arch = "s390x"
        )
    )
)))]
compile_error!("O_NONBLOCK and O_NOCTTY values are not known for this target; add them to src/safe.rs");
// The FFI declarations (mmap, lseek) pass offsets as 64-bit integers.
#[cfg(not(target_pointer_width = "64"))]
compile_error!("glassbox supports 64-bit targets only");

/// Open `path` read-only without blocking or acquiring a terminal, whatever kind of file it is.
pub fn open_nonblocking(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).custom_flags(FLAGS).open(path)
}

/// Open `path` read-only only if it is a regular file, checked before and after opening.
pub fn open_regular(path: &Path) -> io::Result<File> {
    let not_regular = || io::Error::new(io::ErrorKind::InvalidInput, "not a regular file");
    if !fs::metadata(path)?.is_file() {
        return Err(not_regular());
    }
    let file = open_nonblocking(path)?;
    if file.metadata()?.is_file() { Ok(file) } else { Err(not_regular()) }
}

/// At most `cap` bytes of a regular file, and its full size.
pub fn read_capped(path: &Path, cap: u64) -> io::Result<(Vec<u8>, u64)> {
    let file = open_regular(path)?;
    let size = file.metadata()?.len();
    let mut buf = Vec::new();
    file.take(cap).read_to_end(&mut buf)?;
    Ok((buf, size))
}

/// SHA-256 of a whole regular file, streamed in 1 MiB chunks.
pub fn hash_file(file: &mut File) -> io::Result<[u8; 32]> {
    let mut hasher = sha256::Hasher::default();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(hasher.finish());
        }
        hasher.update(&buf[..n]);
    }
}

/// Text with control, bidirectional and zero-width characters made visible, so names
/// cannot rewrite, reorder or hide parts of the report.
pub fn visible(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\x{:02x}", c as u32)),
            '\u{00ad}' | '\u{034f}' | '\u{061c}' | '\u{115f}' | '\u{1160}' | '\u{180e}' | '\u{200b}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206f}' | '\u{3164}'
            | '\u{fe00}'..='\u{fe0f}' | '\u{feff}' | '\u{ffa0}' | '\u{fff9}'..='\u{fffb}'
            | '\u{e0000}'..='\u{e007f}' | '\u{e0100}'..='\u{e01ef}' => {
                out.push_str(&format!("\\u{:04x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out
}

/// `s` escaped for use inside a JSON string.
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
