# wikiquotes-structured

Turns a Wikiquote XML dump into structured quote data.

Wikiquote holds about a million quotes, but they are wikitext: a quote is a `*`
line, its source is the `**` line below it, and the rest of the citation is in
the section heading above it. This tool reads that structure with
[tree-sitter-wikitext](https://github.com/wikimedia/tree-sitter-wikitext) and
writes it as data you can query.

## Output

The pipeline has two layers.

**Layer 1** is a document model: a faithful tree of the page, with sections,
list items, dialogue turns and templates. It makes no guess about meaning.
It is written as JSON Lines.

**Layer 2** is the quote record: a flat row with the speaker, the source, the
topic and the status of each quote. Every derived field records where it came
from, so you can filter on how much was a guess. It is written as Parquet.

The split keeps the guesswork in one place. You can re-derive Layer 2 from
Layer 1 without reading the dump again.

```
out/en/redirects-en.jsonl.gz   redirect map
out/en/pages-en.jsonl.gz       Layer 1, one document per line
out/en/quotes-en.parquet       Layer 2, one quote per row
out/en/report-en.json          coverage and quality numbers
```

See [docs/schema.md](docs/schema.md) for the field definitions.

## Examples

Two records from the English dump, as `quotes-en.parquet` holds them. Null
fields are left out here for room; they are present in the file.

A quote by a person. Wikiquote spreads the citation over three places: the
subject of the page said it, a `=== 1910s ===` heading holds the decade, and
the `**` line below the quote holds the letter, the date, the collection and
the page. `source.parts` says which piece came from where, and `source.raw`
joins them into one citation.

```json
{
  "id": "3d0d27db3d52a1e1",
  "content_hash": "7fdff6932a86a86b",
  "wiki_language": "en",
  "page_title": "Bertrand Russell",
  "page_id": 30,
  "page_type": "person",
  "text": "[One] must look into hell before one has any right to speak of heaven.",
  "language": "en",
  "language_from": "wiki_default",
  "kind": "monologue",
  "status": "sourced",
  "speaker": {
    "name": "Bertrand Russell",
    "kind": "person",
    "link_site": "wikiquote",
    "link_lang": "en",
    "link_title": "Bertrand Russell"
  },
  "source": {
    "raw": "1910s — Letter to Colette O'Niel, October 23, 1916; published in ''The Selected Letters of Bertrand Russell: The Public Years, 1914-1970'', p. 87",
    "complete": true,
    "parts": [
      { "origin": "heading", "level": 3, "hint": "period", "raw": "1910s" },
      { "origin": "annotation",
        "raw": "Letter to Colette O'Niel, October 23, 1916; published in ''The Selected Letters of Bertrand Russell: The Public Years, 1914-1970'', p. 87" }
    ],
    "work_title": "The Selected Letters of Bertrand Russell: The Public Years, 1914-1970",
    "date_iso": "1916-10-23",
    "date_precision": "day",
    "date_raw": "October 23, 1916",
    "locator_page": "p. 87",
    "occasion": "letter"
  },
  "context_path": ["Quotes", "1910s"],
  "annotations": [
    { "kind": "citation",
      "text": "Letter to Colette O'Niel, October 23, 1916; published in The Selected Letters of Bertrand Russell: The Public Years, 1914-1970, p. 87" }
  ],
  "provenance": {
    "speaker_from": "page_subject",
    "source_from": "mixed",
    "status_from": "citation_presence",
    "unparsed_annotations": 0
  }
}
```

A line of film dialogue. The speaker is a character, and the actor comes from
the Cast section of the same page. The film itself is the source, so there is
no citation to read and no date to find. `provenance` says as much.

```json
{
  "id": "47f0e70b84557720",
  "content_hash": "83bdd6fae817ab4b",
  "wiki_language": "en",
  "page_title": "Van Helsing",
  "page_id": 4443,
  "page_type": "film",
  "text": "I could never allow him to be used for such evil!",
  "wikitext": "'''Dr. Frankenstein:''' I could never allow him to be used for such evil!",
  "language": "en",
  "language_from": "wiki_default",
  "kind": "dialogue_turn",
  "status": "sourced",
  "speaker": {
    "name": "Dr. Frankenstein",
    "kind": "character",
    "played_by": "Samuel West"
  },
  "source": {
    "raw": "Van Helsing",
    "complete": true,
    "parts": [ { "origin": "page_subject", "raw": "Van Helsing" } ],
    "work_title": "Van Helsing",
    "work_type": "film",
    "work_link_site": "wikiquote",
    "work_link_title": "Van Helsing"
  },
  "context_path": ["Dialogue"],
  "provenance": {
    "speaker_from": "dialogue_marker",
    "source_from": "page_subject",
    "status_from": "citation_presence",
    "unparsed_annotations": 0
  }
}
```

## Usage

You need Rust, GNU Make 4.3 or later, curl and bzip2.

```sh
make                    # download the English dump and build everything
make WIKI_LANG=ml       # the same for Malayalam Wikiquote
make layer1             # stop after the document model
make clean              # remove out/, keep the dump
```

Every stage is a file target, so a repeated `make` does only the work that is
out of date. The dump is downloaded and extracted only when it is missing;
`make distclean` forces a fresh copy.

To work with a dump you already have, put it in `data/` under the name the
Makefile expects, for example
`data/enwikiquote-latest-pages-articles.xml`.

The binary can also be run on its own:

```sh
cargo build --release
./target/release/wikiquotes-structured parse --help
```

## Numbers

The English dump, 71,278 content pages, takes about 75 seconds end to end and
yields 2,681,297 quote records: 1.48 million dialogue turns, 929,000
monologues, 229,000 dialogue exchanges, and the rest taglines, lyrics and
proverbs.

Of those records, 79.9% name a speaker and 88.3% carry a source complete
enough to look up. 0.3% of the non-blank source lines in the dump reached no
node in the document model.

Coverage is uneven by design. On a film page the source is the page itself, so
it is always complete and a date is almost never known. On a person page the
source is built from the heading path and the citation line, so 77.6% is
complete and 68.5% carries a date. Every derived field records where it came
from, so you can filter on how much was a guess.

## Status

Working, and only measured against English Wikiquote. The section names and
the dialogue markup are English for now; the schema carries a language field
from the start and every artifact name carries the language code, so another
wiki needs no schema change.

## License

MIT
