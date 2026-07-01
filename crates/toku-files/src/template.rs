//! Path-template engine for disk organization (issue #152, ADR-011 §4).
//!
//! Renders a configurable template such as `{author}/{title}.{format}` into a
//! filesystem-safe *relative* path. The template's `/` characters are the path
//! separators; every other segment is sanitized so the result is portable across
//! Linux, macOS, and Windows.

use crate::FileError;

/// Fallback used when a book has no author.
pub const UNKNOWN_AUTHOR: &str = "Unknown Author";
/// Fallback used when the template references `{series}` but the book has none.
pub const UNKNOWN_SERIES: &str = "No Series";
/// Fallback used when the template references `{year}` but no year is known.
pub const UNKNOWN_YEAR: &str = "Unknown Year";

/// Maximum length (in characters) of a single sanitized path segment. Keeps well
/// under common filesystem limits (255 bytes) while leaving room for a collision
/// suffix like ` (10)`.
const MAX_SEGMENT_LEN: usize = 200;

/// Values used to resolve template tokens for one file.
#[derive(Debug, Clone)]
pub struct TemplateContext {
    /// Primary author display name. Empty falls back to [`UNKNOWN_AUTHOR`].
    pub author: String,
    /// Book title. Always present.
    pub title: String,
    /// First series name, if any.
    pub series: Option<String>,
    /// File format extension (`epub`, `pdf`, `mobi`, `azw3`).
    pub format: String,
    /// Four-digit publication year, if known.
    pub year: Option<String>,
}

impl TemplateContext {
    /// Resolve a token name to its raw (pre-sanitization) value.
    fn resolve(&self, token: &str) -> Option<String> {
        let value = match token {
            "author" => {
                let a = self.author.trim();
                if a.is_empty() {
                    UNKNOWN_AUTHOR.to_string()
                } else {
                    a.to_string()
                }
            }
            "title" => self.title.clone(),
            "series" => self
                .series
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(UNKNOWN_SERIES)
                .to_string(),
            "format" => self.format.clone(),
            "year" => self
                .year
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(UNKNOWN_YEAR)
                .to_string(),
            _ => return None,
        };
        Some(value)
    }
}

/// A parsed path template. Cheap to clone and reuse across many files.
#[derive(Debug, Clone)]
pub struct PathTemplate {
    raw: String,
}

impl PathTemplate {
    /// Parse a template string, validating that every `{token}` is recognized and
    /// that the template is not empty / not absolute.
    pub fn parse(template: &str) -> Result<Self, FileError> {
        let trimmed = template.trim();
        if trimmed.is_empty() {
            return Err(FileError::Template(
                "template is empty; expected something like {author}/{title}.{format}".to_string(),
            ));
        }
        if trimmed.starts_with('/') || trimmed.starts_with('\\') {
            return Err(FileError::Template(
                "template must be relative (cannot start with a path separator)".to_string(),
            ));
        }

        // Validate tokens.
        for token in extract_tokens(trimmed)? {
            match token.as_str() {
                "author" | "title" | "series" | "format" | "year" => {}
                other => {
                    return Err(FileError::Template(format!(
                        "unknown template token {{{other}}} (supported: author, title, series, format, year)"
                    )));
                }
            }
        }

        Ok(Self {
            raw: trimmed.to_string(),
        })
    }

    /// Render the template into sanitized relative path segments.
    ///
    /// The returned vector is guaranteed non-empty and every segment is a
    /// filesystem-safe, non-empty string.
    pub fn render(&self, ctx: &TemplateContext) -> Result<Vec<String>, FileError> {
        // Split on both separators so a template authored on Windows still works.
        let raw_segments: Vec<&str> = self
            .raw
            .split(['/', '\\'])
            .filter(|s| !s.is_empty())
            .collect();

        let mut segments = Vec::with_capacity(raw_segments.len());
        for raw in raw_segments {
            let substituted = substitute(raw, ctx)?;
            let sanitized = sanitize_segment(&substituted);
            // Drop segments that are purely a path-navigation artifact.
            if sanitized == "." || sanitized == ".." {
                continue;
            }
            segments.push(sanitized);
        }

        if segments.is_empty() {
            return Err(FileError::Template(
                "template rendered to an empty path".to_string(),
            ));
        }
        Ok(segments)
    }
}

/// Extract the set of token names referenced by a template, validating brace
/// balance.
fn extract_tokens(template: &str) -> Result<Vec<String>, FileError> {
    let mut tokens = Vec::new();
    let mut chars = template.char_indices().peekable();
    while let Some((_, c)) = chars.next() {
        match c {
            '{' => {
                let mut name = String::new();
                let mut closed = false;
                for (_, tc) in chars.by_ref() {
                    if tc == '}' {
                        closed = true;
                        break;
                    }
                    if tc == '{' {
                        return Err(FileError::Template("nested '{' in template".to_string()));
                    }
                    name.push(tc);
                }
                if !closed {
                    return Err(FileError::Template("unclosed '{' in template".to_string()));
                }
                tokens.push(name);
            }
            '}' => {
                return Err(FileError::Template(
                    "unexpected '}' in template".to_string(),
                ));
            }
            _ => {}
        }
    }
    Ok(tokens)
}

/// Replace `{token}` occurrences in a single raw segment with resolved values.
fn substitute(segment: &str, ctx: &TemplateContext) -> Result<String, FileError> {
    let mut out = String::with_capacity(segment.len());
    let mut chars = segment.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            let mut name = String::new();
            for tc in chars.by_ref() {
                if tc == '}' {
                    break;
                }
                name.push(tc);
            }
            let value = ctx
                .resolve(&name)
                .ok_or_else(|| FileError::Template(format!("unknown template token {{{name}}}")))?;
            out.push_str(&value);
        } else {
            out.push(c);
        }
    }
    Ok(out)
}

/// Sanitize a single path segment so it is safe on Linux, macOS, and Windows.
///
/// - Replaces filesystem-illegal characters (`< > : " / \ | ? *`) and control
///   characters with `_`.
/// - Collapses runs of whitespace to a single space.
/// - Trims leading/trailing whitespace and dots (Windows rejects trailing dots).
/// - Escapes reserved Windows device names (CON, PRN, ...).
/// - Caps the length to [`MAX_SEGMENT_LEN`], preserving a trailing extension.
/// - Never returns an empty string (falls back to `_`).
pub fn sanitize_segment(segment: &str) -> String {
    // 1. Replace illegal / control characters.
    let mut cleaned = String::with_capacity(segment.len());
    let mut last_was_space = false;
    for c in segment.chars() {
        let mapped = if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            || (c as u32) < 0x20
        {
            '_'
        } else {
            c
        };
        if mapped.is_whitespace() {
            // Collapse whitespace runs.
            if !last_was_space {
                cleaned.push(' ');
                last_was_space = true;
            }
        } else {
            cleaned.push(mapped);
            last_was_space = false;
        }
    }

    // 2. Trim leading/trailing whitespace and dots.
    let trimmed = cleaned
        .trim_matches(|c: char| c.is_whitespace() || c == '.')
        .to_string();

    let mut result = if trimmed.is_empty() {
        "_".to_string()
    } else {
        trimmed
    };

    // 3. Escape reserved Windows device names (case-insensitive), comparing the
    //    stem before any extension.
    let stem = result.split('.').next().unwrap_or(&result);
    if is_reserved_device_name(stem) {
        result = format!("_{result}");
    }

    // 4. Length cap, preserving the extension where possible.
    if result.chars().count() > MAX_SEGMENT_LEN {
        result = truncate_preserving_ext(&result, MAX_SEGMENT_LEN);
    }

    result
}

fn is_reserved_device_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

/// Truncate `s` to at most `max` characters, keeping a trailing `.ext` (if short)
/// intact so the file remains recognizable by extension.
fn truncate_preserving_ext(s: &str, max: usize) -> String {
    if let Some(dot) = s.rfind('.') {
        let ext = &s[dot..];
        // Only preserve reasonably short extensions.
        if ext.chars().count() <= 10 && dot > 0 {
            let stem = &s[..dot];
            let keep = max.saturating_sub(ext.chars().count());
            let truncated: String = stem.chars().take(keep).collect();
            let truncated = truncated
                .trim_matches(|c: char| c.is_whitespace() || c == '.')
                .to_string();
            let truncated = if truncated.is_empty() {
                "_".to_string()
            } else {
                truncated
            };
            return format!("{truncated}{ext}");
        }
    }
    s.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> TemplateContext {
        TemplateContext {
            author: "Ursula K. Le Guin".to_string(),
            title: "The Left Hand of Darkness".to_string(),
            series: Some("Hainish Cycle".to_string()),
            format: "epub".to_string(),
            year: Some("1969".to_string()),
        }
    }

    #[test]
    fn renders_default_template() {
        let t = PathTemplate::parse("{author}/{title}.{format}").unwrap();
        let segs = t.render(&ctx()).unwrap();
        assert_eq!(
            segs,
            vec![
                "Ursula K. Le Guin".to_string(),
                "The Left Hand of Darkness.epub".to_string()
            ]
        );
    }

    #[test]
    fn renders_all_tokens() {
        let t = PathTemplate::parse("{series}/{year} - {title} ({author}).{format}").unwrap();
        let segs = t.render(&ctx()).unwrap();
        assert_eq!(segs[0], "Hainish Cycle");
        assert_eq!(
            segs[1],
            "1969 - The Left Hand of Darkness (Ursula K. Le Guin).epub"
        );
    }

    #[test]
    fn rejects_unknown_token() {
        assert!(PathTemplate::parse("{author}/{bogus}.{format}").is_err());
    }

    #[test]
    fn rejects_empty_and_absolute() {
        assert!(PathTemplate::parse("   ").is_err());
        assert!(PathTemplate::parse("/{title}").is_err());
    }

    #[test]
    fn rejects_unbalanced_braces() {
        assert!(PathTemplate::parse("{author/{title}").is_err());
        assert!(PathTemplate::parse("author}/{title}").is_err());
    }

    #[test]
    fn sanitizes_illegal_characters() {
        // A title with slashes, colons and quotes should not create subfolders.
        let mut c = ctx();
        c.title = "A: \"Weird\" <Title>/With\\Slashes?".to_string();
        let t = PathTemplate::parse("{title}.{format}").unwrap();
        let segs = t.render(&c).unwrap();
        assert_eq!(segs.len(), 1);
        let seg = &segs[0];
        assert!(!seg.contains('/'));
        assert!(!seg.contains('\\'));
        assert!(!seg.contains(':'));
        assert!(!seg.contains('"'));
        assert!(!seg.contains('?'));
        assert!(seg.ends_with(".epub"));
    }

    #[test]
    fn author_fallback_when_empty() {
        let mut c = ctx();
        c.author = "   ".to_string();
        let t = PathTemplate::parse("{author}/{title}.{format}").unwrap();
        let segs = t.render(&c).unwrap();
        assert_eq!(segs[0], UNKNOWN_AUTHOR);
    }

    #[test]
    fn series_and_year_fallbacks() {
        let mut c = ctx();
        c.series = None;
        c.year = None;
        let t = PathTemplate::parse("{series}/{year}/{title}.{format}").unwrap();
        let segs = t.render(&c).unwrap();
        assert_eq!(segs[0], UNKNOWN_SERIES);
        assert_eq!(segs[1], UNKNOWN_YEAR);
    }

    #[test]
    fn escapes_reserved_device_name() {
        let mut c = ctx();
        c.title = "CON".to_string();
        let t = PathTemplate::parse("{title}.{format}").unwrap();
        let segs = t.render(&c).unwrap();
        assert_eq!(segs[0], "_CON.epub");
    }

    #[test]
    fn trims_trailing_dots_and_spaces() {
        assert_eq!(sanitize_segment("name...  "), "name");
        assert_eq!(sanitize_segment("  spaced  name  "), "spaced name");
    }

    #[test]
    fn empty_segment_falls_back() {
        // Whitespace-only collapses to the "_" fallback.
        assert_eq!(sanitize_segment("   "), "_");
        // Illegal characters are replaced 1:1 with underscores (all safe).
        assert_eq!(sanitize_segment("???"), "___");
    }

    #[test]
    fn caps_long_segment_preserving_extension() {
        let long = "a".repeat(500);
        let seg = sanitize_segment(&format!("{long}.epub"));
        assert!(seg.chars().count() <= MAX_SEGMENT_LEN);
        assert!(seg.ends_with(".epub"));
    }

    #[test]
    fn ignores_dot_navigation_segments() {
        let mut c = ctx();
        c.author = "..".to_string();
        let t = PathTemplate::parse("{author}/{title}.{format}").unwrap();
        let segs = t.render(&c).unwrap();
        // ".." author sanitizes to "_" (dots trimmed), so it is kept but safe.
        assert_ne!(segs[0], "..");
    }
}
