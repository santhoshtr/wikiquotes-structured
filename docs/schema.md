# Schema

Two layers. Layer 1 is the document model: a faithful tree of the wikitext, shipped
as JSON Lines. Layer 2 is the quote record: a flat, interpreted view, shipped as
Parquet. Layer 2 is built from Layer 1.

Types are shown as Rust. `Option<T>` is written `T?`.

The two layers use different type styles on purpose.

- Layer 1 uses Rust enums that carry data. JSON holds them well.
- Layer 2 uses **no enums that carry data**. A value that varies becomes a set of
  flat columns with a `*_kind` or `*_precision` string beside it. Parquet can store
  a union, but Arrow unions are awkward for DuckDB, Polars and pandas, and the point
  of Parquet here is that people can query it.

Every enum that reads free text from the wiki has an `Other(String)` arm. The wiki
has no schema, so new values will appear.

## Shared types

### RichText

Every piece of prose keeps three views. `text` is for search and display. `wikitext`
is for round-trip and for anybody who disagrees with our reading. `spans` point into
`text` by byte offset, so links stay usable after the markup is gone.

```rust
struct RichText {
    text: String,        // markup removed
    wikitext: String,    // exact source slice
    spans: Vec<Span>,
}

struct Span {
    start: u32,          // byte offset into `text`
    end: u32,
    kind: SpanKind,
}

enum SpanKind {
    Link { target: PageRef },
    ExternalLink { url: String },
    Italic,
    Bold,
    Template(TemplateRef),
    Comment { text: String },   // <!-- … -->; text is not in `text`
    Entity,                     // &nbsp; &ndash;
}
```

### PageRef

A wikilink target. Wikiquote links to itself, to Wikipedia (`w:`), to Wikisource
(`s:`) and to Wiktionary. The interwiki prefix says which project.

```rust
struct PageRef {
    site: Site,          // Wikiquote | Wikipedia | Wikisource | Wiktionary | Commons | Other(String)
    lang: String,        // "en" unless the link says otherwise, e.g. "w:de:…"
    title: String,
    fragment: String?,   // "#The Knight's Tale"
}
```

### TemplateRef

Templates are recorded, not expanded.

```rust
struct TemplateRef {
    name: String,                    // normalised: lowercase, spaces
    params: Vec<(String, String)>,   // positional params get keys "1", "2", …
    wikitext: String,
}
```

## Language

Two different languages exist in one record and must not be merged.

- `wiki_language` — the wiki the page came from. `"en"` for this dump. It is also in
  every output file name.
- `language` — the language of the quote text itself. A page on English Wikiquote
  often holds a Latin, French or Sanskrit original with an English translation below.

```rust
struct LanguageTag {
    code: String,          // BCP 47, e.g. "en", "la", "sa-Deva"
    from: LanguageSource,  // Declared | WikiDefault
}

enum LanguageSource {
    Declared,      // from {{lang|xx|…}}, {{Hebrew}}, or a matching translation annotation
    WikiDefault,   // assumed to be the wiki language; not verified
}
```

We do not run statistical language detection. If the wikitext does not declare a
language, `from` is `WikiDefault` and a consumer knows the value is an assumption.

## Layer 1: the document model

```rust
struct Document {
    // identity, from the XML
    wiki_language: String,           // "en"
    title: String,
    page_id: u32,
    revision_id: u32,
    timestamp: String,               // ISO 8601
    redirect_to: PageRef?,           // set => all other content fields are empty

    // derived
    page_type: PageType,
    page_type_evidence: Vec<String>, // which category or template decided it

    // page level
    categories: Vec<String>,
    templates: Vec<TemplateRef>,     // templates outside any section body
    interwiki: Vec<PageRef>,         // from {{wikipedia}}, {{w|…}}, {{commonscat}}

    lead: Vec<Block>,                // content before the first heading
    sections: Vec<Section>,          // nested

    parse: ParseStats,
}

struct ParseStats {
    bytes: u32,
    error_nodes: u32,                // tree-sitter ERROR nodes
    unassigned_lines: u32,           // non-blank source lines with no node
}
```

### PageType

```rust
enum PageType {
    Person,
    Film,
    TelevisionSeries,
    TelevisionSeason,
    LiteraryWork,
    VideoGame,
    MusicalWork,
    Theme,           // Category:Themes and topic pages
    Proverbs,
    List,            // "Last words" and similar list pages in ns0
    Disambiguation,
    Placeholder,     // {{year page placeholder}} and similar
    Unknown,
}
```

`Unknown` covers 18,789 pages today. It is a normal value, not a failure.

### Section

```rust
struct Section {
    level: u8,                   // 2 to 6
    heading: RichText,           // headings hold links and italics
    role: SectionRole,
    source_hint: SourceHint?,    // read from the heading shape
    blocks: Vec<Block>,
    sections: Vec<Section>,
}
```

### SectionRole

The role decides how a `*` line is read. This table is the core heuristic of the
pipeline, so it lives in one place and is matched case-insensitively against the
heading with markup removed.

```rust
enum SectionRole {
    Quotes,          // "Quotes", "Quote", "Sourced", "Quotations"
    QuotesAbout,     // "Quotes about X", "About", "About {{PAGENAME}}"
    Attributed,
    Disputed,
    Misattributed,
    Unsourced,       // "Unsourced", "Suggestions"
    Dialogue,
    Taglines,
    SongLyrics,
    Cast,            // "Cast", "Voice cast"
    Episodes,        // "Episodes", "Season N"
    SeeAlso,
    ExternalLinks,
    References,
    AlphaBucket(char),           // a heading that is a single letter
    Other(String),               // keep the original heading text
}
```

A section whose role is `Other` and whose heading looks like a work title is still a
quote container when its parent role is a quote role. The heading then gives a
`SourceHint`.

The role table is English for now. It is a Rust `const` table, not a data file. When
a second language is analysed, move it to a data file then, not before.

### SourceHint

Only these shapes carry source meaning. A heading with no hint contributes nothing
to `Source`.

```rust
enum SourceHint {
    Work { title: String, year: u16? },        // === ''Common Sense'' (1776) ===
    Episode { title: String?, code: String? }, // === ''Pilot'' [1.01] ===
    Season { number: u16 },                    // == Season 1 ==
    Period { from: i32, to: i32 },             // === 1790s ===  or  === 1997 ===
}
```

`AlphaBucket` headings and role headings such as `Quotes about X` have no hint.

### Block

```rust
enum Block {
    Quote(QuoteItem),        // a "*" item and its "**" children
    Exchange(Exchange),      // dialogue turns between two <hr> lines
    Item(RichText),          // a "*" item in Cast, See also, External links
    Paragraph(RichText),
    Media(Media),
    Template(TemplateRef),   // a template alone on a line
    Table { wikitext: String },
    Rule,                    // <hr>, kept so the layout can be rebuilt
}
```

### QuoteItem

The `*` line with its children. Nesting is rebuilt from the marker length, because
the grammar returns a flat list.

```rust
struct QuoteItem {
    text: RichText,
    annotations: Vec<Annotation>,
}

struct Annotation {
    depth: u8,               // 2 for "**", 3 for "***"
    text: RichText,
    kind: AnnotationKind,
}

enum AnnotationKind {
    Citation,      // holds a work title, a date, a page number or a cite template
    Attribution,   // names a person: the speaker, in a "Quotes about" section
    Translation,   // "Translation:", or follows an italic non-English quote
    CrossRef,      // "Compare:", "See also:", "Variant:"
    Note,          // anything else
}
```

`AnnotationKind` is a guess. `Citation` and `Attribution` are often the same line:
`** [[John Adams]], in a letter to [[Thomas Jefferson]] (22 June 1819)`.
Layer 2 reads both meanings from one annotation. They need not be separated here.

### Exchange and Turn

```rust
struct Exchange {
    turns: Vec<Turn>,
    annotations: Vec<Annotation>,   // a trailing "**" or ":*" citation
}

struct Turn {
    speaker: String?,               // from :'''Name''': — None for a stage direction
    speaker_link: PageRef?,
    text: RichText,
    is_stage_direction: bool,       // the whole line is italic, e.g. :''[Alice waves]''
}
```

### Media

Image captions on Wikiquote usually repeat a quote that is already in the page body.
Record them, mark them, and let Layer 2 drop them.

```rust
struct Media {
    file: String,
    caption: RichText?,
    caption_attribution: String?,   // the part after "~" in "… ~ [[John Adams]]"
}
```

## Layer 2: the quote record

One flat record per quote. This is the main artifact, written as Parquet.

```rust
struct Quote {
    id: String,               // blake3 of page title + section path + index, 16 hex chars
    content_hash: String,     // blake3 of the normalised text, to group duplicates

    wiki_language: String,    // "en"
    page_title: String,
    page_id: u32,
    page_type: String,        // PageType as a string
    revision_id: u32,

    text: String,             // plain text
    wikitext: String,
    language: String,         // BCP 47, language of `text`
    language_from: String,    // "declared" | "wiki_default"

    kind: String,             // QuoteKind as a string
    status: String,           // QuoteStatus as a string

    speaker: Agent?,
    about: Vec<Agent>,
    source: Source?,

    context_path: Vec<String>,        // heading texts, page root to the quote
    annotations: Vec<AnnotationOut>,  // every "**" line, kept verbatim
    translations: Vec<Translation>,
    links: Vec<Link>,                 // every wikilink inside the quote text

    provenance: Provenance,
}
```

`context_path` is the literal heading path. It always exists and is never a guess.
It is separate from `source` because a heading can be a source, a status marker
(`Misattributed`), a topic marker (`Quotes about Paine`) or a sort bucket (`S`).
Only source-bearing headings enter `source`; see `Source.parts`.

### kind and status

Stored as strings so Parquet stays flat.

`kind` is one of: `monologue`, `dialogue_turn`, `dialogue`, `lyric`, `tagline`,
`proverb`.

`status` is one of: `sourced`, `unsourced`, `attributed`, `disputed`,
`misattributed`. It comes from the section role first, then from the presence of a
citation.

### Agent

```rust
struct Agent {
    name: String,
    kind: String,          // "person" | "character" | "group" | "work" | "topic" | "unknown"
    link_site: String?,    // "wikiquote" | "wikipedia" | …
    link_lang: String?,
    link_title: String?,   // for a later Wikidata lookup
    played_by: String?,    // for a character, from the Cast section of the same page
}
```

`played_by` is a name, not a nested `Agent`, so that the Parquet type does not recurse.

### Link

```rust
struct Link {
    site: String,
    lang: String,
    title: String,
    fragment: String?,
}
```

### Source

The heart of the schema. A source is assembled from several places in the page. Each
place is one `SourcePart`, in page order. The flat fields on top are the merged,
best-effort reading of all parts.

```rust
struct Source {
    raw: String,             // every part joined with " — ", in page order
    complete: bool,          // true when some part gives a work, an occasion or a date
    parts: Vec<SourcePart>,

    // merged fields, best effort
    work_title: String?,
    work_type: String?,      // "book" | "film" | "series" | "episode" | "poem" | "song" | "speech" | "letter" | …
    work_year: u16?,
    work_link_site: String?,
    work_link_title: String?,

    episode_series: String?,
    episode_season: u16?,
    episode_number: String?,
    episode_title: String?,

    date_iso: String?,       // "1776", "1776-12" or "1776-12-23"
    date_end_iso: String?,   // set only for a range
    date_precision: String?, // "year" | "month" | "day" | "decade" | "circa" | "range"
    date_raw: String?,

    locator_page: String?,   // "p. 12", "pp. 77-78"
    locator_chapter: String?,
    locator_part: String?,   // "No. 1", "Book II"
    locator_act_scene: String?,
    locator_line: String?,

    publication: String?,    // journal or newspaper
    publisher: String?,
    isbn: String?,
    url: String?,
    occasion: String?,       // "letter" | "speech" | "interview" | "essay" | "post" | …

    cite_templates: Vec<TemplateRef>,
}

struct SourcePart {
    origin: String,   // "heading" | "annotation" | "page_subject"
    level: u8?,       // heading level, when origin is "heading"
    hint: String?,    // "work" | "episode" | "season" | "period"; None for an annotation
    raw: String,      // wikitext of this part
}
```

Rules for `parts`:

- A heading enters `parts` only when its `SourceHint` is set. `S`, `Quotes`,
  `Misattributed` and `Quotes about X` never enter.
- Headings come first, outermost first, then the `Citation` annotations.
- For a work page such as a film, the page itself is a part with origin
  `page_subject`.
- `complete` is `false` when every part is only a locator, for example a lone
  `** p. 223`. Such a quote has a partial source and a consumer can filter it out.

### Translation and AnnotationOut

```rust
struct Translation {
    text: String,
    language: String?,
    translator: String?,
    is_original: bool,         // true when the quote text is the translation
}

struct AnnotationOut {
    kind: String,              // AnnotationKind as a string
    text: String,
    wikitext: String,
}
```

### Provenance

How much of the record was a guess. A consumer filters on this.

```rust
struct Provenance {
    speaker_from: String?,   // "page_subject" | "annotation" | "dialogue_marker" | "cast_section"
    source_from: String?,    // "heading" | "annotation" | "page_subject" | "mixed"
    status_from: String?,    // "section_role" | "citation_presence"
    unparsed_annotations: u8,
}
```

## How the layers connect

The rules that turn Layer 1 into Layer 2, by page type.

| Page type | Section role | speaker | about | source |
| --- | --- | --- | --- | --- |
| Person | Quotes | the page subject | links in the text | heading path, then the annotation |
| Person | QuotesAbout | from the annotation | the page subject | the annotation |
| Person | Misattributed | none; the page subject is the false attribution | — | the annotation |
| Theme | Quotes | from the annotation | the page subject | the annotation |
| Film, Game | Quotes | from the annotation, often a character | — | the page subject |
| Film, Series | Dialogue | the turn speaker | — | the page subject plus the heading path |
| Series | Season, Episode | the turn speaker | — | series, season, episode from the heading path |
| Proverbs | Quotes | none | the page subject | the annotation |
| Any | Taglines | none | the page subject | the page subject |
| Unknown | Quotes | from the annotation | the page title | the annotation |

Character names in a `Dialogue` section are matched against the `Cast` section of the
same page to fill `Agent.played_by`.

## Output files

`{lang}` is the wiki language code and is part of every name.

```
out/en/redirects-en.jsonl.gz    { "from": "Les Miserables", "to": "Les Misérables" }
out/en/pages-en.jsonl.gz        one Layer 1 Document per line
out/en/quotes-en.parquet        one Layer 2 Quote per row
out/en/report-en.json           counts and coverage per page type
```

Parquet settings: ZSTD compression, 100,000 rows per row group, statistics on
`page_title`, `page_type`, `status`, `kind`, `language` and `content_hash`.

Writing path: `serde_arrow` builds Arrow `RecordBatch` values straight from the same
serde types, and the `parquet` crate writes them. No second schema definition by hand.

## Worked example

Input, from `Thomas Paine`:

```wikitext
== Quotes ==
=== 1770s ===
==== ''Common Sense'' (1776) ====
* These are the times that try men's souls.
** ''The American Crisis'', No. 1 (23 December 1776)

== Quotes about Paine ==
* Without the pen of Paine, the sword of [[George Washington|Washington]] would have been wielded in vain.
** Attributed to [[John Adams]] since at least 1957
```

Layer 2 output, shortened:

```json
{
  "id": "3f2a1c9e5b7d0846",
  "wiki_language": "en",
  "page_title": "Thomas Paine",
  "page_type": "person",
  "text": "These are the times that try men's souls.",
  "language": "en",
  "language_from": "wiki_default",
  "kind": "monologue",
  "status": "sourced",
  "speaker": { "name": "Thomas Paine", "kind": "person",
               "link_site": "wikipedia", "link_lang": "en", "link_title": "Thomas Paine" },
  "about": [],
  "context_path": ["Quotes", "1770s", "Common Sense (1776)"],
  "source": {
    "raw": "1770s — ''Common Sense'' (1776) — ''The American Crisis'', No. 1 (23 December 1776)",
    "complete": true,
    "parts": [
      { "origin": "heading", "level": 3, "hint": "period", "raw": "1770s" },
      { "origin": "heading", "level": 4, "hint": "work", "raw": "''Common Sense'' (1776)" },
      { "origin": "annotation", "hint": null, "raw": "''The American Crisis'', No. 1 (23 December 1776)" }
    ],
    "work_title": "The American Crisis",
    "work_type": "book",
    "work_year": 1776,
    "date_iso": "1776-12-23",
    "date_precision": "day",
    "date_raw": "23 December 1776",
    "locator_part": "No. 1"
  },
  "provenance": { "speaker_from": "page_subject", "source_from": "mixed",
                  "status_from": "citation_presence", "unparsed_annotations": 0 }
}
{
  "id": "8c40de17aa93b265",
  "wiki_language": "en",
  "page_title": "Thomas Paine",
  "page_type": "person",
  "text": "Without the pen of Paine, the sword of Washington would have been wielded in vain.",
  "language": "en",
  "language_from": "wiki_default",
  "kind": "monologue",
  "status": "attributed",
  "speaker": { "name": "John Adams", "kind": "person",
               "link_site": "wikiquote", "link_lang": "en", "link_title": "John Adams" },
  "about": [ { "name": "Thomas Paine", "kind": "person" } ],
  "context_path": ["Quotes about Paine"],
  "source": {
    "raw": "Attributed to [[John Adams]] since at least 1957",
    "complete": false,
    "parts": [ { "origin": "annotation", "raw": "Attributed to [[John Adams]] since at least 1957" } ]
  },
  "links": [ { "site": "wikiquote", "lang": "en", "title": "George Washington" } ],
  "provenance": { "speaker_from": "annotation", "source_from": "annotation",
                  "status_from": "section_role", "unparsed_annotations": 0 }
}
```

Note the second record: `source.complete` is `false`, because the annotation names an
attributor but no work, date or occasion. The heading `Quotes about Paine` is in
`context_path` but not in `source.parts`, because it is a topic marker, not a source.
