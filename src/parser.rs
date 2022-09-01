use color_eyre::eyre::*;
use pest::{iterators::Pair, Parser};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fmt::Debug;
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
        "branch-if-zero" => Function {
            name:"branch-if-zero".into(),
            class: FunctionClass::Builtin(Builtin::BranchIfZero)
        },
        "branch-if-zero-else" => Function {
            name:"branch-if-zero-else".into(),
            class: FunctionClass::Builtin(Builtin::BranchIfZero)
        },
    };
}

pub(crate) trait Transpiler {
    fn render(&self, cs: &ConstraintsSet) -> Result<String>;
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
    pub fn from_str<S: AsRef<str>>(s: S) -> Result<Self> {
        let exprs = parse(s.as_ref())?;
        Compiler::compile(exprs)
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
    // Don't like it :/
    BranchIfNotZero,
    BranchBinIfZero,
    BranchBinIfOne,
    BranchIfNotZeroElse,
    BranchBinIfZeroElse,
    BranchBinIfOneElse,
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

    BranchIfZero,
    BranchIfZeroElse,
}

#[derive(Debug, PartialEq, Clone)]
struct Verb {
    name: String,
}

#[derive(Debug, PartialEq, Clone)]
enum AstNode {
    Ignore,
    Value(i32),
    Symbol(String),
    List { args: Vec<AstNode> },
    TopLevelForm { args: Vec<AstNode> },
}
impl AstNode {
    fn fold<T>(&self, f: &dyn Fn(T, &Self) -> T, ax: T) -> T {
        match self {
            AstNode::Ignore => ax,
            AstNode::Symbol { .. } | AstNode::Value(_) => f(ax, self),
            AstNode::List { args } => {
                let mut aax = ax;
                for s in args.iter() {
                    aax = s.fold(f, aax);
                }
                aax
            }
            AstNode::TopLevelForm { args } => {
                let mut aax = ax;
                for s in args.iter() {
                    aax = s.fold(f, aax);
                }
                aax
            }
        }
    }
}

fn build_ast_from_expr(pair: Pair<Rule>, in_def: bool) -> Result<AstNode> {
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
                .filter(|x| *x != AstNode::Ignore)
                .collect::<Vec<_>>();
            Ok(AstNode::TopLevelForm { args })
        }
        Rule::list => {
            let args = pair
                .into_inner()
                .map(|p| build_ast_from_expr(p, in_def))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .filter(|x| *x != AstNode::Ignore)
                .collect::<Vec<_>>();
            Ok(AstNode::List { args })
        }
        Rule::symbol | Rule::defform => Ok(AstNode::Symbol(pair.as_str().to_owned())),
        Rule::integer => Ok(AstNode::Value(pair.as_str().parse().unwrap())),
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
            Err(anyhow!("`{}` already exists", symbol))
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
            Err(anyhow!(
                "`{}` already exists: {} -> {:?}",
                from,
                from,
                self.symbols[from]
            ))
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
    fn arity(&self) -> isize;
    fn make_arity_error(&self, args: &[T]) -> String {
        let arity = self.arity();
        return format!(
            "expected {} argument{}, received {}",
            match arity {
                VARIADIC_1 => String::from("at least one"),
                VARIADIC_2 => String::from("at least two"),
                EVEN => String::from("even number of"),
                arity @ _ => arity.to_string(),
            },
            if arity <= VARIADIC_1 || arity > 1 {
                "s"
            } else {
                ""
            },
            args.len(),
        );
    }

    fn validate_types(&self, args: &[T]) -> Result<()>;

    fn validate_arity(&self, args: &[T]) -> Result<()> {
        let arity = dbg!(self.arity());
        ((arity == VARIADIC_1 && args.len() >= 1)
            || (arity == VARIADIC_2 && args.len() >= 2)
            || (arity == args.len() as isize)
            || (arity == EVEN && args.len() % 2 == 0))
            .then(|| ())
            .ok_or(eyre!(self.make_arity_error(args)))
    }

    fn validate_args(&self, args: Vec<T>) -> Result<Vec<T>> {
        dbg!(self.validate_arity(&args))
            .and_then(|_| self.validate_types(&args))
            .and(Ok(args))
    }
}
impl FuncVerifier<AstNode> for Form {
    fn arity(&self) -> isize {
        match self {
            Form::Defun => 2,
            Form::Defalias => EVEN,
            Form::Defunalias => 2,
            Form::Defcolumns => VARIADIC_1,
            Form::Defconst => EVEN,
            _ => todo!(),
        }
    }
    fn validate_types(&self, args: &[AstNode]) -> Result<()> {
        match self {
            Form::Defcolumns => {
                if args.iter().all(|a| matches!(a, AstNode::Symbol(_))) {
                    Ok(())
                } else {
                    Err(eyre!("DEFCOLUMNS expects only symbols"))
                }
            }
            Form::Defun => {
                if matches!(args[0], AstNode::List { .. })
                    && matches!(args[1], AstNode::List { .. })
                {
                    Ok(())
                } else {
                    Err(eyre!("DEFUN expects two expressions; received: {:?}", args))
                }
            }
            Form::Defalias | Form::Defunalias => {
                if args.iter().all(|a| matches!(a, AstNode::Symbol(_))) {
                    Ok(())
                } else {
                    Err(eyre!("DEFCOLUMNS expects only symbols"))
                }
            }
            _ => {
                unimplemented!("{:?}", self)
            }
        }
    }
}

impl FuncVerifier<Constraint> for Builtin {
    fn arity(&self) -> isize {
        match self {
            Builtin::Add => VARIADIC_2,
            Builtin::Sub => VARIADIC_2,
            Builtin::Mul => VARIADIC_2,
            Builtin::Neg => 1,
            Builtin::Inv => 1,
            Builtin::IfZero => 2,
            Builtin::Shift => 2,
            Builtin::Begin => VARIADIC_1,
            Builtin::BranchIfZero => 2,
            Builtin::BranchIfZeroElse => 3,
            Builtin::Ith => 2,
        }
    }
    fn validate_types(&self, args: &[Constraint]) -> Result<()> {
        println!("{:?}: {:?}", self, &args);
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
    fn arity(&self) -> isize {
        self.args.len() as isize
    }

    fn validate_types(&self, args: &[Constraint]) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct Function {
    name: String,
    class: FunctionClass,
}

const VARIADIC_0: isize = -1;
const VARIADIC_1: isize = -2;
const VARIADIC_2: isize = -3;
const EVEN: isize = -4;

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
struct Compiler {
    ast: ParsingAst,
}

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
                    if let AstNode::Symbol(name) = arg {
                        ctx.borrow_mut().insert_symbol(&name)?
                    }
                }

                Ok(None)
            }
            (Form::Defconst, Pass::Definition) => {
                for pair in args.chunks(2) {
                    if let [AstNode::Symbol(name), AstNode::Value(x)] = pair {
                        ctx.borrow_mut().insert_constant(name, *x)?
                    }
                }

                Ok(None)
            }
            (Form::Defalias, Pass::Definition) => {
                for pair in args.chunks(2) {
                    if let [AstNode::Symbol(from), AstNode::Symbol(to)] = pair {
                        ctx.borrow_mut().insert_alias(from, to)?
                    }
                }

                Ok(None)
            }
            (Form::Defunalias, Pass::Definition) => {
                for pair in args.chunks(2) {
                    if let [AstNode::Symbol(from), AstNode::Symbol(to)] = pair {
                        ctx.borrow_mut().insert_funalias(from, to)?
                    }
                }

                Ok(None)
            }
            (Form::Defun, Pass::Definition) => {
                let header = &args[0];
                let body = &args[1];

                if let AstNode::List { args } = header {
                    if let AstNode::Symbol(fname) = &args[0] {
                        let arg_names = args
                            .iter()
                            .map(|a| {
                                if let AstNode::Symbol(n) = a {
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
        match (e, pass) {
            (AstNode::Ignore, _) => Ok(None),
            (AstNode::Value(x), _) => Ok(Some(Constraint::Const(*x))),
            (AstNode::Symbol(name), _) => Ok(Some(ctx.borrow_mut().resolve_symbol(name)?)),
            (AstNode::TopLevelForm { args }, Pass::Definition) => {
                if let AstNode::Symbol(verb) = &args[0] {
                    let func = ctx
                        .borrow()
                        .resolve_function(&verb)
                        .with_context(|| eyre!("resolving form `{}`", verb))?;

                    Ok(self.apply(&func, &args[1..], ctx, pass)?)
                } else {
                    unimplemented!()
                }
            }
            (AstNode::List { args }, Pass::Compilation) => {
                if let AstNode::Symbol(verb) = &args[0] {
                    let func = ctx
                        .borrow()
                        .resolve_function(&verb)
                        .with_context(|| eyre!("resolving function `{}`", verb))?;

                    self.apply(&func, &args[1..], ctx, pass)
                } else {
                    Err(eyre!("Not a function: {:?}", args[0]))
                }
            }
            (AstNode::List { .. }, Pass::Definition) => Ok(None),
            (AstNode::TopLevelForm { .. }, Pass::Compilation) => Ok(None),
        }
    }

    fn build_constraints<'a>(
        &mut self,
        ctx: Rc<RefCell<SymbolTable>>,
        pass: Pass,
    ) -> Result<ConstraintsSet> {
        let mut cs = ConstraintsSet {
            constraints: vec![],
        };

        for exp in &self.ast.exprs.to_vec() {
            self.reduce(exp, ctx.clone(), pass)
                .with_context(|| "assembling top-level constraint")?
                .map(|c| cs.constraints.push(c));
        }
        Ok(cs)
    }

    fn compile(ast: ParsingAst) -> Result<ConstraintsSet> {
        let mut compiler = Compiler { ast };
        let table = Rc::new(RefCell::new(SymbolTable::new_root()));

        let _ = compiler
            .build_constraints(table.clone(), Pass::Definition)
            .with_context(|| eyre!("parsing top-level definitions"))?;

        let cs = compiler
            .build_constraints(table.clone(), Pass::Compilation)
            .with_context(|| eyre!("compiling constraints"))?;
        Ok(cs)
    }
}
