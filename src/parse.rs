//! Builds the Layer 1 document model from wikitext.
//!
//! The grammar returns a nested tree of `section` nodes but a flat list of list
//! items: `**` is a sibling of `*` with a longer marker. Rebuilding that
//! parent-child link is the main job here.

use tree_sitter::Node;

use crate::classify;
use crate::dump::RawPage;
use crate::model::*;
use crate::roles;
use crate::wikitext::{page_ref, rich_text, slice, template_ref};

pub struct DocumentBuilder {
    parser: tree_sitter::Parser,
    /// A heading holds its markup as plain text, so its content has to be
    /// parsed on its own. A second parser keeps that out of the main one.
    heading_parser: tree_sitter::Parser,
}

impl Default for DocumentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentBuilder {
    pub fn new() -> Self {
        Self {
            parser: new_parser(),
            heading_parser: new_parser(),
        }
    }

    pub fn build(&mut self, page: &RawPage, wiki_language: &str) -> Document {
        let mut document = Document {
            wiki_language: wiki_language.to_string(),
            title: page.title.clone(),
            page_id: page.page_id,
            revision_id: page.revision_id,
            timestamp: page.timestamp.clone(),
            redirect_to: page.redirect_to.as_deref().map(page_ref),
            page_type: PageType::Unknown,
            page_type_evidence: Vec::new(),
            categories: Vec::new(),
            templates: Vec::new(),
            interwiki: Vec::new(),
            lead: Vec::new(),
            sections: Vec::new(),
            parse: ParseStats {
                bytes: page.text.len() as u32,
                ..ParseStats::default()
            },
        };
        if document.redirect_to.is_some() {
            return document;
        }

        let source = &page.text;
        let Some(tree) = self.parser.parse(source, None) else {
            return document;
        };
        let root = tree.root_node();

        let mut page_builder = PageBuilder::new(source, &page.title, &mut self.heading_parser);
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            if child.kind() != "section" {
                page_builder.mark(child);
                continue;
            }
            match heading_of(child) {
                Some(_) => {
                    let section = page_builder.section(child);
                    document.sections.push(section);
                }
                // A section with no heading holds the text above the first one.
                None => {
                    let (blocks, nested) = page_builder.body(child, &SectionRole::Quotes);
                    document.lead.extend(blocks);
                    document.sections.extend(nested);
                }
            }
        }

        document.categories = page_builder.categories;
        document.templates = document
            .lead
            .iter()
            .filter_map(block_template)
            .cloned()
            .collect::<Vec<_>>();
        document.interwiki = interwiki_of(&document.templates, &page.title);
        (document.page_type, document.page_type_evidence) = classify::classify(&document);
        document.parse.error_nodes = count_errors(root);
        document.parse.source_lines =
            source.lines().filter(|l| !l.trim().is_empty()).count() as u32;
        document.parse.unassigned_lines = page_builder.coverage.unassigned(source);
        document
    }
}

fn new_parser() -> tree_sitter::Parser {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_wikitext::LANGUAGE.into())
        .expect("the wikitext grammar must load");
    parser
}

fn block_template(block: &Block) -> Option<&TemplateRef> {
    match block {
        Block::Template(template) => Some(template),
        _ => None,
    }
}

/// Records which source lines reached a node, so the report can name the lines
/// the model dropped.
struct Coverage {
    lines: Vec<bool>,
}

impl Coverage {
    fn new(source: &str) -> Self {
        Self {
            lines: vec![false; source.lines().count() + 1],
        }
    }

    fn mark(&mut self, node: Node) {
        let (from, to) = (node.start_position().row, node.end_position().row);
        for line in from..=to.min(self.lines.len().saturating_sub(1)) {
            self.lines[line] = true;
        }
    }

    fn unassigned(&self, source: &str) -> u32 {
        source
            .lines()
            .enumerate()
            .filter(|(index, line)| {
                !line.trim().is_empty() && !self.lines.get(*index).copied().unwrap_or(false)
            })
            .count() as u32
    }
}

struct PageBuilder<'a> {
    source: &'a str,
    title: &'a str,
    heading_parser: &'a mut tree_sitter::Parser,
    categories: Vec<String>,
    coverage: Coverage,
}

impl<'a> PageBuilder<'a> {
    fn new(source: &'a str, title: &'a str, heading_parser: &'a mut tree_sitter::Parser) -> Self {
        Self {
            source,
            title,
            heading_parser,
            categories: Vec::new(),
            coverage: Coverage::new(source),
        }
    }

    fn mark(&mut self, node: Node) {
        self.coverage.mark(node);
    }

    fn text(&mut self, node: Node) -> RichText {
        self.mark(node);
        rich_text(self.source, node, self.title)
    }

    fn section(&mut self, node: Node) -> Section {
        let heading_node = heading_of(node).expect("caller checked for a heading");
        let level = heading_node.kind().as_bytes()[7] - b'0';
        let heading = self.heading_text(heading_node);
        let role = roles::role_of(&heading.text);
        let is_italic = heading.wikitext.starts_with("''");
        let source_hint = roles::source_hint(&heading.text, is_italic);
        let (blocks, sections) = self.body(node, &role);
        Section {
            level,
            heading,
            role,
            source_hint,
            blocks,
            sections,
        }
    }

    /// The grammar leaves the inside of a heading as plain text, so `''X''`
    /// and `[[X|Y]]` arrive as literal markup. Parse that text on its own.
    fn heading_text(&mut self, node: Node) -> RichText {
        self.mark(node);
        let inner = slice(self.source, node).trim().trim_matches('=').trim();
        let Some(tree) = self.heading_parser.parse(inner, None) else {
            return RichText {
                text: inner.to_string(),
                wikitext: inner.to_string(),
                spans: vec![],
            };
        };
        let mut text = rich_text(inner, tree.root_node(), self.title);
        text.wikitext = inner.to_string();
        text
    }

    /// Splits a section node into its own blocks and its nested sections.
    fn body(&mut self, node: Node, role: &SectionRole) -> (Vec<Block>, Vec<Section>) {
        let mut blocks = Vec::new();
        let mut sections = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "section" => match heading_of(child) {
                    Some(_) => sections.push(self.section(child)),
                    None => {
                        let (inner_blocks, inner_sections) = self.body(child, role);
                        blocks.extend(inner_blocks);
                        sections.extend(inner_sections);
                    }
                },
                kind if kind.starts_with("heading") => {}
                _ => self.block(child, role, &mut blocks),
            }
        }
        (blocks, sections)
    }

    fn block(&mut self, node: Node, role: &SectionRole, out: &mut Vec<Block>) {
        match node.kind() {
            "unordered_list" | "ordered_list" => self.list(node, role, out),
            "definition_list" => self.definition_list(node, out),
            "paragraph" => self.paragraph(node, out),
            "table" => {
                self.mark(node);
                out.push(Block::Table {
                    wikitext: slice(self.source, node).to_string(),
                });
            }
            "horizontal_rule" => {
                self.mark(node);
                out.push(Block::Rule);
            }
            _ => {
                let text = self.text(node);
                if !text.is_empty() {
                    out.push(Block::Paragraph(text));
                }
            }
        }
    }

    /// Rebuilds the parent-child link the grammar leaves flat: a `*` item opens
    /// a quote, and every `**` or deeper item below it is one of its
    /// annotations.
    fn list(&mut self, node: Node, role: &SectionRole, out: &mut Vec<Block>) {
        let mut cursor = node.walk();
        let mut current: Option<QuoteItem> = None;
        for item in node.named_children(&mut cursor) {
            if !item.kind().ends_with("list_item") {
                continue;
            }
            let depth = marker_depth(self.source, item);
            let Some(content) = child_of_kind(item, "list_item_content") else {
                continue;
            };
            let text = self.text(content);
            if depth <= 1 || current.is_none() {
                if let Some(previous) = current.take() {
                    out.push(finish_item(previous, role));
                }
                current = Some(QuoteItem {
                    text,
                    annotations: Vec::new(),
                });
            } else if let Some(item) = current.as_mut() {
                let kind = annotation_kind(&text);
                item.annotations.push(Annotation { depth, text, kind });
            }
        }
        if let Some(previous) = current.take() {
            out.push(finish_item(previous, role));
        }
    }

    /// A run of `:` lines. A `<hr>` between two runs ends the exchange, so one
    /// definition list is one exchange.
    fn definition_list(&mut self, node: Node, out: &mut Vec<Block>) {
        let mut cursor = node.walk();
        let mut turns = Vec::new();
        for line in node.named_children(&mut cursor) {
            let Some(content) = child_of_kind(line, "list_item_content") else {
                continue;
            };
            let text = self.text(content);
            if text.is_empty() {
                continue;
            }
            turns.push(self.turn(content, text));
        }
        if !turns.is_empty() {
            out.push(Block::Exchange(Exchange {
                turns,
                annotations: Vec::new(),
            }));
        }
    }

    fn turn(&mut self, content: Node, text: RichText) -> Turn {
        let first = content.named_child(0);
        let is_bold = first.map(|n| n.kind() == "bold").unwrap_or(false);
        // A stage direction is a whole line in italics, as in `:''[he waves]''`.
        let is_stage_direction = first
            .map(|n| n.kind() == "italic" && slice(self.source, n).trim() == text.wikitext.trim())
            .unwrap_or(false);

        let mut speaker = None;
        let mut speaker_link = None;
        if is_bold {
            let name = rich_text(self.source, first.unwrap(), self.title);
            let name_text = name.text.trim().trim_end_matches(':').trim().to_string();
            // The bold run is a speaker only when a colon follows it.
            let after = &self.source[first.unwrap().end_byte()..content.end_byte()];
            if !name_text.is_empty()
                && (after.trim_start().starts_with(':') || name.text.trim_end().ends_with(':'))
            {
                speaker_link = name.spans.iter().find_map(|span| match &span.kind {
                    SpanKind::Link { target } => Some(target.clone()),
                    _ => None,
                });
                speaker = Some(name_text);
            }
        }

        let text = match &speaker {
            Some(_) => strip_speaker(text),
            None => text,
        };
        Turn {
            speaker,
            speaker_link,
            text,
            is_stage_direction,
        }
    }

    fn paragraph(&mut self, node: Node, out: &mut Vec<Block>) {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.named_children(&mut cursor).collect();

        let mut plain = Vec::new();
        for child in &children {
            match child.kind() {
                "medialink" => {
                    self.mark(*child);
                    out.push(Block::Media(self.media(*child)));
                }
                "wikilink" if self.category_of(*child).is_some() => {
                    self.mark(*child);
                    let category = self.category_of(*child).unwrap();
                    self.categories.push(category);
                }
                "html_tag" if is_rule(self.source, *child) => {
                    self.mark(*child);
                    out.push(Block::Rule);
                }
                _ => plain.push(*child),
            }
        }
        if plain.is_empty() {
            return;
        }
        if plain.len() == 1 && plain[0].kind() == "template" {
            self.mark(plain[0]);
            out.push(Block::Template(template_ref(self.source, plain[0])));
            return;
        }
        // Rebuild the remaining run as one paragraph.
        let from = plain.first().unwrap().start_byte();
        let to = plain.last().unwrap().end_byte();
        self.mark(node);
        let mut text = rich_text(self.source, node, self.title);
        text.wikitext = self.source[from..to].to_string();
        if !text.is_empty() {
            out.push(Block::Paragraph(text));
        }
    }

    fn media(&mut self, node: Node) -> Media {
        let file = child_of_kind(node, "filename")
            .map(|n| slice(self.source, n).trim().to_string())
            .unwrap_or_default();
        let caption = child_of_kind(node, "file_caption").map(|n| self.text(n));
        // A caption often repeats a quote and names its author after a tilde.
        let attribution = caption.as_ref().and_then(|c| {
            c.text
                .rsplit_once(" ~ ")
                .map(|(_, author)| author.trim().to_string())
        });
        Media {
            file,
            caption,
            caption_attribution: attribution,
        }
    }

    fn category_of(&self, node: Node) -> Option<String> {
        let page = child_of_kind(node, "wikilink_page")?;
        let title = slice(self.source, page).trim();
        let rest = title
            .strip_prefix("Category:")
            .or_else(|| title.strip_prefix("category:"))?;
        Some(rest.trim().to_string())
    }
}

/// Non-quote sections hold plain list entries, not quotes. An entry with an
/// annotation stays a quote, so nothing is lost when the role is wrong.
fn finish_item(item: QuoteItem, role: &SectionRole) -> Block {
    let is_list = matches!(
        role,
        SectionRole::Cast
            | SectionRole::SeeAlso
            | SectionRole::ExternalLinks
            | SectionRole::References
    );
    if is_list && item.annotations.is_empty() {
        Block::Item(item.text)
    } else {
        Block::Quote(item)
    }
}

/// Removes the `Name:` that opens a dialogue turn, and moves the spans with it.
fn strip_speaker(mut text: RichText) -> RichText {
    let Some(colon) = text.text.find(':') else {
        return text;
    };
    let cut = text.text[colon + 1..].len();
    let removed = (text.text.len() - cut) as u32;
    // Some pages write ":'''Name''': :''[stage direction]''", with a second
    // colon that belongs to nothing.
    text.text = text.text[colon + 1..]
        .trim_start()
        .trim_start_matches(':')
        .trim_start()
        .to_string();
    let dropped = removed + (cut - text.text.len()) as u32;
    let end = text.text.len() as u32;
    text.spans.retain(|span| span.end > dropped);
    for span in &mut text.spans {
        span.start = span.start.saturating_sub(dropped).min(end);
        span.end = span.end.saturating_sub(dropped).min(end);
    }
    text
}

/// A first guess at what an annotation is for. Layer 2 refines it.
fn annotation_kind(text: &RichText) -> AnnotationKind {
    let lower = text.text.trim().to_lowercase();
    // A proverb page gives the words in the original script, then a
    // transliteration and a meaning, each on its own line.
    for label in [
        "translation",
        "translated",
        "transliteration",
        "meaning",
        "literally",
    ] {
        if lower.starts_with(label) {
            return AnnotationKind::Translation;
        }
    }
    if lower.starts_with("compare") || lower.starts_with("variant") || lower.starts_with("see also")
    {
        return AnnotationKind::CrossRef;
    }
    if !lower.is_empty() {
        return AnnotationKind::Citation;
    }
    // A line that is only a {{cite …}} call or a link renders as no text at
    // all, but it is still the citation.
    let has_citation = text.spans.iter().any(|span| match &span.kind {
        SpanKind::Template(template) => {
            template.name.starts_with("cite") || template.name == "citation"
        }
        SpanKind::ExternalLink { url } => !url.is_empty(),
        _ => false,
    });
    if has_citation {
        AnnotationKind::Citation
    } else {
        AnnotationKind::Note
    }
}

fn heading_of(section: Node) -> Option<Node> {
    let mut cursor = section.walk();
    section.named_children(&mut cursor).find(|c| {
        c.kind().starts_with("heading") && c.kind().len() == 8 && c.kind() != "heading_marker"
    })
}

fn child_of_kind<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).find(|c| c.kind() == kind)
}

fn marker_depth(source: &str, item: Node) -> u8 {
    child_of_kind(item, "list_marker")
        .map(|m| slice(source, m).trim().len() as u8)
        .unwrap_or(1)
}

fn is_rule(source: &str, node: Node) -> bool {
    child_of_kind(node, "html_tag_name")
        .map(|n| slice(source, n).eq_ignore_ascii_case("hr"))
        .unwrap_or(false)
}

fn count_errors(root: Node) -> u32 {
    if !root.has_error() {
        return 0;
    }
    let mut count = 0;
    let mut cursor = root.walk();
    let mut go_deeper = true;
    loop {
        if go_deeper && (cursor.node().is_error() || cursor.node().is_missing()) {
            count += 1;
        }
        if go_deeper && cursor.goto_first_child() {
            continue;
        }
        if cursor.goto_next_sibling() {
            go_deeper = true;
            continue;
        }
        if !cursor.goto_parent() {
            return count;
        }
        go_deeper = false;
    }
}

fn interwiki_of(templates: &[TemplateRef], title: &str) -> Vec<PageRef> {
    templates
        .iter()
        .filter_map(|template| {
            let site = match template.name.as_str() {
                "wikipedia" | "wikipediapar" => Site::Wikipedia,
                "wikisource" | "wikisource author" => Site::Wikisource,
                "wiktionary" => Site::Wiktionary,
                "commons" | "commonscat" | "commons category" | "commons cat" => Site::Commons,
                _ => return None,
            };
            let target = template
                .param("1")
                .filter(|v| !v.is_empty())
                .unwrap_or(title);
            Some(PageRef {
                site,
                lang: "en".to_string(),
                title: target.to_string(),
                fragment: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(wikitext: &str) -> Document {
        let page = RawPage {
            title: "Test Page".to_string(),
            page_id: 1,
            namespace: 0,
            redirect_to: None,
            revision_id: 2,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            text: wikitext.to_string(),
        };
        DocumentBuilder::new().build(&page, "en")
    }

    const PERSON: &str = r#"'''Test''' was a writer.

== Quotes ==
=== 1770s ===
==== ''Common Sense'' (1776) ====
* These are the times that try men's souls.
** ''The American Crisis'', No. 1 (23 December 1776)
** Translation: not really
* A second quote.

== See also ==
* [[Other page]]

[[Category:1737 births]]
[[Category:Living people]]
"#;

    #[test]
    fn nests_the_sections_by_heading_level() {
        let document = document(PERSON);
        assert_eq!(document.sections.len(), 2);
        let quotes = &document.sections[0];
        assert_eq!(quotes.level, 2);
        assert_eq!(quotes.role, SectionRole::Quotes);
        let decade = &quotes.sections[0];
        assert_eq!(decade.level, 3);
        assert_eq!(
            decade.source_hint,
            Some(SourceHint::Period {
                from: 1770,
                to: 1779
            })
        );
        let work = &decade.sections[0];
        assert_eq!(work.level, 4);
        assert_eq!(
            work.source_hint,
            Some(SourceHint::Work {
                title: "Common Sense".into(),
                year: Some(1776)
            })
        );
    }

    #[test]
    fn hangs_the_double_star_lines_under_the_quote() {
        let document = document(PERSON);
        let work = &document.sections[0].sections[0].sections[0];
        let Block::Quote(first) = &work.blocks[0] else {
            panic!("expected a quote")
        };
        assert_eq!(first.text.text, "These are the times that try men's souls.");
        assert_eq!(first.annotations.len(), 2);
        assert_eq!(first.annotations[0].kind, AnnotationKind::Citation);
        assert_eq!(first.annotations[1].kind, AnnotationKind::Translation);
        let Block::Quote(second) = &work.blocks[1] else {
            panic!("expected a quote")
        };
        assert!(second.annotations.is_empty());
    }

    #[test]
    fn collects_the_categories_and_the_lead() {
        let document = document(PERSON);
        assert_eq!(document.categories, ["1737 births", "Living people"]);
        assert_eq!(document.lead.len(), 1);
    }

    #[test]
    fn a_see_also_entry_is_not_a_quote() {
        let document = document(PERSON);
        let see_also = &document.sections[1];
        assert!(matches!(see_also.blocks[0], Block::Item(_)));
    }

    #[test]
    fn accounts_for_every_line() {
        assert_eq!(document(PERSON).parse.unassigned_lines, 0);
    }

    const DIALOGUE: &str = r#"== Dialogue ==
:'''Alice''': Hello there.
:'''Bob''': ''[waves]'' Hi.
<hr width="50%"/>
:''[Alice leaves]''
:'''Alice''': Bye.
"#;

    #[test]
    fn a_rule_ends_an_exchange() {
        let document = document(DIALOGUE);
        let blocks = &document.sections[0].blocks;
        let exchanges: Vec<_> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::Exchange(e) => Some(e),
                _ => None,
            })
            .collect();
        assert_eq!(exchanges.len(), 2);
        assert_eq!(exchanges[0].turns.len(), 2);
        assert_eq!(exchanges[1].turns.len(), 2);
        assert!(blocks.iter().any(|b| matches!(b, Block::Rule)));
    }

    #[test]
    fn reads_the_speaker_and_drops_it_from_the_text() {
        let document = document(DIALOGUE);
        let Block::Exchange(first) = &document.sections[0].blocks[0] else {
            panic!("no exchange")
        };
        assert_eq!(first.turns[0].speaker.as_deref(), Some("Alice"));
        assert_eq!(first.turns[0].text.text, "Hello there.");
        assert_eq!(first.turns[1].speaker.as_deref(), Some("Bob"));
        assert_eq!(first.turns[1].text.text, "[waves] Hi.");
    }

    #[test]
    fn marks_a_stage_direction() {
        let document = document(DIALOGUE);
        let Block::Exchange(second) = &document.sections[0].blocks[2] else {
            panic!("no exchange")
        };
        assert!(second.turns[0].is_stage_direction);
        assert_eq!(second.turns[0].speaker, None);
        assert!(!second.turns[1].is_stage_direction);
    }

    #[test]
    fn keeps_a_redirect_short() {
        let page = RawPage {
            title: "Les Miserables".to_string(),
            redirect_to: Some("Les Misérables".to_string()),
            text: "#REDIRECT [[Les Misérables]]".to_string(),
            ..RawPage::default()
        };
        let document = DocumentBuilder::new().build(&page, "en");
        assert_eq!(document.redirect_to.unwrap().title, "Les Misérables");
        assert!(document.sections.is_empty());
    }

    #[test]
    fn reads_an_image_caption_and_its_attribution() {
        let document =
            document("== Quotes ==\n[[File:X.jpg|thumb|A quote here ~ [[John Adams]]]]\n* Body\n");
        let Block::Media(media) = &document.sections[0].blocks[0] else {
            panic!("no media")
        };
        assert_eq!(media.file, "File:X.jpg");
        assert_eq!(
            media.caption.as_ref().unwrap().text,
            "A quote here ~ John Adams"
        );
        assert_eq!(media.caption_attribution.as_deref(), Some("John Adams"));
    }
}
