//! A small text format for writing programs by hand.
//!
//! ```text
//! global counter
//! lock m
//!
//! fn fill(n: 0..64) {
//! entry:
//!   buf = alloc 64
//!   i = 0
//! head:
//!   if i < n goto body else done
//! body:
//!   store buf[i] = i
//!   i = i + 1
//!   goto head
//! done:
//!   return
//! }
//!
//! threads worker() {
//!   lock m
//!   v = read counter
//!   v = v + 1
//!   write counter = v
//!   unlock m
//! }
//! ```

use crate::domain::Interval;
use crate::ir::{
    BinOp, Block, CmpOp, Cond, Function, Instr, Operand, Program, Term, ThreadKind, Var,
};
use std::collections::HashMap;

#[derive(Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: u32,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

#[allow(clippy::type_complexity)]
struct Draft {
    name: String,
    thread: ThreadKind,
    params: Vec<(Var, Interval)>,
    var_names: Vec<String>,
    vars: HashMap<String, Var>,
    /// Each block's name, instructions, and terminator in terms of labels.
    blocks: Vec<(String, Vec<(Instr, u32)>, Option<(RawTerm, u32)>)>,
}

enum RawTerm {
    Goto(String),
    Branch(Cond, String, String),
    Return,
}

impl Draft {
    fn var(&mut self, name: &str) -> Var {
        if let Some(v) = self.vars.get(name) {
            return *v;
        }
        let v = self.var_names.len() as Var;
        self.var_names.push(name.to_string());
        self.vars.insert(name.to_string(), v);
        v
    }

    fn operand(&mut self, token: &str, line: u32) -> Result<Operand, ParseError> {
        if let Ok(c) = token.parse::<i128>() {
            return Ok(Operand::Const(c));
        }
        if token
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Ok(Operand::Var(self.var(token)));
        }
        Err(ParseError {
            line,
            message: format!("`{token}` is not a variable or a number"),
        })
    }

    fn target(&mut self, token: &str, line: u32) -> Result<Var, ParseError> {
        match self.operand(token, line)? {
            Operand::Var(v) => Ok(v),
            Operand::Const(_) => Err(ParseError {
                line,
                message: format!("`{token}` is a number, and a variable is needed"),
            }),
        }
    }

    fn cond(&mut self, a: &str, op: &str, b: &str, line: u32) -> Result<Cond, ParseError> {
        let op = match op {
            "<" => CmpOp::Lt,
            "<=" => CmpOp::Le,
            ">" => CmpOp::Gt,
            ">=" => CmpOp::Ge,
            "==" => CmpOp::Eq,
            "!=" => CmpOp::Ne,
            other => {
                return Err(ParseError {
                    line,
                    message: format!("`{other}` is not a comparison"),
                });
            }
        };
        Ok(Cond {
            op,
            a: self.operand(a, line)?,
            b: self.operand(b, line)?,
            signed_bits: None,
        })
    }

    fn push(&mut self, instr: Instr, line: u32) {
        if self.blocks.is_empty() || self.blocks.last().unwrap().2.is_some() {
            let name = format!("@{}", self.blocks.len());
            self.blocks.push((name, Vec::new(), None));
        }
        self.blocks.last_mut().unwrap().1.push((instr, line));
    }

    fn terminate(&mut self, term: RawTerm, line: u32) {
        if self.blocks.is_empty() || self.blocks.last().unwrap().2.is_some() {
            let name = format!("@{}", self.blocks.len());
            self.blocks.push((name, Vec::new(), None));
        }
        self.blocks.last_mut().unwrap().2 = Some((term, line));
    }

    fn finish(self) -> Result<Function, ParseError> {
        let mut blocks = self.blocks;
        if blocks.is_empty() {
            blocks.push(("@0".to_string(), Vec::new(), Some((RawTerm::Return, 0))));
        }
        let index: HashMap<String, usize> = blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.0.clone(), i))
            .collect();
        let count = blocks.len();
        let mut out = Vec::with_capacity(count);
        for (i, (name, instrs, term)) in blocks.into_iter().enumerate() {
            let find = |label: &str, line: u32| {
                index.get(label).copied().ok_or_else(|| ParseError {
                    line,
                    message: format!("there is no label `{label}`"),
                })
            };
            let term = match term {
                Some((RawTerm::Return, _)) => Term::Return,
                Some((RawTerm::Goto(t), line)) => Term::Goto(find(&t, line)?),
                Some((RawTerm::Branch(c, a, b), line)) => {
                    Term::Branch(c, find(&a, line)?, find(&b, line)?)
                }
                // Falling off the end of a block goes into the next one, or returns.
                None if i + 1 < count => Term::Goto(i + 1),
                None => Term::Return,
            };
            out.push(Block { name, instrs, term });
        }
        Ok(Function {
            name: self.name,
            thread: self.thread,
            params: self.params,
            blocks: out,
            var_names: self.var_names,
        })
    }
}

fn tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
        } else if "[]=<>!+*/%&|^(),:{}".contains(c) || (c == '-' && !word.is_empty()) {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            let mut symbol = c.to_string();
            if let Some(next) = chars.peek().copied() {
                let pair = format!("{c}{next}");
                if ["<=", ">=", "==", "!=", "<<", ">>"].contains(&pair.as_str()) {
                    symbol = pair;
                    chars.next();
                }
            }
            out.push(symbol);
        } else {
            word.push(c);
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

fn range_from(text: &str, line: u32) -> Result<Interval, ParseError> {
    let bad = || ParseError {
        line,
        message: format!("`{text}` is not a range such as 0..100"),
    };
    let (a, b) = text.split_once("..").ok_or_else(bad)?;
    let (a, b): (i128, i128) = (a.parse().map_err(|_| bad())?, b.parse().map_err(|_| bad())?);
    if a > b {
        return Err(bad());
    }
    Ok(Interval::new(a, b))
}

pub fn parse(source: &str) -> Result<Program, ParseError> {
    let mut program = Program::default();
    let mut draft: Option<Draft> = None;
    for (index, raw) in source.lines().enumerate() {
        let line = index as u32 + 1;
        let text = raw
            .split('#')
            .next()
            .unwrap_or("")
            .split("//")
            .next()
            .unwrap_or("")
            .trim();
        if text.is_empty() {
            continue;
        }
        let t = tokens(text);
        let t: Vec<&str> = t.iter().map(String::as_str).collect();
        let err = |message: String| ParseError { line, message };

        let Some(d) = draft.as_mut() else {
            match t.as_slice() {
                ["global", name] => program.globals.push((*name).to_string()),
                ["lock", name] => program.locks.push((*name).to_string()),
                [kind @ ("fn" | "thread" | "threads"), name, "(", rest @ ..] => {
                    let thread = match *kind {
                        "fn" => ThreadKind::None,
                        "thread" => ThreadKind::One,
                        _ => ThreadKind::Many,
                    };
                    let mut new = Draft {
                        name: (*name).to_string(),
                        thread,
                        params: Vec::new(),
                        var_names: Vec::new(),
                        vars: HashMap::new(),
                        blocks: Vec::new(),
                    };
                    if rest.last() != Some(&"{")
                        || rest.get(rest.len().wrapping_sub(2)) != Some(&")")
                    {
                        return Err(err("a function header has to end with `) {`".into()));
                    }
                    for param in rest[..rest.len() - 2].split(|p| *p == ",") {
                        match param {
                            [] => {}
                            [p] => {
                                let v = new.target(p, line)?;
                                new.params.push((v, Interval::TOP));
                            }
                            [p, ":", range] => {
                                let v = new.target(p, line)?;
                                new.params.push((v, range_from(range, line)?));
                            }
                            other => {
                                return Err(err(format!(
                                    "cannot read the parameter `{}`",
                                    other.join(" ")
                                )));
                            }
                        }
                    }
                    draft = Some(new);
                }
                _ => return Err(err(format!("expected a declaration, and found `{text}`"))),
            }
            continue;
        };

        match t.as_slice() {
            ["}"] => program.functions.push(draft.take().unwrap().finish()?),
            [label, ":"] => {
                if d.blocks.iter().any(|b| b.0 == *label) {
                    return Err(err(format!("the label `{label}` is used twice")));
                }
                d.blocks.push(((*label).to_string(), Vec::new(), None));
            }
            ["goto", label] => d.terminate(RawTerm::Goto((*label).to_string()), line),
            ["return"] => d.terminate(RawTerm::Return, line),
            ["if", a, op, b, "goto", yes, "else", no] => {
                let c = d.cond(a, op, b, line)?;
                d.terminate(
                    RawTerm::Branch(c, (*yes).to_string(), (*no).to_string()),
                    line,
                );
            }
            ["assert", a, op, b] => {
                let c = d.cond(a, op, b, line)?;
                d.push(Instr::Assert(c), line);
            }
            ["assume", a, op, b] => {
                let c = d.cond(a, op, b, line)?;
                d.push(Instr::Assume(c), line);
            }
            ["store", p, "[", i, "]", "=", v] => {
                let (ptr, index, value) =
                    (d.target(p, line)?, d.operand(i, line)?, d.operand(v, line)?);
                d.push(Instr::Store { ptr, index, value }, line);
            }
            ["write", g, "=", v] => {
                let global = find(&program.globals, g)
                    .ok_or_else(|| err(format!("no global is named `{g}`")))?;
                let value = d.operand(v, line)?;
                d.push(Instr::WriteGlobal { global, value }, line);
            }
            ["lock", l] => {
                let id = find(&program.locks, l)
                    .ok_or_else(|| err(format!("no lock is named `{l}`")))?;
                d.push(Instr::Lock(id), line);
            }
            ["unlock", l] => {
                let id = find(&program.locks, l)
                    .ok_or_else(|| err(format!("no lock is named `{l}`")))?;
                d.push(Instr::Unlock(id), line);
            }
            [x, "=", "null"] => {
                let v = d.target(x, line)?;
                d.push(Instr::Null(v), line);
            }
            [x, "=", kind @ ("alloc" | "maybe_alloc"), size] => {
                let (dst, size) = (d.target(x, line)?, d.operand(size, line)?);
                d.push(
                    Instr::Alloc {
                        dst,
                        size,
                        nullable: *kind == "maybe_alloc",
                    },
                    line,
                );
            }
            [x, "=", "load", p, "[", i, "]"] => {
                let (dst, ptr, index) =
                    (d.target(x, line)?, d.target(p, line)?, d.operand(i, line)?);
                d.push(Instr::Load { dst, ptr, index }, line);
            }
            [x, "=", "read", g] => {
                let global = find(&program.globals, g)
                    .ok_or_else(|| err(format!("no global is named `{g}`")))?;
                let dst = d.target(x, line)?;
                d.push(Instr::ReadGlobal { dst, global }, line);
            }
            [x, "=", "input"] => {
                let v = d.target(x, line)?;
                d.push(Instr::Havoc(v, Interval::TOP), line);
            }
            [x, "=", "input", range] => {
                let v = d.target(x, line)?;
                d.push(Instr::Havoc(v, range_from(range, line)?), line);
            }
            [x, "=", o] => {
                let (v, o) = (d.target(x, line)?, d.operand(o, line)?);
                d.push(Instr::Copy(v, o), line);
            }
            [x, "=", a, op, b] => {
                let op = match *op {
                    "+" => BinOp::Add,
                    "-" => BinOp::Sub,
                    "*" => BinOp::Mul,
                    "/" => BinOp::Div,
                    "%" => BinOp::Rem,
                    "&" => BinOp::And,
                    "|" => BinOp::Or,
                    "^" => BinOp::Xor,
                    "<<" => BinOp::Shl,
                    ">>" => BinOp::Shr,
                    other => return Err(err(format!("`{other}` is not an operator"))),
                };
                let (v, a, b) = (d.target(x, line)?, d.operand(a, line)?, d.operand(b, line)?);
                d.push(Instr::Bin(v, op, a, b), line);
            }
            _ => return Err(err(format!("cannot read `{text}`"))),
        }
    }
    if let Some(d) = draft {
        return Err(ParseError {
            line: source.lines().count() as u32,
            message: format!("`{}` has no closing brace", d.name),
        });
    }
    Ok(program)
}

fn find(names: &[String], name: &str) -> Option<u32> {
    names.iter().position(|n| n == name).map(|i| i as u32)
}
