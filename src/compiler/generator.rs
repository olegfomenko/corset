use eyre::*;
use std::cell::RefCell;
use std::rc::Rc;

use super::common::*;
use crate::compiler::definitions::SymbolTable;
use crate::compiler::parser::*;
use std::fmt::{Debug, Formatter};

#[derive(Clone)]
pub enum Constraint {
    TopLevel {
        name: String,
        expr: Box<Constraint>,
    },
    Funcall {
        func: Builtin,
        args: Vec<Constraint>,
    },
    Const(i32),
    Column(String),
    ArrayColumn(String, Vec<usize>),
    ArrayColumnElement(String, usize),
    List(Vec<Constraint>),
}
impl Constraint {
    pub fn flat_fold<T>(&self, f: &dyn Fn(&Constraint) -> T) -> Vec<T> {
        let mut ax = vec![];
        match self {
            Constraint::List(xs) => {
                for x in xs {
                    ax.push(f(x));
                }
            }
            x => ax.push(f(x)),
        }
        ax
    }
}
impl Debug for Constraint {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        fn format_list(cs: &[Constraint]) -> String {
            cs.iter()
                .map(|c| format!("{:?}", c))
                .collect::<Vec<_>>()
                .join(" ")
        }

        match self {
            Constraint::TopLevel { name, expr } => write!(f, "{}: {:?}", name, expr),
            Constraint::Const(x) => write!(f, "{}:CONST", x),
            Constraint::Column(name) => write!(f, "{}:COLUMN", name),
            Constraint::ArrayColumn(name, range) => {
                write!(
                    f,
                    "{}[{}:{}]:ARRAYCOLUMN",
                    name,
                    range.first().unwrap(),
                    range.last().unwrap()
                )
            }
            Constraint::ArrayColumnElement(name, i) => {
                write!(f, "{}[{}]:COLUMN", name, i)
            }
            Constraint::List(cs) => write!(f, "'({})", format_list(cs)),
            Self::Funcall { func, args } => write!(f, "({:?} {})", func, format_list(args)),
        }
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Builtin {
    Add,
    Sub,
    Mul,
    IfZero,
    Shift,
    Neg,
    Inv,
    Nth,

    Begin,

    // Don't like it :/
    BranchIfZero,
    BranchIfZeroElse,
    BranchIfNotZero,
    BranchIfNotZeroElse,
    // BranchBinIfOne,
    // BranchBinIfZero,

    // BranchBinIfOneElse,
    // BranchBinIfZeroElse,
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub class: FunctionClass,
}
#[derive(Debug, Clone)]
pub enum FunctionClass {
    UserDefined(Defined),
    SpecialForm(Form),
    Builtin(Builtin),
    Alias(String),
}

#[derive(Debug, Clone)]
pub struct Defined {
    pub args: Vec<String>,
    pub body: AstNode,
}
impl FuncVerifier<Constraint> for Defined {
    fn arity(&self) -> Arity {
        Arity::Exactly(self.args.len())
    }

    fn validate_types(&self, _args: &[Constraint]) -> Result<()> {
        Ok(())
    }
}

impl FuncVerifier<Constraint> for Builtin {
    fn arity(&self) -> Arity {
        match self {
            Builtin::Add => Arity::AtLeast(2),
            Builtin::Sub => Arity::AtLeast(2),
            Builtin::Mul => Arity::AtLeast(2),
            Builtin::Neg => Arity::Monadic,
            Builtin::Inv => Arity::Monadic,
            Builtin::IfZero => Arity::Dyadic,
            Builtin::Shift => Arity::Dyadic,
            Builtin::Begin => Arity::AtLeast(1),
            Builtin::BranchIfZero => Arity::Dyadic,
            Builtin::BranchIfZeroElse => Arity::Exactly(3),
            Builtin::BranchIfNotZero => Arity::Dyadic,
            Builtin::BranchIfNotZeroElse => Arity::Exactly(3),
            Builtin::Nth => Arity::Dyadic,
        }
    }
    fn validate_types(&self, args: &[Constraint]) -> Result<()> {
        match self {
            f @ (Builtin::Add | Builtin::Sub | Builtin::Mul | Builtin::IfZero) => {
                if args.iter().all(|a| !matches!(a, Constraint::List(_))) {
                    Ok(())
                } else {
                    Err(eyre!(
                        "`{:?}` expects scalar arguments but received a list",
                        f,
                    ))
                }
            }
            Builtin::Neg | Builtin::Inv => {
                if args.iter().all(|a| !matches!(a, Constraint::List(_))) {
                    Ok(())
                } else {
                    Err(eyre!(
                        "`{:?}` expects a scalar argument but received a list",
                        self
                    ))
                }
            }
            Builtin::Shift => {
                if matches!(args[0], Constraint::Column(_))
                    && matches!(args[1], Constraint::Const(x) if x != 0)
                {
                    Ok(())
                } else {
                    Err(eyre!(
                        "`{:?}` expects a COLUMN and a non-null INTEGER but received {:?}",
                        self,
                        args
                    ))
                }
            }
            Builtin::Nth => {
                if matches!(args[0], Constraint::ArrayColumn(..))
                    && matches!(args[1], Constraint::Const(x) if x >= 0)
                {
                    Ok(())
                } else {
                    Err(eyre!(
                        "`{:?}` expects [SYMBOL CONST] but received {:?}",
                        self,
                        args
                    ))
                }
            }
            Builtin::BranchIfZero | Builtin::BranchIfNotZero => {
                if !matches!(args[0], Constraint::List(_)) && matches!(args[1], Constraint::List(_))
                {
                    Ok(())
                } else {
                    Err(eyre!(
                        "`{:?}` expects scalar arguments but received a list",
                        self
                    ))
                }
            }
            Builtin::BranchIfZeroElse | Builtin::BranchIfNotZeroElse => {
                if !matches!(args[0], Constraint::List(_))
                    && matches!(args[1], Constraint::List(_))
                    && matches!(args[2], Constraint::List(_))
                {
                    Ok(())
                } else {
                    Err(eyre!(
                        "`{:?}` expects (SCALAR, LIST, LIST) but received {:?}",
                        self,
                        args
                    ))
                }
            }
            Builtin::Begin => Ok(()),
        }
    }
}

#[derive(Debug)]
pub struct ConstraintsSet {
    pub constraints: Vec<Constraint>,
}

// Compared to a function, a form do not evaluate all of its arguments by default
fn apply_form(
    f: Form,
    args: &[AstNode],
    ctx: Rc<RefCell<SymbolTable>>,
) -> Result<Option<Constraint>> {
    let args = f
        .validate_args(args.to_vec())
        .with_context(|| eyre!("evaluating call to {:?}", f))?;

    match f {
        // TODO in compilation
        Form::For => {
            if let (Token::Symbol(i_name), Token::Range(is), body) =
                (&args[0].class, &args[1].class, &args[2])
            {
                let mut l = vec![];
                for i in is {
                    let new_ctx = SymbolTable::derived(ctx.clone());
                    new_ctx
                        .borrow_mut()
                        .insert_symbol(i_name, Constraint::Const(*i as i32))?;

                    let r = reduce(&body.clone(), new_ctx)?.unwrap();
                    l.push(r);
                }

                Ok(Some(Constraint::List(l)))
            } else {
                unreachable!()
            }
        }
    }
}

fn apply(
    f: &Function,
    args: &[AstNode],
    ctx: Rc<RefCell<SymbolTable>>,
) -> Result<Option<Constraint>> {
    if let FunctionClass::SpecialForm(sf) = f.class {
        apply_form(sf, args, ctx)
    } else {
        let mut traversed_args: Vec<Constraint> = vec![];
        for arg in args.iter() {
            let traversed = reduce(arg, ctx.clone())?;
            if let Some(traversed) = traversed {
                traversed_args.push(traversed);
            }
        }

        match &f.class {
            FunctionClass::Builtin(b) => {
                let traversed_args = b
                    .validate_args(traversed_args)
                    .with_context(|| eyre!("validating call to `{}`", f.name))?;
                match b {
                    Builtin::Begin => Ok(Some(Constraint::List(traversed_args))),
                    Builtin::BranchIfZero => {
                        let cond = traversed_args[0].clone();
                        if let Constraint::List(then) = &traversed_args[1] {
                            Ok(Some(Constraint::List(
                                then.iter()
                                    .map(|a| Constraint::Funcall {
                                        func: Builtin::IfZero,
                                        args: vec![cond.clone(), a.clone()],
                                    })
                                    .collect(),
                            )))
                        } else {
                            unreachable!()
                        }
                    }
                    Builtin::BranchIfZeroElse => {
                        let cond = traversed_args[0].clone();
                        if let (Constraint::List(tthen), Constraint::List(eelse)) =
                            (&traversed_args[1], &traversed_args[2])
                        {
                            Ok(Some(Constraint::List(
                                tthen
                                    .iter()
                                    .cloned()
                                    .flat_map(|c: Constraint| {
                                        c.flat_fold(&|x| Constraint::Funcall {
                                            func: Builtin::IfZero,
                                            args: vec![cond.clone(), x.clone()],
                                        })
                                    })
                                    .into_iter()
                                    .chain(eelse.iter().cloned().flat_map(|c: Constraint| {
                                        c.flat_fold(&|x| Constraint::Funcall {
                                            func: Builtin::Mul,
                                            args: vec![cond.clone(), x.clone()],
                                        })
                                    }))
                                    .collect(),
                            )))
                        } else {
                            unreachable!()
                        }
                    }
                    Builtin::BranchIfNotZero => {
                        let cond = traversed_args[0].clone();
                        if let Constraint::List(then) = &traversed_args[1] {
                            Ok(Some(Constraint::List(
                                then.iter()
                                    .map(|a| Constraint::Funcall {
                                        func: Builtin::Mul,
                                        args: vec![cond.clone(), a.clone()],
                                    })
                                    .collect(),
                            )))
                        } else {
                            unreachable!()
                        }
                    }
                    Builtin::BranchIfNotZeroElse => {
                        let cond = traversed_args[0].clone();
                        if let (Constraint::List(tthen), Constraint::List(eelse)) =
                            (&traversed_args[1], &traversed_args[2])
                        {
                            Ok(Some(Constraint::List(
                                tthen
                                    .iter()
                                    .cloned()
                                    .flat_map(|c: Constraint| {
                                        c.flat_fold(&|x| Constraint::Funcall {
                                            func: Builtin::Mul,
                                            args: vec![cond.clone(), x.clone()],
                                        })
                                    })
                                    .into_iter()
                                    .chain(eelse.iter().cloned().flat_map(|c: Constraint| {
                                        c.flat_fold(&|x| Constraint::Funcall {
                                            func: Builtin::IfZero,
                                            args: vec![cond.clone(), x.clone()],
                                        })
                                    }))
                                    .collect(),
                            )))
                        } else {
                            unreachable!()
                        }
                    }
                    Builtin::Nth => {
                        if let (Constraint::ArrayColumn(cname, ..), Constraint::Const(x)) =
                            (&traversed_args[0], &traversed_args[1])
                        {
                            let x = *x as usize;
                            match &ctx.borrow().resolve_symbol(cname)? {
                                array @ Constraint::ArrayColumn(name, range) => {
                                    if range.contains(&x) {
                                        Ok(Some(Constraint::ArrayColumnElement(name.to_owned(), x)))
                                    } else {
                                        Err(eyre!("tried to access `{:?}` at index {}", array, x))
                                    }
                                }
                                _ => unimplemented!(),
                            }
                        } else {
                            unreachable!()
                        }
                    }

                    b => Ok(Some(Constraint::Funcall {
                        func: *b,
                        args: traversed_args,
                    })),
                }
            }

            FunctionClass::UserDefined(b @ Defined { args: f_args, body }) => {
                let traversed_args = b
                    .validate_args(traversed_args)
                    .with_context(|| eyre!("validating call to `{}`", f.name))?;
                let new_ctx = SymbolTable::derived(ctx);
                for (i, f_arg) in f_args.iter().enumerate() {
                    new_ctx
                        .borrow_mut()
                        .insert_symbol(f_arg, traversed_args[i].clone())?;
                }

                reduce(body, new_ctx)
            }
            _ => unimplemented!("{:?}", f),
        }
    }
}

fn reduce(e: &AstNode, ctx: Rc<RefCell<SymbolTable>>) -> Result<Option<Constraint>> {
    match &e.class {
        Token::Ignore => Ok(None),
        Token::Value(x) => Ok(Some(Constraint::Const(*x))),
        Token::Symbol(name) => Ok(Some(ctx.borrow_mut().resolve_symbol(name)?)),
        Token::Form(args) => {
            if args.is_empty() {
                Ok(Some(Constraint::List(vec![])))
            } else if let Token::Symbol(verb) = &args[0].class {
                let func = ctx
                    .borrow()
                    .resolve_function(verb)
                    .with_context(|| eyre!("resolving function `{}`", verb))?;

                apply(&func, &args[1..], ctx)
            } else {
                Err(eyre!("Not a function: {:?}", args[0]))
            }
        }
        Token::DefConstraint(name, expr) => Ok(Some(Constraint::TopLevel {
            name: name.into(),
            expr: Box::new(reduce(expr, ctx)?.unwrap()), // the parser ensures that the body is never empty
        })),

        Token::Range(_) => Ok(None),
        Token::DefColumns(_) => Ok(None),
        Token::DefColumn(_) => Ok(None),
        Token::DefArrayColumn(..) => Ok(None),
        Token::DefAliases(_) => Ok(None),
        Token::DefAlias(..) => Ok(None),
        Token::DefunAlias(..) => Ok(None),
        Token::DefConst(..) => Ok(None),
        Token::Defun(..) => Ok(None),
    }
    .with_context(|| format!("at line {}, col.{}: \"{}\"", e.lc.0, e.lc.1, e.src))
}

pub fn pass(ast: &Ast, ctx: Rc<RefCell<SymbolTable>>) -> Result<Vec<Constraint>> {
    let mut r = vec![];

    for exp in ast.exprs.iter().cloned() {
        if let Some(c) = reduce(&exp, ctx.clone())
            .with_context(|| format!("at line {}, col.{}: \"{}\"", exp.lc.0, exp.lc.1, exp.src))?
        {
            r.push(c)
        }
    }
    Ok(r)
}
