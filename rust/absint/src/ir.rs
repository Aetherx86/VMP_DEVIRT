//! The program representation every frontend lowers to except x86, and its exact concrete meaning.
//!
//! Integers here are mathematical, and machine wrapping is an explicit instruction. So every claim the
//! analyzer makes can be checked against [`run`], which is how the tests establish soundness.

use crate::domain::Interval;
use std::collections::HashMap;

pub type Var = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operand {
    Var(Var),
    Const(i128),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    And,
    Or,
    Xor,
    Shl,
    Shr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

impl CmpOp {
    pub fn negate(self) -> CmpOp {
        match self {
            CmpOp::Lt => CmpOp::Ge,
            CmpOp::Le => CmpOp::Gt,
            CmpOp::Gt => CmpOp::Le,
            CmpOp::Ge => CmpOp::Lt,
            CmpOp::Eq => CmpOp::Ne,
            CmpOp::Ne => CmpOp::Eq,
        }
    }
}

/// A comparison. With `signed_bits`, the operands are unsigned representations of machine integers
/// of that width, compared as signed ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cond {
    pub op: CmpOp,
    pub a: Operand,
    pub b: Operand,
    pub signed_bits: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Instr {
    Copy(Var, Operand),
    Bin(Var, BinOp, Operand, Operand),
    /// Reduces modulo `2^bits` into the unsigned range.
    Wrap(Var, Operand, u32),
    /// Any value in the range: an input, or something the frontend does not model.
    Havoc(Var, Interval),
    /// A new allocation of `size` cells. With `nullable`, the result may be null instead.
    Alloc {
        dst: Var,
        size: Operand,
        nullable: bool,
    },
    Null(Var),
    Load {
        dst: Var,
        ptr: Var,
        index: Operand,
    },
    Store {
        ptr: Var,
        index: Operand,
        value: Operand,
    },
    /// An access to a linear memory of at least `limit` bytes.
    MemAccess {
        addr: Operand,
        offset: u64,
        size: u32,
        limit: u128,
    },
    Assert(Cond),
    /// Execution only continues if this holds.
    Assume(Cond),
    ReadGlobal {
        dst: Var,
        global: u32,
    },
    WriteGlobal {
        global: u32,
        value: Operand,
    },
    Lock(u32),
    Unlock(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Term {
    Goto(usize),
    Branch(Cond, usize, usize),
    Return,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub name: String,
    /// Each instruction and the source line it came from, if there was one.
    pub instrs: Vec<(Instr, u32)>,
    pub term: Term,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadKind {
    /// Not started as a thread.
    None,
    One,
    /// Several instances can run at once, so it can race with itself.
    Many,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    pub thread: ThreadKind,
    pub params: Vec<(Var, Interval)>,
    /// The first block is the entry.
    pub blocks: Vec<Block>,
    pub var_names: Vec<String>,
}

impl Function {
    pub fn var_count(&self) -> usize {
        self.var_names.len()
    }

    pub fn successors(&self, block: usize) -> Vec<usize> {
        match self.blocks[block].term {
            Term::Goto(t) => vec![t],
            Term::Branch(_, a, b) => vec![a, b],
            Term::Return => Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Program {
    pub functions: Vec<Function>,
    pub globals: Vec<String>,
    pub locks: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Concrete {
    Int(i128),
    /// An allocation number, and its size.
    Ptr(u32, i128),
    Null,
}

/// What can go wrong, and what the analyzer has to check for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Fault {
    Uninitialized,
    NotAnInteger,
    NotAPointer,
    NullDereference,
    OutOfBounds,
    NegativeAllocation,
    DivisionByZero,
    AssertionFailed,
}

impl Fault {
    pub fn describe(self) -> &'static str {
        match self {
            Fault::Uninitialized => "use of an uninitialized variable",
            Fault::NotAnInteger => "arithmetic on a pointer",
            Fault::NotAPointer => "dereference of something that is not a pointer",
            Fault::NullDereference => "null dereference",
            Fault::OutOfBounds => "out of bounds access",
            Fault::NegativeAllocation => "allocation with a negative size",
            Fault::DivisionByZero => "division by zero",
            Fault::AssertionFailed => "assertion failure",
        }
    }
}

/// Where a run ended up, for the soundness tests.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Returned,
    /// A failed assumption, which is not an error and only means this path does not exist.
    Infeasible,
    OutOfSteps,
    Fault {
        fault: Fault,
        block: usize,
        instr: usize,
    },
}

fn compare(op: CmpOp, a: i128, b: i128) -> bool {
    match op {
        CmpOp::Lt => a < b,
        CmpOp::Le => a <= b,
        CmpOp::Gt => a > b,
        CmpOp::Ge => a >= b,
        CmpOp::Eq => a == b,
        CmpOp::Ne => a != b,
    }
}

pub fn to_signed(v: i128, bits: u32) -> i128 {
    let wrapped = v.rem_euclid(1i128 << bits);
    if wrapped >= 1i128 << (bits - 1) {
        wrapped - (1i128 << bits)
    } else {
        wrapped
    }
}

pub fn eval_bin(op: BinOp, a: i128, b: i128) -> Option<i128> {
    Some(match op {
        BinOp::Add => a.checked_add(b)?,
        BinOp::Sub => a.checked_sub(b)?,
        BinOp::Mul => a.checked_mul(b)?,
        BinOp::Div => a.checked_div(b)?,
        BinOp::Rem => a.checked_rem(b)?,
        BinOp::And => a & b,
        BinOp::Or => a | b,
        BinOp::Xor => a ^ b,
        BinOp::Shl => {
            if !(0..=100).contains(&b) {
                return None;
            }
            a.checked_mul(1i128 << b)?
        }
        BinOp::Shr => {
            if !(0..=100).contains(&b) {
                return None;
            }
            a >> b
        }
    })
}

#[allow(clippy::type_complexity)]
/// Runs a function on concrete inputs, calling `observe` before each instruction and terminator with
/// the block, the instruction index, and the variables. Global and havoc values come from `choose`.
pub fn run(
    function: &Function,
    args: &[i128],
    max_steps: u32,
    choose: &mut dyn FnMut(Interval) -> i128,
    observe: &mut dyn FnMut(usize, usize, &[Option<Concrete>]),
) -> Outcome {
    let mut vars: Vec<Option<Concrete>> = vec![None; function.var_count()];
    for ((var, _), value) in function.params.iter().zip(args) {
        vars[*var as usize] = Some(Concrete::Int(*value));
    }
    let mut block = 0;
    let mut steps = 0;
    let mut next_alloc = 0;
    let mut memory: HashMap<(u32, i128), Concrete> = HashMap::new();
    loop {
        for (i, (instr, _)) in function.blocks[block].instrs.iter().enumerate() {
            observe(block, i, &vars);
            steps += 1;
            if steps > max_steps {
                return Outcome::OutOfSteps;
            }
            let fault = |fault| Outcome::Fault {
                fault,
                block,
                instr: i,
            };
            let get = |o: Operand| match o {
                Operand::Const(c) => Ok(Concrete::Int(c)),
                Operand::Var(v) => vars[v as usize].ok_or(Fault::Uninitialized),
            };
            let int = |o: Operand| match get(o)? {
                Concrete::Int(v) => Ok(v),
                _ => Err(Fault::NotAnInteger),
            };
            macro_rules! tri {
                ($e:expr) => {
                    match $e {
                        Ok(v) => v,
                        Err(f) => return fault(f),
                    }
                };
            }
            match instr {
                Instr::Copy(d, o) => vars[*d as usize] = Some(tri!(get(*o))),
                Instr::Bin(d, op, a, b) => {
                    let (x, y) = (tri!(int(*a)), tri!(int(*b)));
                    if matches!(op, BinOp::Div | BinOp::Rem) && y == 0 {
                        return fault(Fault::DivisionByZero);
                    }
                    match eval_bin(*op, x, y) {
                        Some(v) => vars[*d as usize] = Some(Concrete::Int(v)),
                        None => return Outcome::OutOfSteps,
                    }
                }
                Instr::Wrap(d, o, bits) => {
                    let v = tri!(int(*o));
                    vars[*d as usize] = Some(Concrete::Int(v.rem_euclid(1i128 << bits)));
                }
                Instr::Havoc(d, range) => vars[*d as usize] = Some(Concrete::Int(choose(*range))),
                Instr::Alloc {
                    dst,
                    size,
                    nullable,
                } => {
                    let n = tri!(int(*size));
                    if n < 0 {
                        return fault(Fault::NegativeAllocation);
                    }
                    if *nullable && choose(Interval::new(0, 1)) == 0 {
                        vars[*dst as usize] = Some(Concrete::Null);
                    } else {
                        next_alloc += 1;
                        vars[*dst as usize] = Some(Concrete::Ptr(next_alloc, n));
                    }
                }
                Instr::Null(d) => vars[*d as usize] = Some(Concrete::Null),
                Instr::Load { ptr, index, .. } | Instr::Store { ptr, index, .. } => {
                    let target = tri!(get(Operand::Var(*ptr)));
                    let at = tri!(int(*index));
                    let (id, size) = match target {
                        Concrete::Null => return fault(Fault::NullDereference),
                        Concrete::Int(_) => return fault(Fault::NotAPointer),
                        Concrete::Ptr(id, size) => (id, size),
                    };
                    if at < 0 || at >= size {
                        return fault(Fault::OutOfBounds);
                    }
                    match instr {
                        Instr::Load { dst, .. } => {
                            // Unwritten cells read as zero.
                            let v = memory.get(&(id, at)).copied().unwrap_or(Concrete::Int(0));
                            vars[*dst as usize] = Some(v);
                        }
                        Instr::Store { value, .. } => {
                            let v = tri!(get(*value));
                            memory.insert((id, at), v);
                        }
                        _ => unreachable!(),
                    }
                }
                Instr::MemAccess {
                    addr,
                    offset,
                    size,
                    limit,
                } => {
                    let a = tri!(int(*addr));
                    if a + i128::from(*offset) + i128::from(*size) > *limit as i128 {
                        return fault(Fault::OutOfBounds);
                    }
                }
                Instr::Assert(c) => {
                    if !tri!(eval_cond(c, &vars)) {
                        return fault(Fault::AssertionFailed);
                    }
                }
                Instr::Assume(c) => {
                    if !tri!(eval_cond(c, &vars)) {
                        return Outcome::Infeasible;
                    }
                }
                Instr::ReadGlobal { dst, .. } => {
                    vars[*dst as usize] = Some(Concrete::Int(choose(Interval::TOP)))
                }
                Instr::WriteGlobal { value, .. } => {
                    tri!(get(*value));
                }
                Instr::Lock(_) | Instr::Unlock(_) => {}
            }
        }
        let end = function.blocks[block].instrs.len();
        observe(block, end, &vars);
        steps += 1;
        if steps > max_steps {
            return Outcome::OutOfSteps;
        }
        block = match &function.blocks[block].term {
            Term::Goto(t) => *t,
            Term::Return => return Outcome::Returned,
            Term::Branch(c, a, b) => match eval_cond(c, &vars) {
                Ok(true) => *a,
                Ok(false) => *b,
                Err(fault) => {
                    return Outcome::Fault {
                        fault,
                        block,
                        instr: end,
                    };
                }
            },
        };
    }
}

pub fn eval_cond(c: &Cond, vars: &[Option<Concrete>]) -> Result<bool, Fault> {
    let get = |o: Operand| match o {
        Operand::Const(v) => Ok(Concrete::Int(v)),
        Operand::Var(v) => vars[v as usize].ok_or(Fault::Uninitialized),
    };
    let (a, b) = (get(c.a)?, get(c.b)?);
    match (a, b) {
        (Concrete::Int(x), Concrete::Int(y)) => Ok(match c.signed_bits {
            Some(bits) => compare(c.op, to_signed(x, bits), to_signed(y, bits)),
            None => compare(c.op, x, y),
        }),
        // A pointer can only be tested for being null, or against another pointer.
        (Concrete::Null, Concrete::Int(0))
        | (Concrete::Int(0), Concrete::Null)
        | (Concrete::Null, Concrete::Null)
            if matches!(c.op, CmpOp::Eq | CmpOp::Ne) =>
        {
            Ok(c.op == CmpOp::Eq)
        }
        (Concrete::Ptr(..), Concrete::Int(0) | Concrete::Null)
        | (Concrete::Int(0) | Concrete::Null, Concrete::Ptr(..))
            if matches!(c.op, CmpOp::Eq | CmpOp::Ne) =>
        {
            Ok(c.op == CmpOp::Ne)
        }
        (Concrete::Ptr(x, _), Concrete::Ptr(y, _)) if matches!(c.op, CmpOp::Eq | CmpOp::Ne) => {
            Ok((x == y) == (c.op == CmpOp::Eq))
        }
        _ => Err(Fault::NotAnInteger),
    }
}
