//! Format conversion by shelling out to Calibre's `ebook-convert` CLI.
//!
//! Building an ebook converter in Rust is a multi-month effort; Calibre's is
//! mature, free, and battle-tested (see ROADMAP §6.2 / ADR-011 §5). The
//! dependency is **optional and never hard**: if `ebook-convert` is not on
//! `$PATH`, [`Converter::ensure_available`] returns [`ConvertError::NotInstalled`]
//! carrying actionable install guidance instead of panicking.
//!
//! **DRM-free files only.** Toku does not strip DRM and will not add DRM-removal
//! tooling.

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

/// Default Calibre conversion binary looked up on `$PATH`.
pub const DEFAULT_BINARY: &str = "ebook-convert";

/// Where to point users when `ebook-convert` is missing.
const INSTALL_URL: &str = "https://calibre-ebook.com/download";

/// Runs Calibre's `ebook-convert` to convert ebook files between formats.
///
/// The binary name/path is configurable ([`Converter::with_binary`]) so tests
/// can inject a stand-in without a real Calibre install.
#[derive(Debug, Clone)]
pub struct Converter {
    binary: OsString,
}

impl Default for Converter {
    fn default() -> Self {
        Self::new()
    }
}

impl Converter {
    /// A converter that invokes the default `ebook-convert` binary from `$PATH`.
    pub fn new() -> Self {
        Self {
            binary: OsString::from(DEFAULT_BINARY),
        }
    }

    /// A converter that invokes a specific binary name or path. Useful for
    /// tests, or when Calibre lives outside `$PATH`.
    pub fn with_binary(binary: impl Into<OsString>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    /// Returns `true` if the conversion binary can be executed.
    pub fn is_available(&self) -> bool {
        self.ensure_available().is_ok()
    }

    /// Verify the conversion binary is runnable, running `<binary> --version`.
    ///
    /// Maps a missing binary to [`ConvertError::NotInstalled`] with install
    /// guidance, so callers can surface a friendly message and exit cleanly
    /// rather than panicking.
    pub fn ensure_available(&self) -> Result<(), ConvertError> {
        match Command::new(&self.binary).arg("--version").output() {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(ConvertError::NotInstalled {
                binary: self.binary.to_string_lossy().into_owned(),
            }),
            Err(e) => Err(ConvertError::Io(e.to_string())),
        }
    }

    /// Convert `src` into `dst`, inferring both formats from their extensions
    /// (this is how `ebook-convert` itself decides). Returns the captured
    /// stderr on a non-zero exit via [`ConvertError::Subprocess`].
    ///
    /// The caller is responsible for validating `src` exists and that `dst`
    /// does not clobber anything it shouldn't.
    pub fn convert(&self, src: &Path, dst: &Path) -> Result<(), ConvertError> {
        self.ensure_available()?;

        let output = Command::new(&self.binary)
            .arg(src)
            .arg(dst)
            .output()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    ConvertError::NotInstalled {
                        binary: self.binary.to_string_lossy().into_owned(),
                    }
                } else {
                    ConvertError::Io(e.to_string())
                }
            })?;

        if output.status.success() {
            Ok(())
        } else {
            Err(ConvertError::Subprocess {
                code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            })
        }
    }
}

/// Errors raised while converting ebook formats.
#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error(
        "`{binary}` was not found on your PATH.\n\n\
         Format conversion is optional and relies on Calibre. Install it from\n\
         {INSTALL_URL} (the `ebook-convert` command ships with Calibre), then\n\
         ensure it is on your PATH and try again."
    )]
    NotInstalled { binary: String },

    #[error("`ebook-convert` exited with {}: {stderr}", match code { Some(c) => c.to_string(), None => "a signal".to_string() })]
    Subprocess { code: Option<i32>, stderr: String },

    #[error("I/O error running the converter: {0}")]
    Io(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_binary_reports_not_installed() {
        let converter = Converter::with_binary("toku-definitely-not-a-real-binary-xyz");
        let err = converter.ensure_available().unwrap_err();
        assert!(matches!(err, ConvertError::NotInstalled { .. }));
        assert!(!converter.is_available());
    }

    #[test]
    fn not_installed_message_is_actionable() {
        let err = ConvertError::NotInstalled {
            binary: "ebook-convert".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("ebook-convert"));
        assert!(msg.contains("Calibre"));
        assert!(msg.contains(INSTALL_URL));
    }

    #[test]
    fn subprocess_message_includes_stderr() {
        let err = ConvertError::Subprocess {
            code: Some(1),
            stderr: "boom: bad input".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("boom: bad input"));
        assert!(msg.contains('1'));
    }
}
