# Notes for agents

This file holds what is not obvious from reading the code. Read it before you
change a rule. `docs/schema.md` holds the field definitions and the reasons
behind them.

## The one rule

**Layer 1 makes no guesses. Layer 2 holds every heuristic.**

`parse.rs` turns wikitext into a `Document`: a faithful tree of the page.
`normalize.rs` turns a `Document` into `Quote` records with a speaker, a source
and a status. If you find yourself about to guess something in `parse.rs`,
move it to `normalize.rs`.

The reason is cost. Layer 1 takes 27 seconds over the dump and Layer 2 takes
15. If a heuristic lives in Layer 2, a change to it needs no re-parse and no
dump. It also lets a reader who disagrees with our attribution start again
from `pages-en.jsonl.gz`.

Heuristics live in four files only:

| File | Decides |
| --- | --- |
| `roles.rs` | what a section heading means, and whether it says anything about the source |
| `classify.rs` | what kind of subject a page has |
| `citation.rs` | what fields a citation line gives up |
| `normalize.rs` | speaker, topic, status, and how the parts make a source |

## How to run it

```sh
make                    # the whole pipeline for English, about 75 seconds
make layer1             # stop after the document model
cargo test              # 73 unit tests and 2 fixture tests
```

The Makefile downloads and extracts the dump only when the `.xml` is missing.
Every stage is a file target, so a repeated `make` does only what is out of
date. Recipes write `target.part` and rename on success, so an interrupted run
leaves nothing that `make` would treat as finished.

The variable is `WIKI_LANG`, not `LANG`. Your shell exports `LANG` and make
imports the environment.

## Golden fixtures

`tests/fixtures/*.wiki` are seven real pages from the dump, next to a snapshot
of what the pipeline makes of them. Any rule change shows up as a diff.

```sh
cargo test --test fixtures                      # check
UPDATE_FIXTURES=1 cargo test --test fixtures    # accept
```

**Read the diff before you accept it.** The fixtures found two silent faults
on their first run. Both looked like nothing in the code and like everything
in the diff.

## Traps in the grammar

`tree-sitter-wikitext` is error tolerant, which is what we want, but its tree
is not shaped the way you expect.

- **A heading holds its markup as plain text.** `=== ''Common Sense'' (1776) ===`
  gives one `text` node with the quote marks still in it. There is no italic
  node. `PageBuilder::heading_text` re-parses the inside of a heading with a
  second parser. Without that, no role and no source hint is readable.
- **Lists are flat.** `**` is a *sibling* of `*` with a longer `list_marker`,
  not a child. `PageBuilder::list` rebuilds the parent-child link. Do not
  expect nesting.
- **An `<hr>` splits a `definition_list` in two.** That is convenient: one
  definition list is one dialogue exchange, which is exactly the Wikiquote
  convention. Do not add your own grouping.
- **`{{PAGENAME}}` is literal text inside a heading but a node inside a body.**
  Both paths are handled, in `wikitext.rs`.
- **18.5% of pages hold an `ERROR` node.** This is normal. Tree-sitter keeps
  the rest of the tree. Do not treat an error as a reason to skip a page.

## Traps outside the grammar

- **quick-xml 0.42 reports an entity reference as its own event**
  (`Event::GeneralRef`). Without that arm, every `&lt;` and `&amp;` in a quote
  is silently dropped. `dump.rs` uses `resolve_xml_entity`, which is XML only
  on purpose: anything else in a dump arrives already escaped, as text.
- **A page holds several `<id>` elements.** The page id, the revision id and
  the contributor id all look the same. `dump.rs` keeps an element path to tell
  them apart.
- **The `<redirect>` element is authoritative, not the wikitext.** It finds
  34,715 redirects where a scan for `#REDIRECT` finds 34,709.
- **serde_arrow and arrow must agree on a version.** `serde_arrow` has one
  feature per arrow release. If you bump `arrow`, bump the feature.

## Invariants to keep

- **`RichText` spans are byte offsets into `text`, not into `wikitext`.** If
  you trim or cut the text, move the spans with it. `Builder::trim` and
  `strip_speaker` both do this. A test covers it, because the bug is invisible
  otherwise.
- **`Source::raw` is never empty when a source exists.** Parsed fields are
  best effort; the raw line is the promise.
- **`context_path` is literal and always true.** It is the heading path. Only a
  heading whose `SourceHint` is set enters `source.parts`. `Quotes`,
  `Misattributed`, `Quotes about X` and the single-letter sort buckets say
  nothing about a source and must stay out of it.
- **`provenance` must not claim what no rule found.** If the speaker is
  `None`, `speaker_from` is `None`. See the `Attributed` trait in
  `normalize.rs`.
- **Layer 2 has no Rust enum that carries data.** A value that varies becomes
  columns beside a `*_kind` or `*_precision` string. Arrow unions round-trip
  badly into DuckDB and Polars, and Parquet is only worth choosing if the file
  is easy to query.
- **`PageType::Unknown` is a normal answer.** 22% of pages get it. Those are
  nearly all topic pages, and Layer 2 reads a topic page and an unknown page
  the same way, so the gap costs nothing. Do not force a guess.

## What we deliberately do not do

Named here so nobody adds them back out of habit.

- **No template expansion.** A short table in `wikitext.rs` renders only the
  templates that carry words inside a quote: `{{w}}` (64,000 uses), `{{lang}}`,
  `{{ISBN}}`, `{{nbsp}}` and a few layout ones. Everything else renders as
  nothing and is kept as a span with its parameters.
- **No Wikidata lookup.** We record the Wikipedia title from `[[w:…]]` and
  `{{w|…}}`. Resolving it to a QID is a separate job.
- **No full citation parsing.** `citation.rs` extracts only what it can find
  with confidence. A four-digit number is read as a year only inside brackets,
  or `p. 1776` becomes a date. A date after `Retrieved` or `Accessed` is when
  somebody visited a web page, not when the words were said.
- **No deduplication.** The same quote sits on a person page and a theme page.
  Both records stay. `content_hash` lets a reader group them.
- **No text cleanup beyond markup removal.** Do not fix spelling or punctuation.

## Numbers to watch

`out/en/report-en.json` holds them. After a rule change, compare:

| Number | Now |
| --- | --- |
| Source lines that reached no node | 0.3% |
| Quotes with a speaker | 79.9% |
| Quotes with a complete source | 88.3% |
| Quote records | 2,681,297 |
| Pages with an `ERROR` node | 18.5% |

The report also lists the forty most common headings that no rule could name.
That list is the best place to look for the next cheap improvement. It is how
the `Locator` source hint was found: `Chapter 5`, `Act II` and `Preface` were
near the top, and they are places inside a work rather than works.

Coverage is uneven **by design**. On a film page the source is the page itself,
so it is always complete and a date is almost never known. On a person page the
source is built from the heading path and the citation line, so 78% is complete
and 69% carries a date. Do not read a low number for one page type as a fault
without checking what the page type can offer.

## Known gaps

- Pages whose only category is something like `Shakespearean comedies` fall to
  `Unknown`, so their source has no work title. Catching them needs a large
  category list, which nobody has written.
- The `unnamed_headings` list still holds speaker headings such as `Narrator`
  and `Bugs Bunny`. A heading that names a speaker has no place in the schema
  yet.
- Band pages fall to `Unknown`. `PageType` has no `Group`.
- Only English is measured. `roles.rs` is a Rust `const` table on purpose:
  move it to a data file when a second dump has been counted, not before.

## House style

- Small commits, one idea each.
- Comments say why, not what.
- Simplified Technical English in comments and documents, so that a reader
  whose first language is not English can follow.
- Do not improve code next to the code you came to change.
