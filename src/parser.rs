use color_eyre::eyre::*;
use pest::{iterators::Pair, Parser};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fmt::Debug;
use std::io::BufWriter;
use std::rc::Rc;

#[derive(Parser)]
#[grammar = "corset.pest"]
struct CorsetParser;

lazy_static::lazy_static! {
    static ref BUILTINS: HashMap<&'static str, Function> = maplit::hashmap!{
        "defun" => Function {
            name: "defun".into(),
            class: FunctionClass::SpecialForm(Form::Defun),
        },

        "defunalias" => Function {
            name: "defunalias".into(),
            class: FunctionClass::SpecialForm(Form::Defunalias),
        },

        "defalias" => Function {
            name: "defalias".into(),
            class: FunctionClass::SpecialForm(Form::Defalias),
        },

        "defcolumns" => Function {
            name: "defcolumns".into(),
            class: FunctionClass::SpecialForm(Form::Defcolumns),
        },

        "defconst" => Function {
            name: "defconst".into(),
            class: FunctionClass::SpecialForm(Form::Defconst),
        },

        "ith" => Function {
            name: "ith".into(),
            class: FunctionClass::Builtin(Builtin::Ith),
        },


        // monadic
        "inv" => Function{
            name: "inv".into(),
            class: FunctionClass::Builtin(Builtin::Inv)
        },
        "neg" => Function{
            name: "neg".into(),
            class: FunctionClass::Builtin(Builtin::Neg)
        },

        // Dyadic
        "if-zero" => Function{
            name: "if-zero".into(),
            class: FunctionClass::Builtin(Builtin::IfZero),
        },

        "shift" => Function{
            name: "shift".into(),
            class: FunctionClass::Builtin(Builtin::Shift),
        },


        // polyadic
        "add" => Function{
            name: "add".into(),
            class: FunctionClass::Builtin(Builtin::Add)
        },
        "+" => Function {
            name: "+".into(),
            class: FunctionClass::Alias("add".into())
        },


        "mul" => Function{
            name: "mul".into(),
            class: FunctionClass::Builtin(Builtin::Mul)
        },
        "*" => Function {
            name: "*".into(),
            class: FunctionClass::Alias("mul".into())
        },
        "and" => Function {
            name: "and".into(),
            class: FunctionClass::Alias("mul".into())
        },

        "sub" => Function{
            name: "mul".into(),
            class: FunctionClass::Builtin(Builtin::Sub,)
        },
        "-" => Function {
            name: "sub".into(),
            class: FunctionClass::Alias("sub".into())
        },
        "eq" => Function {
            name: "sub".into(),
            class: FunctionClass::Alias("sub".into())
        },
        "=" => Function {
            name: "sub".into(),
            class: FunctionClass::Alias("sub".into())
        },

        "begin" => Function{name: "begin".into(), class: FunctionClass::Builtin(Builtin::Begin)},

        // Special form for now, see later if implementing map...
        "branch-if-zero" => Function {
            name:"branch-if-zero".into(),
            class: FunctionClass::Builtin(Builtin::BranchIfZero)
        },

        "branch-if-zero-else" => Function {
            name:"branch-if-zero-else".into(),
            class: FunctionClass::Builtin(Builtin::BranchIfZeroElse)
        },
    };
}

pub(crate) trait Transpiler {
    fn render<'a>(
        &self,
        cs: &ConstraintsSet,
        out: BufWriter<Box<dyn std::io::Write + 'a>>,
    ) -> Result<()>;
}

#[derive(Clone)]
pub enum Constraint {
    Funcall {
        func: Builtin,
        args: Vec<Constraint>,
    },
    Const(i32),
    Column(String),
    List(Vec<Constraint>),
}
impl Debug for Constraint {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fn format_list(cs: &[Constraint]) -> String {
            cs.iter()
                .map(|c| format!("{:?}", c))
                .collect::<Vec<_>>()
                .join(" ")
        }

        match self {
            Constraint::Const(x) => write!(f, "{}:CONST", x),
            Constraint::Column(name) => write!(f, "{}:COLUMN", name),
            Constraint::List(cs) => write!(f, "'({})", format_list(cs)),
            Self::Funcall { func, args } => write!(f, "({:?} {})", func, format_list(args)),
        }
    }
}

#[derive(Debug)]
pub struct ConstraintsSet {
    pub constraints: Vec<Constraint>,
}
impl ConstraintsSet {
    pub fn from_sources<S: AsRef<str>>(sources: &[(&str, S)]) -> Result<Self> {
        Compiler::compile(
            &sources
                .iter()
                .map(|(n, s)| (*n, s.as_ref()))
                .collect::<Vec<_>>(),
        )
    }
}

#[derive(Debug)]
struct ParsingAst {
    exprs: Vec<AstNode>,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Form {
    Defun,
    Defalias,
    Defunalias,
    Defcolumns,
    Defconst,
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

    Begin,
    Ith,

    // Don't like it :/
    BranchIfZero,
    BranchIfZeroElse,
    // BranchIfNotZero,
    // BranchIfNotZeroElse,

    // BranchBinIfOne,
    // BranchBinIfZero,

    // BranchBinIfOneElse,
    // BranchBinIfZeroElse,
}

#[derive(Debug, PartialEq, Clone)]
struct Verb {
    name: String,
}

#[derive(PartialEq, Clone)]
struct AstNode {
    class: Token,
    src: String,
    lc: (usize, usize),
}
#[derive(Debug, PartialEq, Clone)]
enum Token {
    Ignore,
    Value(i32),
    Symbol(String),
    List { args: Vec<AstNode> },
    TopLevelForm { args: Vec<AstNode> },
}
impl Debug for AstNode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fn format_list(cs: &[AstNode]) -> String {
            cs.iter()
                .map(|c| format!("{:?}", c))
                .collect::<Vec<_>>()
                .join(" ")
        }

        match self.class {
            Token::Ignore => write!(f, "IGNORED VALUE"),
            Token::Value(x) => write!(f, "{}:IMMEDIATE VALUE", x),
            Token::Symbol(ref name) => write!(f, "{}:SYMBOL", name),
            Token::List { ref args } => write!(f, "'({})", format_list(&args)),
            Token::TopLevelForm { ref args } => write!(f, "{}", format_list(&args)),
        }
    }
}

fn build_ast_from_expr(pair: Pair<Rule>, in_def: bool) -> Result<AstNode> {
    let lc = pair.as_span().start_pos().line_col();
    let src = pair.as_str().to_owned();

    match pair.as_rule() {
        Rule::expr | Rule::constraint => {
            build_ast_from_expr(pair.into_inner().next().unwrap(), in_def)
        }
        Rule::definition => {
            let mut inner = pair.into_inner();
            let args = vec![inner.next().unwrap()]
                .into_iter()
                .chain(inner)
                .map(|p| build_ast_from_expr(p, in_def))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .filter(|x| x.class != Token::Ignore)
                .collect::<Vec<_>>();
            Ok(AstNode {
                class: Token::TopLevelForm { args },
                lc,
                src,
            })
        }
        Rule::list => {
            let args = pair
                .into_inner()
                .map(|p| build_ast_from_expr(p, in_def))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .filter(|x| x.class != Token::Ignore)
                .collect::<Vec<_>>();
            Ok(AstNode {
                class: Token::List { args },
                lc,
                src,
            })
        }
        Rule::symbol | Rule::defform => Ok(AstNode {
            class: Token::Symbol(pair.as_str().to_owned()),
            lc,
            src,
        }),
        Rule::integer => Ok(AstNode {
            class: Token::Value(pair.as_str().parse().unwrap()),
            lc,
            src,
        }),
        x @ _ => unimplemented!("{:?}", x),
    }
}

fn parse(source: &str) -> Result<ParsingAst> {
    let mut ast = ParsingAst { exprs: vec![] };

    for pair in CorsetParser::parse(Rule::corset, source)? {
        match &pair.as_rule() {
            Rule::corset => {
                for constraint in pair.into_inner() {
                    if constraint.as_rule() != Rule::EOI {
                        ast.exprs.push(build_ast_from_expr(constraint, false)?);
                    }
                }
            }
            _ => {}
        }
    }

    Ok(ast)
}

#[derive(Debug)]
enum Symbol {
    Alias(String),
    Final(Constraint),
}
#[derive(Debug)]
struct SymbolTable {
    local_context: HashMap<String, Constraint>,
    funcs: HashMap<String, Function>,
    symbols: HashMap<String, Symbol>,
    parent: Option<Rc<RefCell<SymbolTable>>>,
}
impl SymbolTable {
    pub fn new_root() -> SymbolTable {
        SymbolTable {
            local_context: HashMap::new(),
            funcs: BUILTINS
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            symbols: HashMap::new(),
            parent: None,
        }
    }

    pub fn new_derived(
        parent: Rc<RefCell<SymbolTable>>,
        local_context: HashMap<String, Constraint>,
    ) -> SymbolTable {
        SymbolTable {
            local_context,
            funcs: HashMap::new(),
            symbols: HashMap::new(),
            parent: Some(parent),
        }
    }

    fn _resolve_symbol(&self, name: &str, ax: &mut HashSet<String>) -> Result<Constraint> {
        if ax.contains(name) {
            Err(eyre!("Circular definitions found for {}", name))
        } else {
            ax.insert(name.into());
            match self.symbols.get(name) {
                Some(Symbol::Alias(name)) => self._resolve_symbol(name, ax),
                Some(Symbol::Final(name)) => Ok(name.clone()),
                None => self
                    .parent
                    .as_ref()
                    .map_or(Err(eyre!("Column `{}` unknown", name)), |parent| {
                        parent.borrow().resolve_symbol(name)
                    }),
            }
        }
    }

    fn _resolve_function(&self, name: &str, ax: &mut HashSet<String>) -> Result<Function> {
        if ax.contains(name) {
            Err(eyre!("Circular definitions found for {}", name))
        } else {
            ax.insert(name.into());
            match self.funcs.get(name) {
                Some(Function {
                    class: FunctionClass::Alias(ref name),
                    ..
                }) => self._resolve_function(name, ax),
                Some(f) => Ok(f.to_owned()),
                None => self
                    .parent
                    .as_ref()
                    .map_or(Err(eyre!("Function `{}` unknown", name)), |parent| {
                        parent.borrow().resolve_function(name)
                    }),
            }
        }
    }

    fn insert_symbol(&mut self, symbol: &str) -> Result<()> {
        if self.symbols.contains_key(symbol) {
            Err(anyhow!("column `{}` already exists", symbol))
        } else {
            self.symbols.insert(
                symbol.into(),
                Symbol::Final(Constraint::Column(symbol.to_string())),
            );
            Ok(())
        }
    }

    fn insert_func(&mut self, f: Function) -> Result<()> {
        if self.funcs.contains_key(&f.name) {
            Err(anyhow!("function `{}` already defined", &f.name))
        } else {
            self.funcs.insert(f.name.clone(), f);
            Ok(())
        }
    }

    fn insert_alias(&mut self, from: &str, to: &str) -> Result<()> {
        if self.symbols.contains_key(from) {
            Err(anyhow!("`{}` already exists", from))
        } else {
            self.symbols.insert(from.into(), Symbol::Alias(to.into()));
            Ok(())
        }
    }

    fn insert_funalias(&mut self, from: &str, to: &str) -> Result<()> {
        if self.symbols.contains_key(from) {
            Err(anyhow!(
                "`{}` already exists: {} -> {:?}",
                from,
                from,
                self.symbols[from]
            ))
        } else {
            self.funcs.insert(
                from.into(),
                Function {
                    name: from.into(),
                    class: FunctionClass::Alias(to.into()),
                },
            );
            Ok(())
        }
    }

    fn resolve_symbol(&self, name: &str) -> Result<Constraint> {
        self.local_context
            .get(name)
            .map(|x| x.to_owned())
            .ok_or(eyre!("SHOULD NEVER HAPPEN"))
            .or(self._resolve_symbol(name, &mut HashSet::new()))
    }

    fn resolve_function(&self, name: &str) -> Result<Function> {
        self._resolve_function(name, &mut HashSet::new())
    }

    fn insert_constant(&mut self, name: &str, value: i32) -> Result<()> {
        if self.symbols.contains_key(name) {
            Err(anyhow!("`{}` already exists", name))
        } else {
            self.symbols
                .insert(name.into(), Symbol::Final(Constraint::Const(value)));
            Ok(())
        }
    }
}

trait FuncVerifier<T> {
    fn arity(&self) -> Arity;

    fn validate_types(&self, args: &[T]) -> Result<()>;

    fn validate_arity(&self, args: &[T]) -> Result<()> {
        self.arity().validate(args.len())
    }

    fn validate_args(&self, args: Vec<T>) -> Result<Vec<T>> {
        self.validate_arity(&args)
            .and_then(|_| self.validate_types(&args))
            .and(Ok(args))
    }
}
impl FuncVerifier<AstNode> for Form {
    fn arity(&self) -> Arity {
        match self {
            Form::Defun => Arity::Exactly(2),
            Form::Defalias => Arity::Even,
            Form::Defunalias => Arity::Exactly(2),
            Form::Defcolumns => Arity::AtLeast(1),
            Form::Defconst => Arity::Even,
        }
    }
    fn validate_types(&self, args: &[AstNode]) -> Result<()> {
        match self {
            Form::Defcolumns => {
                if args.iter().all(|a| matches!(a.class, Token::Symbol(_))) {
                    Ok(())
                } else {
                    Err(eyre!("DEFCOLUMNS expects only symbols"))
                }
            }
            Form::Defun => {
                if matches!(args[0].class, Token::List { .. }) {
                    Ok(())
                } else {
                    Err(eyre!("invalid DEFUN syntax; received: {:?}", args))
                }
            }
            Form::Defalias | Form::Defunalias => {
                if args.iter().all(|a| matches!(a.class, Token::Symbol(_))) {
                    Ok(())
                } else {
                    Err(eyre!("DEFCOLUMNS expects only symbols"))
                }
            }
            Form::Defconst => {
                if args.chunks(2).all(|p| {
                    matches!(p[0].class, Token::Symbol(_)) && matches!(p[1].class, Token::Value(_))
                }) {
                    Ok(())
                } else {
                    Err(eyre!("DEFCONST expects alternating symbols and values"))
                }
            }
        }
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
            Builtin::Ith => Arity::Dyadic,
            Builtin::Begin => Arity::AtLeast(1),
            Builtin::BranchIfZero => Arity::Dyadic,
            Builtin::BranchIfZeroElse => Arity::Exactly(3),
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
            Builtin::BranchIfZero => {
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
            Builtin::BranchIfZeroElse => {
                if !matches!(args[0], Constraint::List(_))
                    && matches!(args[1], Constraint::List(_))
                    && matches!(args[2], Constraint::List(_))
                {
                    Ok(())
                } else {
                    Err(eyre!(
                        "`{:?}` expects scalar arguments but received a list",
                        self
                    ))
                }
            }
            Builtin::Ith => {
                if matches!(args[0], Constraint::Column(_))
                    && matches!(args[1], Constraint::Const(_))
                {
                    Ok(())
                } else {
                    Err(eyre!(
                        "`{:?}` expects (COLUMN, CONST) but received {:?}",
                        self,
                        args
                    ))
                }
            }
            _ => todo!(),
        }
    }
}

impl FuncVerifier<Constraint> for Defined {
    fn arity(&self) -> Arity {
        Arity::Exactly(self.args.len())
    }

    fn validate_types(&self, _args: &[Constraint]) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct Function {
    name: String,
    class: FunctionClass,
}

enum Arity {
    AtLeast(usize),
    AtMost(usize),
    Even,
    Odd,
    Monadic,
    Dyadic,
    Exactly(usize),
}
impl Arity {
    fn make_error(&self, l: usize) -> String {
        fn arg_count(x: usize) -> String {
            format!("{} argument{}", x, if x > 1 { "s" } else { "" })
        }
        match self {
            Arity::AtLeast(x) => format!("expected at least {}, but received {}", arg_count(*x), l),
            Arity::AtMost(x) => format!("expected at most {}, but received {}", arg_count(*x), l),
            Arity::Even => format!("expected an even numer of arguments, but received {}", l),
            Arity::Odd => format!("expected an odd numer of arguments, but received {}", l),
            Arity::Monadic => format!("expected {}, but received {}", arg_count(1), l),
            Arity::Dyadic => format!("expected {}, but received {}", arg_count(2), l),
            Arity::Exactly(x) => format!("expected {}, but received {}", arg_count(*x), l),
        }
    }

    fn validate(&self, l: usize) -> Result<()> {
        match self {
            Arity::AtLeast(x) => l >= *x,
            Arity::AtMost(x) => l <= *x,
            Arity::Even => l % 2 == 0,
            Arity::Odd => l % 2 == 1,
            Arity::Monadic => l == 1,
            Arity::Dyadic => l == 2,
            Arity::Exactly(x) => l == *x,
        }
        .then(|| ())
        .ok_or_else(|| eyre!(self.make_error(l)))
    }
}

#[derive(Debug, Clone)]
struct Defined {
    args: Vec<String>,
    body: AstNode,
}

#[derive(Debug, Clone)]
enum FunctionClass {
    UserDefined(Defined),
    SpecialForm(Form),
    Builtin(Builtin),
    Alias(String),
}

struct Compiler {}

#[derive(Debug, Clone, Copy)]
enum Pass {
    Definition,
    Compilation,
}
impl Compiler {
    fn apply_form<'a>(
        &self,
        f: Form,
        args: &[AstNode],
        ctx: Rc<RefCell<SymbolTable>>,
        pass: Pass,
    ) -> Result<Option<Constraint>> {
        let args = f
            .validate_args(args.to_vec())
            .with_context(|| eyre!("evaluating call to {:?}", f))?;

        match (f, pass) {
            (Form::Defcolumns, Pass::Definition) => {
                for arg in args.iter() {
                    if let Token::Symbol(name) = &arg.class {
                        ctx.borrow_mut().insert_symbol(name)?
                    }
                }

                Ok(None)
            }
            (Form::Defconst, Pass::Definition) => {
                for p in args.chunks(2) {
                    if let (Token::Symbol(name), Token::Value(x)) = (&p[0].class, &p[1].class) {
                        ctx.borrow_mut().insert_constant(name, *x)?
                    }
                }

                Ok(None)
            }
            (Form::Defalias, Pass::Definition) => {
                for p in args.chunks(2) {
                    if let (Token::Symbol(from), Token::Symbol(to)) = (&p[0].class, &p[1].class) {
                        ctx.borrow_mut().insert_alias(from, to)?
                    }
                }

                Ok(None)
            }
            (Form::Defunalias, Pass::Definition) => {
                for p in args.chunks(2) {
                    if let (Token::Symbol(from), Token::Symbol(to)) = (&p[0].class, &p[1].class) {
                        ctx.borrow_mut().insert_funalias(from, to)?
                    }
                }

                Ok(None)
            }
            (Form::Defun, Pass::Definition) => {
                let header = &args[0];
                let body = &args[1];

                if let Token::List { ref args } = header.class {
                    if let Token::Symbol(fname) = &args[0].class {
                        let arg_names = args
                            .iter()
                            .map(|a| {
                                if let Token::Symbol(ref n) = a.class {
                                    Ok(n.to_owned())
                                } else {
                                    Err(eyre!("{:?} is not a valid argument", a))
                                }
                            })
                            .collect::<Result<Vec<_>>>()
                            .with_context(|| format!("parsing function {}", fname))?;

                        ctx.borrow_mut().insert_func({
                            Function {
                                name: arg_names[0].to_owned(),
                                class: FunctionClass::UserDefined(Defined {
                                    args: arg_names[1..].to_vec(),
                                    body: body.to_owned(),
                                }),
                            }
                        })
                    } else {
                        unreachable!()
                    }
                } else {
                    unreachable!()
                }?;
                Ok(None)
            }
            _ => {
                bail!("{:?}/{:?}: not yet implemented", f, pass)
            }
        }
    }

    fn apply<'a>(
        &self,
        f: &Function,
        args: &[AstNode],
        ctx: Rc<RefCell<SymbolTable>>,
        pass: Pass,
    ) -> Result<Option<Constraint>> {
        if let FunctionClass::SpecialForm(sf) = f.class {
            self.apply_form(sf, args, ctx, pass)
        } else if matches!(pass, Pass::Compilation) {
            let mut traversed_args: Vec<Constraint> = vec![];
            for arg in args.iter() {
                let traversed = self.reduce(arg, ctx.clone(), pass)?;
                if let Some(traversed) = traversed {
                    traversed_args.push(traversed);
                }
            }

            match &f.class {
                FunctionClass::Builtin(Builtin::Begin) => {
                    Ok(Some(Constraint::List(traversed_args)))
                }
                FunctionClass::Builtin(Builtin::BranchIfZero) => {
                    let cond = traversed_args[0].clone();
                    if let Constraint::List(then) = &traversed_args[1] {
                        Ok(Some(Constraint::List(
                            then.into_iter()
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
                FunctionClass::Builtin(Builtin::BranchIfZeroElse) => {
                    let cond = traversed_args[0].clone();
                    if let Constraint::List(tthen) = &traversed_args[1] {
                        if let Constraint::List(eelse) = &traversed_args[2] {
                            Ok(Some(Constraint::List(
                                tthen
                                    .into_iter()
                                    .map(|a| Constraint::Funcall {
                                        func: Builtin::IfZero,
                                        args: vec![cond.clone(), a.clone()],
                                    })
                                    .chain(eelse.iter().map(|a| Constraint::Funcall {
                                        func: Builtin::Mul,
                                        args: vec![cond.clone(), a.clone()],
                                    }))
                                    .collect(),
                            )))
                        } else {
                            unreachable!()
                        }
                    } else {
                        unreachable!()
                    }
                }
                FunctionClass::Builtin(b @ builtin) => match b {
                    Builtin::Ith => {
                        if let (Constraint::Column(c), Constraint::Const(x)) =
                            (&traversed_args[0], &traversed_args[1])
                        {
                            let ith = format!("{}_{}", c, x);
                            ctx.borrow()
                                .resolve_symbol(&ith)
                                .and_then(|_| Ok(Some(Constraint::Column(ith))))
                                .with_context(|| eyre!("evaluating ith {:?}", traversed_args))
                        } else {
                            unreachable!()
                        }
                    }
                    _ => Ok(Some(Constraint::Funcall {
                        func: *builtin,
                        args: b
                            .validate_args(traversed_args)
                            .with_context(|| eyre!("validating call to `{}`", f.name))?,
                    })),
                },

                FunctionClass::UserDefined(b @ Defined { args: f_args, body }) => {
                    let traversed_args = b
                        .validate_args(traversed_args)
                        .with_context(|| eyre!("validating call to `{}`", f.name))?;
                    self.reduce(
                        &body,
                        Rc::new(RefCell::new(SymbolTable::new_derived(
                            ctx,
                            f_args
                                .into_iter()
                                .enumerate()
                                .map(|(i, f_arg)| (f_arg.to_owned(), traversed_args[i].clone()))
                                .collect(),
                        ))),
                        pass,
                    )
                }
                _ => unimplemented!("{:?}", f),
            }
        } else {
            Ok(None)
        }
    }

    fn reduce<'a>(
        &self,
        e: &AstNode,
        ctx: Rc<RefCell<SymbolTable>>,
        pass: Pass,
    ) -> Result<Option<Constraint>> {
        match (&e.class, pass) {
            (Token::Ignore, _) => Ok(None),
            (Token::Value(x), _) => Ok(Some(Constraint::Const(*x))),
            (Token::Symbol(name), _) => Ok(Some(ctx.borrow_mut().resolve_symbol(&name)?)),
            (Token::TopLevelForm { args }, Pass::Definition) => {
                if let Token::Symbol(verb) = &args[0].class {
                    let func = ctx
                        .borrow()
                        .resolve_function(&verb)
                        .with_context(|| eyre!("resolving form `{}`", verb))?;

                    Ok(self.apply(&func, &args[1..], ctx, pass)?)
                } else {
                    unimplemented!()
                }
            }
            (Token::List { args }, Pass::Compilation) => {
                if let Token::Symbol(verb) = &args[0].class {
                    let func = ctx
                        .borrow()
                        .resolve_function(&verb)
                        .with_context(|| eyre!("resolving function `{}`", verb))?;

                    self.apply(&func, &args[1..], ctx, pass)
                } else {
                    Err(eyre!("Not a function: {:?}", args[0]))
                }
            }
            (Token::List { .. }, Pass::Definition) => Ok(None),
            (Token::TopLevelForm { .. }, Pass::Compilation) => Ok(None),
        }
        .with_context(|| format!("at line {}, col.{}: \"{}\"", e.lc.0, e.lc.1, e.src))
    }

    fn build_constraints<'a>(
        &mut self,
        ast: &ParsingAst,
        ctx: Rc<RefCell<SymbolTable>>,
        pass: Pass,
    ) -> Result<Vec<Constraint>> {
        let mut r = vec![];

        for exp in ast.exprs.to_vec() {
            self.reduce(&exp, ctx.clone(), pass)
                .with_context(|| {
                    format!("at line {}, col.{}: \"{}\"", exp.lc.0, exp.lc.1, exp.src)
                })?
                .map(|c| r.push(c));
        }
        Ok(r)
    }

    fn compile(sources: &[(&str, &str)]) -> Result<ConstraintsSet> {
        let table = Rc::new(RefCell::new(SymbolTable::new_root()));
        let mut compiler = Compiler {};
        let mut asts = vec![];

        for (name, content) in sources.iter() {
            let ast = parse(content).with_context(|| eyre!("parsing `{}`", name))?;
            let _ = compiler
                .build_constraints(&ast, table.clone(), Pass::Definition)
                .with_context(|| eyre!("parsing top-level definitions in `{}`", name))?;
            asts.push((name, ast));
        }

        let constraints = asts
            .into_iter()
            .map(|(name, ast)| {
                compiler
                    .build_constraints(&ast, table.clone(), Pass::Compilation)
                    .with_context(|| eyre!("compiling constraints in `{}`", name))
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect();
        Ok(ConstraintsSet { constraints })
    }
}
