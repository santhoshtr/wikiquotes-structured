//! Reads what it safely can out of a citation line.
//!
//! A `**` line on Wikiquote is free text. Samples range from `** p. 223` to a
//! 300-word `{{cite journal}}` to a bare URL to `** Lu Xun's 1st Musou Attack`.
//! No parser will read all of it, so this module extracts only fields it can
//! find with high confidence and always keeps the raw line.
//!
//! The structure of the line comes from Layer 1: the first italic run is the
//! work title, link spans give the URL and the work page, and template spans
//! give the `{{cite …}}` parameters. That is far steadier than a regex over
//! the wikitext.

use std::sync::LazyLock;

use regex::Regex;

use crate::model::{RichText, Site, SpanKind};
use crate::quote::{CiteTemplate, Param, Source};

/// Fills in whatever `part` says. Later parts win over earlier ones, because
/// the parts arrive outermost heading first and the line nearest the quote is
/// the most specific.
pub fn apply(source: &mut Source, part: &RichText) {
    let text = &part.text;

    if let Some(title) = first_italic(part) {
        set(&mut source.work_title, Some(title));
    }
    read_spans(source, part);

    // "Retrieved 29 July 2014" is when somebody visited a web page, not when
    // the words were said.
    if let Some((iso, end, precision, raw)) = read_date(before_retrieval(text)) {
        set(&mut source.date_iso, Some(iso));
        set(&mut source.date_end_iso, end);
        set(&mut source.date_precision, Some(precision));
        set(&mut source.date_raw, Some(raw));
    }

    set(&mut source.locator_page, capture(&PAGE, text));
    set(&mut source.locator_chapter, capture(&CHAPTER, text));
    set(&mut source.locator_part, capture(&PART, text));
    set(&mut source.locator_act_scene, capture(&ACT, text));
    set(&mut source.locator_line, capture(&LINE, text));
    // The number alone, not the "ISBN " in front of it.
    set(&mut source.isbn, capture_group(&ISBN, text));
    set(&mut source.occasion, read_occasion(text));
}

/// Sets a field only when the new value says something.
fn set<T>(field: &mut Option<T>, value: Option<T>) {
    if let Some(value) = value {
        *field = Some(value);
    }
}

/// The first italic run of a citation is the title of the work.
fn first_italic(part: &RichText) -> Option<String> {
    let span = part
        .spans
        .iter()
        .filter(|s| matches!(s.kind, SpanKind::Italic | SpanKind::BoldItalic))
        .min_by_key(|s| s.start)?;
    let title = part
        .text
        .get(span.start as usize..span.end as usize)?
        .trim();
    // A one-word italic run is usually emphasis, not a title.
    (title.len() > 2).then(|| title.trim_matches(',').trim().to_string())
}

fn read_spans(source: &mut Source, part: &RichText) {
    for span in &part.spans {
        match &span.kind {
            SpanKind::ExternalLink { url } if !url.is_empty() => {
                set(&mut source.url, Some(url.clone()));
            }
            SpanKind::Link { target } => {
                let is_work = matches!(target.site, Site::Wikisource)
                    || part
                        .text
                        .get(span.start as usize..span.end as usize)
                        .map(|shown| source.work_title.as_deref() == Some(shown.trim()))
                        .unwrap_or(false);
                if is_work {
                    set(&mut source.work_link_site, Some(site_name(&target.site)));
                    set(&mut source.work_link_title, Some(target.title.clone()));
                }
            }
            SpanKind::Template(template) => {
                if template.name == "isbn" {
                    set(&mut source.isbn, template.param("1").map(str::to_string));
                }
                if !template.name.starts_with("cite") && template.name != "citation" {
                    continue;
                }
                read_cite_template(source, template);
            }
            _ => {}
        }
    }
}

fn read_cite_template(source: &mut Source, template: &crate::model::TemplateRef) {
    let param = |key: &str| {
        template
            .param(key)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    set(&mut source.work_title, param("title"));
    set(
        &mut source.publication,
        param("journal")
            .or_else(|| param("work"))
            .or_else(|| param("newspaper")),
    );
    set(&mut source.publisher, param("publisher"));
    set(&mut source.isbn, param("isbn"));
    set(&mut source.url, param("url"));
    set(&mut source.work_type, cite_work_type(&template.name));
    if let Some(date) = param("date").or_else(|| param("year"))
        && let Some((iso, end, precision, raw)) = read_date(&date)
    {
        set(&mut source.date_iso, Some(iso));
        set(&mut source.date_end_iso, end);
        set(&mut source.date_precision, Some(precision));
        set(&mut source.date_raw, Some(raw));
    }
    source.cite_templates.push(CiteTemplate {
        name: template.name.clone(),
        params: template
            .params
            .iter()
            .map(|(key, value)| Param {
                key: key.clone(),
                value: value.clone(),
            })
            .collect(),
    });
}

fn cite_work_type(name: &str) -> Option<String> {
    let kind = match name {
        "cite book" => "book",
        "cite journal" => "journal article",
        "cite news" => "news article",
        "cite web" => "web page",
        "cite episode" => "episode",
        "cite interview" => "interview",
        "cite speech" => "speech",
        _ => return None,
    };
    Some(kind.to_string())
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

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

static DAY_MONTH_YEAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b([0-3]?\d)\s+([a-z]{3,9})\.?,?\s+(\d{3,4})\b").unwrap());
static MONTH_DAY_YEAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b([a-z]{3,9})\.?\s+([0-3]?\d),\s*(\d{3,4})\b").unwrap());
static MONTH_YEAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b([a-z]{3,9})\.?\s+(\d{3,4})\b").unwrap());
static CIRCA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bc(?:irca)?\.?\s*(\d{3,4})\b").unwrap());
static BCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(\d{1,4})\s*(?:BCE?|B\.C\.E?\.)\b").unwrap());
static YEAR_RANGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\((\d{4})\s*[-\x{2013}]\s*(\d{4})\)").unwrap());
/// A bare year is trusted only inside brackets. Elsewhere a four-digit number
/// is as likely to be a page or a line number.
static YEAR_IN_BRACKETS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[(\[](?:in\s+)?(1\d{3}|20\d{2}|[1-9]\d{2})[)\],;]").unwrap());

static PAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bpp?\.\s*([0-9ivxlc]+(?:\s*[-\x{2013}]\s*[0-9ivxlc]+)?)").unwrap()
});
static CHAPTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:ch\.|chapter)\s*([0-9]+|[ivxlc]+)\b").unwrap());
static PART: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:no\.|number|book|part|vol\.|volume)\s*([0-9]+|[ivxlc]+)\b").unwrap()
});
static ACT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bact\s+([0-9]+|[ivxlc]+)(?:\s*,?\s*sc(?:ene)?\.?\s*([0-9]+|[ivxlc]+))?")
        .unwrap()
});
static LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\blines?\s*([0-9]+(?:\s*[-\x{2013}]\s*[0-9]+)?)").unwrap());
static ISBN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bISBN\s*([0-9][0-9\-x]{8,})").unwrap());

/// The whole match. Locators keep their label, so that `p. 12` reads as written.
fn capture(pattern: &Regex, text: &str) -> Option<String> {
    let captures = pattern.captures(text)?;
    Some(captures.get(0)?.as_str().trim().to_string())
}

fn capture_group(pattern: &Regex, text: &str) -> Option<String> {
    let captures = pattern.captures(text)?;
    Some(captures.get(1)?.as_str().trim().to_string())
}

fn month_number(name: &str) -> Option<u8> {
    let name = name.trim_end_matches('.').to_lowercase();
    MONTHS
        .iter()
        .position(|m| *m == name || (name.len() >= 3 && m.starts_with(&name)))
        .map(|i| i as u8 + 1)
}

/// Returns an ISO date, an end date for a range, the precision and the text it
/// was read from.
fn read_date(text: &str) -> Option<(String, Option<String>, String, String)> {
    if let Some(c) = BCE.captures(text) {
        let year: i32 = c[1].parse().ok()?;
        return Some((
            format!("-{year:04}"),
            None,
            "year".into(),
            c[0].trim().to_string(),
        ));
    }
    if let Some(c) = YEAR_RANGE.captures(text) {
        return Some((
            c[1].to_string(),
            Some(c[2].to_string()),
            "range".into(),
            c[0].to_string(),
        ));
    }
    if let Some(c) = DAY_MONTH_YEAR.captures(text)
        && let Some(month) = month_number(&c[2])
    {
        let day: u8 = c[1].parse().ok()?;
        let year: u16 = c[3].parse().ok()?;
        return Some((
            format!("{year:04}-{month:02}-{day:02}"),
            None,
            "day".into(),
            c[0].trim().to_string(),
        ));
    }
    if let Some(c) = MONTH_DAY_YEAR.captures(text)
        && let Some(month) = month_number(&c[1])
    {
        let day: u8 = c[2].parse().ok()?;
        let year: u16 = c[3].parse().ok()?;
        return Some((
            format!("{year:04}-{month:02}-{day:02}"),
            None,
            "day".into(),
            c[0].trim().to_string(),
        ));
    }
    if let Some(c) = MONTH_YEAR.captures(text)
        && let Some(month) = month_number(&c[1])
    {
        let year: u16 = c[2].parse().ok()?;
        return Some((
            format!("{year:04}-{month:02}"),
            None,
            "month".into(),
            c[0].trim().to_string(),
        ));
    }
    if let Some(c) = CIRCA.captures(text) {
        let year: u16 = c[1].parse().ok()?;
        return Some((
            format!("{year:04}"),
            None,
            "circa".into(),
            c[0].trim().to_string(),
        ));
    }
    if let Some(c) = YEAR_IN_BRACKETS.captures(text) {
        let year: u16 = c[1].parse().ok()?;
        return Some((format!("{year:04}"), None, "year".into(), c[1].to_string()));
    }
    // A citation that is only a year, as a whole heading or a cite parameter.
    let trimmed = text.trim();
    if trimmed.len() == 4
        && let Ok(year) = trimmed.parse::<u16>()
    {
        return Some((
            format!("{year:04}"),
            None,
            "year".into(),
            trimmed.to_string(),
        ));
    }
    None
}

/// Cuts the line at the point where it stops talking about the quote.
fn before_retrieval(text: &str) -> &str {
    let lower = text.to_lowercase();
    ["retrieved", "accessed", "archived", "as reported in"]
        .iter()
        .filter_map(|needle| lower.find(needle))
        .min()
        .map_or(text, |cut| &text[..cut])
}

/// What kind of thing the words came from. Read from the nouns a citation uses.
/// The multi-word phrases come first, because "address to the nation" is a
/// speech while "his address" is where somebody lives.
fn read_occasion(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    for (needle, occasion) in [
        ("press conference", "press conference"),
        ("news conference", "press conference"),
        ("address to", "speech"),
        ("address at", "speech"),
        ("blog post", "post"),
        ("facebook post", "post"),
    ] {
        if lower.contains(needle) {
            return Some(occasion.to_string());
        }
    }
    for (word, occasion) in [
        ("letter", "letter"),
        ("letters", "letter"),
        ("interview", "interview"),
        ("speech", "speech"),
        ("lecture", "lecture"),
        ("sermon", "sermon"),
        ("testimony", "testimony"),
        ("diary", "diary"),
        ("essay", "essay"),
        ("editorial", "editorial"),
        ("tweet", "post"),
    ] {
        if contains_word(&lower, word) {
            return Some(occasion.to_string());
        }
    }
    None
}

/// True when `word` appears in `text` on its own, not inside a longer word.
fn contains_word(text: &str, word: &str) -> bool {
    let mut from = 0;
    while let Some(found) = text[from..].find(word) {
        let start = from + found;
        let end = start + word.len();
        let before_ok = start == 0 || !text[..start].ends_with(|c: char| c.is_alphanumeric());
        let after_ok = !text[end..].starts_with(|c: char| c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        from = end;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_of(wikitext: &str) -> Source {
        let mut source = Source::default();
        apply(&mut source, &rich(wikitext));
        source
    }

    fn rich(wikitext: &str) -> RichText {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_wikitext::LANGUAGE.into())
            .unwrap();
        let line = format!("* {wikitext}");
        let tree = parser.parse(&line, None).unwrap();
        let content = find(tree.root_node(), "list_item_content").expect("no list item");
        crate::wikitext::rich_text(&line, content, "Test")
    }

    fn find<'t>(node: tree_sitter::Node<'t>, kind: &str) -> Option<tree_sitter::Node<'t>> {
        if node.kind() == kind {
            return Some(node);
        }
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .find_map(|child| find(child, kind))
    }

    #[test]
    fn reads_a_work_title_and_a_full_date() {
        let source = source_of("''The American Crisis'', No. 1 (23 December 1776)");
        assert_eq!(source.work_title.as_deref(), Some("The American Crisis"));
        assert_eq!(source.date_iso.as_deref(), Some("1776-12-23"));
        assert_eq!(source.date_precision.as_deref(), Some("day"));
        assert_eq!(source.locator_part.as_deref(), Some("No. 1"));
    }

    #[test]
    fn reads_the_date_forms() {
        assert_eq!(
            source_of("(5 February 2006)").date_iso.as_deref(),
            Some("2006-02-05")
        );
        assert_eq!(
            source_of("(February 5, 2006)").date_iso.as_deref(),
            Some("2006-02-05")
        );
        assert_eq!(
            source_of("(November 1886)").date_iso.as_deref(),
            Some("1886-11")
        );
        assert_eq!(
            source_of("''Book'' (1990)").date_iso.as_deref(),
            Some("1990")
        );
        assert_eq!(
            source_of("c. 1200").date_precision.as_deref(),
            Some("circa")
        );
        assert_eq!(source_of("399 BCE").date_iso.as_deref(), Some("-0399"));
        let range = source_of("''War'' (1914-1918)");
        assert_eq!(range.date_iso.as_deref(), Some("1914"));
        assert_eq!(range.date_end_iso.as_deref(), Some("1918"));
        assert_eq!(range.date_precision.as_deref(), Some("range"));
    }

    #[test]
    fn does_not_read_a_page_number_as_a_year() {
        let source = source_of("p. 1776");
        assert_eq!(source.date_iso, None);
        assert_eq!(source.locator_page.as_deref(), Some("p. 1776"));
    }

    #[test]
    fn reads_the_locators() {
        assert_eq!(
            source_of("pp. 77-78").locator_page.as_deref(),
            Some("pp. 77-78")
        );
        assert_eq!(
            source_of("Ch. 11").locator_chapter.as_deref(),
            Some("Ch. 11")
        );
        assert_eq!(
            source_of("Act II, scene i").locator_act_scene.as_deref(),
            Some("Act II, scene i")
        );
        assert_eq!(
            source_of("lines 39-40").locator_line.as_deref(),
            Some("lines 39-40")
        );
    }

    #[test]
    fn reads_a_cite_template() {
        let source = source_of(
            "{{cite journal|title=Building Up of a University|journal=The Nineteenth Century|date=November 1886|pages=724-741}}",
        );
        assert_eq!(
            source.work_title.as_deref(),
            Some("Building Up of a University")
        );
        assert_eq!(
            source.publication.as_deref(),
            Some("The Nineteenth Century")
        );
        assert_eq!(source.work_type.as_deref(), Some("journal article"));
        assert_eq!(source.date_iso.as_deref(), Some("1886-11"));
        assert_eq!(source.cite_templates.len(), 1);
    }

    #[test]
    fn reads_an_isbn_and_a_url() {
        assert_eq!(
            source_of("{{ISBN|978-0-444-52133-0}}").isbn.as_deref(),
            Some("978-0-444-52133-0")
        );
        assert_eq!(
            source_of("ISBN 0-444-52133-X").isbn.as_deref(),
            Some("0-444-52133-X")
        );
        assert_eq!(
            source_of("[http://a.example/x A page]").url.as_deref(),
            Some("http://a.example/x")
        );
    }

    #[test]
    fn does_not_read_a_word_inside_another_word() {
        assert_eq!(source_of("Letterman show").occasion, None);
        assert_eq!(
            source_of("Letter no. 155").occasion.as_deref(),
            Some("letter")
        );
    }

    #[test]
    fn reads_the_occasion() {
        let source = source_of("in a letter to [[Thomas Jefferson]] (22 June 1819)");
        assert_eq!(source.occasion.as_deref(), Some("letter"));
        assert_eq!(source.date_iso.as_deref(), Some("1819-06-22"));
    }

    #[test]
    fn ignores_the_date_a_web_page_was_visited() {
        let source = source_of(
            "Letter no. 155 (June 1880), published in [http://a.example the letters]. Retrieved 29 July 2014.",
        );
        assert_eq!(source.date_iso.as_deref(), Some("1880-06"));
    }

    #[test]
    fn a_bare_note_gives_nothing_but_stays_raw() {
        let source = source_of("Lu Xun's 1st Musou Attack");
        assert_eq!(source.work_title, None);
        assert_eq!(source.date_iso, None);
        assert_eq!(source.locator_page, None);
    }

    #[test]
    fn a_later_part_wins_and_an_empty_one_changes_nothing() {
        let mut source = source_of("''Common Sense'' (1776)");
        assert_eq!(source.work_title.as_deref(), Some("Common Sense"));
        // The line nearest the quote is the most specific, so it wins.
        apply(&mut source, &rich("''The American Crisis'', p. 3"));
        assert_eq!(source.work_title.as_deref(), Some("The American Crisis"));
        assert_eq!(source.locator_page.as_deref(), Some("p. 3"));
        // A part that names no work leaves the one already found alone.
        apply(&mut source, &rich("p. 12"));
        assert_eq!(source.work_title.as_deref(), Some("The American Crisis"));
        assert_eq!(source.date_iso.as_deref(), Some("1776"));
    }

    #[test]
    fn links_a_wikisource_work() {
        let source =
            source_of("''[[s:African Slavery in America|African Slavery in America]]'' (1775)");
        assert_eq!(source.work_link_site.as_deref(), Some("wikisource"));
        assert_eq!(
            source.work_link_title.as_deref(),
            Some("African Slavery in America")
        );
    }
}
