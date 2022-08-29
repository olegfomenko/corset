#[macro_use]
extern crate pest_derive;
use crate::parser::Transpiler;
use clap::Parser;
use color_eyre::eyre::*;
use std::io::prelude::*;
use std::path::Path;

mod go;
mod parser;

#[derive(Parser, Debug, Clone)]
#[clap(version)]
pub struct Args {
    #[clap(long = "CE", default_value = "CE")]
    columns_assignment: String,

    #[clap(short, long, value_parser)]
    name: String,

    #[clap(required = true)]
    source: String,

    #[clap(short = 'P', long = "package", required = true)]
    package: String,

    #[clap(short = 'o', long = "out")]
    out_file: Option<String>,

    #[clap(long = "no-stdlib")]
    no_stdlib: bool,
}

fn main() -> Result<()> {
    color_eyre::install()?;
    let args = Args::parse();

    let mut source = if Path::new(&args.source).is_file() {
        std::fs::read_to_string(&args.source)?
    } else {
        args.source.clone()
    };
    if !args.no_stdlib {
        source.push_str(include_str!("stdlib.lisp"))
    }

    let constraints = parser::ConstraintsSet::from_str(&source)
        .with_context(|| format!("while parsing `{}`", &args.source))?;

    let go_exporter = go::GoExporter {
        settings: args.clone(),
    };
    let out = go_exporter.render(&constraints)?;
    if let Some(out_file) = args.out_file {
        std::fs::File::create(&out_file)?.write_all(out.as_bytes())?;
        println!("{} generated", out_file);
    } else {
        println!("{}", out);
    }

    Ok(())
}
