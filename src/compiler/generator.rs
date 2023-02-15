use anyhow::*;
use colored::Colorize;
use log::*;
use num_bigint::BigInt;
use num_traits::cast::ToPrimitive;
use num_traits::{One, Zero};
use once_cell::sync::OnceCell;
use pairing_ce::bn256::Fr;
use pairing_ce::ff::{Field, PrimeField};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::io::Write;
use std::rc::Rc;
use std::sync::atomic::AtomicUsize;

use super::definitions::ComputationTable;
use super::{common::*, CompileSettings, Expression, Handle, Magma, Node, Type};
use crate::column::{Column, ColumnSet, Computation};
use crate::compiler::definitions::SymbolTable;
use crate::compiler::parser::*;
use crate::errors::{CompileError, RuntimeError};
use crate::pretty::Pretty;

static COUNTER: OnceCell<AtomicUsize> = OnceCell::new();

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum Constraint {
    Vanishes {
        handle: Handle,
        domain: Option<Vec<isize>>,
        expr: Box<Node>,
    },
    Plookup {
        handle: Handle,
        including: Vec<Node>,
        included: Vec<Node>,
    },
    Permutation {
        handle: Handle,
        from: Vec<Handle>,
        to: Vec<Handle>,
    },
    InRange {
        handle: Handle,
        exp: Node,
        max: Fr,
    },
}
impl Constraint {
    pub fn name(&self) -> String {
        match self {
            Constraint::Vanishes { handle, .. } => handle.to_string(),
            Constraint::Plookup { handle, .. } => handle.to_string(),
            Constraint::Permutation { handle, .. } => handle.to_string(),
            Constraint::InRange { handle, .. } => handle.to_string(),
        }
    }

    pub fn add_id_to_handles(&mut self, set_id: &dyn Fn(&mut Handle)) {
        match self {
            Constraint::Vanishes { expr, .. } => expr.add_id_to_handles(set_id),
            Constraint::Plookup {
                handle: _,
                including: xs,
                included: ys,
            } => xs
                .iter_mut()
                .chain(ys.iter_mut())
                .for_each(|e| e.add_id_to_handles(set_id)),
            Constraint::Permutation {
                handle: _,
                from: hs1,
                to: hs2,
            } => hs1.iter_mut().chain(hs2.iter_mut()).for_each(|h| set_id(h)),
            Constraint::InRange {
                handle: _,
                exp,
                max: _,
            } => exp.add_id_to_handles(set_id),
        }
    }

    pub(crate) fn size(&self) -> usize {
        match self {
            Constraint::Vanishes { expr, .. } => expr.size(),
            Constraint::Plookup { .. } => 1,
            Constraint::Permutation { .. } => 1,
            Constraint::InRange { .. } => 1,
        }
    }
}

/// Options used when evaluating an expression
pub struct EvalSettings {
    /// If true, negative indices will loop from the end of the column;
    /// otherwise, they will go up in the padding.
    pub wrap: bool,
}
impl Default for EvalSettings {
    fn default() -> Self {
        EvalSettings { wrap: true }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
pub enum Builtin {
    Add,
    Sub,
    Mul,
    Exp,
    Shift,
    Neg,
    Inv,
    Not,

    Nth,
    Len,
    Eq,
    Begin,

    IfZero,
    IfNotZero,
}
impl Builtin {
    pub fn call(self, args: &[Node]) -> Result<Node> {
        self.validate_args(args)?;
        Ok(Node::from_expr(self.raw_call(args)))
    }

    pub fn raw_call(self, args: &[Node]) -> Expression {
        Expression::Funcall {
            func: self,
            args: args.to_owned(),
        }
    }

    pub fn typing(&self, argtype: &[Type]) -> Type {
        match self {
            Builtin::Add | Builtin::Sub | Builtin::Neg | Builtin::Inv => {
                // Boolean is a corner case, as it is not stable under these operations
                match argtype.iter().fold(Type::INFIMUM, |a, b| a.max(*b)) {
                    Type::Scalar(Magma::Boolean) => Type::Scalar(Magma::Integer),
                    Type::Column(Magma::Boolean) => Type::Column(Magma::Integer),
                    x => x,
                }
            }
            Builtin::Exp => argtype[0],
            Builtin::Eq => argtype.iter().max().cloned().unwrap_or(Type::INFIMUM),
            Builtin::Not => argtype
                .iter()
                .max()
                .cloned()
                .unwrap_or(Type::INFIMUM)
                .same_scale(Magma::Boolean),
            Builtin::Mul => argtype.iter().max().cloned().unwrap_or(Type::INFIMUM),
            Builtin::IfZero | Builtin::IfNotZero => {
                argtype[1].max(argtype.get(2).cloned().unwrap_or(Type::INFIMUM))
            }
            Builtin::Begin => {
                Type::List(argtype.iter().fold(Type::INFIMUM, |a, b| a.max(*b)).magma())
            }
            Builtin::Nth => Type::Column(argtype[0].magma()),
            Builtin::Shift => argtype[0],
            Builtin::Len => Type::Scalar(Magma::Integer),
        }
    }
}
impl std::fmt::Display for Builtin {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Builtin::Eq => "eq",
                Builtin::Add => "+",
                Builtin::Sub => "-",
                Builtin::Mul => "*",
                Builtin::Exp => "^",
                Builtin::Shift => "shift",
                Builtin::Neg => "-",
                Builtin::Inv => "inv",
                Builtin::Not => "not",
                Builtin::Nth => "nth",
                Builtin::Begin => "begin",
                Builtin::IfZero => "if-zero",
                Builtin::IfNotZero => "if-not-zero",
                Builtin::Len => "len",
            }
        )
    }
}

#[derive(Debug, Clone)]
pub struct Function {
    pub handle: Handle,
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
    pub pure: bool,
    pub args: Vec<String>,
    pub body: AstNode,
}
impl FuncVerifier<Node> for Defined {
    fn arity(&self) -> Arity {
        Arity::Exactly(self.args.len())
    }

    fn validate_types(&self, _args: &[Node]) -> Result<()> {
        Ok(())
    }
}

impl FuncVerifier<Node> for Builtin {
    fn arity(&self) -> Arity {
        match self {
            Builtin::Add => Arity::AtLeast(1),
            Builtin::Sub => Arity::AtLeast(1),
            Builtin::Mul => Arity::AtLeast(1),
            Builtin::Exp => Arity::Exactly(2),
            Builtin::Eq => Arity::Exactly(2),
            Builtin::Neg => Arity::Monadic,
            Builtin::Inv => Arity::Monadic,
            Builtin::Not => Arity::Monadic,
            Builtin::Shift => Arity::Dyadic,
            Builtin::Begin => Arity::AtLeast(1),
            Builtin::IfZero => Arity::Between(2, 3),
            Builtin::IfNotZero => Arity::Between(2, 3),
            Builtin::Nth => Arity::Dyadic,
            Builtin::Len => Arity::Monadic,
        }
    }
    fn validate_types(&self, args: &[Node]) -> Result<()> {
        let args_t = args.iter().map(|a| a.t()).collect::<Vec<_>>();
        let expected_t: &[&[Type]] = match self {
            Builtin::Add | Builtin::Sub | Builtin::Mul => {
                &[&[Type::Scalar(Magma::Any), Type::Column(Magma::Any)]]
            }
            Builtin::Exp => &[
                &[Type::Scalar(Magma::Any), Type::Column(Magma::Any)],
                &[Type::Scalar(Magma::Any)],
            ],
            Builtin::Eq => &[&[Type::Column(Magma::Any), Type::Scalar(Magma::Any)]],
            Builtin::Not => &[&[Type::Scalar(Magma::Boolean), Type::Column(Magma::Boolean)]],
            Builtin::Neg => &[&[Type::Scalar(Magma::Any), Type::Column(Magma::Any)]],
            Builtin::Inv => &[&[Type::Column(Magma::Any)]],
            Builtin::Shift => &[&[Type::Column(Magma::Any)], &[Type::Scalar(Magma::Any)]],
            Builtin::Nth => &[
                &[Type::ArrayColumn(Magma::Any)],
                &[Type::Scalar(Magma::Any)],
            ],
            Builtin::IfZero | Builtin::IfNotZero => &[
                &[
                    Type::Scalar(Magma::Any),
                    Type::Column(Magma::Any),
                    Type::List(Magma::Any),
                ],
                &[
                    Type::Scalar(Magma::Any),
                    Type::Column(Magma::Any),
                    Type::List(Magma::Any),
                ],
            ],
            Builtin::Begin => &[&[
                Type::Scalar(Magma::Any),
                Type::Column(Magma::Any),
                Type::List(Magma::Any),
            ]],
            Builtin::Len => &[&[Type::ArrayColumn(Magma::Any)]],
        };

        if super::compatible_with(expected_t, &args_t) {
            Ok(())
        } else {
            bail!(CompileError::TypeError(
                self.to_string(),
                expected_t,
                args_t
            ))
        }
    }
    fn validate_args(&self, args: &[Node]) -> Result<()> {
        FuncVerifier::validate_arity(self, args)
            .with_context(|| format!("while validating {}", self))
    }
}

#[derive(Default, Debug, Serialize, Deserialize, Clone)]
pub struct ConstraintSet {
    pub modules: ColumnSet,
    pub constraints: Vec<Constraint>,
    pub constants: HashMap<Handle, BigInt>,
    pub computations: ComputationTable,

    /// The spilling of a module is the maximum of the absolute values
    /// of the forward- and backward-shift observed in its constraints.
    /// The spilling is used to compute the padding of its columns.
    _spilling: HashMap<String, isize>, // module -> (past-spilling, future-spilling)
}
impl ConstraintSet {
    pub fn new(
        columns: ColumnSet,
        constraints: Vec<Constraint>,
        constants: HashMap<Handle, BigInt>,
        computations: ComputationTable,
    ) -> Self {
        let mut r = ConstraintSet {
            constraints,
            modules: columns,
            constants,
            computations,

            _spilling: Default::default(),
        };
        r.update_ids();
        r
    }

    // TODO I hate this, see if we can automate it
    pub fn update_ids(&mut self) {
        let set_id = |h: &mut Handle| h.set_id(self.modules.id_of(h));
        self.constraints
            .iter_mut()
            .for_each(|x| x.add_id_to_handles(&set_id));
        self.computations.update_ids(&set_id)
    }

    fn get(&self, handle: &Handle) -> Result<&Column> {
        self.modules.get(handle)
    }

    fn get_mut(&mut self, handle: &Handle) -> Result<&mut Column> {
        self.modules.get_mut(handle)
    }

    fn compute_interleaved(&mut self, froms: &[Handle], target: &Handle) -> Result<()> {
        for from in froms.iter() {
            self.compute_column(from)?;
        }

        if !froms
            .iter()
            .map(|h| self.get(h).unwrap().len().unwrap())
            .collect::<Vec<_>>()
            .windows(2)
            .all(|w| w[0] == w[1])
        {
            bail!("interleaving columns of incoherent lengths")
        }

        let final_len = froms
            .iter()
            .map(|h| self.get(h).unwrap().len().unwrap())
            .sum();
        let count = froms.len();
        let values = (0..final_len)
            .into_par_iter()
            .map(|k| {
                let i = k / count;
                let j = k % count;
                *self
                    .get(&froms[j as usize])
                    .unwrap()
                    .get(i as isize, false)
                    .unwrap()
            })
            .collect();

        self.get_mut(target)?.set_raw_value(values, 0);
        assert!(
            self.get(target).unwrap().len().unwrap()
                == self.get(target).unwrap().padded_len().unwrap()
        );
        assert!(
            self.get(target).unwrap().len().unwrap()
                == froms.len() * self.get(&froms[0]).unwrap().len().unwrap()
        );
        Ok(())
    }

    fn compute_sorted(&mut self, froms: &[Handle], tos: &[Handle]) -> Result<()> {
        let spilling = self.spilling_or_insert(&froms[0].module);
        for from in froms.iter() {
            self.compute_column(from)?;
        }

        let from_cols = froms
            .iter()
            .map(|c| self.get(c).unwrap())
            .collect::<Vec<_>>();

        if !from_cols
            .windows(2)
            .all(|w| w[0].padded_len() == w[1].padded_len())
        {
            bail!("sorted columns are of incoherent lengths")
        }
        let len = from_cols[0].len().unwrap();

        let mut sorted_is = (0..len).collect::<Vec<_>>();
        sorted_is.sort_by(|i, j| {
            for from in from_cols.iter() {
                let x_i = from.get(*i as isize, false).unwrap();
                let x_j = from.get(*j as isize, false).unwrap();
                if let x @ (Ordering::Greater | Ordering::Less) = x_i.cmp(x_j) {
                    return x;
                }
            }
            Ordering::Equal
        });

        for (k, from) in froms.iter().enumerate() {
            let value: Vec<Fr> = vec![Fr::zero(); spilling as usize]
                .into_iter()
                .chain(sorted_is.iter().map(|i| {
                    *self
                        .get(from)
                        .unwrap()
                        .get((*i).try_into().unwrap(), false)
                        .unwrap()
                }))
                .collect();

            self.get_mut(&tos[k])
                .unwrap()
                .set_raw_value(value, spilling);
        }

        Ok(())
    }

    fn compute_cyclic(&mut self, froms: &[Handle], to: &Handle, modulo: usize) -> Result<()> {
        let spilling = self.spilling_or_insert(&froms[0].module);
        for from in froms.iter() {
            self.compute_column(from)?;
        }
        let len = self.get(&froms[0]).unwrap().len().unwrap();
        if len < modulo {
            bail!(
                "unable to compute cyclic column {}: {} < {}",
                to.to_string().bold().white(),
                len,
                modulo
            )
        }

        let value: Vec<Fr> = vec![Fr::zero(); spilling as usize]
            .into_iter()
            .chain((0..len).map(|i| Fr::from_str(&((i % modulo).to_string())).unwrap()))
            .collect();

        self.get_mut(to).unwrap().set_raw_value(value, spilling);

        Ok(())
    }

    pub fn compute_composite(&mut self, exp: &Node, target: &Handle) -> Result<()> {
        let spilling = self.spilling_or_insert(&target.module);
        let cols_in_expr = exp.dependencies();
        for c in &cols_in_expr {
            self.compute_column(c)?
        }
        let length = *cols_in_expr
            .iter()
            .map(|handle| Ok(self.get(handle).unwrap().len().unwrap().to_owned()))
            .collect::<Result<Vec<_>>>()?
            .iter()
            .max()
            .unwrap();

        let values = (-spilling..length as isize)
            .into_par_iter()
            .map(|i| {
                exp.eval(
                    i,
                    &mut |handle, j, _| {
                        self.modules._cols[handle.id.unwrap()]
                            .get(j, false)
                            .cloned()
                    },
                    &mut None,
                    &EvalSettings { wrap: false },
                )
                .unwrap_or_else(Fr::zero)
            })
            .collect();

        self.modules
            .get_mut(target)
            .unwrap()
            .set_raw_value(values, spilling);
        Ok(())
    }

    pub fn compute_composite_static(&self, exp: &Node) -> Result<Vec<Fr>> {
        let cols_in_expr = exp.dependencies();
        for c in &cols_in_expr {
            if !self.get(c)?.is_computed() {
                bail!("column {} not yet computed", c.to_string().red())
            }
        }

        let length = *cols_in_expr
            .iter()
            .map(|handle| {
                Ok(self
                    .get(handle)
                    .with_context(|| anyhow!("while reading {}", handle.to_string().red().bold()))?
                    .len()
                    .ok_or_else(|| anyhow!("{} has no len", handle.to_string().red().bold()))?
                    .to_owned())
            })
            .collect::<Result<Vec<_>>>()?
            .iter()
            .max()
            .unwrap();

        let values = (0..length as isize)
            .into_par_iter()
            .map(|i| {
                exp.eval(
                    i,
                    &mut |handle, j, _| {
                        self.modules._cols[handle.id.unwrap()]
                            .get(j, false)
                            .cloned()
                    },
                    &mut None,
                    &EvalSettings { wrap: false },
                )
                .unwrap_or_else(Fr::zero)
            })
            .collect::<Vec<_>>();

        Ok(values)
    }

    fn compute_sorting_auxs(
        &mut self,
        ats: &[Handle],
        eq: &Handle,
        delta: &Handle,
        delta_bytes: &[Handle],
        signs: &[bool],
        from: &[Handle],
        sorted: &[Handle],
    ) -> Result<()> {
        assert!(delta_bytes.len() == 16);
        for c in from.iter().chain(sorted.iter()) {
            self.compute_column(c)?;
        }
        let spilling = self.spilling_or_insert(&from[0].module);
        let len = self.modules.by_handle(&from[0]).unwrap().len().unwrap();

        let mut at_values = std::iter::repeat_with(|| vec![Fr::zero(); spilling as usize])
            .take(ats.len())
            .collect::<Vec<_>>();
        // in the spilling, all @ == 0; thus Eq = 1
        let mut eq_values = vec![Fr::one(); spilling as usize];
        let mut delta_values = vec![Fr::zero(); spilling as usize];
        let mut delta_bytes_values = std::iter::repeat_with(|| vec![Fr::zero(); spilling as usize])
            .take(delta_bytes.len())
            .collect::<Vec<_>>();
        let sorted_cols = sorted
            .iter()
            .map(|f| {
                self.modules
                    .by_handle(f)
                    .ok_or_else(|| anyhow!("column `{}` not found", f))
            })
            .collect::<Result<Vec<_>>>()?;
        for i in 0..len as isize {
            // Compute @s
            let mut found = false;
            for l in 0..ats.len() {
                let eq = sorted_cols[l]
                    .get(i, false)
                    .zip(sorted_cols[l].get(i - 1, false)) // may fail @0 if no padding; in this case, @ = 0
                    .map(|(v1, v2)| v1.eq(v2))
                    .unwrap_or(true);

                let v = if !eq {
                    if found {
                        Fr::zero()
                    } else {
                        found = true;
                        Fr::one()
                    }
                } else {
                    Fr::zero()
                };

                at_values[l].push(v);
            }

            // Compute Eq
            eq_values.push(if found { Fr::zero() } else { Fr::one() });

            // Compute Delta
            let mut delta = Fr::zero();
            if eq_values.last().unwrap().is_zero() {
                for l in 0..ats.len() {
                    let mut term = *sorted_cols[l].get(i, false).unwrap();
                    term.sub_assign(sorted_cols[l].get(i - 1, false).unwrap());
                    term.mul_assign(at_values[l].last().unwrap());
                    if !signs[l] {
                        term.negate();
                    }
                    delta.add_assign(&term);
                }
            }
            // delta.sub_assign(&Fr::one());
            delta_values.push(delta);

            delta
                .into_repr()
                .as_ref()
                .iter()
                .flat_map(|u| u.to_le_bytes().into_iter())
                .map(|i| Fr::from_str(&i.to_string()).unwrap())
                .enumerate()
                .take(16)
                .for_each(|(i, b)| delta_bytes_values[i].push(b));
        }

        for (at, value) in ats.iter().zip(at_values.into_iter()) {
            self.get_mut(at).unwrap().set_raw_value(value, spilling);
        }
        self.get_mut(eq).unwrap().set_raw_value(eq_values, spilling);
        self.get_mut(delta)
            .unwrap()
            .set_raw_value(delta_values, spilling);
        for (delta_byte, value) in delta_bytes.iter().zip(delta_bytes_values.into_iter()) {
            self.get_mut(delta_byte)
                .unwrap()
                .set_raw_value(value, spilling);
        }
        Ok(())
    }

    fn compute_column(&mut self, target: &Handle) -> Result<()> {
        let target_col = self.get(target).unwrap();
        if target_col.is_computed() {
            Ok(())
        } else if matches!(target_col.kind, Kind::Atomic) {
            bail!(anyhow!(RuntimeError::EmptyColumn(target.clone())))
        } else {
            self.compute(
                self.computations
                    .dependencies(target)
                    .ok_or_else(|| anyhow!("No computations found for `{}`", target))?,
            )
        }
    }

    fn compute(&mut self, i: usize) -> Result<()> {
        let comp = self.computations.get(i).unwrap().clone();
        debug!("Computing `{}`", comp.pretty_target());

        match &comp {
            Computation::Composite { target, exp } => {
                if !self.modules.get(target)?.is_computed() {
                    self.compute_composite(exp, target)
                } else {
                    Ok(())
                }
            }
            Computation::Interleaved { target, froms } => {
                if !self.modules.get(target)?.is_computed() {
                    self.compute_interleaved(froms, target)
                } else {
                    Ok(())
                }
            }
            Computation::Sorted { froms, tos } => self.compute_sorted(froms, tos),
            Computation::CyclicFrom {
                target,
                froms,
                modulo,
            } => self.compute_cyclic(froms, target, *modulo),
            Computation::SortingConstraints {
                ats,
                eq,
                delta,
                delta_bytes,
                signs,
                froms,
                sorted,
            } => self.compute_sorting_auxs(ats, eq, delta, delta_bytes, signs, froms, sorted),
        }
    }

    pub fn compute_all(&mut self) -> Result<()> {
        for i in 0..self.computations.iter().count() {
            if let Err(e) = self.compute(i) {
                warn!("{:?}", e);
            }
        }

        Ok(())
    }

    pub fn raw_len_for_or_set(&mut self, m: &str, x: isize) -> isize {
        *self.modules.raw_len.entry(m.to_string()).or_insert(x)
    }

    pub fn spilling(&self, m: &str) -> Option<isize> {
        self._spilling.get(m).cloned()
    }

    pub fn spilling_or_insert(&mut self, m: &str) -> isize {
        *self._spilling.entry(m.to_string()).or_insert_with(|| {
            self.computations
                .iter()
                .filter_map(|c| match c {
                    Computation::Composite { target, exp } => {
                        if target.module == m {
                            Some(exp.past_spill() as isize)
                        } else {
                            None
                        }
                    }
                    _ => None,
                })
                .min()
                .unwrap_or(0)
                .abs()
                .max(
                    self.computations
                        .iter()
                        .filter_map(|c| match c {
                            Computation::Composite { target, exp } => {
                                if target.module == m {
                                    Some(exp.future_spill() as isize)
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        })
                        .max()
                        .unwrap_or(0),
                )
        })
    }

    pub fn length_multiplier(&self, h: &Handle) -> usize {
        self.computations
            .computation_for(h)
            .map(|comp| match comp {
                Computation::Composite { exp, .. } => {
                    self.length_multiplier(exp.dependencies().iter().next().unwrap())
                }
                Computation::Interleaved { froms, .. } => {
                    self.length_multiplier(&froms[0]) * froms.len()
                }
                Computation::Sorted { froms, .. } | Computation::CyclicFrom { froms, .. } => {
                    self.length_multiplier(&froms[0])
                }
                Computation::SortingConstraints { .. } => 1,
            })
            .unwrap_or(1)
            * self
                .modules
                .get(h)
                .unwrap()
                .intrinsic_size_factor
                .unwrap_or(1)
    }

    pub fn write(&mut self, out: &mut impl Write) -> Result<()> {
        out.write_all("{\"columns\":{\n".as_bytes())?;

        for (i, (module, columns)) in self.modules.cols.iter().enumerate() {
            info!("Exporting {}", &module);
            if i > 0 {
                out.write_all(b",")?;
            }

            let empty_vec = Vec::new();
            let mut current_col = columns.iter().peekable();
            while let Some((name, &i)) = current_col.next() {
                trace!("Writing {}/{}", module, name);
                let column = &self.modules._cols[i];
                let handle = Handle::new(&module, &name);
                let value = column.value().unwrap_or(&empty_vec);
                let padding = if handle.name.starts_with("__SRT__Eq_") {
                    // NOTE ugly, find a better way to handle that.
                    Fr::one()
                } else {
                    value.get(0).cloned().unwrap_or_else(|| {
                        self.computations
                            .computation_for(&handle)
                            .map(|c| match c {
                                Computation::Composite { exp, .. } => exp
                                    .eval(
                                        0,
                                        &mut |_, _, _| Some(Fr::zero()),
                                        &mut None,
                                        &EvalSettings::default(),
                                    )
                                    .unwrap_or_else(Fr::zero),
                                Computation::Interleaved { .. } => Fr::zero(),
                                Computation::Sorted { .. } => Fr::zero(),
                                Computation::CyclicFrom { .. } => Fr::zero(),
                                Computation::SortingConstraints { .. } => Fr::zero(),
                            })
                            .unwrap_or_else(Fr::zero)
                    })
                };

                out.write_all(format!("\"{}\":{{\n", handle).as_bytes())?;
                out.write_all("\"values\":[".as_bytes())?;
                out.write_all(
                    value
                        .par_iter()
                        .map(|x| {
                            format!(
                                "\"0x0{}\"",
                                x.into_repr().to_string()[2..].trim_start_matches('0')
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(",")
                        .as_bytes(),
                )?;
                out.write_all(b"],\n")?;
                out.write_all(
                    format!(
                        "\"padding_strategy\": {{\"action\": \"prepend\", \"value\": \"{}\"}}",
                        padding.pretty()
                    )
                    .as_bytes(),
                )?;
                out.write_all(b"\n}\n")?;
                if current_col.peek().is_some() {
                    out.write_all(b",")?;
                }
            }
        }
        out.write_all("}}".as_bytes())?;

        Ok(())
    }
}

// Compared to a function, a form do not evaluate all of its arguments by default
fn apply_form(
    f: Form,
    args: &[AstNode],
    root_ctx: Rc<RefCell<SymbolTable>>,
    ctx: &mut Rc<RefCell<SymbolTable>>,
    settings: &CompileSettings,
) -> Result<Option<Node>> {
    f.validate_args(args)
        .with_context(|| anyhow!("evaluating call to {:?}", f))?;

    match f {
        Form::For => {
            if let (Token::Symbol(i_name), Token::Range(is), body) =
                (&args[0].class, &args[1].class, &args[2])
            {
                let mut l = vec![];
                let mut t = Type::INFIMUM;
                for i in is {
                    let for_ctx_pretty_name = &ctx.borrow().pretty_name.clone();
                    let mut for_ctx = SymbolTable::derived(
                        ctx.clone(),
                        &format!(
                            "for-{}-{}",
                            COUNTER
                                .get_or_init(|| AtomicUsize::new(0))
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                            i
                        ),
                        for_ctx_pretty_name,
                        false,
                        false,
                    );
                    for_ctx.borrow_mut().insert_symbol(
                        i_name,
                        Expression::Const(BigInt::from(*i), Fr::from_str(&i.to_string())).into(),
                    )?;

                    let r =
                        reduce(&body.clone(), root_ctx.clone(), &mut for_ctx, settings)?.unwrap();
                    t = t.max(r.t());
                    l.push(r);
                }

                Ok(Some(Node {
                    _e: Expression::List(l),
                    _t: Some(t),
                }))
            } else {
                unreachable!()
            }
        }
        Form::Debug => {
            if !settings.debug {
                Ok(None)
            } else {
                let reduced = args
                    .iter()
                    .map(|e| reduce(e, root_ctx.clone(), ctx, settings))
                    .collect::<Result<Vec<_>>>()?;
                match reduced.len() {
                    0 => Ok(None),
                    1 => Ok(reduced[0].to_owned()),
                    _ => Ok(Some(
                        Builtin::Begin.call(
                            &reduced
                                .into_iter()
                                .map(|e| e.unwrap_or_else(|| Expression::Void.into()))
                                .collect::<Vec<_>>(),
                        )?,
                    )),
                }
            }
        }
        Form::Let => {
            let sub_ctx_name = format!("let-{}", ctx.borrow().name);
            let mut sub_ctx =
                SymbolTable::derived(ctx.clone(), &sub_ctx_name, &sub_ctx_name, false, false);
            for pair in args[0].as_list().unwrap().iter() {
                let pair = pair.as_list().unwrap();
                let name = pair[0].as_symbol().unwrap();
                let value = reduce(&pair[1], root_ctx.clone(), &mut sub_ctx, settings)?.unwrap();
                sub_ctx.borrow_mut().insert_symbol(name, value)?;
            }
            let body = reduce(&args[1], root_ctx, &mut sub_ctx, settings)?.unwrap();

            Ok(Some(body))
        }
    }
}

fn apply(
    f: &Function,
    args: &[AstNode],
    root_ctx: Rc<RefCell<SymbolTable>>,
    ctx: &mut Rc<RefCell<SymbolTable>>,
    settings: &CompileSettings,
) -> Result<Option<Node>> {
    if let FunctionClass::SpecialForm(sf) = f.class {
        apply_form(sf, args, root_ctx, ctx, settings)
    } else {
        let mut traversed_args = vec![];
        let mut traversed_args_t = vec![];
        for arg in args.iter() {
            let traversed = reduce(arg, root_ctx.clone(), ctx, settings)?;
            if let Some(traversed) = traversed {
                traversed_args_t.push(traversed.t());
                traversed_args.push(traversed);
            }
        }

        match &f.class {
            FunctionClass::Builtin(b) => {
                b.validate_args(&traversed_args)?;
                match b {
                    // Begin flattens & concatenate any list argument
                    Builtin::Begin => Ok(Some(Node {
                        _e: Expression::List(traversed_args.into_iter().fold(
                            vec![],
                            |mut ax, mut e| match e.e_mut() {
                                Expression::List(ref mut es) => {
                                    ax.append(es);
                                    ax
                                }
                                _ => {
                                    ax.push(e);
                                    ax
                                }
                            },
                        )),
                        _t: Some(
                            traversed_args_t
                                .iter()
                                .fold(Type::INFIMUM, |a, b| a.max(*b)),
                        ),
                    })),

                    b @ (Builtin::IfZero | Builtin::IfNotZero) => {
                        Ok(Some(b.call(&traversed_args)?))
                    }

                    Builtin::Nth => {
                        if let Expression::ArrayColumn(handle, ..) = &traversed_args[0].e() {
                            let i = traversed_args[1].pure_eval()?.to_usize().ok_or_else(|| {
                                anyhow!("{:?} is not a valid indice", traversed_args[1].pure_eval())
                            })?;
                            let array = ctx.borrow_mut().resolve_symbol(&handle.name)?;
                            match array.e() {
                                Expression::ArrayColumn(handle, range) => {
                                    if range.contains(&i) {
                                        Ok(Some(Node {
                                            _e: Expression::Column(
                                                Handle::new(
                                                    &handle.module,
                                                    format!("{}_{}", handle.name, i),
                                                ),
                                                Kind::Atomic,
                                            ),
                                            _t: Some(Type::Column(array.t().magma())),
                                        }))
                                    } else {
                                        bail!("tried to access `{:?}` at index {}", array, i)
                                    }
                                }
                                _ => unimplemented!(),
                            }
                        } else {
                            unreachable!()
                        }
                    }
                    Builtin::Not => Ok(Some(
                        Builtin::Sub
                            .call(&[Node::one(), traversed_args[0].to_owned()])?
                            .with_type(traversed_args[0].t().same_scale(Magma::Boolean)),
                    )),

                    Builtin::Eq => {
                        let x = &traversed_args[0];
                        let y = &traversed_args[1];
                        if traversed_args_t[0].is_bool() && traversed_args_t[1].is_bool() {
                            Ok(Some(Node {
                                _e: Builtin::Mul.raw_call(&[
                                    Builtin::Sub.call(&[x.clone(), y.clone()])?,
                                    Builtin::Sub.call(&[x.clone(), y.clone()])?,
                                ]),
                                // NOTE in this very specific case, we are sure that (x - y)² is boolean
                                _t: Some(x.t().same_scale(Magma::Boolean)),
                            }))
                        } else {
                            Ok(Some(Builtin::Sub.call(&[
                                traversed_args[0].to_owned(),
                                traversed_args[1].to_owned(),
                            ])?))
                        }
                    }

                    b @ (Builtin::Add
                    | Builtin::Sub
                    | Builtin::Mul
                    | Builtin::Exp
                    | Builtin::Neg
                    | Builtin::Inv
                    | Builtin::Shift) => Ok(Some(b.call(&traversed_args)?)),
                    Builtin::Len => {
                        if let Expression::ArrayColumn(_, domain) = traversed_args[0].e() {
                            Ok(Some(Node::from_const(domain.len().try_into().unwrap())))
                        } else {
                            bail!(RuntimeError::NotAnArray(traversed_args[0].e().clone()))
                        }
                    }
                }
            }

            FunctionClass::UserDefined(
                b @ Defined {
                    args: f_args,
                    body,
                    pure,
                },
            ) => {
                let f_mangle = format!(
                    "fn-{}-{}",
                    f.handle,
                    COUNTER
                        .get_or_init(|| AtomicUsize::new(0))
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                );
                b.validate_args(&traversed_args)
                    .with_context(|| anyhow!("validating call to `{}`", f.handle))?;
                let mut f_ctx = SymbolTable::derived(
                    ctx.clone(),
                    &f_mangle,
                    &f.handle.to_string(),
                    *pure,
                    false,
                );
                for (i, f_arg) in f_args.iter().enumerate() {
                    f_ctx
                        .borrow_mut()
                        .insert_symbol(f_arg, traversed_args[i].clone())?;
                }
                reduce(body, root_ctx, &mut f_ctx, settings)
            }
            _ => unimplemented!("{:?}", f),
        }
    }
}

pub fn reduce(
    e: &AstNode,
    root_ctx: Rc<RefCell<SymbolTable>>,
    ctx: &mut Rc<RefCell<SymbolTable>>,
    settings: &CompileSettings,
) -> Result<Option<Node>> {
    match &e.class {
        Token::Keyword(_) | Token::Type(_) | Token::Range(_) => Ok(None),
        Token::Value(x) => Ok(Some(Node {
            _e: Expression::Const(x.clone(), Fr::from_str(&x.to_string())),
            _t: Some(if *x >= Zero::zero() && *x <= One::one() {
                Type::Scalar(Magma::Boolean)
            } else {
                Type::Scalar(Magma::Integer)
            }),
        })),
        Token::Symbol(name) => {
            let r = ctx
                .borrow_mut()
                .resolve_symbol(name)
                .with_context(|| make_ast_error(e))?;
            Ok(Some(r))
        }

        Token::List(args) => {
            if args.is_empty() {
                Ok(Some(Expression::List(vec![]).into()))
            } else if let Token::Symbol(verb) = &args[0].class {
                let func = ctx
                    .borrow()
                    .resolve_function(verb)
                    .with_context(|| make_ast_error(e))?;

                apply(&func, &args[1..], root_ctx, ctx, settings)
            } else {
                Err(anyhow!("not a function: `{:?}`", args[0])).with_context(|| make_ast_error(e))
            }
        }

        Token::DefColumn {
            name,
            t: _,
            kind: k,
        } => match k {
            Kind::Composite(e) => {
                let n = reduce(e, root_ctx, ctx, settings)?.unwrap();
                ctx.borrow_mut().edit_symbol(name, &|x| {
                    if let Expression::Column(_, kind) = x {
                        *kind = Kind::Composite(Box::new(n.clone()))
                    }
                })?;
                Ok(None)
            }
            Kind::Interleaved(froms, _) => {
                let from_handles = froms
                    .iter()
                    .map(
                        |f| match reduce(f, root_ctx.clone(), ctx, settings)?.unwrap().e() {
                            Expression::Column(h, _) => Ok(h.to_owned()),
                            x => Err(anyhow!("expected column, found {:?}", x)),
                        },
                    )
                    .collect::<Result<Vec<_>>>()
                    .with_context(|| anyhow!("while defining {}", name))?;

                ctx.borrow_mut().edit_symbol(name, &|x| {
                    if let Expression::Column(_, kind) = x {
                        *kind = Kind::Interleaved(vec![], Some(from_handles.to_vec()))
                    }
                })?;
                Ok(None)
            }
            _ => Ok(None),
        },
        Token::DefColumns(_)
        | Token::DefConstraint { .. }
        | Token::DefArrayColumn { .. }
        | Token::DefModule(_)
        | Token::DefAliases(_)
        | Token::DefAlias(..)
        | Token::DefunAlias(..)
        | Token::DefConsts(..)
        | Token::Defun { .. }
        | Token::Defpurefun { .. }
        | Token::DefPermutation { .. }
        | Token::DefPlookup { .. }
        | Token::DefInrange(..) => Ok(None),
    }
    .with_context(|| make_ast_error(e))
}

fn reduce_toplevel(
    e: &AstNode,
    root_ctx: Rc<RefCell<SymbolTable>>,
    ctx: &mut Rc<RefCell<SymbolTable>>,
    settings: &CompileSettings,
) -> Result<Option<Constraint>> {
    match &e.class {
        Token::DefConstraint {
            name,
            domain,
            guard,
            body: expr,
        } => {
            let handle = Handle::new(&ctx.borrow().name, name);
            Ok(Some(Constraint::Vanishes {
                handle,
                domain: domain.to_owned(),
                expr: Box::new({
                    let expr = reduce(expr, root_ctx.clone(), ctx, settings)?
                        .unwrap_or_else(|| Expression::Void.into());
                    if let Some(guard) = guard {
                        let guard_expr = reduce(guard, root_ctx, ctx, settings)?
                            .with_context(|| anyhow!("guard `{:?}` is empty", guard))?;
                        Builtin::IfNotZero.call(&[guard_expr, expr])?
                    } else {
                        expr
                    }
                }),
            }))
        }
        Token::DefPlookup {
            name,
            including: parent,
            included: child,
        } => {
            let handle = Handle::new(&ctx.borrow().name, name);
            let parents = parent
                .iter()
                .map(|e| reduce(e, root_ctx.clone(), ctx, settings).map(Option::unwrap))
                .collect::<Result<Vec<_>>>()?;
            let children = child
                .iter()
                .map(|e| reduce(e, root_ctx.clone(), ctx, settings).map(Option::unwrap))
                .collect::<Result<Vec<_>>>()?;
            if parents.len() != children.len() {
                bail!(
                    "in {}, parents and children have different lengths: {} and {}",
                    name.red(),
                    parents.len(),
                    children.len()
                )
            } else {
                Ok(Some(Constraint::Plookup {
                    handle,
                    including: parents,
                    included: children,
                }))
            }
        }
        Token::DefInrange(e, range) => {
            let handle = Handle::new(
                &ctx.borrow().name,
                names::Generator::default().next().unwrap(),
            );
            Ok(Some(Constraint::InRange {
                handle,
                exp: reduce(e, root_ctx, ctx, settings)?.unwrap(),
                max: Fr::from_str(&range.to_string())
                    .ok_or_else(|| anyhow!("`{range}` is not representable in Fr"))?,
            }))
        }
        Token::DefColumns(columns) => {
            for c in columns {
                reduce(c, root_ctx.clone(), ctx, settings)?;
            }
            Ok(None)
        }
        Token::DefModule(name) => {
            *ctx = SymbolTable::derived(root_ctx, name, name, false, true);
            Ok(None)
        }
        Token::Value(_) | Token::Symbol(_) | Token::List(_) | Token::Range(_) => {
            bail!("Unexpected top-level form: {:?}", e)
        }
        Token::Defun { .. }
        | Token::Defpurefun { .. }
        | Token::DefAliases(_)
        | Token::DefunAlias(..)
        | Token::DefConsts(..) => Ok(None),
        Token::DefPermutation { from, to } => {
            // We look up the columns involved in the permutation just to ensure that they
            // are marked as "used" in the symbol table
            from.iter()
                .map(|f| ctx.borrow_mut().resolve_symbol(f))
                .collect::<Result<Vec<_>>>()
                .with_context(|| anyhow!("while defining permutation"))?;
            to.iter()
                .map(|f| ctx.borrow_mut().resolve_symbol(f))
                .collect::<Result<Vec<_>>>()
                .with_context(|| anyhow!("while defining permutation"))?;

            Ok(Some(Constraint::Permutation {
                handle: Handle::new(
                    &ctx.borrow().name,
                    names::Generator::default().next().unwrap(),
                ),
                from: from
                    .iter()
                    .map(|f| Handle::new(&ctx.borrow().name, f))
                    .collect::<Vec<_>>(),
                to: to
                    .iter()
                    .map(|f| Handle::new(&ctx.borrow().name, f))
                    .collect::<Vec<_>>(),
            }))
        }
        _ => unreachable!("{:?}", e.src),
    }
}

pub fn make_ast_error(exp: &AstNode) -> String {
    make_src_error(&exp.src, exp.lc)
}

pub fn pass(
    ast: &Ast,
    ctx: Rc<RefCell<SymbolTable>>,
    settings: &CompileSettings,
) -> Result<Vec<Constraint>> {
    let mut r = vec![];

    let mut module = ctx.clone();
    for exp in ast.exprs.iter() {
        if let Some(c) = reduce_toplevel(exp, ctx.clone(), &mut module, settings)? {
            r.push(c)
        }
    }
    Ok(r)
}
