//! Streaming reader for a MediaWiki XML export.
//!
//! The export is one big file, so it is read as a stream and never held in
//! memory. Only the fields the pipeline needs are kept.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use anyhow::{Context, Result};
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};

/// One `<page>` of the export, before any wikitext parsing.
#[derive(Debug, Default, Clone)]
pub struct RawPage {
    pub title: String,
    pub page_id: u32,
    pub namespace: i32,
    /// From the `<redirect title="…"/>` element, which is authoritative.
    pub redirect_to: Option<String>,
    pub revision_id: u32,
    pub timestamp: String,
    pub text: String,
}

pub struct DumpReader<R: Read> {
    reader: Reader<BufReader<R>>,
    buf: Vec<u8>,
    /// Element names from `<mediawiki>` down to the current element. The path
    /// tells `<id>` inside `<page>` apart from `<id>` inside `<contributor>`.
    path: Vec<String>,
    page: Option<RawPage>,
}

impl DumpReader<File> {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        Ok(Self::new(file))
    }
}

impl<R: Read> DumpReader<R> {
    pub fn new(source: R) -> Self {
        let mut reader = Reader::from_reader(BufReader::with_capacity(1 << 20, source));
        reader.config_mut().trim_text(false);
        Self { reader, buf: Vec::new(), path: Vec::new(), page: None }
    }

    /// Path below `<page>`, joined with `/`. Empty when outside a page.
    fn field(&self) -> String {
        match self.path.iter().position(|e| e == "page") {
            Some(i) => self.path[i + 1..].join("/"),
            None => String::new(),
        }
    }

    fn on_start(&mut self, name: &str) {
        self.path.push(name.to_string());
        if name == "page" {
            self.page = Some(RawPage::default());
        }
    }

    fn on_empty(&mut self, e: &quick_xml::events::BytesStart) -> Result<()> {
        if e.name().local_name().into_inner() != "redirect" {
            return Ok(());
        }
        let Some(page) = self.page.as_mut() else { return Ok(()) };
        for attr in e.attributes() {
            let attr = attr?;
            if attr.key.local_name().into_inner() == "title" {
                page.redirect_to = Some(attr.normalized_value(XmlVersion::Explicit1_0)?.into_owned());
            }
        }
        Ok(())
    }

    fn on_text(&mut self, text: &str) {
        let field = self.field();
        let Some(page) = self.page.as_mut() else { return };
        match field.as_str() {
            "title" => page.title.push_str(text),
            "ns" => page.namespace = text.trim().parse().unwrap_or(0),
            // A page holds several `<id>` elements. Keep the first of each kind.
            "id" if page.page_id == 0 => page.page_id = text.trim().parse().unwrap_or(0),
            "revision/id" if page.revision_id == 0 => {
                page.revision_id = text.trim().parse().unwrap_or(0)
            }
            "revision/timestamp" if page.timestamp.is_empty() => page.timestamp.push_str(text),
            "revision/text" => page.text.push_str(text),
            _ => {}
        }
    }
}

impl<R: Read> Iterator for DumpReader<R> {
    type Item = Result<RawPage>;

    fn next(&mut self) -> Option<Self::Item> {
        // The buffer moves out of `self` for the call, because an event borrows
        // the buffer while the handlers below need `&mut self`.
        let mut buf = std::mem::take(&mut self.buf);
        let item = self.next_page(&mut buf);
        buf.clear();
        self.buf = buf;
        item
    }
}

impl<R: Read> DumpReader<R> {
    fn next_page(&mut self, buf: &mut Vec<u8>) -> Option<Result<RawPage>> {
        loop {
            buf.clear();
            let event = match self.reader.read_event_into(buf) {
                Ok(event) => event,
                Err(e) => return Some(Err(e.into())),
            };
            match event {
                Event::Start(e) => {
                    let name = e.name().local_name().into_inner().to_string();
                    self.on_start(&name);
                }
                Event::Empty(e) => {
                    let e = e.into_owned();
                    if let Err(err) = self.on_empty(&e) {
                        return Some(Err(err));
                    }
                }
                Event::Text(e) => {
                    let text = e.xml10_content().into_owned();
                    self.on_text(&text);
                }
                Event::CData(e) => {
                    let text = e.to_string();
                    self.on_text(&text);
                }
                // quick-xml reports `&lt;` and friends as their own event, so
                // text around an entity arrives in several pieces.
                Event::GeneralRef(e) => {
                    let text = resolve_entity(&e);
                    self.on_text(&text);
                }
                Event::End(e) => {
                    let is_page = e.name().local_name().into_inner() == "page";
                    self.path.pop();
                    if is_page && let Some(page) = self.page.take() {
                        return Some(Ok(page));
                    }
                }
                Event::Eof => return None,
                _ => {}
            }
        }
    }
}

/// Turns the body of `&…;` into the text it stands for. An entity that XML does
/// not define is kept as written, so nothing is silently lost.
fn resolve_entity(name: &str) -> String {
    if let Some(digits) = name.strip_prefix("#") {
        let code = match digits.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => digits.parse().ok(),
        };
        if let Some(c) = code.and_then(char::from_u32) {
            return c.to_string();
        }
    }
    // XML defines only lt, gt, amp, apos and quot. Anything else in a dump is
    // stored escaped, so it reaches us as text, not as a reference.
    match quick_xml::escape::resolve_xml_entity(name) {
        Some(text) => text.to_string(),
        None => format!("&{name};"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<mediawiki xmlns="http://www.mediawiki.org/xml/export-0.11/">
  <siteinfo><sitename>Wikiquote</sitename></siteinfo>
  <page>
    <title>Harry S Truman</title>
    <ns>0</ns>
    <id>26</id>
    <redirect title="Harry S. Truman" />
    <revision>
      <id>69978</id>
      <timestamp>2003-07-11T12:28:31Z</timestamp>
      <contributor><username>Nanobug</username><id>13</id></contributor>
      <text bytes="29">#REDIRECT [[Harry_S._Truman]]</text>
    </revision>
  </page>
  <page>
    <title>Talk:Something</title>
    <ns>1</ns>
    <id>27</id>
    <revision>
      <id>70000</id>
      <timestamp>2004-01-01T00:00:00Z</timestamp>
      <contributor><id>99</id></contributor>
      <text>* A quote with &lt;ref&gt; and &amp;amp; in it</text>
    </revision>
  </page>
</mediawiki>"#;

    fn read_all() -> Vec<RawPage> {
        DumpReader::new(SAMPLE.as_bytes()).map(|p| p.unwrap()).collect()
    }

    #[test]
    fn reads_both_pages() {
        assert_eq!(read_all().len(), 2);
    }

    #[test]
    fn takes_the_page_id_not_the_contributor_id() {
        let pages = read_all();
        assert_eq!(pages[0].page_id, 26);
        assert_eq!(pages[0].revision_id, 69978);
        assert_eq!(pages[1].page_id, 27);
        assert_eq!(pages[1].revision_id, 70000);
    }

    #[test]
    fn reads_the_redirect_element() {
        let pages = read_all();
        assert_eq!(pages[0].redirect_to.as_deref(), Some("Harry S. Truman"));
        assert_eq!(pages[1].redirect_to, None);
    }

    #[test]
    fn resolves_entities() {
        assert_eq!(resolve_entity("lt"), "<");
        assert_eq!(resolve_entity("#38"), "&");
        assert_eq!(resolve_entity("#x2014"), "\u{2014}");
        assert_eq!(resolve_entity("ndash"), "&ndash;");
    }

    #[test]
    fn unescapes_the_text() {
        let pages = read_all();
        assert_eq!(pages[1].text, "* A quote with <ref> and &amp; in it");
        assert_eq!(pages[1].namespace, 1);
        assert_eq!(pages[1].timestamp, "2004-01-01T00:00:00Z");
    }
}
