//! Writes quote records as Parquet.
//!
//! The Arrow schema is traced from the Rust types, so there is no second
//! schema by hand that could drift from the first.

use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result};
use arrow::datatypes::FieldRef;
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;
use serde_arrow::schema::{SchemaLike, TracingOptions};

use crate::quote::Quote;

/// Rows per row group. Small enough that a reader can skip, large enough that
/// the column statistics stay useful.
const ROW_GROUP: usize = 100_000;

pub struct QuoteWriter {
    writer: ArrowWriter<File>,
    fields: Vec<FieldRef>,
    pending: Vec<Quote>,
    rows: u64,
}

impl QuoteWriter {
    pub fn create(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Nullable fields have to be traced from the type, not from a sample,
        // or a column that is empty in the first batch gets the wrong type.
        let options = TracingOptions::default().allow_null_fields(true);
        let fields = Vec::<FieldRef>::from_type::<Quote>(options)
            .context("cannot derive the Arrow schema for a quote")?;
        let schema = arrow::datatypes::Schema::new(fields.clone());
        let properties = WriterProperties::builder()
            .set_compression(Compression::ZSTD(ZstdLevel::try_new(3)?))
            .set_max_row_group_row_count(Some(ROW_GROUP))
            .build();
        let file =
            File::create(path).with_context(|| format!("cannot write {}", path.display()))?;
        let writer = ArrowWriter::try_new(file, schema.into(), Some(properties))?;
        Ok(Self { writer, fields, pending: Vec::with_capacity(ROW_GROUP), rows: 0 })
    }

    pub fn push(&mut self, quote: Quote) -> Result<()> {
        self.pending.push(quote);
        if self.pending.len() >= ROW_GROUP {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let batch = serde_arrow::to_record_batch(&self.fields, &self.pending)?;
        self.writer.write(&batch)?;
        self.rows += self.pending.len() as u64;
        self.pending.clear();
        Ok(())
    }

    pub fn finish(mut self) -> Result<u64> {
        self.flush()?;
        self.writer.close()?;
        Ok(self.rows)
    }
}
