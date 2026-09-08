mod dump;
mod model;
mod output;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
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

#[derive(Serialize)]
struct Redirect<'a> {
    from: &'a str,
    to: &'a str,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Parse { wiki_language, input, pages, redirects } => {
            let _ = (wiki_language, pages);
            let mut redirect_writer = output::JsonLines::create(&redirects)?;
            let (mut seen, mut content, mut redirected) = (0u32, 0u32, 0u32);

            for page in dump::DumpReader::open(&input)? {
                let page = page?;
                seen += 1;
                if page.namespace != 0 {
                    continue;
                }
                match &page.redirect_to {
                    Some(target) => {
                        redirected += 1;
                        redirect_writer.write(&Redirect { from: &page.title, to: target })?;
                    }
                    None => content += 1,
                }
            }

            redirect_writer.finish()?;
            eprintln!("pages {seen}, content {content}, redirects {redirected}");
            Ok(())
        }
    }
}
