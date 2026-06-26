//! Account command helpers: rendering the Emergency Kit to PDF.
//!
//! The plain-text and HTML renderings live in `toku-core` (pure, WASM/FFI-safe).
//! PDF generation lives here in the CLI so the heavier `pdf-writer` dependency
//! stays out of the core library. `pdf-writer` is a lightweight, well-maintained
//! crate (no `lopdf` in its dependency tree) used to emit a single-page document
//! with the standard base-14 fonts — no font embedding required.

use anyhow::Result;
use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref, Str};
use toku_core::{EMERGENCY_KIT_APP_LABEL, EMERGENCY_KIT_WARNING, EmergencyKit};

/// A4 page width in PDF points (1/72 inch).
const PAGE_W: f32 = 595.28;
/// A4 page height in PDF points.
const PAGE_H: f32 = 841.89;
/// Left margin in PDF points (~20 mm).
const MARGIN_X: f32 = 56.7;

/// Resource name for the regular (Helvetica) font.
const FONT_REGULAR: &[u8] = b"F1";
/// Resource name for the bold (Helvetica-Bold) font.
const FONT_BOLD: &[u8] = b"F2";
/// Resource name for the monospace (Courier) font.
const FONT_MONO: &[u8] = b"F3";

/// Draw a single line of text at the current vertical cursor and advance it.
///
/// Positions use absolute text matrices with the PDF bottom-left origin, so
/// `y` is measured down from the top of the page for readability.
fn draw_line(content: &mut Content, text: &str, size: f32, font: &[u8], y: &mut f32, gap: f32) {
    let sanitized = sanitize(text);
    content.set_font(Name(font), size);
    content.set_text_matrix([1.0, 0.0, 0.0, 1.0, MARGIN_X, PAGE_H - *y]);
    content.show(Str(sanitized.as_bytes()));
    *y += gap;
}

/// Map text to the printable ASCII range covered by the base-14 fonts'
/// standard encoding, replacing common typographic characters with ASCII
/// equivalents and dropping anything else.
fn sanitize(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            '\u{2014}' | '\u{2013}' => Some('-'),
            '\u{2018}' | '\u{2019}' => Some('\''),
            '\u{201C}' | '\u{201D}' => Some('"'),
            ' ' => Some(' '),
            c if c.is_ascii_graphic() => Some(c),
            _ => None,
        })
        .collect()
}

/// Render the Emergency Kit as a single-page A4 PDF and return the bytes.
pub fn render_pdf(kit: &EmergencyKit) -> Result<Vec<u8>> {
    let mut alloc = Ref::new(1);
    let catalog_id = alloc.bump();
    let page_tree_id = alloc.bump();
    let page_id = alloc.bump();
    let content_id = alloc.bump();
    let regular_id = alloc.bump();
    let bold_id = alloc.bump();
    let mono_id = alloc.bump();

    // Build the page content stream.
    let mut content = Content::new();
    content.begin_text();

    // Vertical cursor measured from the top of the page; `draw_line` advances it.
    let mut y = 70.0;

    draw_line(
        &mut content,
        &format!("{EMERGENCY_KIT_APP_LABEL} - Emergency Kit"),
        22.0,
        FONT_BOLD,
        &mut y,
        34.0,
    );
    draw_line(
        &mut content,
        &format!("Created: {}", kit.created_at.format("%Y-%m-%d %H:%M UTC")),
        10.0,
        FONT_REGULAR,
        &mut y,
        40.0,
    );

    draw_line(&mut content, "ACCOUNT", 9.0, FONT_BOLD, &mut y, 16.0);
    draw_line(
        &mut content,
        &kit.account_email,
        12.0,
        FONT_REGULAR,
        &mut y,
        34.0,
    );

    draw_line(&mut content, "SERVER", 9.0, FONT_BOLD, &mut y, 16.0);
    draw_line(
        &mut content,
        kit.server_url.as_deref().unwrap_or("(not set)"),
        12.0,
        FONT_REGULAR,
        &mut y,
        34.0,
    );

    draw_line(&mut content, "SECRET KEY", 9.0, FONT_BOLD, &mut y, 16.0);
    draw_line(&mut content, &kit.secret_key, 15.0, FONT_MONO, &mut y, 40.0);

    draw_line(&mut content, "PASSWORD", 9.0, FONT_BOLD, &mut y, 16.0);
    draw_line(
        &mut content,
        "________________________________________",
        14.0,
        FONT_MONO,
        &mut y,
        14.0,
    );
    draw_line(
        &mut content,
        "(write your account password here, by hand)",
        9.0,
        FONT_REGULAR,
        &mut y,
        44.0,
    );

    // Warning, wrapped to a readable width.
    draw_line(&mut content, "IMPORTANT", 10.0, FONT_BOLD, &mut y, 18.0);
    for wrapped in wrap_text(EMERGENCY_KIT_WARNING, 70) {
        draw_line(&mut content, &wrapped, 10.0, FONT_REGULAR, &mut y, 16.0);
    }

    content.end_text();
    let content_data = content.finish();

    // Assemble the document structure.
    let mut pdf = Pdf::new();
    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.pages(page_tree_id).kids([page_id]).count(1);

    let mut page = pdf.page(page_id);
    page.parent(page_tree_id);
    page.media_box(Rect::new(0.0, 0.0, PAGE_W, PAGE_H));
    page.contents(content_id);
    {
        let mut resources = page.resources();
        let mut fonts = resources.fonts();
        fonts.pair(Name(FONT_REGULAR), regular_id);
        fonts.pair(Name(FONT_BOLD), bold_id);
        fonts.pair(Name(FONT_MONO), mono_id);
    }
    page.finish();

    // Standard base-14 Type1 fonts; no embedding required.
    pdf.type1_font(regular_id).base_font(Name(b"Helvetica"));
    pdf.type1_font(bold_id).base_font(Name(b"Helvetica-Bold"));
    pdf.type1_font(mono_id).base_font(Name(b"Courier"));

    pdf.stream(content_id, &content_data);

    Ok(pdf.finish())
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
