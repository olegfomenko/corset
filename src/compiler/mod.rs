use crate::column::{ColumnSet, Computation};
use anyhow::*;
use itertools::Itertools;
use log::*;
use std::collections::HashMap;

pub use common::*;
pub use generator::{Constraint, ConstraintSet, EvalSettings};
pub use node::{Expression, Node};
pub use parser::{Ast, AstNode, Kind, Token};
pub use tables::ComputationTable;
pub use types::*;

pub mod codetyper;
mod common;
mod compiletime;
mod definitions;
pub mod generator;
mod node;
mod parser;
pub mod tables;
mod types;

const MAIN_MODULE: &str = "<prelude>";

pub struct CompileSettings {
    pub debug: bool,
    pub allow_dups: bool,
}

#[cfg(feature = "interactive")]
pub fn make<S: AsRef<str>>(
    sources: &[(&str, S)],
    settings: &CompileSettings,
) -> Result<(Vec<Ast>, ConstraintSet)> {
    use colored::Colorize;
    use num_bigint::BigInt;

    use crate::{
        compiler::tables::{Scope, Symbol},
        errors::CompileError,
        structs::Handle,
    };

    let mut asts = vec![];
    let mut ctx = Scope::new();

    for (name, content) in sources.iter() {
        info!("Parsing {}", name.bright_white().bold());
        let ast = parser::parse(content.as_ref()).with_context(|| anyhow!("parsing `{}`", name))?;
        definitions::pass(&ast, ctx.clone())
            .with_context(|| anyhow!("parsing definitions in `{}`", name))?;
        asts.push((name, ast));
    }

    let mut columns: ColumnSet = Default::default();
    let mut constants: HashMap<Handle, BigInt> = Default::default();
    let mut computations = ctx.computations();

    for (name, ast) in asts.iter_mut() {
        info!(
            "Evaluating compile-time values in {}",
            name.bright_white().bold()
        );
        compiletime::pass(ast, ctx.clone(), settings).with_context(|| {
            anyhow!(
                "evaluating compile-time values in {}",
                name.bright_white().bold()
            )
        })?
    }

    let constraints = asts
        .iter()
        .map(|(name, ast)| {
            generator::pass(ast, ctx.clone(), settings)
                .with_context(|| anyhow!("compiling constraints in {}", name.bright_white().bold()))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        // Sort by decreasing complexity for more efficient multi-threaded computation
        .sorted_by_cached_key(|x| -(x.size() as isize))
        .collect::<Vec<_>>();

    ctx.visit_mut::<()>(&mut |handle, symbol| {
        match symbol {
            Symbol::Alias(_) => {}
            Symbol::Final(symbol, used) => {
                if !*used {
                    warn!("{}", CompileError::NotUsed(handle.clone()));
                }

                match symbol.e() {
                    Expression::Column {
                        handle,
                        kind: k,
                        padding_value,
                        base,
                    } => {
                        columns.insert_column(
                            handle,
                            symbol.t(),
                            *used,
                            k.to_nil(),
                            settings.allow_dups,
                            padding_value.to_owned(),
                            None,
                            *base,
                        )?;
                        match k {
                            Kind::Atomic | Kind::Phantom => (),
                            Kind::Composite(e) => computations.insert(
                                handle,
                                Computation::Composite {
                                    target: handle.clone(),
                                    exp: *e.clone(),
                                },
                            )?,
                            Kind::Interleaved(_, froms) => computations.insert(
                                handle,
                                Computation::Interleaved {
                                    target: handle.clone(),
                                    froms: froms.as_ref().unwrap().to_owned(),
                                },
                            )?,
                        }
                    }
                    Expression::Const(ref x, _) => {
                        constants.insert(handle, x.clone());
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    })?;

    Ok((
        asts.into_iter().map(|x| x.1).collect(),
        ConstraintSet::new(columns, constraints, constants, computations),
    ))
}
