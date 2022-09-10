use eyre::*;
use std::{cell::RefCell, rc::Rc};

use definitions::SymbolTable;

pub use generator::{Builtin, Columns, Constraint, ConstraintsSet, Expression};
pub use parser::{Ast, AstNode, Token};

use crate::{column::Column, expander::expand};

use self::definitions::Symbol;

mod common;
mod definitions;
mod generator;
mod parser;

pub fn make<S: AsRef<str>>(sources: &[(&str, S)]) -> Result<(Vec<Ast>, ConstraintsSet)> {
    let mut asts = vec![];
    let ctx = Rc::new(RefCell::new(SymbolTable::new_root()));

    for (name, content) in sources.iter() {
        let ast = parser::parse(content.as_ref()).with_context(|| eyre!("parsing `{}`", name))?;
        definitions::pass(&ast, ctx.clone())
            .with_context(|| eyre!("parsing definitions in `{}`", name))?;
        asts.push((name, ast));
    }

    let mut r = ConstraintsSet {
        constraints: asts
            .iter()
            .map(|(name, ast)| {
                generator::pass(ast, ctx.clone())
                    .with_context(|| eyre!("compiling constraints in `{}`", name))
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect(),
        columns: ctx
            .borrow()
            .symbols()
            .filter_map(|s| match s {
                Symbol::Alias(_) => None,
                Symbol::Final(symbol, used) => {
                    if !*used {
                        eprintln!("WARN unused: {:?}", symbol);
                        None
                    } else {
                        match symbol {
                            Expression::Column(name) => {
                                Some((name.to_owned(), Column::Atomic(vec![])))
                            }
                            Expression::ArrayColumn(name, range) => Some((
                                name.to_owned(),
                                Column::Array {
                                    range: range.clone(),
                                    content: Default::default(),
                                },
                            )),
                            _ => None,
                        }
                    }
                }
            })
            .collect::<Columns>(),
    };
    expand(&mut r)?;

    Ok((asts.into_iter().map(|x| x.1).collect(), r))
}