//! Account command helpers: rendering the Emergency Kit to PDF.
//!
//! The plain-text and HTML renderings live in `toku-core` (pure, WASM/FFI-safe).
//! PDF generation lives here in the CLI so the heavier `printpdf` dependency
//! stays out of the core library.

use anyhow::{Context, Result};
use printpdf::{BuiltinFont, IndirectFontRef, Mm, PdfLayerReference};
use toku_core::{EMERGENCY_KIT_APP_LABEL, EMERGENCY_KIT_WARNING, EmergencyKit};

/// A4 page width in millimetres.
const PAGE_W: f32 = 210.0;
/// A4 page height in millimetres.
const PAGE_H: f32 = 297.0;
/// Left margin in millimetres.
const MARGIN_X: f32 = 20.0;

/// Render the Emergency Kit as a single-page A4 PDF and return the bytes.
pub fn render_pdf(kit: &EmergencyKit) -> Result<Vec<u8>> {
    let (doc, page, layer) =
        printpdf::PdfDocument::new("Toku Emergency Kit", Mm(PAGE_W), Mm(PAGE_H), "Layer 1");
    let regular = doc
        .add_builtin_font(BuiltinFont::Helvetica)
        .context("failed to load PDF font")?;
    let bold = doc
        .add_builtin_font(BuiltinFont::HelveticaBold)
        .context("failed to load PDF font")?;
    let mono = doc
        .add_builtin_font(BuiltinFont::Courier)
        .context("failed to load PDF font")?;

    let current = doc.get_page(page).get_layer(layer);

    // Vertical cursor measured from the top of the page (PDF origin is bottom-left).
    let mut y = PAGE_H - 25.0;

    let line = |layer: &PdfLayerReference,
                text: &str,
                size: f32,
                font: &IndirectFontRef,
                y: &mut f32,
                gap: f32| {
        layer.use_text(text, size, Mm(MARGIN_X), Mm(*y), font);
        *y -= gap;
    };

    line(
        &current,
        &format!("{EMERGENCY_KIT_APP_LABEL} — Emergency Kit"),
        22.0,
        &bold,
        &mut y,
        12.0,
    );
    line(
        &current,
        &format!("Created: {}", kit.created_at.format("%Y-%m-%d %H:%M UTC")),
        10.0,
        &regular,
        &mut y,
        16.0,
    );

    line(&current, "ACCOUNT", 9.0, &bold, &mut y, 6.0);
    line(&current, &kit.account_email, 12.0, &regular, &mut y, 14.0);

    line(&current, "SERVER", 9.0, &bold, &mut y, 6.0);
    line(
        &current,
        kit.server_url.as_deref().unwrap_or("(not set)"),
        12.0,
        &regular,
        &mut y,
        14.0,
    );

    line(&current, "SECRET KEY", 9.0, &bold, &mut y, 6.0);
    line(&current, &kit.secret_key, 16.0, &mono, &mut y, 16.0);

    line(&current, "PASSWORD", 9.0, &bold, &mut y, 6.0);
    line(
        &current,
        "________________________________________",
        14.0,
        &mono,
        &mut y,
        5.0,
    );
    line(
        &current,
        "(write your account password here, by hand)",
        9.0,
        &regular,
        &mut y,
        18.0,
    );

    // Warning, wrapped to a readable width.
    line(&current, "IMPORTANT", 10.0, &bold, &mut y, 7.0);
    for wrapped in wrap_text(EMERGENCY_KIT_WARNING, 70) {
        line(&current, &wrapped, 10.0, &regular, &mut y, 6.0);
    }

    let mut buf: Vec<u8> = Vec::new();
    {
        let mut writer = std::io::BufWriter::new(&mut buf);
        doc.save(&mut writer).context("failed to serialize PDF")?;
    }
    Ok(buf)
}

/// Greedy word-wrap to a maximum number of characters per line.
fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.len() + 1 + word.len() <= max_chars {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kit() -> EmergencyKit {
        EmergencyKit::new(
            "reader@example.com",
            Some("https://toku.example.com".to_string()),
            "TK-ABCDEF-GHIJK-LMNOP-QRSTU-VWXYZ-23",
        )
    }

    #[test]
    fn render_pdf_produces_pdf_bytes() {
        let bytes = render_pdf(&kit()).unwrap();
        assert!(bytes.starts_with(b"%PDF"), "output should be a PDF");
        assert!(bytes.len() > 500, "PDF should have content");
    }

    #[test]
    fn wrap_text_respects_width() {
        let wrapped = wrap_text("the quick brown fox jumps over", 10);
        assert!(wrapped.iter().all(|l| l.len() <= 10 || !l.contains(' ')));
        assert!(wrapped.len() > 1);
    }
}
