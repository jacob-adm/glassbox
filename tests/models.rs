//! Model residency tests against real files and the real page cache. No mocks.
use glassbox::models::{discover, residency};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

const MIB: u64 = 1 << 20;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glassbox-models-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write `len` bytes so that, afterwards, the file is NOT in the page cache.
fn write_cold(path: &Path, len: u64) {
    let mut f = fs::File::create(path).unwrap();
    no_cache(&f);
    let chunk = vec![0xA5u8; MIB as usize];
    for _ in 0..len / MIB {
        f.write_all(&chunk).unwrap();
    }
    f.sync_all().unwrap();
    drop_cache(&f);
}

#[cfg(target_os = "macos")]
fn no_cache(f: &fs::File) {
    use std::os::fd::AsRawFd;
    unsafe extern "C" {
        fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    }
    const F_NOCACHE: i32 = 48;
    assert_eq!(unsafe { fcntl(f.as_raw_fd(), F_NOCACHE, 1) }, 0);
}
#[cfg(target_os = "macos")]
fn drop_cache(_: &fs::File) {}

#[cfg(target_os = "linux")]
fn no_cache(_: &fs::File) {}
#[cfg(target_os = "linux")]
fn drop_cache(f: &fs::File) {
    use std::os::fd::AsRawFd;
    unsafe extern "C" {
        fn posix_fadvise(fd: i32, offset: i64, len: i64, advice: i32) -> i32;
    }
    const POSIX_FADV_DONTNEED: i32 = 4;
    assert_eq!(unsafe { posix_fadvise(f.as_raw_fd(), 0, 0, POSIX_FADV_DONTNEED) }, 0);
}

fn read_all(path: &Path) {
    let mut buf = Vec::new();
    fs::File::open(path).unwrap().read_to_end(&mut buf).unwrap();
}

#[test]
fn cold_file_reads_near_zero_then_full_after_reading() {
    let file = scratch("cold").join("weights.gguf");
    write_cold(&file, 32 * MIB);
    let cold = residency(&file).unwrap();
    assert_eq!(cold.bytes, 32 * MIB);
    assert!(cold.resident <= 2 * MIB, "cold file shows {} bytes resident", cold.resident);
    read_all(&file);
    let warm = residency(&file).unwrap();
    assert!(warm.resident >= 30 * MIB, "after a full read only {} bytes resident", warm.resident);
}

#[test]
fn measuring_does_not_warm_the_file() {
    let file = scratch("observer").join("weights.safetensors");
    write_cold(&file, 32 * MIB);
    let first = residency(&file).unwrap().resident;
    let second = residency(&file).unwrap().resident;
    assert!(second <= first + MIB, "measuring pulled the file in: {first} -> {second}");
}

#[test]
fn empty_file_is_zero_not_an_error() {
    let file = scratch("empty").join("empty.gguf");
    fs::File::create(&file).unwrap();
    let r = residency(&file).unwrap();
    assert_eq!((r.bytes, r.resident), (0, 0));
}

/// A home directory laid out like Ollama, LM Studio and the Hugging Face cache.
fn fake_home(name: &str) -> PathBuf {
    let home = scratch(name);
    let ollama = home.join(".ollama/models");
    let blob = "sha256-1111111111111111111111111111111111111111111111111111111111111111";
    fs::create_dir_all(ollama.join("blobs")).unwrap();
    fs::write(ollama.join("blobs").join(blob), vec![1u8; 3 * MIB as usize]).unwrap();
    fs::write(ollama.join("blobs/sha256-2222"), b"{{ .Prompt }}").unwrap();
    let manifest = ollama.join("manifests/registry.ollama.ai/library/tiny/1b");
    fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    fs::write(&manifest, format!(
        r#"{{"schemaVersion":2,"layers":[{{"mediaType":"application/vnd.ollama.image.model","digest":"{}","size":3145728}},{{"mediaType":"application/vnd.ollama.image.template","digest":"sha256:2222","size":13}}]}}"#,
        blob.replacen('-', ":", 1)
    )).unwrap();

    let lms = home.join(".lmstudio/models/acme/two-shard");
    fs::create_dir_all(&lms).unwrap();
    fs::write(lms.join("model-00001-of-00002.safetensors"), vec![2u8; MIB as usize]).unwrap();
    fs::write(lms.join("model-00002-of-00002.safetensors"), vec![2u8; MIB as usize]).unwrap();
    fs::write(lms.join("config.json"), b"{}").unwrap();

    let hf = home.join(".cache/huggingface/hub/models--org--small");
    fs::create_dir_all(hf.join("blobs")).unwrap();
    fs::create_dir_all(hf.join("snapshots/abc")).unwrap();
    fs::write(hf.join("blobs/deadbeef"), vec![3u8; MIB as usize]).unwrap();
    symlink("../../blobs/deadbeef", hf.join("snapshots/abc/model.safetensors")).unwrap();
    fs::create_dir_all(hf.join("snapshots/def")).unwrap();
    symlink("../../blobs/deadbeef", hf.join("snapshots/def/model.safetensors")).unwrap();
    home
}

#[test]
fn discovers_ollama_lmstudio_and_huggingface_models() {
    let home = fake_home("home-discover");
    let models = discover(&home, &[]);
    let summary: Vec<(String, String, u64)> =
        models.iter().map(|m| (m.source.to_string(), m.name.clone(), m.bytes)).collect();
    assert!(summary.contains(&("ollama".into(), "tiny:1b".into(), 3 * MIB)), "{summary:?}");
    assert!(summary.contains(&("lmstudio".into(), "acme/two-shard".into(), 2 * MIB)), "{summary:?}");
    // Two snapshots point at one blob: counted once.
    assert!(summary.contains(&("huggingface".into(), "org/small".into(), MIB)), "{summary:?}");
    assert_eq!(models.len(), 3, "{summary:?}");
}

#[test]
fn extra_dir_finds_loose_weight_files() {
    let dir = scratch("extra");
    fs::write(dir.join("a.gguf"), vec![4u8; MIB as usize]).unwrap();
    fs::write(dir.join("notes.txt"), b"not a model").unwrap();
    let models = discover(&scratch("nohome"), std::slice::from_ref(&dir));
    assert_eq!(models.len(), 1);
    assert_eq!((models[0].source, models[0].name.as_str(), models[0].bytes), ("dir", "a.gguf", MIB));
}

#[test]
fn cli_json_lists_models_with_residency() {
    let home = fake_home("home-cli");
    let out = Command::new(env!("CARGO_BIN_EXE_glassbox")).args(["models", "--json", "--home"]).arg(&home).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let json = String::from_utf8(out.stdout).unwrap();
    assert!(json.contains("\"name\":\"tiny:1b\",\"source\":\"ollama\",\"bytes\":3145728,\"resident\":"), "{json}");
}
