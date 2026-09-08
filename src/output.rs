//! Writers for the pipeline artifacts.

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result};
use flate2::Compression;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use serde::Serialize;

/// Reads a gzip JSON Lines file back, one line at a time.
pub fn read_lines(path: &Path) -> Result<impl Iterator<Item = std::io::Result<String>>> {
    let file = File::open(path).with_context(|| format!("cannot read {}", path.display()))?;
    let reader = BufReader::with_capacity(1 << 20, MultiGzDecoder::new(file));
    Ok(reader.lines())
}

/// Writes one JSON value per line into a gzip file.
pub struct JsonLines {
    inner: GzEncoder<BufWriter<File>>,
}

impl JsonLines {
    /// Compression level 3. Level 6 spends about a third of the whole run time
    /// on the biggest artifact for a few per cent of size.
    const LEVEL: Compression = Compression::new(3);

    pub fn create(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file =
            File::create(path).with_context(|| format!("cannot write {}", path.display()))?;
        let inner = GzEncoder::new(BufWriter::with_capacity(1 << 20, file), Self::LEVEL);
        Ok(Self { inner })
    }

    pub fn write(&mut self, value: &impl Serialize) -> Result<()> {
        serde_json::to_writer(&mut self.inner, value)?;
        self.inner.write_all(b"\n")?;
        Ok(())
    }

    /// Writes a line that was serialised elsewhere, for example on a worker.
    pub fn write_line(&mut self, line: &str) -> Result<()> {
        self.inner.write_all(line.as_bytes())?;
        self.inner.write_all(b"\n")?;
        Ok(())
    }

    /// Flushes the gzip stream. Dropping the writer would hide an error here.
    pub fn finish(self) -> Result<()> {
        self.inner.finish()?.flush()?;
        Ok(())
    }
}
