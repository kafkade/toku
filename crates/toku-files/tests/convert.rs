//! Integration tests for the Calibre `ebook-convert` shell-out.
//!
//! These use a fake `ebook-convert` shell script so the tests do not require a
//! real Calibre install. They are Unix-only because the fakes are `/bin/sh`
//! scripts.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use toku_files::{ConvertError, Converter};

/// Write an executable fake `ebook-convert` script and return its path.
fn write_fake(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    path
}

#[test]
fn converts_successfully_with_fake_binary() {
    let dir = tempfile::tempdir().unwrap();
    // Fake converter: appends a marker so the output differs from the input.
    let bin = write_fake(
        dir.path(),
        "ebook-convert",
        "#!/bin/sh\n\
         if [ \"$1\" = \"--version\" ]; then echo fake; exit 0; fi\n\
         { cat \"$1\"; echo converted; } > \"$2\"\n\
         exit 0\n",
    );

    let src = dir.path().join("book.epub");
    fs::write(&src, b"epub bytes").unwrap();
    let dst = dir.path().join("book.mobi");

    let converter = Converter::with_binary(&bin);
    assert!(converter.is_available());
    converter.convert(&src, &dst).unwrap();

    assert!(dst.is_file(), "output file should exist");
    let contents = fs::read_to_string(&dst).unwrap();
    assert!(contents.contains("epub bytes"));
    assert!(contents.contains("converted"));
}

#[test]
fn surfaces_stderr_on_subprocess_failure() {
    let dir = tempfile::tempdir().unwrap();
    let bin = write_fake(
        dir.path(),
        "ebook-convert",
        "#!/bin/sh\n\
         if [ \"$1\" = \"--version\" ]; then echo fake; exit 0; fi\n\
         echo 'boom: unsupported input' 1>&2\n\
         exit 7\n",
    );

    let src = dir.path().join("book.epub");
    fs::write(&src, b"epub bytes").unwrap();
    let dst = dir.path().join("book.mobi");

    let converter = Converter::with_binary(&bin);
    let err = converter.convert(&src, &dst).unwrap_err();
    match err {
        ConvertError::Subprocess { code, stderr } => {
            assert_eq!(code, Some(7));
            assert!(stderr.contains("boom: unsupported input"));
        }
        other => panic!("expected Subprocess error, got {other:?}"),
    }
    assert!(!dst.exists(), "no output should be produced on failure");
}

#[test]
fn missing_binary_is_not_installed() {
    let converter = Converter::with_binary("toku-no-such-ebook-convert-binary");
    assert!(!converter.is_available());
    let err = converter.ensure_available().unwrap_err();
    assert!(matches!(err, ConvertError::NotInstalled { .. }));

    // `convert` should short-circuit with the same error, without touching disk.
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("book.epub");
    fs::write(&src, b"x").unwrap();
    let dst = dir.path().join("book.mobi");
    let err = converter.convert(&src, &dst).unwrap_err();
    assert!(matches!(err, ConvertError::NotInstalled { .. }));
    assert!(!dst.exists());
}
