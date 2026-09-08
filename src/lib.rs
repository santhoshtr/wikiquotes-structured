//! Wikiquote dump to structured quotes.
//!
//! The pipeline has two layers. `parse` builds the Layer 1 document model, a
//! faithful tree of one page. `normalize` turns that into Layer 2 quote
//! records. Every heuristic lives in Layer 2, so Layer 1 never has to be built
//! again when a rule changes. See `docs/schema.md`.

pub mod citation;
pub mod classify;
pub mod dump;
pub mod model;
pub mod normalize;
pub mod output;
pub mod parquet_out;
pub mod parse;
pub mod quote;
pub mod report;
pub mod roles;
pub mod wikitext;
