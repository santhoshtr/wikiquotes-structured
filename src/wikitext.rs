//! Turns a tree-sitter subtree into `RichText`.
//!
//! The grammar gives one node per markup element. This module walks those nodes
//! and produces the plain text a reader sees, plus spans that point back at the
//! links, emphasis and templates that were removed.

use tree_sitter::Node;

use crate::model::{PageRef, RichText, Site, Span, SpanKind, TemplateRef};

/// Builds `RichText` for the content of `node`.
///
/// `page_title` fills in `{{PAGENAME}}`, which the grammar leaves as literal
/// text. It is the only template-like thing this module substitutes.
pub fn rich_text(source: &str, node: Node, page_title: &str) -> RichText {
    let mut builder = Builder {
        source,
        page_title,
        out: RichText { wikitext: slice(source, node).to_string(), ..RichText::default() },
    };
    builder.children(node);
    builder.trim();
    builder.out
}

pub fn slice<'a>(source: &'a str, node: Node) -> &'a str {
    &source[node.byte_range()]
}

/// Reads a `template` node.
pub fn template_ref(source: &str, node: Node) -> TemplateRef {
    let mut name = String::new();
    let mut params: Vec<(String, String)> = Vec::new();
    let mut positional = 0u32;

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "template_name" => name = normalise_name(slice(source, child)),
            "template_argument" => {
                let mut key = None;
                let mut value = String::new();
                let mut inner = child.walk();
                for part in child.named_children(&mut inner) {
                    match part.kind() {
                        "template_param_name" => key = Some(slice(source, part).trim().to_string()),
                        "template_param_value" => value = slice(source, part).trim().to_string(),
                        _ => {}
                    }
                }
                let key = key.unwrap_or_else(|| {
                    positional += 1;
                    positional.to_string()
                });
                params.push((key, value));
            }
            _ => {}
        }
    }
    TemplateRef { name, params, wikitext: slice(source, node).to_string() }
}

fn normalise_name(raw: &str) -> String {
    raw.trim().replace('_', " ").to_lowercase()
}

/// Splits a wikilink target into a project, a title and a fragment.
pub fn page_ref(target: &str) -> PageRef {
    let target = target.trim().trim_start_matches(':').trim();
    let (title, fragment) = match target.split_once('#') {
        Some((t, f)) => (t.trim(), Some(f.trim().to_string())),
        None => (target, None),
    };

    let (site, rest) = match title.split_once(':') {
        Some((prefix, rest)) => match site_for_prefix(prefix.trim()) {
            Some(site) => (site, rest.trim()),
            None => (Site::Wikiquote, title),
        },
        None => (Site::Wikiquote, title),
    };

    // A project prefix can carry a language, as in "w:de:Berlin".
    let (lang, title) = match rest.split_once(':') {
        Some((prefix, rest)) if is_language_code(prefix) => (prefix.to_string(), rest.trim()),
        _ => ("en".to_string(), rest),
    };

    PageRef { site, lang, title: title.to_string(), fragment }
}

fn site_for_prefix(prefix: &str) -> Option<Site> {
    match prefix.to_lowercase().as_str() {
        "w" | "wikipedia" => Some(Site::Wikipedia),
        "s" | "wikisource" => Some(Site::Wikisource),
        "wikt" | "wiktionary" => Some(Site::Wiktionary),
        "commons" | "c" => Some(Site::Commons),
        "q" | "wikiquote" => Some(Site::Wikiquote),
        "b" | "n" | "v" | "m" | "meta" | "d" | "wikidata" | "voy" | "species" => {
            Some(Site::Other(prefix.to_lowercase()))
        }
        _ => None,
    }
}

fn is_language_code(prefix: &str) -> bool {
    let len = prefix.chars().count();
    (2..=3).contains(&len) && prefix.chars().all(|c| c.is_ascii_lowercase())
}

struct Builder<'a> {
    source: &'a str,
    page_title: &'a str,
    out: RichText,
}

impl<'a> Builder<'a> {
    fn at(&self) -> u32 {
        self.out.text.len() as u32
    }

    /// Removes the space around the text and moves the spans with it, so that
    /// every span still points at the words it describes.
    fn trim(&mut self) {
        let leading = (self.out.text.len() - self.out.text.trim_start().len()) as u32;
        self.out.text = self.out.text.trim().to_string();
        let end = self.out.text.len() as u32;
        for span in &mut self.out.spans {
            span.start = span.start.saturating_sub(leading).min(end);
            span.end = span.end.saturating_sub(leading).min(end);
        }
    }

    fn push(&mut self, text: &str) {
        self.out.text.push_str(text);
    }

    fn span(&mut self, start: u32, kind: SpanKind) {
        let end = self.at();
        self.out.spans.push(Span { start, end, kind });
    }

    fn children(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.visit(child);
        }
    }

    fn visit(&mut self, node: Node) {
        match node.kind() {
            "text" | "url" | "url_bare" | "raw_text" | "param_text" => {
                let text = slice(self.source, node);
                if self.page_title.is_empty() || !text.contains("{{PAGENAME}}") {
                    self.push(text);
                } else {
                    let filled = text.replace("{{PAGENAME}}", self.page_title);
                    self.push(&filled);
                }
            }
            "entity" => {
                let text = decode_entity(slice(self.source, node));
                self.push(&text);
            }
            "wikilink" => self.wikilink(node),
            "external_link" => self.external_link(node),
            "italic" => self.emphasis(node, SpanKind::Italic),
            "bold" => self.emphasis(node, SpanKind::Bold),
            "bold_italic" => self.emphasis(node, SpanKind::BoldItalic),
            "template" => self.template(node),
            "comment" => {
                let raw = slice(self.source, node);
                let text =
                    raw.trim_start_matches("<!--").trim_end_matches("-->").trim().to_string();
                let at = self.at();
                self.span(at, SpanKind::Comment { text });
            }
            "html_tag" => self.html_tag(node),
            "nowiki_inline_element" | "nowiki_tag_block" => {
                // Keep the escaped content, drop the <nowiki> wrapper.
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() == "raw_text" {
                        let text = slice(self.source, child).to_string();
                        self.push(&text);
                    }
                }
            }
            "magic_word" => {
                if slice(self.source, node).eq_ignore_ascii_case("{{PAGENAME}}") {
                    let title = self.page_title.to_string();
                    self.push(&title);
                }
            }
            // A signature renders as nothing useful in a quote.
            "signature" | "user_signature" | "user_signature_with_date" | "current_date"
            | "parser_function" => {}
            _ => self.children(node),
        }
    }

    fn emphasis(&mut self, node: Node, kind: SpanKind) {
        let start = self.at();
        self.children(node);
        self.span(start, kind);
    }

    fn wikilink(&mut self, node: Node) {
        let mut target = String::new();
        let mut label = None;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "wikilink_page" => target = slice(self.source, child).to_string(),
                // The segment after the pipe is the text the reader sees.
                "page_name_segment" => label = Some(slice(self.source, child).to_string()),
                _ => {}
            }
        }
        let shown = label.unwrap_or_else(|| display_title(&target));
        let start = self.at();
        self.push(&shown);
        self.span(start, SpanKind::Link { target: page_ref(&target) });
    }

    fn external_link(&mut self, node: Node) {
        let mut url = String::new();
        let mut label = None;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "url" => url = slice(self.source, child).to_string(),
                "page_name_segment" | "text" => {
                    label = Some(slice(self.source, child).trim().to_string())
                }
                _ => {}
            }
        }
        let start = self.at();
        self.push(label.as_deref().unwrap_or(""));
        self.span(start, SpanKind::ExternalLink { url });
    }

    fn html_tag(&mut self, node: Node) {
        let name = tag_name(self.source, node).to_lowercase();
        match name.as_str() {
            "br" => self.push("\n"),
            "ref" => {
                // A footnote is not part of what was said.
                let text = inner_text(self.source, node);
                let at = self.at();
                self.span(at, SpanKind::Reference { text });
            }
            _ => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() != "html_tag_name" && child.kind() != "html_attribute" {
                        self.visit(child);
                    }
                }
            }
        }
    }

    fn template(&mut self, node: Node) {
        let template = template_ref(self.source, node);
        let start = self.at();
        if template.name == "pagename" {
            let title = self.page_title.to_string();
            self.push(&title);
        } else if let Some(rendered) = render_template(&template) {
            self.push(&rendered);
        }
        // A {{w|…}} call is a Wikipedia link as well as text.
        if matches!(template.name.as_str(), "w") {
            if let Some(target) = template.param("1") {
                let target = page_ref(target);
                self.span(start, SpanKind::Link { target: PageRef { site: Site::Wikipedia, ..target } });
            }
        }
        self.span(start, SpanKind::Template(template));
    }
}

/// The text a small set of common templates puts on the page.
///
/// This is not template expansion. It covers only the templates that carry
/// words inside a quote. `{{w}}` alone appears 64,000 times. Anything else
/// renders as nothing and is kept as a span.
fn render_template(template: &TemplateRef) -> Option<String> {
    let param = |key: &str| template.param(key).filter(|v| !v.is_empty());
    match template.name.as_str() {
        "w" | "wikipedia-inline" => param("2").or_else(|| param("1")).map(str::to_string),
        "lang" => param("2").map(str::to_string),
        "smallcaps" | "sc" | "small" | "big" | "nowrap" | "center" | "resize" => {
            param("1").map(str::to_string)
        }
        "isbn" => param("1").map(|v| format!("ISBN {v}")),
        "nbsp" => Some("\u{00a0}".to_string()),
        "pb" | "pbr" | "line" | "-" => Some("\n".to_string()),
        _ => None,
    }
}

fn tag_name<'a>(source: &'a str, node: Node) -> &'a str {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|c| c.kind() == "html_tag_name")
        .map(|c| slice(source, c))
        .unwrap_or_default()
}

fn inner_text(source: &str, node: Node) -> String {
    let mut out = String::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() != "html_tag_name" && child.kind() != "html_attribute" {
            out.push_str(slice(source, child));
        }
    }
    out.trim().to_string()
}

/// `[[Foo bar]]` shows "Foo bar"; `[[w:Foo|]]` and `[[Help:Foo]]` show the part
/// a reader cares about.
fn display_title(target: &str) -> String {
    let target = target.trim().trim_start_matches(':').trim();
    let without_fragment = target.split('#').next().unwrap_or(target);
    match without_fragment.split_once(':') {
        Some((prefix, rest)) if site_for_prefix(prefix.trim()).is_some() => rest.trim().to_string(),
        _ => without_fragment.trim().to_string(),
    }
}

fn decode_entity(raw: &str) -> String {
    let name = raw.trim_start_matches('&').trim_end_matches(';');
    if let Some(digits) = name.strip_prefix('#') {
        let code = match digits.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => digits.parse().ok(),
        };
        if let Some(c) = code.and_then(char::from_u32) {
            return c.to_string();
        }
    }
    match quick_xml::escape::resolve_html5_entity(name) {
        Some(text) => text.to_string(),
        None => raw.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses `"* …"` and returns the `RichText` of the list item content.
    fn item(wikitext: &str) -> RichText {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_wikitext::LANGUAGE.into()).unwrap();
        let tree = parser.parse(wikitext, None).unwrap();
        let node = find(tree.root_node(), "list_item_content").expect("no list item");
        rich_text(wikitext, node, "Thomas Paine")
    }

    fn find<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
        if node.kind() == kind {
            return Some(node);
        }
        let mut cursor = node.walk();
        node.named_children(&mut cursor).find_map(|child| find(child, kind))
    }

    fn span_kinds(text: &RichText) -> Vec<&'static str> {
        text.spans
            .iter()
            .map(|s| match &s.kind {
                SpanKind::Link { .. } => "link",
                SpanKind::ExternalLink { .. } => "external",
                SpanKind::Italic => "italic",
                SpanKind::Bold => "bold",
                SpanKind::BoldItalic => "bold_italic",
                SpanKind::Template(_) => "template",
                SpanKind::Comment { .. } => "comment",
                SpanKind::Reference { .. } => "reference",
            })
            .collect()
    }

    #[test]
    fn keeps_the_words_a_reader_sees() {
        let text = item("* Plain [[Foo]] and [[w:Bar|Bar shown]] and ''emphasis''.");
        assert_eq!(text.text, "Plain Foo and Bar shown and emphasis.");
        assert_eq!(span_kinds(&text), ["link", "link", "italic"]);
    }

    #[test]
    fn spans_point_at_the_plain_text() {
        let text = item("* A [[Foo|bar]] c");
        let span = &text.spans[0];
        assert_eq!(&text.text[span.start as usize..span.end as usize], "bar");
    }

    #[test]
    fn reads_the_link_target() {
        let text = item("* [[w:de:Berlin|Berlin]] and [[Baz#Sec]] and [[:Category:X|C]]");
        let targets: Vec<_> = text
            .spans
            .iter()
            .filter_map(|s| match &s.kind {
                SpanKind::Link { target } => Some(target.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(targets[0].site, Site::Wikipedia);
        assert_eq!(targets[0].lang, "de");
        assert_eq!(targets[0].title, "Berlin");
        assert_eq!(targets[1].site, Site::Wikiquote);
        assert_eq!(targets[1].title, "Baz");
        assert_eq!(targets[1].fragment.as_deref(), Some("Sec"));
        assert_eq!(targets[2].title, "Category:X");
    }

    #[test]
    fn shows_a_bare_link_without_its_project_prefix() {
        assert_eq!(item("* See [[w:Nikola Tesla]].").text, "See Nikola Tesla.");
    }

    #[test]
    fn renders_the_templates_that_carry_words() {
        assert_eq!(item("* By {{w|Nikola Tesla|Tesla}} today").text, "By Tesla today");
        assert_eq!(item("* {{lang|fr|bonjour}} all").text, "bonjour all");
        assert_eq!(item("* {{ISBN|0-306-40615-2}}").text, "ISBN 0-306-40615-2");
    }

    #[test]
    fn an_unknown_template_leaves_no_text_but_keeps_a_span() {
        let text = item("* Quote {{cite book|title=T|year=1990}}");
        assert_eq!(text.text, "Quote");
        match &text.spans[0].kind {
            SpanKind::Template(t) => {
                assert_eq!(t.name, "cite book");
                assert_eq!(t.param("title"), Some("T"));
                assert_eq!(t.param("year"), Some("1990"));
            }
            other => panic!("expected a template span, found {other:?}"),
        }
    }

    #[test]
    fn positional_template_parameters_are_numbered() {
        let text = item("* {{unknown|first|second|key=value}}");
        match &text.spans[0].kind {
            SpanKind::Template(t) => {
                assert_eq!(t.param("1"), Some("first"));
                assert_eq!(t.param("2"), Some("second"));
                assert_eq!(t.param("key"), Some("value"));
            }
            other => panic!("expected a template span, found {other:?}"),
        }
    }

    #[test]
    fn drops_comments_and_footnotes_from_the_text() {
        let text = item("* Said <!-- check this --> it<ref>Smith, p. 1</ref> once");
        assert_eq!(text.text, "Said  it once");
        assert_eq!(span_kinds(&text), ["comment", "reference"]);
    }

    #[test]
    fn decodes_entities() {
        assert_eq!(item("* a &ndash; b &amp; c &#38; d").text, "a – b & c & d");
    }

    #[test]
    fn keeps_nowiki_content() {
        assert_eq!(item("* shows <nowiki>[[raw]]</nowiki> here").text, "shows [[raw]] here");
    }

    #[test]
    fn fills_in_the_page_name() {
        assert_eq!(item("* About {{PAGENAME}} only").text, "About Thomas Paine only");
    }

    #[test]
    fn keeps_the_source_wikitext() {
        let text = item("* Plain [[Foo]].");
        assert_eq!(text.wikitext, " Plain [[Foo]].");
    }
}
