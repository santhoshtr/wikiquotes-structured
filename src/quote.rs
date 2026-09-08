//! Layer 2: the quote record.
//!
//! One flat row per quote, written as Parquet. No field carries a Rust enum
//! with data in it: a value that varies becomes a set of columns beside a
//! `*_kind` or `*_precision` string. Arrow unions round-trip badly into DuckDB
//! and Polars, and Parquet is only worth choosing here if the file is easy to
//! query. `docs/schema.md` gives the reasoning.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quote {
    /// Stable across runs: the page, the heading path and the position.
    pub id: String,
    /// The same text on another page gets the same hash, so duplicates group.
    pub content_hash: String,

    pub wiki_language: String,
    pub page_title: String,
    pub page_id: u32,
    pub page_type: String,
    pub revision_id: u32,

    pub text: String,
    pub wikitext: String,
    /// BCP 47 tag for the language of `text`.
    pub language: String,
    /// "declared" when the wikitext says so, "wiki_default" when assumed.
    pub language_from: String,

    pub kind: String,
    pub status: String,

    pub speaker: Option<Agent>,
    pub about: Vec<Agent>,
    pub source: Option<Source>,

    /// Heading texts from the page root down to the quote. Always literal.
    pub context_path: Vec<String>,
    pub annotations: Vec<AnnotationOut>,
    pub translations: Vec<Translation>,
    /// Every wikilink inside the quote text.
    pub links: Vec<Link>,

    pub provenance: Provenance,
}

/// Who said a quote, or who it is about.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    pub name: String,
    /// "person", "character", "group", "work", "topic" or "unknown".
    pub kind: String,
    pub link_site: Option<String>,
    pub link_lang: Option<String>,
    pub link_title: Option<String>,
    /// For a character: the actor from the Cast section of the same page. A
    /// name rather than another Agent, so the Parquet type does not recurse.
    pub played_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub site: String,
    pub lang: String,
    pub title: String,
    pub fragment: Option<String>,
}

/// Where a quote comes from, assembled from several places on the page.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Source {
    /// Every part joined in page order, so this alone is a full citation.
    pub raw: String,
    /// False when the parts give only a locator, such as a lone "p. 223".
    pub complete: bool,
    pub parts: Vec<SourcePart>,

    pub work_title: Option<String>,
    pub work_type: Option<String>,
    pub work_year: Option<u16>,
    pub work_link_site: Option<String>,
    pub work_link_title: Option<String>,

    pub episode_series: Option<String>,
    pub episode_season: Option<u16>,
    pub episode_number: Option<String>,
    pub episode_title: Option<String>,

    /// "1776", "1776-12" or "1776-12-23".
    pub date_iso: Option<String>,
    pub date_end_iso: Option<String>,
    /// "year", "month", "day", "decade", "circa" or "range".
    pub date_precision: Option<String>,
    pub date_raw: Option<String>,

    pub locator_page: Option<String>,
    pub locator_chapter: Option<String>,
    pub locator_part: Option<String>,
    pub locator_act_scene: Option<String>,
    pub locator_line: Option<String>,

    pub publication: Option<String>,
    pub publisher: Option<String>,
    pub isbn: Option<String>,
    pub url: Option<String>,
    /// "letter", "speech", "interview", "essay", "post" and so on.
    pub occasion: Option<String>,

    pub cite_templates: Vec<CiteTemplate>,
}

/// One place on the page that gave part of the citation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourcePart {
    /// "heading", "annotation" or "page_subject".
    pub origin: String,
    /// The heading level, when the origin is a heading.
    pub level: Option<u8>,
    /// "work", "episode", "season" or "period". None for an annotation.
    pub hint: Option<String>,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiteTemplate {
    pub name: String,
    pub params: Vec<Param>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Param {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Translation {
    pub text: String,
    pub language: Option<String>,
    pub translator: Option<String>,
    /// True when the quote text is itself the translation.
    pub is_original: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationOut {
    pub kind: String,
    pub text: String,
    pub wikitext: String,
}

/// How much of the record was a guess. A consumer filters on this.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Provenance {
    /// "page_subject", "annotation", "dialogue_marker" or "cast_section".
    pub speaker_from: Option<String>,
    /// "heading", "annotation", "page_subject" or "mixed".
    pub source_from: Option<String>,
    /// "section_role" or "citation_presence".
    pub status_from: Option<String>,
    pub unparsed_annotations: u8,
}
