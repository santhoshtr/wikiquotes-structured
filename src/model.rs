//! Layer 1: the document model.
//!
//! A faithful tree of one wikitext page. It is built by mechanical rules only.
//! Nothing here guesses what a quote means; that is Layer 2's job.
//! `docs/schema.md` describes these types and the reasons behind them.

use serde::{Deserialize, Serialize};

// --- rich text --------------------------------------------------------------

/// Prose in three views. `text` is for reading and search, `wikitext` is the
/// exact source, and `spans` point into `text` so links survive markup removal.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RichText {
    pub text: String,
    pub wikitext: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<Span>,
}

impl RichText {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.wikitext.trim().is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    /// Byte offset into `RichText::text`.
    pub start: u32,
    pub end: u32,
    #[serde(flatten)]
    pub kind: SpanKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "span", rename_all = "snake_case")]
pub enum SpanKind {
    Link {
        target: PageRef,
    },
    ExternalLink {
        url: String,
    },
    Italic,
    Bold,
    BoldItalic,
    Template(TemplateRef),
    /// `<!-- … -->`. The comment text is not part of `RichText::text`.
    Comment {
        text: String,
    },
    /// `<ref>…</ref>`. The note text is not part of `RichText::text`.
    Reference {
        text: String,
    },
}

/// A wikilink target. The interwiki prefix says which project it points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageRef {
    pub site: Site,
    pub lang: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fragment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Site {
    Wikiquote,
    Wikipedia,
    Wikisource,
    Wiktionary,
    Commons,
    Other(String),
}

/// A template call. Templates are recorded, never expanded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateRef {
    /// Lowercase, spaces instead of underscores.
    pub name: String,
    /// Positional arguments get the keys "1", "2", and so on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<(String, String)>,
    pub wikitext: String,
}

impl TemplateRef {
    pub fn param(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

// --- the page ---------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub wiki_language: String,
    pub title: String,
    pub page_id: u32,
    pub revision_id: u32,
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect_to: Option<PageRef>,

    pub page_type: PageType,
    /// The category or template that decided the page type.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub page_type_evidence: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    /// Templates outside any section body.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub templates: Vec<TemplateRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interwiki: Vec<PageRef>,

    /// Content before the first heading.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lead: Vec<Block>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<Section>,

    pub parse: ParseStats,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ParseStats {
    pub bytes: u32,
    /// Tree-sitter ERROR nodes. The rest of the tree is still usable.
    pub error_nodes: u32,
    /// Non-blank source lines in the page.
    pub source_lines: u32,
    /// Non-blank source lines that reached no node.
    pub unassigned_lines: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageType {
    Person,
    Film,
    TelevisionSeries,
    TelevisionSeason,
    LiteraryWork,
    VideoGame,
    MusicalWork,
    Theme,
    Proverbs,
    List,
    Disambiguation,
    Placeholder,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub level: u8,
    pub heading: RichText,
    pub role: SectionRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hint: Option<SourceHint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<Block>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<Section>,
}

/// What a section is for. The role decides how a `*` line is read, so it is the
/// central heuristic of the pipeline. English only for now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "role", content = "value", rename_all = "snake_case")]
pub enum SectionRole {
    Quotes,
    QuotesAbout,
    Attributed,
    Disputed,
    Misattributed,
    Unsourced,
    Dialogue,
    Taglines,
    SongLyrics,
    Cast,
    Episodes,
    SeeAlso,
    ExternalLinks,
    References,
    /// A heading that is a single letter, used to sort a long list.
    AlphaBucket(char),
    Other(String),
}

/// Source meaning read from the shape of a heading. A heading with no hint
/// gives nothing to the source of a quote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "hint", rename_all = "snake_case")]
pub enum SourceHint {
    Work {
        title: String,
        year: Option<u16>,
    },
    Episode {
        title: Option<String>,
        code: Option<String>,
    },
    Season {
        number: u16,
    },
    Period {
        from: i32,
        to: i32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "block", rename_all = "snake_case")]
pub enum Block {
    /// A `*` item together with its `**` children.
    Quote(QuoteItem),
    /// Dialogue turns between two `<hr>` rules.
    Exchange(Exchange),
    /// A `*` item in Cast, See also or External links.
    Item(RichText),
    Paragraph(RichText),
    Media(Media),
    /// A template alone on a line.
    Template(TemplateRef),
    Table {
        wikitext: String,
    },
    /// `<hr>`, kept so the page layout can be rebuilt.
    Rule,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteItem {
    pub text: RichText,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annotation {
    /// 2 for `**`, 3 for `***`.
    pub depth: u8,
    pub text: RichText,
    pub kind: AnnotationKind,
}

/// A guess about what an annotation is for. One line can be both a citation and
/// an attribution, so Layer 2 reads both meanings from the same line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationKind {
    Citation,
    Attribution,
    Translation,
    CrossRef,
    Note,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exchange {
    pub turns: Vec<Turn>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    /// From `:'''Name''':`. None for a stage direction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_link: Option<PageRef>,
    pub text: RichText,
    /// The whole line is italic, for example `:''[Alice waves]''`.
    pub is_stage_direction: bool,
}

/// An image. Captions on Wikiquote usually repeat a quote from the page body,
/// so they are marked and Layer 2 drops them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Media {
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<RichText>,
    /// The part after `~` in `… ~ [[John Adams]]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption_attribution: Option<String>,
}
