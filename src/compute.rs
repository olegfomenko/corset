use eyre::*;
use log::*;
use pairing_ce::{
    bn256::Fr,
    ff::{Field, PrimeField},
};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

use crate::{
    column::ColumnSet,
    compiler::{ConstraintSet, Handle, Type},
};

type F = Fr;

#[derive(Default, Serialize, Debug)]
pub struct ComputeResult {
    pub columns: HashMap<String, Vec<F>>,
}

fn validate(t: Type, x: F) -> Result<F> {
    match t {
        Type::Boolean => {
            if x.is_zero() || x == F::one() {
                Ok(x)
            } else {
                Err(eyre!("expected bool, found {}", x))
            }
        }
        Type::Numeric => Ok(x),
        _ => unreachable!(),
    }
}
fn parse_column(xs: &[Value], t: Type) -> Result<Vec<F>> {
    xs.iter()
        .map(|x| match x {
            Value::Number(n) => Fr::from_str(&n.to_string())
                .with_context(|| format!("while parsing `{:?}`", x))
                .and_then(|x| validate(t, x)),
            Value::String(s) => Fr::from_str(s)
                .with_context(|| format!("while parsing `{:?}`", x))
                .and_then(|x| validate(t, x)),
            _ => Err(eyre!("expected numeric value, found `{}`", x)),
        })
        .collect()
}

fn fill_traces(v: &Value, path: Vec<String>, columns: &mut ColumnSet<F>) -> Result<()> {
    match v {
        Value::Object(map) => {
            for (k, v) in map.iter() {
                if k == "Trace" || k == "Assignment" {
                    info!("importing {:?}", path[path.len() - 1]);
                    fill_traces(v, path.clone(), columns)?;
                } else {
                    let mut path = path.clone();
                    path.push(k.to_owned());
                    fill_traces(v, path, columns)?;
                }
            }
            Ok(())
        }
        Value::Null => Ok(()),
        Value::Bool(_) => Ok(()),
        Value::Number(_) => Ok(()),
        Value::String(_) => Ok(()),
        Value::Array(xs) => {
            if path.len() >= 2 {
                let module = &path[path.len() - 2];
                let colname = &path[path.len() - 1];

                let _r = columns
                    .cols
                    .get_mut(module)
                    .and_then(|module| module.get_mut(colname))
                    .map(|column| column.set_value(parse_column(xs, column.t).unwrap()));
                // if let Err(e) = r {
                //     debug!("{}", e);
                // }
            }
            Ok(())
        }
    }
}

fn pad(r: &mut ColumnSet<F>) -> Result<()> {
    if r.is_empty() {
        return Ok(());
    }
    let max_len = r.len();
    let pad_to = (max_len + 1).next_power_of_two();
    let padded = max_len - pad_to;
    let _255 = Fr::from_str("255").unwrap();

    r.cols
        .values_mut()
        .flat_map(|module| module.values_mut())
        .for_each(|x| {
            x.map(&|xs| {
                xs.reverse();
                xs.resize(pad_to, Fr::zero());
                xs.reverse();
            })
        });

    if let Some(col) = r.cols.get_mut("binary").and_then(|m| m.get_mut("NOT")) {
        col.map(&|xs| {
            for x in xs.iter_mut().take(padded) {
                *x = _255;
            }
        });
    }

    Ok(())
}

pub fn compute(tracefile: &str, cs: &mut ConstraintSet) -> Result<ComputeResult> {
    let v: Value = serde_json::from_str(
        &std::fs::read_to_string(tracefile)
            .with_context(|| format!("while reading `{}`", tracefile))?,
    )?;

    fill_traces(&v, vec![], &mut cs.columns)
        .with_context(|| eyre!("reading columns from `{}`", tracefile))?;
    pad(&mut cs.columns).with_context(|| "padding columns")?;
    cs.compute_all().with_context(|| "computing columns")?;

    let mut r = ComputeResult::default();
    for (module, columns) in cs.columns.cols.iter_mut() {
        for (name, col) in columns.iter_mut() {
            let handle = Handle::new(&module, &name);
            r.columns.insert(handle.mangle(), col.value().unwrap());
        }
    }
    Ok(r)
}
