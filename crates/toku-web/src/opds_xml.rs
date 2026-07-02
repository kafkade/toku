//! OPDS 1.2 (Atom) feed construction for the e-reader catalog.
//!
//! Feeds are hand-built as strings (with strict XML escaping) to avoid pulling
//! in an XML crate. Two feed kinds exist per the OPDS spec: **navigation**
//! feeds (menus that link to other feeds) and **acquisition** feeds (lists of
//! books with download links).

use toku_files::FileFormat;

/// MIME type for an OPDS navigation feed.
pub const NAVIGATION_TYPE: &str = "application/atom+xml;profile=opds-catalog;kind=navigation";
/// MIME type for an OPDS acquisition feed.
pub const ACQUISITION_TYPE: &str = "application/atom+xml;profile=opds-catalog;kind=acquisition";
/// MIME type for an OpenSearch description document.
pub const OPENSEARCH_TYPE: &str = "application/opensearchdescription+xml";

/// Root path all OPDS routes hang off of.
pub const OPDS_ROOT: &str = "/opds";

/// Map an ebook [`FileFormat`] to the MIME type used for its acquisition link
/// and download response.
pub fn format_mime(format: FileFormat) -> &'static str {
    match format {
        FileFormat::Epub => "application/epub+zip",
        FileFormat::Pdf => "application/pdf",
        FileFormat::Mobi => "application/x-mobipocket-ebook",
        FileFormat::Azw3 => "application/vnd.amazon.ebook",
    }
}

/// An entry in a navigation feed: a link to another feed.
pub struct NavEntry {
    pub title: String,
    pub href: String,
    pub content: Option<String>,
    /// True when the target is an acquisition feed (a book list) rather than a
    /// nested navigation feed.
    pub acquisition: bool,
}

/// A single download link within an acquisition entry.
pub struct AcqLink {
    pub href: String,
    pub mime: String,
}

/// A book entry in an acquisition feed.
pub struct AcqEntry {
    /// Stable identifier, e.g. `urn:uuid:<book id>`.
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    /// RFC 3339 timestamp of the book's last update.
    pub updated: String,
    pub summary: Option<String>,
    pub language: Option<String>,
    pub isbns: Vec<String>,
    pub publisher_year: Option<String>,
    /// Cover image href (served from `/opds/cover/{hash}`), if any.
    pub cover_href: Option<String>,
    /// One acquisition link per associated ebook file.
    pub links: Vec<AcqLink>,
}

/// Escape a string for inclusion in XML text or attribute values.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Strip control chars that are illegal in XML 1.0.
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' => {}
            c => out.push(c),
        }
    }
    out
}

fn feed_header(
    out: &mut String,
    id: &str,
    title: &str,
    self_href: &str,
    self_type: &str,
    updated: &str,
) {
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(
        "<feed xmlns=\"http://www.w3.org/2005/Atom\" \
         xmlns:opds=\"http://opds-spec.org/2010/catalog\" \
         xmlns:dc=\"http://purl.org/dc/terms/\">\n",
    );
    out.push_str(&format!("  <id>{}</id>\n", xml_escape(id)));
    out.push_str(&format!("  <title>{}</title>\n", xml_escape(title)));
    out.push_str(&format!("  <updated>{}</updated>\n", xml_escape(updated)));
    out.push_str(&format!(
        "  <link rel=\"self\" href=\"{}\" type=\"{}\"/>\n",
        xml_escape(self_href),
        xml_escape(self_type),
    ));
    out.push_str(&format!(
        "  <link rel=\"start\" href=\"{OPDS_ROOT}\" type=\"{NAVIGATION_TYPE}\"/>\n",
    ));
    out.push_str(&format!(
        "  <link rel=\"search\" href=\"{OPDS_ROOT}/opensearch.xml\" type=\"{OPENSEARCH_TYPE}\"/>\n",
    ));
}

/// Build a navigation feed from a list of sub-feed links.
pub fn navigation_feed(
    id: &str,
    title: &str,
    self_href: &str,
    updated: &str,
    entries: &[NavEntry],
) -> String {
    let mut out = String::new();
    feed_header(&mut out, id, title, self_href, NAVIGATION_TYPE, updated);

    for e in entries {
        let link_type = if e.acquisition {
            ACQUISITION_TYPE
        } else {
            NAVIGATION_TYPE
        };
        out.push_str("  <entry>\n");
        out.push_str(&format!("    <title>{}</title>\n", xml_escape(&e.title)));
        out.push_str(&format!(
            "    <id>{}</id>\n",
            xml_escape(&format!("{id}:{}", e.href))
        ));
        out.push_str(&format!("    <updated>{}</updated>\n", xml_escape(updated)));
        if let Some(content) = &e.content {
            out.push_str(&format!(
                "    <content type=\"text\">{}</content>\n",
                xml_escape(content)
            ));
        }
        out.push_str(&format!(
            "    <link rel=\"subsection\" href=\"{}\" type=\"{}\"/>\n",
            xml_escape(&e.href),
            link_type,
        ));
        out.push_str("  </entry>\n");
    }

    out.push_str("</feed>\n");
    out
}

/// Build an acquisition feed from a list of book entries.
pub fn acquisition_feed(
    id: &str,
    title: &str,
    self_href: &str,
    updated: &str,
    entries: &[AcqEntry],
) -> String {
    let mut out = String::new();
    feed_header(&mut out, id, title, self_href, ACQUISITION_TYPE, updated);

    for e in entries {
        out.push_str("  <entry>\n");
        out.push_str(&format!("    <title>{}</title>\n", xml_escape(&e.title)));
        out.push_str(&format!("    <id>{}</id>\n", xml_escape(&e.id)));
        out.push_str(&format!(
            "    <updated>{}</updated>\n",
            xml_escape(&e.updated)
        ));
        for author in &e.authors {
            out.push_str("    <author>\n");
            out.push_str(&format!("      <name>{}</name>\n", xml_escape(author)));
            out.push_str("    </author>\n");
        }
        if let Some(lang) = &e.language {
            out.push_str(&format!(
                "    <dc:language>{}</dc:language>\n",
                xml_escape(lang)
            ));
        }
        if let Some(year) = &e.publisher_year {
            out.push_str(&format!(
                "    <dc:issued>{}</dc:issued>\n",
                xml_escape(year)
            ));
        }
        for isbn in &e.isbns {
            out.push_str(&format!(
                "    <dc:identifier>urn:isbn:{}</dc:identifier>\n",
                xml_escape(isbn)
            ));
        }
        if let Some(summary) = &e.summary {
            out.push_str(&format!(
                "    <summary type=\"text\">{}</summary>\n",
                xml_escape(summary)
            ));
        }
        if let Some(cover) = &e.cover_href {
            out.push_str(&format!(
                "    <link rel=\"http://opds-spec.org/image\" href=\"{}\" type=\"image/jpeg\"/>\n",
                xml_escape(cover)
            ));
            out.push_str(&format!(
                "    <link rel=\"http://opds-spec.org/image/thumbnail\" href=\"{}\" type=\"image/jpeg\"/>\n",
                xml_escape(cover)
            ));
        }
        for link in &e.links {
            out.push_str(&format!(
                "    <link rel=\"http://opds-spec.org/acquisition\" href=\"{}\" type=\"{}\"/>\n",
                xml_escape(&link.href),
                xml_escape(&link.mime),
            ));
        }
        out.push_str("  </entry>\n");
    }

    out.push_str("</feed>\n");
    out
}

/// Build the OpenSearch description document advertising the search endpoint.
pub fn opensearch_description() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <OpenSearchDescription xmlns=\"http://a9.com/-/spec/opensearch/1.1/\">\n\
         \x20 <ShortName>Toku</ShortName>\n\
         \x20 <Description>Search the Toku library</Description>\n\
         \x20 <InputEncoding>UTF-8</InputEncoding>\n\
         \x20 <Url type=\"{ACQUISITION_TYPE}\" \
         template=\"{OPDS_ROOT}/search?q={{searchTerms}}\"/>\n\
         </OpenSearchDescription>\n",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_xml_special_chars() {
        assert_eq!(
            xml_escape("a & b <c> \"d\" 'e'"),
            "a &amp; b &lt;c&gt; &quot;d&quot; &apos;e&apos;"
        );
    }

    #[test]
    fn strips_illegal_control_chars() {
        assert_eq!(xml_escape("a\u{0}b\u{1}c"), "abc");
        assert_eq!(xml_escape("keep\ttabs\nnewlines"), "keep\ttabs\nnewlines");
    }

    #[test]
    fn navigation_feed_has_expected_shape() {
        let feed = navigation_feed(
            "urn:toku:opds",
            "Toku Library",
            "/opds",
            "2024-01-01T00:00:00Z",
            &[NavEntry {
                title: "By Author".into(),
                href: "/opds/authors".into(),
                content: Some("Browse by author".into()),
                acquisition: false,
            }],
        );
        assert!(feed.contains("<feed"));
        assert!(feed.contains("profile=opds-catalog;kind=navigation"));
        assert!(feed.contains("rel=\"subsection\" href=\"/opds/authors\""));
        assert!(feed.contains("rel=\"search\""));
    }

    #[test]
    fn acquisition_feed_includes_download_and_cover() {
        let feed = acquisition_feed(
            "urn:toku:opds:all",
            "All Books",
            "/opds/all",
            "2024-01-01T00:00:00Z",
            &[AcqEntry {
                id: "urn:uuid:1234".into(),
                title: "Dune".into(),
                authors: vec!["Frank Herbert".into()],
                updated: "2024-01-01T00:00:00Z".into(),
                summary: Some("Desert planet".into()),
                language: Some("en".into()),
                isbns: vec!["9780441172719".into()],
                publisher_year: None,
                cover_href: Some("/opds/cover/abc".into()),
                links: vec![AcqLink {
                    href: "/opds/download/file1".into(),
                    mime: "application/epub+zip".into(),
                }],
            }],
        );
        assert!(feed.contains("<name>Frank Herbert</name>"));
        assert!(feed.contains("urn:isbn:9780441172719"));
        assert!(
            feed.contains("rel=\"http://opds-spec.org/acquisition\" href=\"/opds/download/file1\"")
        );
        assert!(feed.contains("rel=\"http://opds-spec.org/image\""));
    }

    #[test]
    fn format_mime_maps_all_variants() {
        assert_eq!(format_mime(FileFormat::Epub), "application/epub+zip");
        assert_eq!(format_mime(FileFormat::Pdf), "application/pdf");
        assert_eq!(
            format_mime(FileFormat::Mobi),
            "application/x-mobipocket-ebook"
        );
        assert_eq!(
            format_mime(FileFormat::Azw3),
            "application/vnd.amazon.ebook"
        );
    }
}
