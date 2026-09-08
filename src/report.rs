//! Counts what the pipeline produced and how much of it was a guess.
//!
//! The numbers here are the ones to watch when a rule changes: the share of
//! source lines that reached no node, the share of quotes with a speaker or a
//! complete source, and the headings no rule could name.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use arrow::datatypes::FieldRef;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde::Serialize;
use serde_arrow::schema::{SchemaLike, TracingOptions};

use crate::model::{Document, Section, SectionRole};
use crate::output;
use crate::quote::Quote;

#[derive(Default, Serialize)]
pub struct Report {
    pub pages: Pages,
    pub quotes: Quotes,
    /// Headings that no rule could name, most common first.
    pub unnamed_headings: Vec<Count>,
}

#[derive(Default, Serialize)]
pub struct Pages {
    pub total: u64,
    pub by_type: BTreeMap<String, u64>,
    pub with_error_node: u64,
    pub error_nodes: u64,
    /// Non-blank source lines that reached no node.
    pub unassigned_lines: u64,
    pub source_lines: u64,
    pub unassigned_share: f64,
}

#[derive(Default, Serialize)]
pub struct Quotes {
    pub total: u64,
    pub by_kind: BTreeMap<String, u64>,
    pub by_status: BTreeMap<String, u64>,
    pub by_page_type: BTreeMap<String, Coverage>,
    pub overall: Coverage,
}

/// How much of a group of records the rules could fill in.
#[derive(Default, Serialize)]
pub struct Coverage {
    pub quotes: u64,
    pub with_speaker: f64,
    pub with_source: f64,
    pub with_complete_source: f64,
    pub with_work: f64,
    pub with_date: f64,
    pub unparsed_annotations: u64,
}

#[derive(Serialize)]
pub struct Count {
    pub heading: String,
    pub sections: u64,
}

pub fn build(pages: &Path, quotes: &Path) -> Result<Report> {
    let mut report = Report::default();
    let mut headings: BTreeMap<String, u64> = BTreeMap::new();

    for line in output::read_lines(pages)? {
        let line = line?;
        let document: Document = serde_json::from_str(&line)?;
        report.pages.total += 1;
        *report
            .pages
            .by_type
            .entry(type_name(&document))
            .or_default() += 1;
        report.pages.error_nodes += u64::from(document.parse.error_nodes);
        if document.parse.error_nodes > 0 {
            report.pages.with_error_node += 1;
        }
        report.pages.unassigned_lines += u64::from(document.parse.unassigned_lines);
        report.pages.source_lines += u64::from(document.parse.source_lines);
        collect_headings(&document.sections, &mut headings);
    }
    report.pages.unassigned_share = share(report.pages.unassigned_lines, report.pages.source_lines);

    let mut counters: BTreeMap<String, Counter> = BTreeMap::new();
    let mut overall = Counter::default();
    for quote in read_quotes(quotes)? {
        let quote = quote?;
        report.quotes.total += 1;
        *report.quotes.by_kind.entry(quote.kind.clone()).or_default() += 1;
        *report
            .quotes
            .by_status
            .entry(quote.status.clone())
            .or_default() += 1;
        counters
            .entry(quote.page_type.clone())
            .or_default()
            .add(&quote);
        overall.add(&quote);
    }
    report.quotes.by_page_type = counters
        .into_iter()
        .map(|(name, counter)| (name, counter.finish()))
        .collect();
    report.quotes.overall = overall.finish();

    let mut unnamed: Vec<Count> = headings
        .into_iter()
        .map(|(heading, sections)| Count { heading, sections })
        .collect();
    unnamed.sort_by(|a, b| b.sections.cmp(&a.sections).then(a.heading.cmp(&b.heading)));
    unnamed.truncate(40);
    report.unnamed_headings = unnamed;

    Ok(report)
}

#[derive(Default)]
struct Counter {
    quotes: u64,
    speaker: u64,
    source: u64,
    complete: u64,
    work: u64,
    date: u64,
    unparsed: u64,
}

impl Counter {
    fn add(&mut self, quote: &Quote) {
        self.quotes += 1;
        self.speaker += u64::from(quote.speaker.is_some());
        self.unparsed += u64::from(quote.provenance.unparsed_annotations);
        let Some(source) = &quote.source else { return };
        self.source += 1;
        self.complete += u64::from(source.complete);
        self.work += u64::from(source.work_title.is_some());
        self.date += u64::from(source.date_iso.is_some());
    }

    fn finish(self) -> Coverage {
        Coverage {
            quotes: self.quotes,
            with_speaker: share(self.speaker, self.quotes),
            with_source: share(self.source, self.quotes),
            with_complete_source: share(self.complete, self.quotes),
            with_work: share(self.work, self.quotes),
            with_date: share(self.date, self.quotes),
            unparsed_annotations: self.unparsed,
        }
    }
}

fn share(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        return 0.0;
    }
    (1000.0 * part as f64 / whole as f64).round() / 10.0
}

fn type_name(document: &Document) -> String {
    serde_json::to_value(document.page_type)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

fn collect_headings(sections: &[Section], out: &mut BTreeMap<String, u64>) {
    for section in sections {
        if let SectionRole::Other(heading) = &section.role
            && section.source_hint.is_none()
            && !heading.is_empty()
        {
            *out.entry(heading.clone()).or_default() += 1;
        }
        collect_headings(&section.sections, out);
    }
}

fn read_quotes(path: &Path) -> Result<impl Iterator<Item = Result<Quote>>> {
    let file = std::fs::File::open(path)?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)?
        .with_batch_size(8192)
        .build()?;
    let fields =
        Vec::<FieldRef>::from_type::<Quote>(TracingOptions::default().allow_null_fields(true))?;
    Ok(reader.flat_map(move |batch| {
        let rows: Vec<Result<Quote>> = match batch {
            Ok(batch) => match serde_arrow::from_record_batch::<Vec<Quote>>(&batch) {
                Ok(quotes) => quotes.into_iter().map(Ok).collect(),
                Err(error) => vec![Err(error.into())],
            },
            Err(error) => vec![Err(error.into())],
        };
        let _ = &fields;
        rows
    }))
}
