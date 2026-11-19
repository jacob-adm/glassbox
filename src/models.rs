//! Model residency: how much of each local model's weights is in the page cache.
//!
//! A file is mapped read-only and the kernel is asked, page by page, which pages
//! are resident (mincore). No page is touched, so measuring does not load the file.
use std::collections::BTreeSet;
use std::ffi::c_void;
use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

unsafe extern "C" {
    fn mmap(addr: *mut c_void, len: usize, prot: i32, flags: i32, fd: i32, offset: i64) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> i32;
    fn mincore(addr: *mut c_void, len: usize, vec: *mut u8) -> i32;
    fn getpagesize() -> i32;
}
const PROT_READ: i32 = 1;
const MAP_SHARED: i32 = 1;
const WEIGHT_EXTENSIONS: &[&str] = &["gguf", "safetensors"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Residency {
    pub bytes: u64,
    pub resident: u64,
}

/// Size of `path` and how many of its bytes are in the page cache.
pub fn residency(path: &Path) -> io::Result<Residency> {
    let file = crate::safe::open_regular(path)?;
    let bytes = file.metadata()?.len();
    if bytes == 0 {
        return Ok(Residency { bytes, resident: 0 });
    }
    let len = usize::try_from(bytes).map_err(|_| io::Error::other("file larger than address space"))?;
    let page = unsafe { getpagesize() } as usize;
    // SAFETY: read-only shared mapping of an open file; unmapped before return; never dereferenced.
    let addr = unsafe { mmap(std::ptr::null_mut(), len, PROT_READ, MAP_SHARED, file.as_raw_fd(), 0) };
    if addr as isize == -1 {
        return Err(io::Error::last_os_error());
    }
    let mut vec = vec![0u8; len.div_ceil(page)];
    let rc = unsafe { mincore(addr, len, vec.as_mut_ptr()) };
    let err = io::Error::last_os_error();
    unsafe { munmap(addr, len) };
    if rc != 0 {
        return Err(err);
    }
    let pages = vec.iter().filter(|v| **v & 1 == 1).count() as u64;
    Ok(Residency { bytes, resident: (pages * page as u64).min(bytes) })
}

#[derive(Debug)]
pub struct Model {
    pub name: String,
    pub source: &'static str,
    pub files: Vec<PathBuf>,
    pub bytes: u64,
}

impl Model {
    /// Bytes of this model in the page cache. Unreadable files count as 0.
    pub fn resident(&self) -> u64 {
        self.files.iter().filter_map(|f| residency(f).ok()).map(|r| r.resident).sum()
    }
}

/// Find model weight files under `home` (Ollama, LM Studio, Hugging Face cache) and in `extra` dirs.
pub fn discover(home: &Path, extra: &[PathBuf]) -> Vec<Model> {
    let mut found = Vec::new();
    ollama(&home.join(".ollama/models"), &mut found);
    for root in [".lmstudio/models", ".cache/lm-studio/models"] {
        lmstudio(&home.join(root), &mut found);
    }
    huggingface(&home.join(".cache/huggingface/hub"), &mut found);
    for dir in extra {
        let mut files = Vec::new();
        walk(dir, 4, &mut files);
        for f in files.into_iter().filter(|f| is_weight(f)) {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            found.push(("dir", name, vec![f]));
        }
    }
    // A file can be reachable twice (snapshots, symlinks between tools): count it once, first claim wins.
    let mut seen = BTreeSet::new();
    found
        .into_iter()
        .filter_map(|(source, name, files)| {
            let files: Vec<PathBuf> =
                files.into_iter().filter_map(|f| fs::canonicalize(&f).ok()).filter(|f| seen.insert(f.clone())).collect();
            let bytes = files.iter().filter_map(|f| fs::metadata(f).ok()).map(|m| m.len()).sum();
            (!files.is_empty()).then_some(Model { name, source, files, bytes })
        })
        .collect()
}

type Found = Vec<(&'static str, String, Vec<PathBuf>)>;

fn ollama(root: &Path, out: &mut Found) {
    let manifests = root.join("manifests");
    let mut files = Vec::new();
    walk(&manifests, 6, &mut files);
    for manifest in files {
        let Ok(rel) = manifest.strip_prefix(&manifests) else { continue };
        let parts: Vec<String> = rel.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        let [host, path @ .., model, tag] = parts.as_slice() else { continue };
        let mut name: Vec<&str> = Vec::new();
        if host != "registry.ollama.ai" {
            name.push(host);
        }
        name.extend(path.iter().map(String::as_str).filter(|p| !(host == "registry.ollama.ai" && *p == "library")));
        name.push(model);
        let Ok((raw, _)) = crate::safe::read_capped(&manifest, 1 << 20) else { continue };
        let text = String::from_utf8_lossy(&raw);
        let blobs: Vec<PathBuf> = text
            .split('}')
            .filter(|layer| layer.contains("vnd.ollama.image.model") || layer.contains("vnd.ollama.image.projector"))
            .filter_map(|layer| layer.split("\"digest\":\"").nth(1)?.split('"').next().map(str::to_string))
            .filter_map(|digest| blob_name(&digest))
            .map(|name| root.join("blobs").join(name))
            .collect();
        out.push(("ollama", format!("{}:{tag}", name.join("/")), blobs));
    }
}

fn lmstudio(root: &Path, out: &mut Found) {
    for publisher in subdirs(root) {
        for model in subdirs(&publisher) {
            let mut files = Vec::new();
            walk(&model, 3, &mut files);
            let name = format!("{}/{}", file_name(&publisher), file_name(&model));
            out.push(("lmstudio", name, files.into_iter().filter(|f| is_weight(f)).collect()));
        }
    }
}

fn huggingface(root: &Path, out: &mut Found) {
    for repo in subdirs(root) {
        let dir = file_name(&repo);
        let Some(rest) = dir.strip_prefix("models--") else { continue };
        let mut files = Vec::new();
        walk(&repo.join("snapshots"), 4, &mut files);
        out.push(("huggingface", rest.replace("--", "/"), files.into_iter().filter(|f| is_weight(f)).collect()));
    }
}

/// `sha256:<64 hex>` to its blob file name; anything else (for example `../`) is refused.
fn blob_name(digest: &str) -> Option<String> {
    let hex = digest.strip_prefix("sha256:")?;
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then(|| format!("sha256-{hex}"))
}

fn is_weight(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| WEIGHT_EXTENSIONS.contains(&e))
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| fs::symlink_metadata(p).is_ok_and(|m| m.is_dir()))
        .collect();
    dirs.sort();
    dirs
}

/// Regular files under `dir` (symlinked files followed, symlinked directories not), up to `depth` levels.
fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        match fs::symlink_metadata(&p) {
            Ok(m) if m.is_dir() && depth > 0 => walk(&p, depth - 1, out),
            Ok(m) if m.is_file() => out.push(p),
            Ok(m) if m.file_type().is_symlink() && fs::metadata(&p).is_ok_and(|t| t.is_file()) => out.push(p),
            _ => {}
        }
    }
}
