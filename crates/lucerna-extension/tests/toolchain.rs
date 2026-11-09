use lucerna_extension::builder::BuildError;
use lucerna_extension::process::CancelToken;
use lucerna_extension::test_support::{fake_wasi_sdk, file_url};
use lucerna_extension::toolchain::{WASI_SDK_VERSION, ensure_wasi_sdk, ensure_wasi_sdk_from};
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::path::Path;
use std::process::Command;

fn wasm_fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../lucerna-syntax/tests/fixtures/wasm/json.wasm")
}

/// A `.tar.gz` holding `wasi-sdk-34.0-test/{VERSION,bin/clang}`.
fn fake_tarball(dir: &Path) -> std::path::PathBuf {
    let tree = dir.join("tree");
    fake_wasi_sdk(&tree.join("wasi-sdk-34.0-test"), &wasm_fixture());
    let tarball = dir.join("sdk.tar.gz");
    let ok = Command::new("tar")
        .arg("-czf")
        .arg(&tarball)
        .arg("-C")
        .arg(&tree)
        .arg("wasi-sdk-34.0-test")
        .status()
        .unwrap()
        .success();
    assert!(ok);
    tarball
}

fn sha256_hex(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap();
    Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn ensure_wasi_sdk_prefers_env() {
    let sdk = tempfile::tempdir().unwrap();
    fake_wasi_sdk(sdk.path(), &wasm_fixture());
    let root = tempfile::tempdir().unwrap();
    // SAFETY: this is the only test in this binary that reads or writes
    // WASI_SDK_PATH (the others call `ensure_wasi_sdk_from`).
    unsafe { std::env::set_var("WASI_SDK_PATH", sdk.path()) };
    let got = ensure_wasi_sdk(
        root.path(),
        &CancelToken::default(),
        &mut std::io::sink(),
        &|| {},
    );
    unsafe { std::env::remove_var("WASI_SDK_PATH") };
    assert_eq!(got.unwrap(), sdk.path());
    assert!(!root.path().join("toolchain").exists());
}

#[test]
fn ensure_wasi_sdk_reuses_existing_toolchain() {
    let root = tempfile::tempdir().unwrap();
    let dest = root.path().join("toolchain/wasi-sdk-34");
    fake_wasi_sdk(&dest, &wasm_fixture());
    let called = Cell::new(0);
    let got = ensure_wasi_sdk_from(
        root.path(),
        "file:///nonexistent/sdk.tar.gz",
        "00",
        &CancelToken::default(),
        &mut std::io::sink(),
        &|| called.set(called.get() + 1),
    )
    .unwrap();
    assert_eq!(got, dest);
    assert_eq!(called.get(), 0);
}

#[test]
fn ensure_wasi_sdk_downloads_verifies_and_extracts() {
    let tmp = tempfile::tempdir().unwrap();
    let tarball = fake_tarball(tmp.path());
    let root = tempfile::tempdir().unwrap();
    let called = Cell::new(0);
    let got = ensure_wasi_sdk_from(
        root.path(),
        &file_url(&tarball),
        &sha256_hex(&tarball),
        &CancelToken::default(),
        &mut std::io::sink(),
        &|| called.set(called.get() + 1),
    )
    .unwrap();
    assert_eq!(got, root.path().join("toolchain/wasi-sdk-34"));
    assert_eq!(
        std::fs::read_to_string(got.join("VERSION")).unwrap().trim(),
        WASI_SDK_VERSION
    );
    assert!(got.join("bin/clang").is_file());
    assert!(called.get() > 0);
    // Only the toolchain is left behind: no tarball, no temp dir.
    let names: Vec<_> = std::fs::read_dir(root.path().join("toolchain"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["wasi-sdk-34"]);
}

#[test]
fn ensure_wasi_sdk_rejects_bad_checksum() {
    let tmp = tempfile::tempdir().unwrap();
    let tarball = fake_tarball(tmp.path());
    let root = tempfile::tempdir().unwrap();
    let err = ensure_wasi_sdk_from(
        root.path(),
        &file_url(&tarball),
        &"0".repeat(64),
        &CancelToken::default(),
        &mut std::io::sink(),
        &|| {},
    )
    .unwrap_err();
    assert!(matches!(err, BuildError::Checksum), "{err:?}");
    assert!(!root.path().join("toolchain/wasi-sdk-34").exists());
    let left: Vec<_> = std::fs::read_dir(root.path().join("toolchain"))
        .map(|rd| rd.map(|e| e.unwrap().file_name()).collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "downloaded file not deleted: {left:?}");
}

#[test]
fn ensure_wasi_sdk_download_failure_is_process_error() {
    let root = tempfile::tempdir().unwrap();
    let err = ensure_wasi_sdk_from(
        root.path(),
        "file:///nonexistent/lucerna/sdk.tar.gz",
        &"0".repeat(64),
        &CancelToken::default(),
        &mut std::io::sink(),
        &|| {},
    )
    .unwrap_err();
    assert!(matches!(err, BuildError::Process(_)), "{err:?}");
}
