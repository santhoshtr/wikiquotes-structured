# Orchestrates the Wikiquote extraction pipeline.
# Every stage is a file target, so `make` repeats only the work that is out of date.
#
#   make                     build everything for English
#   make WIKI_LANG=ml        build everything for Malayalam Wikiquote
#   make quotes              stop after Layer 2
#   make clean               remove the outputs, keep the dump
#   make distclean           remove the outputs and the dump
#
# Do not name the variable LANG. The shell exports LANG, and make imports the
# environment, so `LANG ?= en` would silently take the locale instead.

WIKI_LANG ?= en
WIKI      := $(WIKI_LANG)wikiquote
DUMP_BASE := $(WIKI)-latest-pages-articles
DUMP_URL  := https://dumps.wikimedia.org/$(WIKI)/latest/$(DUMP_BASE).xml.bz2

DATA := data
OUT  := out/$(WIKI_LANG)
BIN  := target/release/wikiquotes-structured

BZ2       := $(DATA)/$(DUMP_BASE).xml.bz2
XML       := $(DATA)/$(DUMP_BASE).xml
PAGES     := $(OUT)/pages-$(WIKI_LANG).jsonl.gz
REDIRECTS := $(OUT)/redirects-$(WIKI_LANG).jsonl.gz
QUOTES    := $(OUT)/quotes-$(WIKI_LANG).parquet
REPORT    := $(OUT)/report-$(WIKI_LANG).json

SOURCES := Cargo.toml $(shell find src -name '*.rs' 2>/dev/null)

.PHONY: all download extract layer1 quotes report clean distclean
.DEFAULT_GOAL := all

all: report

download: $(BZ2)
extract:  $(XML)
layer1:   $(PAGES) $(REDIRECTS)
quotes:   $(QUOTES)
report:   $(REPORT)

# Keep the dump when make decides a file was only an intermediate step.
.PRECIOUS: $(BZ2) $(XML)

# --- the dump ---------------------------------------------------------------
# curl resumes a part file, so a broken download does not start again from zero.
# The rename happens only after curl succeeds, so make never sees a half file.
$(BZ2):
	mkdir -p $(DATA)
	curl --fail --location --continue-at - --output $@.part $(DUMP_URL)
	mv $@.part $@

# The extract rule exists only while the xml is missing. So an existing xml is
# never re-extracted, and the bz2 is never fetched just to satisfy a timestamp.
# Run `make distclean` to force a fresh dump.
ifeq ($(wildcard $(XML)),)
$(XML): $(BZ2)
	bzip2 --decompress --stdout $< > $@.part
	mv $@.part $@
endif

# --- the binary -------------------------------------------------------------
$(BIN): $(SOURCES)
	cargo build --release

# --- Layer 1: the document model -------------------------------------------
# One run makes two files. `&:` is a grouped target and needs GNU Make 4.3 or later.
$(PAGES) $(REDIRECTS) &: $(XML) $(BIN)
	mkdir -p $(OUT)
	$(BIN) parse --wiki-language $(WIKI_LANG) --input $(XML) \
		--pages $(PAGES).part --redirects $(REDIRECTS).part
	mv $(PAGES).part $(PAGES)
	mv $(REDIRECTS).part $(REDIRECTS)

# --- Layer 2: the quote records --------------------------------------------
$(QUOTES): $(PAGES) $(BIN)
	$(BIN) normalize --wiki-language $(WIKI_LANG) --pages $(PAGES) --out $@.part
	mv $@.part $@

# --- quality report ---------------------------------------------------------
$(REPORT): $(PAGES) $(QUOTES) $(BIN)
	$(BIN) report --pages $(PAGES) --quotes $(QUOTES) --out $@.part
	mv $@.part $@

clean:
	rm -rf out

distclean: clean
	rm -rf $(DATA) target
