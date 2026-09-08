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

## Status

Early. English Wikiquote is the first target. The section names and the
dialogue markup are English for now; the schema carries a language field from
the start, so other wikis need no schema change.

## License

MIT
