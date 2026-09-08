mod dump;
mod model;
mod output;
mod parse;
mod roles;
mod wikitext;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use rayon::prelude::*;
use serde::Serialize;

#[derive(Parser)]
#[command(version, about = "Turn a Wikiquote XML dump into structured quotes")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Read the dump and write the Layer 1 document model.
    Parse {
        #[arg(long, default_value = "en")]
        wiki_language: String,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        pages: PathBuf,
        #[arg(long)]
        redirects: PathBuf,
    },
}

/// Pages handed to the workers at a time. Large enough to keep them busy,
/// small enough that the batch stays in cache.
const BATCH: usize = 256;

#[derive(Serialize)]
struct Redirect<'a> {
    from: &'a str,
    to: &'a str,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Parse { wiki_language, input, pages, redirects } => {
            let mut redirect_writer = output::JsonLines::create(&redirects)?;
            let mut page_writer = output::JsonLines::create(&pages)?;
            let (mut seen, mut content, mut redirected) = (0u32, 0u32, 0u32);

            // Reading and writing stay on this thread; the wikitext parse runs
            // in parallel over a batch. A tree-sitter parser is not Sync, so
            // every worker builds its own.
            let mut batch: Vec<dump::RawPage> = Vec::with_capacity(BATCH);
            let flush = |batch: &mut Vec<dump::RawPage>,
                             writer: &mut output::JsonLines|
             -> Result<()> {
                let lines: Vec<String> = batch
                    .par_iter()
                    .map_init(parse::DocumentBuilder::new, |builder, page| {
                        serde_json::to_string(&builder.build(page, &wiki_language))
                    })
                    .collect::<serde_json::Result<Vec<String>>>()?;
                for line in lines {
                    writer.write_line(&line)?;
                }
                batch.clear();
                Ok(())
            };

            for page in dump::DumpReader::open(&input)? {
                let page = page?;
                seen += 1;
                if page.namespace != 0 {
                    continue;
                }
                if let Some(target) = &page.redirect_to {
                    redirected += 1;
                    redirect_writer.write(&Redirect { from: &page.title, to: target })?;
                    continue;
                }
                content += 1;
                batch.push(page);
                if batch.len() == BATCH {
                    flush(&mut batch, &mut page_writer)?;
                }
            }
            flush(&mut batch, &mut page_writer)?;

            redirect_writer.finish()?;
            page_writer.finish()?;
            eprintln!("pages {seen}, content {content}, redirects {redirected}");
            Ok(())
        }
    }
}
