//! Layer 2: turns the document model into quote records.
//!
//! Every heuristic in the pipeline lives here or in `citation`. Layer 1 stays
//! free of guesses, so a reader who disagrees with these rules can start again
//! from `pages-<lang>.jsonl.gz` without parsing wikitext.
//!
//! The rules by page type are the table at the end of `docs/schema.md`.

use std::collections::HashMap;

use crate::citation;
use crate::model::*;
use crate::quote::{self as out, Quote};

pub fn quotes(document: &Document) -> Vec<Quote> {
    if document.redirect_to.is_some()
        || matches!(document.page_type, PageType::Disambiguation | PageType::Placeholder)
    {
        return Vec::new();
    }
    let mut builder = QuoteBuilder {
        document,
        cast: cast_of(document),
        subject: subject_of(document),
        out: Vec::new(),
    };
    let root = Context { path: Vec::new(), parts: Vec::new(), role: SectionRole::Quotes };
    // Many pages, short ones above all, put their quotes before any heading.
    for (index, block) in document.lead.iter().enumerate() {
        builder.block(block, &root, index);
    }
    for section in &document.sections {
        builder.section(section, &root);
    }
    builder.out
}

/// What the page is about, as an agent.
fn subject_of(document: &Document) -> out::Agent {
    let kind = match document.page_type {
        PageType::Person => "person",
        PageType::Theme | PageType::Unknown | PageType::List => "topic",
        _ => "work",
    };
    out::Agent {
        name: document.title.clone(),
        kind: kind.to_string(),
        link_site: Some("wikiquote".to_string()),
        link_lang: Some(document.wiki_language.clone()),
        link_title: Some(document.title.clone()),
        played_by: None,
    }
}

/// Reads `* [[Leah Lewis]] as Ellie Chu` from every Cast section.
fn cast_of(document: &Document) -> HashMap<String, String> {
    let mut cast = HashMap::new();
    walk_sections(&document.sections, &mut |section| {
        if section.role != SectionRole::Cast {
            return;
        }
        for block in &section.blocks {
            let text = match block {
                Block::Item(text) => text,
                Block::Quote(item) => &item.text,
                _ => continue,
            };
            if let Some((actor, character)) = text.text.split_once(" as ") {
                cast.insert(character.trim().to_string(), actor.trim().to_string());
            }
        }
    });
    cast
}

fn walk_sections(sections: &[Section], visit: &mut impl FnMut(&Section)) {
    for section in sections {
        visit(section);
        walk_sections(&section.sections, visit);
    }
}

/// What the walk carries down the section tree.
struct Context {
    /// Literal heading texts, page root first.
    path: Vec<String>,
    /// Only the headings whose shape says something about the source.
    parts: Vec<(out::SourcePart, Option<SourceHint>, RichText)>,
    /// The nearest heading that says what kind of quotes these are.
    role: SectionRole,
}

struct QuoteBuilder<'a> {
    document: &'a Document,
    cast: HashMap<String, String>,
    subject: out::Agent,
    out: Vec<Quote>,
}

impl QuoteBuilder<'_> {
    fn section(&mut self, section: &Section, parent: &Context) {
        let mut path = parent.path.clone();
        path.push(section.heading.text.clone());

        let mut parts = parent.parts.clone();
        if let Some(hint) = &section.source_hint {
            parts.push((
                out::SourcePart {
                    origin: "heading".to_string(),
                    level: Some(section.level),
                    hint: Some(hint_name(hint).to_string()),
                    raw: section.heading.wikitext.clone(),
                },
                Some(hint.clone()),
                section.heading.clone(),
            ));
        }

        // A heading such as a work title or a sort letter says nothing about
        // what kind of quotes follow, so the nearest one that does still holds.
        let role = match &section.role {
            SectionRole::Other(_) | SectionRole::AlphaBucket(_) | SectionRole::Episodes => {
                parent.role.clone()
            }
            role => role.clone(),
        };
        let context = Context { path, parts, role };

        for (index, block) in section.blocks.iter().enumerate() {
            self.block(block, &context, index);
        }
        for child in &section.sections {
            self.section(child, &context);
        }
    }

    fn block(&mut self, block: &Block, context: &Context, index: usize) {
        match block {
            Block::Quote(item) => {
                if let Some(quote) = self.from_item(item, context, index) {
                    self.out.push(quote);
                }
            }
            Block::Exchange(exchange) => self.from_exchange(exchange, context, index),
            _ => {}
        }
    }

    fn from_item(&self, item: &QuoteItem, context: &Context, index: usize) -> Option<Quote> {
        if item.text.text.trim().is_empty() {
            return None;
        }
        let kind = match context.role {
            SectionRole::Taglines => "tagline",
            SectionRole::SongLyrics => "lyric",
            _ if self.document.page_type == PageType::Proverbs => "proverb",
            _ => "monologue",
        };
        let mut quote = self.base(&item.text, context, index, kind);

        let citations: Vec<&Annotation> = item
            .annotations
            .iter()
            .filter(|a| matches!(a.kind, AnnotationKind::Citation | AnnotationKind::Attribution))
            .collect();

        let (source, source_from) = self.source(context, &citations);
        let (speaker, speaker_from) = self.speaker(context, &citations);
        quote.about = self.about(context, &quote);
        quote.speaker = speaker;
        quote.annotations =
            item.annotations.iter().map(annotation_out).collect();
        quote.translations = item
            .annotations
            .iter()
            .filter(|a| a.kind == AnnotationKind::Translation)
            .map(|a| out::Translation {
                text: strip_label(&a.text.text),
                language: None,
                translator: None,
                is_original: false,
            })
            .collect();

        let (status, status_from) = status_of(&context.role, source.as_ref());
        quote.status = status;
        quote.provenance = out::Provenance {
            speaker_from,
            source_from,
            status_from: Some(status_from.to_string()),
            unparsed_annotations: item
                .annotations
                .iter()
                .filter(|a| a.kind == AnnotationKind::Note)
                .count()
                .min(255) as u8,
        };
        quote.source = source;
        Some(quote)
    }

    /// A dialogue makes one record for the exchange and one for each turn that
    /// has a speaker. The exchange is the quotable unit; the turns carry the
    /// attribution.
    fn from_exchange(&mut self, exchange: &Exchange, context: &Context, index: usize) {
        let citations: Vec<&Annotation> = exchange.annotations.iter().collect();
        let (source, source_from) = self.source(context, &citations);
        let (status, status_from) = status_of(&context.role, source.as_ref());

        let speaking: Vec<&Turn> =
            exchange.turns.iter().filter(|t| t.speaker.is_some() && !t.is_stage_direction).collect();

        if speaking.len() > 1 {
            let joined = exchange
                .turns
                .iter()
                .map(|turn| match &turn.speaker {
                    Some(name) => format!("{name}: {}", turn.text.text),
                    None => turn.text.text.clone(),
                })
                .collect::<Vec<_>>()
                .join("\n");
            let whole = RichText {
                text: joined,
                wikitext: exchange
                    .turns
                    .iter()
                    .map(|t| t.text.wikitext.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                spans: Vec::new(),
            };
            let mut quote = self.base(&whole, context, index, "dialogue");
            quote.about = self.about(context, &quote);
            quote.status = status.clone();
            quote.source = source.clone();
            quote.provenance = out::Provenance {
                speaker_from: None,
                source_from: source_from.clone(),
                status_from: Some(status_from.to_string()),
                unparsed_annotations: 0,
            };
            self.out.push(quote);
        }

        for (turn_index, turn) in exchange.turns.iter().enumerate() {
            let Some(name) = &turn.speaker else { continue };
            if turn.text.text.trim().is_empty() {
                continue;
            }
            let mut quote =
                self.base(&turn.text, context, index * 100 + turn_index + 1, "dialogue_turn");
            quote.speaker = Some(self.character(name, turn.speaker_link.as_ref()));
            quote.about = self.about(context, &quote);
            quote.status = status.clone();
            quote.source = source.clone();
            quote.provenance = out::Provenance {
                speaker_from: Some("dialogue_marker".to_string()),
                source_from: source_from.clone(),
                status_from: Some(status_from.to_string()),
                unparsed_annotations: 0,
            };
            self.out.push(quote);
        }
    }

    fn base(&self, text: &RichText, context: &Context, index: usize, kind: &str) -> Quote {
        let (language, language_from) = language_of(text, &self.document.wiki_language);
        Quote {
            id: quote_id(&self.document.title, &context.path, index),
            content_hash: content_hash(&text.text),
            wiki_language: self.document.wiki_language.clone(),
            page_title: self.document.title.clone(),
            page_id: self.document.page_id,
            page_type: page_type_name(self.document.page_type).to_string(),
            revision_id: self.document.revision_id,
            text: text.text.clone(),
            wikitext: text.wikitext.clone(),
            language,
            language_from,
            kind: kind.to_string(),
            status: "unsourced".to_string(),
            speaker: None,
            about: Vec::new(),
            source: None,
            context_path: context.path.clone(),
            annotations: Vec::new(),
            translations: Vec::new(),
            links: links_of(text),
            provenance: out::Provenance::default(),
        }
    }

    /// Who said it. The rule depends on the page and on the section role.
    fn speaker(
        &self,
        context: &Context,
        citations: &[&Annotation],
    ) -> (Option<out::Agent>, Option<String>) {
        let from_annotation = citations.first().and_then(|a| agent_of(&a.text));
        match (self.document.page_type, &context.role) {
            (_, SectionRole::QuotesAbout) => from_annotation.into_agent("annotation"),
            (PageType::Person, SectionRole::Misattributed) => (None, None),
            (PageType::Person, _) => {
                (Some(self.subject.clone()), Some("page_subject".to_string()))
            }
            (
                PageType::Film
                | PageType::TelevisionSeries
                | PageType::TelevisionSeason
                | PageType::VideoGame,
                _,
            ) => {
                let character = from_annotation.map(|mut agent| {
                    agent.kind = "character".to_string();
                    agent.played_by = self.cast.get(&agent.name).cloned();
                    agent
                });
                character.into_agent("annotation")
            }
            _ => from_annotation.into_agent("annotation"),
        }
    }

    fn character(&self, name: &str, link: Option<&PageRef>) -> out::Agent {
        out::Agent {
            name: name.to_string(),
            kind: "character".to_string(),
            link_site: link.map(|l| site_name(&l.site)),
            link_lang: link.map(|l| l.lang.clone()),
            link_title: link.map(|l| l.title.clone()),
            played_by: self.cast.get(name).cloned(),
        }
    }

    /// Who or what the quote is about. The page subject, unless the quote is by
    /// the page subject.
    fn about(&self, context: &Context, quote: &Quote) -> Vec<out::Agent> {
        let subject_speaks = matches!(self.document.page_type, PageType::Person)
            && context.role != SectionRole::QuotesAbout;
        if subject_speaks || quote.kind == "dialogue" || quote.kind == "dialogue_turn" {
            return Vec::new();
        }
        vec![self.subject.clone()]
    }

    /// Assembles the source from the headings above the quote and the citation
    /// lines below it. Later parts win, because the line nearest the quote is
    /// the most specific.
    fn source(
        &self,
        context: &Context,
        citations: &[&Annotation],
    ) -> (Option<out::Source>, Option<String>) {
        let is_work_page = matches!(
            self.document.page_type,
            PageType::Film
                | PageType::TelevisionSeries
                | PageType::TelevisionSeason
                | PageType::VideoGame
                | PageType::LiteraryWork
                | PageType::MusicalWork
        );
        if context.parts.is_empty() && citations.is_empty() && !is_work_page {
            return (None, None);
        }

        let mut source = out::Source::default();
        let mut from_heading = false;
        let mut from_annotation = false;

        if is_work_page {
            source.parts.push(out::SourcePart {
                origin: "page_subject".to_string(),
                level: None,
                hint: None,
                raw: self.document.title.clone(),
            });
            source.work_title = Some(self.document.title.clone());
            source.work_type = Some(work_type_of(self.document.page_type).to_string());
            source.work_link_site = Some("wikiquote".to_string());
            source.work_link_title = Some(self.document.title.clone());
        }

        for (part, hint, text) in &context.parts {
            from_heading = true;
            source.parts.push(part.clone());
            apply_hint(&mut source, hint.as_ref(), is_work_page);
            citation::apply(&mut source, text);
        }
        for annotation in citations {
            from_annotation = true;
            source.parts.push(out::SourcePart {
                origin: "annotation".to_string(),
                level: None,
                hint: None,
                raw: annotation.text.wikitext.trim().to_string(),
            });
            citation::apply(&mut source, &annotation.text);
        }

        source.raw = source
            .parts
            .iter()
            .map(|p| p.raw.trim())
            .filter(|raw| !raw.is_empty())
            .collect::<Vec<_>>()
            .join(" \u{2014} ");
        // A source made only of a locator, such as a lone "p. 223", names
        // nothing a reader could look up.
        source.complete = source.work_title.is_some()
            || source.occasion.is_some()
            || source.date_iso.is_some()
            || source.episode_title.is_some()
            || source.publication.is_some();

        let origin = match (from_heading, from_annotation) {
            (true, true) => "mixed",
            (true, false) => "heading",
            (false, true) => "annotation",
            (false, false) => "page_subject",
        };
        (Some(source), Some(origin.to_string()))
    }
}

fn apply_hint(source: &mut out::Source, hint: Option<&SourceHint>, is_work_page: bool) {
    match hint {
        Some(SourceHint::Work { title, year }) => {
            source.work_title = Some(title.clone());
            if let Some(year) = year {
                source.work_year = Some(*year);
                source.date_iso.get_or_insert(format!("{year:04}"));
                source.date_precision.get_or_insert("year".to_string());
            }
        }
        Some(SourceHint::Episode { title, code }) => {
            source.episode_title = title.clone();
            source.episode_number = code.clone();
            if let Some(season) = code.as_deref().and_then(|c| c.split('.').next()) {
                source.episode_season = season.parse().ok();
            }
        }
        Some(SourceHint::Season { number }) => source.episode_season = Some(*number),
        Some(SourceHint::Period { from, to }) => {
            source.date_iso.get_or_insert(format!("{from:04}"));
            if from == to {
                source.date_precision.get_or_insert("year".to_string());
            } else {
                source.date_end_iso.get_or_insert(format!("{to:04}"));
                source.date_precision.get_or_insert("decade".to_string());
            }
        }
        None => {}
    }
    if is_work_page && source.episode_title.is_some() {
        source.episode_series = source.work_title.clone();
    }
}

/// The person a citation names. Usually the first link that is not part of a
/// work title: `[[John Adams]], in a letter to [[Thomas Jefferson]]` and
/// `Attributed to [[John Adams]] since 1957` both name Adams, while
/// `''[[s:Common Sense|Common Sense]]''` names a work, not a person.
fn agent_of(text: &RichText) -> Option<out::Agent> {
    let in_italics = |span: &Span| {
        text.spans.iter().any(|other| {
            matches!(other.kind, SpanKind::Italic | SpanKind::BoldItalic)
                && other.start <= span.start
                && other.end >= span.end
        })
    };
    let link = text
        .spans
        .iter()
        .filter(|span| matches!(span.kind, SpanKind::Link { .. }) && !in_italics(span))
        .min_by_key(|span| span.start);

    if let Some(span) = link
        && let SpanKind::Link { target } = &span.kind
        && !matches!(target.site, Site::Wikisource)
    {
        let name = text.text.get(span.start as usize..span.end as usize)?.trim();
        if !name.is_empty() {
            return Some(out::Agent {
                name: name.to_string(),
                kind: "person".to_string(),
                link_site: Some(site_name(&target.site)),
                link_lang: Some(target.lang.clone()),
                link_title: Some(target.title.clone()),
                played_by: None,
            });
        }
    }

    if let Some(author) = cite_author(text) {
        return Some(out::Agent {
            name: author,
            kind: "person".to_string(),
            ..out::Agent::default()
        });
    }

    // No link: take the words before the first comma, when they read as a name.
    let head = text.text.split(',').next()?.trim();
    let looks_like_a_name = !head.is_empty()
        && head.len() <= 60
        && head.split_whitespace().count() <= 5
        && head.starts_with(char::is_uppercase)
        && !head.ends_with('.');
    looks_like_a_name.then(|| out::Agent {
        name: head.to_string(),
        kind: "unknown".to_string(),
        ..out::Agent::default()
    })
}

/// The author named inside a `{{cite …}}` call.
fn cite_author(text: &RichText) -> Option<String> {
    for span in &text.spans {
        let SpanKind::Template(template) = &span.kind else { continue };
        if !template.name.starts_with("cite") && template.name != "citation" {
            continue;
        }
        if let Some(author) = template.param("author").filter(|v| !v.is_empty()) {
            return Some(author.to_string());
        }
        if let Some(last) = template.param("last").filter(|v| !v.is_empty()) {
            let full = match template.param("first").filter(|v| !v.is_empty()) {
                Some(first) => format!("{first} {last}"),
                None => last.to_string(),
            };
            return Some(full);
        }
    }
    None
}

/// Keeps `provenance` honest: a rule that found nobody records no source for
/// the speaker.
trait Attributed {
    fn into_agent(self, from: &str) -> (Option<out::Agent>, Option<String>);
}

impl Attributed for Option<out::Agent> {
    fn into_agent(self, from: &str) -> (Option<out::Agent>, Option<String>) {
        match self {
            Some(agent) => (Some(agent), Some(from.to_string())),
            None => (None, None),
        }
    }
}

fn status_of(role: &SectionRole, source: Option<&out::Source>) -> (String, &'static str) {
    let by_role = match role {
        SectionRole::Misattributed => Some("misattributed"),
        SectionRole::Disputed => Some("disputed"),
        SectionRole::Attributed => Some("attributed"),
        SectionRole::Unsourced => Some("unsourced"),
        _ => None,
    };
    match by_role {
        Some(status) => (status.to_string(), "section_role"),
        None => {
            let complete = source.map(|s| s.complete).unwrap_or(false);
            ((if complete { "sourced" } else { "unsourced" }).to_string(), "citation_presence")
        }
    }
}

fn language_of(text: &RichText, wiki_language: &str) -> (String, String) {
    for span in &text.spans {
        if let SpanKind::Template(template) = &span.kind
            && template.name == "lang"
            && let Some(code) = template.param("1")
            && !code.is_empty()
        {
            return (code.to_lowercase(), "declared".to_string());
        }
    }
    (wiki_language.to_string(), "wiki_default".to_string())
}

fn links_of(text: &RichText) -> Vec<out::Link> {
    text.spans
        .iter()
        .filter_map(|span| match &span.kind {
            SpanKind::Link { target } => Some(out::Link {
                site: site_name(&target.site),
                lang: target.lang.clone(),
                title: target.title.clone(),
                fragment: target.fragment.clone(),
            }),
            _ => None,
        })
        .collect()
}

fn annotation_out(annotation: &Annotation) -> out::AnnotationOut {
    out::AnnotationOut {
        kind: annotation_kind_name(annotation.kind).to_string(),
        text: annotation.text.text.clone(),
        wikitext: annotation.text.wikitext.trim().to_string(),
    }
}

/// Removes a leading `Translation:` label.
fn strip_label(text: &str) -> String {
    match text.split_once(':') {
        Some((label, rest)) if label.len() <= 20 => rest.trim().to_string(),
        _ => text.trim().to_string(),
    }
}

fn quote_id(title: &str, path: &[String], index: usize) -> String {
    let key = format!("{title}\n{}\n{index}", path.join("/"));
    blake3::hash(key.as_bytes()).to_hex()[..16].to_string()
}

fn content_hash(text: &str) -> String {
    let normalised = text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    blake3::hash(normalised.as_bytes()).to_hex()[..16].to_string()
}

fn hint_name(hint: &SourceHint) -> &'static str {
    match hint {
        SourceHint::Work { .. } => "work",
        SourceHint::Episode { .. } => "episode",
        SourceHint::Season { .. } => "season",
        SourceHint::Period { .. } => "period",
    }
}

fn work_type_of(page_type: PageType) -> &'static str {
    match page_type {
        PageType::Film => "film",
        PageType::TelevisionSeries => "series",
        PageType::TelevisionSeason => "series",
        PageType::VideoGame => "video game",
        PageType::LiteraryWork => "book",
        PageType::MusicalWork => "song",
        _ => "work",
    }
}

fn page_type_name(page_type: PageType) -> &'static str {
    match page_type {
        PageType::Person => "person",
        PageType::Film => "film",
        PageType::TelevisionSeries => "television_series",
        PageType::TelevisionSeason => "television_season",
        PageType::LiteraryWork => "literary_work",
        PageType::VideoGame => "video_game",
        PageType::MusicalWork => "musical_work",
        PageType::Theme => "theme",
        PageType::Proverbs => "proverbs",
        PageType::List => "list",
        PageType::Disambiguation => "disambiguation",
        PageType::Placeholder => "placeholder",
        PageType::Unknown => "unknown",
    }
}

fn annotation_kind_name(kind: AnnotationKind) -> &'static str {
    match kind {
        AnnotationKind::Citation => "citation",
        AnnotationKind::Attribution => "attribution",
        AnnotationKind::Translation => "translation",
        AnnotationKind::CrossRef => "cross_ref",
        AnnotationKind::Note => "note",
    }
}

fn site_name(site: &Site) -> String {
    match site {
        Site::Wikiquote => "wikiquote".to_string(),
        Site::Wikipedia => "wikipedia".to_string(),
        Site::Wikisource => "wikisource".to_string(),
        Site::Wiktionary => "wiktionary".to_string(),
        Site::Commons => "commons".to_string(),
        Site::Other(name) => name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dump::RawPage;
    use crate::parse::DocumentBuilder;

    fn quotes_of(title: &str, wikitext: &str) -> Vec<Quote> {
        let page = RawPage {
            title: title.to_string(),
            page_id: 1,
            namespace: 0,
            redirect_to: None,
            revision_id: 2,
            timestamp: String::new(),
            text: wikitext.to_string(),
        };
        let document = DocumentBuilder::new().build(&page, "en");
        quotes(&document)
    }

    const PERSON: &str = r#"'''Thomas Paine''' was a writer.

== Quotes ==
=== 1790s ===
==== ''Common Sense'' (1776) ====
* These are the times that try men's souls.
** ''The American Crisis'', No. 1 (23 December 1776)

== Quotes about Paine ==
* Without the pen of Paine, the sword of [[George Washington|Washington]] would have been wielded in vain.
** Attributed to [[John Adams]] since at least 1957

== Misattributed ==
* Lead, follow, or get out of the way.
** No source found.

[[Category:1737 births]]
"#;

    #[test]
    fn a_person_speaks_their_own_quotes() {
        let quotes = quotes_of("Thomas Paine", PERSON);
        let own = &quotes[0];
        assert_eq!(own.speaker.as_ref().unwrap().name, "Thomas Paine");
        assert_eq!(own.provenance.speaker_from.as_deref(), Some("page_subject"));
        assert!(own.about.is_empty());
        assert_eq!(own.status, "sourced");
    }

    #[test]
    fn the_heading_path_and_the_citation_both_feed_the_source() {
        let quotes = quotes_of("Thomas Paine", PERSON);
        let source = quotes[0].source.as_ref().unwrap();
        assert_eq!(source.work_title.as_deref(), Some("The American Crisis"));
        assert_eq!(source.date_iso.as_deref(), Some("1776-12-23"));
        assert_eq!(source.locator_part.as_deref(), Some("No. 1"));
        assert!(source.complete);
        assert_eq!(quotes[0].provenance.source_from.as_deref(), Some("mixed"));
        let origins: Vec<&str> = source.parts.iter().map(|p| p.origin.as_str()).collect();
        assert_eq!(origins, ["heading", "heading", "annotation"]);
        let hints: Vec<Option<&str>> = source.parts.iter().map(|p| p.hint.as_deref()).collect();
        assert_eq!(hints, [Some("period"), Some("work"), None]);
    }

    #[test]
    fn the_full_heading_path_stays_outside_the_source() {
        let quotes = quotes_of("Thomas Paine", PERSON);
        assert_eq!(quotes[0].context_path, ["Quotes", "1790s", "Common Sense (1776)"]);
        // "Quotes" is a role heading, so it gives the source nothing.
        let source = quotes[0].source.as_ref().unwrap();
        assert!(!source.parts.iter().any(|p| p.raw == "Quotes"));
    }

    #[test]
    fn a_quote_about_the_subject_names_the_other_speaker() {
        let quotes = quotes_of("Thomas Paine", PERSON);
        let about = quotes.iter().find(|q| q.text.starts_with("Without the pen")).unwrap();
        assert_eq!(about.speaker.as_ref().unwrap().name, "John Adams");
        assert_eq!(about.provenance.speaker_from.as_deref(), Some("annotation"));
        assert_eq!(about.about[0].name, "Thomas Paine");
        assert_eq!(about.context_path, ["Quotes about Paine"]);
        assert!(about.links.iter().any(|l| l.title == "George Washington"));
    }

    #[test]
    fn a_misattributed_quote_has_no_speaker() {
        let quotes = quotes_of("Thomas Paine", PERSON);
        let bad = quotes.iter().find(|q| q.text.starts_with("Lead, follow")).unwrap();
        assert!(bad.speaker.is_none());
        assert_eq!(bad.provenance.speaker_from, None);
        assert_eq!(bad.status, "misattributed");
        assert_eq!(bad.provenance.status_from.as_deref(), Some("section_role"));
    }

    #[test]
    fn a_citation_that_is_only_a_locator_is_not_complete() {
        let quotes = quotes_of("Someone", "== Quotes ==\n* A quote.\n** p. 223\n[[Category:Living people]]\n");
        let source = quotes[0].source.as_ref().unwrap();
        assert_eq!(source.locator_page.as_deref(), Some("p. 223"));
        assert!(!source.complete);
        assert_eq!(quotes[0].status, "unsourced");
    }

    const FILM: &str = r#"'''''The Half of It''''' is a 2020 film.

== Dialogue ==
:'''Ellie Chu''': Aster thinks you're into abstract art.
:'''Paul Munsky''': Yeah.
<hr width="50%"/>
:'''Ellie Chu''': Gravity is matter's response to loneliness.

== Cast ==
* [[w:Leah Lewis|Leah Lewis]] as Ellie Chu

[[Category:2020 American films]]
"#;

    #[test]
    fn a_film_is_the_source_of_its_own_dialogue() {
        let quotes = quotes_of("The Half of It", FILM);
        let source = quotes[0].source.as_ref().unwrap();
        assert_eq!(source.work_title.as_deref(), Some("The Half of It"));
        assert_eq!(source.work_type.as_deref(), Some("film"));
        assert_eq!(source.parts[0].origin, "page_subject");
    }

    #[test]
    fn an_exchange_makes_one_record_and_a_record_for_each_turn() {
        let quotes = quotes_of("The Half of It", FILM);
        let kinds: Vec<&str> = quotes.iter().map(|q| q.kind.as_str()).collect();
        // Two turns, so the exchange is quotable on its own. The single-turn
        // exchange that follows is not repeated.
        assert_eq!(kinds, ["dialogue", "dialogue_turn", "dialogue_turn", "dialogue_turn"]);
        assert!(quotes[0].text.starts_with("Ellie Chu: Aster thinks"));
    }

    #[test]
    fn the_cast_section_says_who_played_a_character() {
        let quotes = quotes_of("The Half of It", FILM);
        let turn = quotes.iter().find(|q| q.kind == "dialogue_turn").unwrap();
        assert_eq!(turn.speaker.as_ref().unwrap().name, "Ellie Chu");
        assert_eq!(turn.speaker.as_ref().unwrap().kind, "character");
        assert_eq!(turn.speaker.as_ref().unwrap().played_by.as_deref(), Some("Leah Lewis"));
        assert_eq!(turn.provenance.speaker_from.as_deref(), Some("dialogue_marker"));
    }

    #[test]
    fn a_theme_page_is_the_topic_not_the_speaker() {
        let quotes = quotes_of(
            "Warmness",
            "== Quotes ==\n* Someone has a great fire in his soul.\n** [[Vincent van Gogh]], Letter no. 155 (June 1880)\n[[Category:Themes]]\n",
        );
        assert_eq!(quotes[0].speaker.as_ref().unwrap().name, "Vincent van Gogh");
        assert_eq!(quotes[0].about[0].name, "Warmness");
        assert_eq!(quotes[0].about[0].kind, "topic");
        let source = quotes[0].source.as_ref().unwrap();
        assert_eq!(source.date_iso.as_deref(), Some("1880-06"));
        assert_eq!(source.occasion.as_deref(), Some("letter"));
    }

    #[test]
    fn a_declared_language_beats_the_wiki_default() {
        let quotes = quotes_of(
            "Someone",
            "== Quotes ==\n* {{lang|la|Alea iacta est}}\n** Suetonius\n[[Category:Living people]]\n",
        );
        assert_eq!(quotes[0].language, "la");
        assert_eq!(quotes[0].language_from, "declared");
        let plain = quotes_of("Someone", "== Quotes ==\n* Plain words\n[[Category:Living people]]\n");
        assert_eq!(plain[0].language, "en");
        assert_eq!(plain[0].language_from, "wiki_default");
    }

    #[test]
    fn the_same_words_get_the_same_content_hash() {
        let one = quotes_of("A", "== Quotes ==\n* The  same   words.\n[[Category:Living people]]\n");
        let two = quotes_of("B", "== Quotes ==\n* the same words.\n[[Category:Living people]]\n");
        assert_eq!(one[0].content_hash, two[0].content_hash);
        assert_ne!(one[0].id, two[0].id);
    }

    #[test]
    fn a_disambiguation_page_has_no_quotes() {
        assert!(quotes_of("Smith", "{{disambig}}\n* [[Smith A]]\n* [[Smith B]]\n").is_empty());
    }

    #[test]
    fn a_list_entry_is_not_a_quote() {
        let quotes = quotes_of(
            "Someone",
            "== Quotes ==\n* A real quote.\n== External links ==\n* [http://a.example Site]\n[[Category:Living people]]\n",
        );
        assert_eq!(quotes.len(), 1);
    }
}
