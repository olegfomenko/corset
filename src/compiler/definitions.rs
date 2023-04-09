use anyhow::*;
use log::*;
use num_bigint::BigInt;
use num_traits::FromPrimitive;
use std::cell::RefCell;
use std::rc::Rc;

use super::generator::{Defined, Function, FunctionClass};
use super::tables::SymbolTable;
use super::{Expression, Magma, Node, Type};
use crate::column::Computation;
use crate::compiler::parser::*;
use crate::pretty::Base;
use crate::structs::Handle;

fn reduce(
    e: &AstNode,
    root_ctx: Rc<RefCell<SymbolTable>>,
    ctx: &mut Rc<RefCell<SymbolTable>>,
) -> Result<()> {
    match &e.class {
        Token::Value(_)
        | Token::Symbol(_)
        | Token::Keyword(_)
        | Token::List(_)
        | Token::Range(_)
        | Token::Type(_)
        | Token::DefPlookup { .. }
        | Token::DefInrange(..) => Ok(()),

        Token::DefConstraint { name, .. } => ctx.borrow_mut().insert_constraint(name),
        Token::DefModule(name) => {
            *ctx = SymbolTable::derived(root_ctx, name, name, false, true);
            Ok(())
        }
        Token::DefColumns(cols) => cols
            .iter()
            .fold(Ok(()), |ax, col| ax.and(reduce(col, root_ctx.clone(), ctx))),
        Token::DefColumn {
            name: col,
            t,
            kind,
            padding_value,
            base,
        } => {
            let module_name = ctx.borrow().module.to_owned();
            let symbol = Node {
                _e: Expression::Column {
                    handle: Handle::new(module_name, col),
                    kind: match kind {
                        Kind::Atomic => Kind::Atomic,
                        Kind::Phantom => Kind::Phantom,
                        Kind::Composite(_) => Kind::Phantom, // The actual expression is computed by the generator
                        Kind::Interleaved(_, _) => Kind::Phantom, // The interleaving is later on set by the generator
                    },
                    padding_value: padding_value.to_owned(),
                    base: *base,
                },
                _t: Some(*t),
            };
            ctx.borrow_mut().insert_symbol(col, symbol)
        }
        Token::DefArrayColumn {
            name: col,
            domain: range,
            t,
            base,
        } => {
            let handle = Handle::new(&ctx.borrow().module, col);
            ctx.borrow_mut().insert_symbol(
                col,
                Node {
                    _e: Expression::ArrayColumn {
                        handle,
                        domain: range.to_owned(),
                        base: *base,
                    },
                    _t: Some(*t),
                },
            )?;
            Ok(())
        }
        Token::DefConsts(cs) => {
            // The actual value will be filled later on by the compile-time pass
            for c in cs.iter() {
                ctx.borrow_mut()
                    .insert_constant(&c.0, BigInt::from_i8(0).unwrap(), false)?;
            }
            Ok(())
        }
        Token::DefPermutation {
            from: froms,
            to: tos,
            signs,
        } => {
            if tos.len() != froms.len() {
                bail!(
                    "cardinality mismatch in permutation declaration: {:?} vs. {:?}",
                    tos,
                    froms
                );
            }

            let mut _froms = Vec::new();
            let mut _tos = Vec::new();
            for (to, from) in tos.iter().zip(froms.iter()) {
                let to_handle = Handle::new(&ctx.borrow().module, to);
                let from_actual_handle = if let Expression::Column { handle, .. } = ctx
                    .borrow_mut()
                    .resolve_symbol(from)
                    .with_context(|| "while defining permutation")?
                    .e()
                {
                    handle.to_owned()
                } else {
                    unreachable!()
                };
                ctx.borrow_mut()
                    .insert_symbol(
                        to,
                        Node {
                            _e: Expression::Column {
                                handle: to_handle.clone(),
                                kind: Kind::Phantom,
                                padding_value: None,
                                base: Base::Hex,
                            },
                            _t: Some(Type::Column(Magma::Integer)),
                        },
                    )
                    .unwrap_or_else(|e| warn!("while defining permutation: {}", e));
                _froms.push(from_actual_handle);
                _tos.push(to_handle);
            }

            ctx.borrow_mut()
                .computation_table
                .borrow_mut()
                .insert_many(
                    &_tos,
                    Computation::Sorted {
                        froms: _froms,
                        tos: _tos.clone(),
                        signs: signs.clone(),
                    },
                )?;
            Ok(())
        }
        Token::DefAliases(aliases) => aliases.iter().fold(Ok(()), |ax, alias| {
            ax.and(reduce(alias, root_ctx.clone(), ctx))
        }),
        Token::Defun { name, args, body } => {
            let module_name = ctx.borrow().module.to_owned();
            ctx.borrow_mut().insert_function(
                name,
                Function {
                    handle: Handle::new(module_name, name),
                    class: FunctionClass::UserDefined(Defined {
                        pure: false,
                        args: args.to_owned(),
                        body: *body.clone(),
                    }),
                },
            )
        }
        Token::Defpurefun { name, args, body } => {
            let module_name = ctx.borrow().module.to_owned();
            ctx.borrow_mut().insert_function(
                name,
                Function {
                    handle: Handle::new(module_name, name),
                    class: FunctionClass::UserDefined(Defined {
                        pure: true,
                        args: args.to_owned(),
                        body: *body.clone(),
                    }),
                },
            )
        }
        Token::DefAlias(from, to) => {
            let _ = ctx
                .borrow_mut()
                .resolve_symbol(to)
                .with_context(|| anyhow!("while defining alias `{}`", from))?;

            ctx.borrow_mut()
                .insert_alias(from, to)
                .with_context(|| anyhow!("defining {} -> {}", from, to))
        }
        Token::DefunAlias(from, to) => ctx
            .borrow_mut()
            .insert_funalias(from, to)
            .with_context(|| anyhow!("defining {} -> {}", from, to)),
    }
}

pub fn pass(ast: &Ast, ctx: Rc<RefCell<SymbolTable>>) -> Result<()> {
    let mut current_ctx = ctx.clone();
    for e in ast.exprs.iter() {
        reduce(e, ctx.clone(), &mut current_ctx)?;
    }

    Ok(())
}
