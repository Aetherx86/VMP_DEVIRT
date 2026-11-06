//! The fixpoint engine: forward interval and nullness analysis over a control-flow graph.
//!
//! Each block has an over-approximation of the states that can reach it. Loops converge through
//! widening to the constants the program mentions, and precision is then recovered by narrowing. A
//! variable missing from a state may be uninitialized there.

use crate::domain::{INF, Interval, Nullness, Value};
use crate::ir::{BinOp, CmpOp, Cond, Fault, Function, Instr, Operand, Term, Var, to_signed};
use std::collections::BTreeSet;

/// One abstract value per variable. `None` means possibly uninitialized.
pub type State = Vec<Option<Value>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    /// It cannot happen on any execution.
    Proved,
    /// The analysis could not rule it out, which may or may not be a real bug.
    Possible,
    /// It happens every time execution gets here.
    Definite,
}

/// One thing that was checked at one place.
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub block: usize,
    pub instr: usize,
    pub line: u32,
    pub fault: Fault,
    pub verdict: Verdict,
    /// Whether nothing can reach this, which makes its verdict trivial.
    pub dead: bool,
}

pub struct Analysis<'a> {
    pub function: &'a Function,
    /// The state at the start of each block, or `None` if it is unreachable.
    pub entry: Vec<Option<State>>,
    pub checks: Vec<Check>,
}

fn operand(state: &State, o: Operand) -> Option<Value> {
    match o {
        Operand::Const(c) => Some(Value::Int(Interval::exact(c))),
        Operand::Var(v) => state[v as usize],
    }
}

fn store_var(state: &mut State, var: Var, value: Value) {
    state[var as usize] = Some(value);
}

/// Whether a comparison settles one way, for operands that are known integers.
fn decide(op: CmpOp, a: Interval, b: Interval) -> Option<bool> {
    let apart = a.hi < b.lo || b.hi < a.lo;
    match op {
        CmpOp::Lt if a.hi < b.lo => Some(true),
        CmpOp::Lt if a.lo >= b.hi => Some(false),
        CmpOp::Le if a.hi <= b.lo => Some(true),
        CmpOp::Le if a.lo > b.hi => Some(false),
        CmpOp::Gt if a.lo > b.hi => Some(true),
        CmpOp::Gt if a.hi <= b.lo => Some(false),
        CmpOp::Ge if a.lo >= b.hi => Some(true),
        CmpOp::Ge if a.hi < b.lo => Some(false),
        CmpOp::Eq if apart => Some(false),
        CmpOp::Ne if apart => Some(true),
        CmpOp::Eq if a.lo == a.hi && a == b => Some(true),
        CmpOp::Ne if a.lo == a.hi && a == b => Some(false),
        _ => None,
    }
}

/// Narrows two intervals by a comparison that holds. `None` if it cannot hold.
fn refine_pair(op: CmpOp, a: Interval, b: Interval) -> Option<(Interval, Interval)> {
    let below = |x: i128| if x >= INF { INF } else { x - 1 };
    let above = |x: i128| if x <= -INF { -INF } else { x + 1 };
    match op {
        CmpOp::Lt => {
            let a2 = a.meet(Interval {
                lo: -INF,
                hi: below(b.hi),
            })?;
            let b2 = b.meet(Interval {
                lo: above(a.lo),
                hi: INF,
            })?;
            Some((a2, b2))
        }
        CmpOp::Le => {
            let a2 = a.meet(Interval { lo: -INF, hi: b.hi })?;
            let b2 = b.meet(Interval { lo: a.lo, hi: INF })?;
            Some((a2, b2))
        }
        CmpOp::Gt => refine_pair(CmpOp::Lt, b, a).map(|(y, x)| (x, y)),
        CmpOp::Ge => refine_pair(CmpOp::Le, b, a).map(|(y, x)| (x, y)),
        CmpOp::Eq => {
            let m = a.meet(b)?;
            Some((m, m))
        }
        CmpOp::Ne => {
            if a.lo == a.hi && a == b {
                return None;
            }
            // Only an excluded value at an end of the other range tells anything.
            let trim = |x: Interval, y: Interval| {
                if y.lo == y.hi && x.lo == y.lo {
                    Interval {
                        lo: x.lo + 1,
                        hi: x.hi,
                    }
                } else if y.lo == y.hi && x.hi == y.lo {
                    Interval {
                        lo: x.lo,
                        hi: x.hi - 1,
                    }
                } else {
                    x
                }
            };
            Some((trim(a, b), trim(b, a)))
        }
    }
}

/// Applies a condition that is taken to hold. `None` if it never can.
pub fn assume(state: &State, cond: &Cond) -> Option<State> {
    let mut out = state.clone();
    let (a, b) = (operand(state, cond.a), operand(state, cond.b));
    // Comparing a pointer against zero or null.
    let ptr_test = |p: Option<Value>, other: Option<Value>| match (p, other) {
        (Some(Value::Ptr { null, size }), Some(Value::Int(i))) if i == Interval::exact(0) => {
            Some((null, size))
        }
        (
            Some(Value::Ptr { null, size }),
            Some(Value::Ptr {
                null: Nullness::Null,
                ..
            }),
        ) => Some((null, size)),
        _ => None,
    };
    if matches!(cond.op, CmpOp::Eq | CmpOp::Ne) {
        for (p_op, other, this) in [(cond.a, b, a), (cond.b, a, b)] {
            if let Some((null, size)) = ptr_test(this, other) {
                let wants_null = cond.op == CmpOp::Eq;
                if (wants_null && null == Nullness::NonNull)
                    || (!wants_null && null == Nullness::Null)
                {
                    return None;
                }
                if let Operand::Var(v) = p_op {
                    let null = if wants_null {
                        Nullness::Null
                    } else {
                        Nullness::NonNull
                    };
                    out[v as usize] = Some(Value::Ptr { null, size });
                }
                return Some(out);
            }
        }
    }
    let (Some(Value::Int(x)), Some(Value::Int(y))) = (a, b) else {
        return Some(out);
    };
    if let Some(bits) = cond.signed_bits {
        // Signed and unsigned agree only when neither can have the sign bit set.
        let limit = 1i128 << (bits - 1);
        if x.hi >= limit || y.hi >= limit || x.lo < 0 || y.lo < 0 {
            if x.lo == x.hi && y.lo == y.hi {
                let holds = decide(
                    cond.op,
                    Interval::exact(to_signed(x.lo, bits)),
                    Interval::exact(to_signed(y.lo, bits)),
                );
                if holds == Some(false) {
                    return None;
                }
            }
            return Some(out);
        }
    }
    let (x2, y2) = refine_pair(cond.op, x, y)?;
    if let Operand::Var(v) = cond.a {
        out[v as usize] = Some(Value::Int(x2));
    }
    if let Operand::Var(v) = cond.b {
        // Comparing a variable to itself has to apply both narrowings.
        let current = out[v as usize];
        let y3 = match (cond.a == cond.b, current) {
            (true, Some(Value::Int(c))) => y2.meet(c)?,
            _ => y2,
        };
        out[v as usize] = Some(Value::Int(y3));
    }
    Some(out)
}

fn is_int(v: Value) -> bool {
    matches!(v, Value::Int(_))
}

/// The verdict for an operand having to be an initialized integer.
fn int_checks(values: &[Option<Value>], report: &mut dyn FnMut(Fault, Verdict)) {
    let mut uninit = false;
    let mut not_int = Verdict::Proved;
    for v in values {
        match v {
            None => uninit = true,
            Some(Value::Int(_)) => {}
            Some(Value::Ptr { .. }) => not_int = Verdict::Definite,
            Some(Value::Top) => not_int = not_int.max(Verdict::Possible),
        }
    }
    report(
        Fault::Uninitialized,
        if uninit {
            Verdict::Possible
        } else {
            Verdict::Proved
        },
    );
    report(Fault::NotAnInteger, not_int);
}

fn cond_checks(state: &State, cond: &Cond, report: &mut dyn FnMut(Fault, Verdict)) {
    let (a, b) = (operand(state, cond.a), operand(state, cond.b));
    let uninit = a.is_none() || b.is_none();
    report(
        Fault::Uninitialized,
        if uninit {
            Verdict::Possible
        } else {
            Verdict::Proved
        },
    );
    let (Some(a), Some(b)) = (a, b) else {
        report(Fault::NotAnInteger, Verdict::Possible);
        return;
    };
    let pointer_like = |v: Value| matches!(v, Value::Ptr { .. });
    let verdict = if is_int(a) && is_int(b) {
        Verdict::Proved
    } else if matches!(cond.op, CmpOp::Eq | CmpOp::Ne) {
        // A pointer can be compared with a pointer, or with an exact zero.
        let fine = |p: Value, o: Value| {
            pointer_like(p) && (pointer_like(o) || o == Value::Int(Interval::exact(0)))
        };
        if fine(a, b) || fine(b, a) {
            Verdict::Proved
        } else {
            Verdict::Possible
        }
    } else if pointer_like(a) || pointer_like(b) {
        Verdict::Definite
    } else {
        Verdict::Possible
    };
    report(Fault::NotAnInteger, verdict);
}

/// Runs one instruction. Returns the state after it, or `None` if execution cannot continue past it.
/// Every fault it could raise is passed to `report`.
pub fn transfer(
    state: &State,
    instr: &Instr,
    report: &mut dyn FnMut(Fault, Verdict),
) -> Option<State> {
    let mut out = state.clone();
    match instr {
        Instr::Copy(d, o) => match operand(state, *o) {
            Some(v) => store_var(&mut out, *d, v),
            None => {
                report(Fault::Uninitialized, Verdict::Possible);
                store_var(&mut out, *d, Value::Top);
            }
        },
        Instr::Bin(d, op, a, b) => {
            let (x, y) = (operand(state, *a), operand(state, *b));
            int_checks(&[x, y], report);
            let (i, j) = (
                x.map_or(Interval::TOP, Value::int_or_top),
                y.map_or(Interval::TOP, Value::int_or_top),
            );
            let result = match op {
                BinOp::Add => i.add(j),
                BinOp::Sub => i.sub(j),
                BinOp::Mul => i.mul(j),
                BinOp::Div | BinOp::Rem => {
                    let verdict = if j == Interval::exact(0) {
                        Verdict::Definite
                    } else if j.contains(0) {
                        Verdict::Possible
                    } else {
                        Verdict::Proved
                    };
                    report(Fault::DivisionByZero, verdict);
                    let r = if *op == BinOp::Div {
                        i.div(j)
                    } else {
                        i.rem(j)
                    };
                    r?
                }
                BinOp::And => i.bit_and(j),
                BinOp::Or => i.bit_or(j),
                BinOp::Xor => i.bit_xor(j),
                BinOp::Shl => i.shl(j),
                BinOp::Shr => i.shr(j),
            };
            store_var(&mut out, *d, Value::Int(result));
        }
        Instr::Wrap(d, o, bits) => {
            let v = operand(state, *o);
            int_checks(&[v], report);
            let i = v.map_or(Interval::TOP, Value::int_or_top);
            store_var(&mut out, *d, Value::Int(i.wrap(*bits)));
        }
        Instr::Havoc(d, range) => store_var(&mut out, *d, Value::Int(*range)),
        Instr::Alloc {
            dst,
            size,
            nullable,
        } => {
            let s = operand(state, *size);
            int_checks(&[s], report);
            let n = s.map_or(Interval::TOP, Value::int_or_top);
            let verdict = if n.hi < 0 {
                Verdict::Definite
            } else if n.lo < 0 {
                Verdict::Possible
            } else {
                Verdict::Proved
            };
            report(Fault::NegativeAllocation, verdict);
            let size = n.meet(Interval { lo: 0, hi: INF })?;
            let null = if *nullable {
                Nullness::MaybeNull
            } else {
                Nullness::NonNull
            };
            store_var(&mut out, *dst, Value::Ptr { null, size });
        }
        Instr::Null(d) => store_var(
            &mut out,
            *d,
            Value::Ptr {
                null: Nullness::Null,
                size: Interval::exact(0),
            },
        ),
        Instr::Load { ptr, index, .. } | Instr::Store { ptr, index, .. } => {
            let (p, at) = (state[*ptr as usize], operand(state, *index));
            let mut uninit = p.is_none() || at.is_none();
            if let Instr::Store { value, .. } = instr {
                uninit = uninit || operand(state, *value).is_none();
            }
            report(
                Fault::Uninitialized,
                if uninit {
                    Verdict::Possible
                } else {
                    Verdict::Proved
                },
            );
            int_checks(&[at.or(Some(Value::Int(Interval::TOP)))], &mut |f, v| {
                if f == Fault::NotAnInteger {
                    report(f, v);
                }
            });
            let i = at.map_or(Interval::TOP, Value::int_or_top);
            let (not_pointer, null, bounds) = match p {
                Some(Value::Ptr { null, size }) => {
                    let null_verdict = match null {
                        Nullness::NonNull => Verdict::Proved,
                        Nullness::MaybeNull => Verdict::Possible,
                        Nullness::Null => Verdict::Definite,
                    };
                    let bounds = if i.lo >= 0 && i.hi < size.lo {
                        Verdict::Proved
                    } else if i.hi < 0 || i.lo >= size.hi {
                        Verdict::Definite
                    } else {
                        Verdict::Possible
                    };
                    (Verdict::Proved, null_verdict, bounds)
                }
                Some(Value::Int(_)) => (Verdict::Definite, Verdict::Proved, Verdict::Proved),
                _ => (Verdict::Possible, Verdict::Possible, Verdict::Possible),
            };
            report(Fault::NotAPointer, not_pointer);
            report(Fault::NullDereference, null);
            report(Fault::OutOfBounds, bounds);
            if let Instr::Load { dst, .. } = instr {
                // Nothing tracks the contents of allocations.
                store_var(&mut out, *dst, Value::Top);
            }
            if not_pointer == Verdict::Definite
                || null == Verdict::Definite
                || bounds == Verdict::Definite
            {
                return None;
            }
            // Past here, the access worked.
            if let Some(Value::Ptr { size, .. }) = p {
                out[*ptr as usize] = Some(Value::Ptr {
                    null: Nullness::NonNull,
                    size,
                });
                if let (Operand::Var(v), Some(Value::Int(iv))) = (*index, at) {
                    let inside = Interval {
                        lo: 0,
                        hi: if size.hi >= INF { INF } else { size.hi - 1 },
                    };
                    if let Some(m) = iv.meet(inside) {
                        out[v as usize] = Some(Value::Int(m));
                    }
                }
            }
        }
        Instr::MemAccess {
            addr,
            offset,
            size,
            limit,
        } => {
            let a = operand(state, *addr);
            int_checks(&[a], report);
            let i = a.map_or(Interval::TOP, Value::int_or_top);
            let extra = i128::from(*offset) + i128::from(*size);
            let limit = i128::try_from(*limit).unwrap_or(INF);
            let verdict = if i.lo >= 0 && i.hi < INF && i.hi + extra <= limit {
                Verdict::Proved
            } else {
                // The memory may have grown, so nothing here is certain to fail.
                Verdict::Possible
            };
            report(Fault::OutOfBounds, verdict);
        }
        Instr::Assert(c) => {
            cond_checks(state, c, report);
            let negated = Cond {
                op: c.op.negate(),
                ..*c
            };
            let can_fail = assume(state, &negated).is_some();
            let holds = assume(state, c);
            let verdict = match (can_fail, holds.is_some()) {
                (false, _) => Verdict::Proved,
                (true, true) => Verdict::Possible,
                (true, false) => Verdict::Definite,
            };
            report(Fault::AssertionFailed, verdict);
            return holds;
        }
        Instr::Assume(c) => {
            cond_checks(state, c, report);
            return assume(state, c);
        }
        Instr::ReadGlobal { dst, .. } => store_var(&mut out, *dst, Value::Int(Interval::TOP)),
        Instr::WriteGlobal { value, .. } => {
            let v = operand(state, *value);
            report(
                Fault::Uninitialized,
                if v.is_none() {
                    Verdict::Possible
                } else {
                    Verdict::Proved
                },
            );
        }
        Instr::Lock(_) | Instr::Unlock(_) => {}
    }
    Some(out)
}

fn join_states(a: &State, b: &State) -> State {
    a.iter()
        .zip(b)
        .map(|(x, y)| match (x, y) {
            (Some(p), Some(q)) => Some(p.join(*q)),
            // Uninitialized on one way in means possibly uninitialized.
            _ => None,
        })
        .collect()
}

fn widen_states(old: &State, next: &State, thresholds: &[i128]) -> State {
    old.iter()
        .zip(next)
        .map(|(x, y)| match (x, y) {
            (Some(p), Some(q)) => Some(p.widen(*q, thresholds)),
            _ => None,
        })
        .collect()
}

/// Which blocks are the target of a back edge, found by depth-first search without recursion.
fn loop_heads(function: &Function) -> Vec<bool> {
    let n = function.blocks.len();
    let mut heads = vec![false; n];
    let mut color = vec![0u8; n];
    let mut stack = vec![(0usize, 0usize)];
    color[0] = 1;
    while let Some((block, next)) = stack.last().copied() {
        let successors = function.successors(block);
        if let Some(t) = successors.get(next).copied() {
            stack.last_mut().unwrap().1 += 1;
            match color[t] {
                0 => {
                    color[t] = 1;
                    stack.push((t, 0));
                }
                1 => heads[t] = true,
                _ => {}
            }
        } else {
            color[block] = 2;
            stack.pop();
        }
    }
    heads
}

fn reverse_postorder(function: &Function) -> Vec<usize> {
    let n = function.blocks.len();
    let mut seen = vec![false; n];
    let mut order = Vec::with_capacity(n);
    let mut stack = vec![(0usize, 0usize)];
    seen[0] = true;
    while let Some((block, next)) = stack.last().copied() {
        let successors = function.successors(block);
        if let Some(t) = successors.get(next).copied() {
            stack.last_mut().unwrap().1 += 1;
            if !seen[t] {
                seen[t] = true;
                stack.push((t, 0));
            }
        } else {
            order.push(block);
            stack.pop();
        }
    }
    order.reverse();
    order
}

/// The constants a function mentions, with the bounds of machine integers, which widening jumps to.
fn thresholds_of(function: &Function) -> Vec<i128> {
    let mut set: BTreeSet<i128> = BTreeSet::new();
    set.extend([
        0,
        1,
        -1,
        (1 << 31) - 1,
        (1 << 32) - 1,
        (1 << 63) - 1,
        (1 << 64) - 1,
    ]);
    let mut add = |o: &Operand| {
        if let Operand::Const(c) = o {
            for v in [*c - 1, *c, *c + 1] {
                set.insert(v);
            }
        }
    };
    for block in &function.blocks {
        for (instr, _) in &block.instrs {
            match instr {
                Instr::Copy(_, o) | Instr::Wrap(_, o, _) => add(o),
                Instr::Bin(_, _, a, b) => {
                    add(a);
                    add(b);
                }
                Instr::Alloc { size, .. } => add(size),
                Instr::Assert(c) | Instr::Assume(c) => {
                    add(&c.a);
                    add(&c.b);
                }
                Instr::Havoc(_, r) => {
                    add(&Operand::Const(r.lo.clamp(-INF / 2, INF / 2)));
                    add(&Operand::Const(r.hi.clamp(-INF / 2, INF / 2)));
                }
                _ => {}
            }
        }
        if let Term::Branch(c, _, _) = &block.term {
            add(&c.a);
            add(&c.b);
        }
    }
    set.into_iter().collect()
}

/// What leaves a block, along each edge.
fn block_exits(function: &Function, block: usize, entry: &State) -> Vec<(usize, State)> {
    let mut state = entry.clone();
    for (instr, _) in &function.blocks[block].instrs {
        match transfer(&state, instr, &mut |_, _| {}) {
            Some(next) => state = next,
            None => return Vec::new(),
        }
    }
    match &function.blocks[block].term {
        Term::Return => Vec::new(),
        Term::Goto(t) => vec![(*t, state)],
        Term::Branch(c, a, b) => {
            let mut out = Vec::new();
            if let Some(s) = assume(&state, c) {
                out.push((*a, s));
            }
            if let Some(s) = assume(
                &state,
                &Cond {
                    op: c.op.negate(),
                    ..*c
                },
            ) {
                out.push((*b, s));
            }
            out
        }
    }
}

/// How many times a loop head joins before it starts widening. A few rounds find small constant
/// bounds exactly, and cost almost nothing.
const JOINS_BEFORE_WIDENING: u32 = 3;
const NARROWING_PASSES: u32 = 3;

pub fn analyze(function: &Function) -> Analysis<'_> {
    let n = function.blocks.len();
    let heads = loop_heads(function);
    let order = reverse_postorder(function);
    let thresholds = thresholds_of(function);
    let mut entry: Vec<Option<State>> = vec![None; n];

    let mut initial: State = vec![None; function.var_count()];
    for (var, range) in &function.params {
        initial[*var as usize] = Some(Value::Int(*range));
    }
    entry[0] = Some(initial.clone());

    let mut position = vec![usize::MAX; n];
    for (i, b) in order.iter().enumerate() {
        position[*b] = i;
    }
    let mut visits = vec![0u32; n];
    let mut pending: BTreeSet<usize> = BTreeSet::from([0]);
    while let Some(at) = pending.pop_first() {
        let block = order[at];
        let Some(state) = entry[block].clone() else {
            continue;
        };
        visits[block] += 1;
        for (target, out) in block_exits(function, block, &state) {
            let updated = match &entry[target] {
                None => Some(out),
                Some(old) => {
                    let joined = join_states(old, &out);
                    if joined == *old {
                        None
                    } else if heads[target] && visits[target] >= JOINS_BEFORE_WIDENING {
                        Some(widen_states(old, &joined, &thresholds))
                    } else {
                        Some(joined)
                    }
                }
            };
            if let Some(state) = updated {
                entry[target] = Some(state);
                pending.insert(position[target]);
            }
        }
    }

    // One application of the transfer functions to any sound result is again sound, so each of these
    // passes can only gain precision. The whole pass reads the previous result, and never a mixture.
    for _ in 0..NARROWING_PASSES {
        let mut descending: Vec<Option<State>> = vec![None; n];
        descending[0] = Some(initial.clone());
        for (block, state) in entry.iter().enumerate() {
            let Some(state) = state else { continue };
            for (target, out) in block_exits(function, block, state) {
                descending[target] = Some(match descending[target].take() {
                    Some(s) => join_states(&s, &out),
                    None => out,
                });
            }
        }
        entry = descending;
    }

    let mut checks = Vec::new();
    for (block, data) in function.blocks.iter().enumerate() {
        let state = entry[block].clone();
        let dead_all = state.is_none();
        let mut current = state;
        for (i, (instr, line)) in data.instrs.iter().enumerate() {
            let mut found: Vec<(Fault, Verdict)> = Vec::new();
            let after = match &current {
                Some(s) => transfer(s, instr, &mut |f, v| found.push((f, v))),
                None => {
                    transfer(&vec![None; function.var_count()], instr, &mut |f, _| {
                        found.push((f, Verdict::Proved))
                    });
                    None
                }
            };
            let dead = current.is_none();
            for (fault, verdict) in merge(found) {
                checks.push(Check {
                    block,
                    instr: i,
                    line: *line,
                    fault,
                    verdict: if dead { Verdict::Proved } else { verdict },
                    dead,
                });
            }
            current = after;
        }
        if let Term::Branch(c, _, _) = &data.term {
            let mut found: Vec<(Fault, Verdict)> = Vec::new();
            let dead = current.is_none() || dead_all;
            match &current {
                Some(s) => cond_checks(s, c, &mut |f, v| found.push((f, v))),
                None => cond_checks(&vec![None; function.var_count()], c, &mut |f, _| {
                    found.push((f, Verdict::Proved))
                }),
            }
            for (fault, verdict) in merge(found) {
                checks.push(Check {
                    block,
                    instr: data.instrs.len(),
                    line: 0,
                    fault,
                    verdict: if dead { Verdict::Proved } else { verdict },
                    dead,
                });
            }
        }
    }
    Analysis {
        function,
        entry,
        checks,
    }
}

/// Keeps the worst verdict for each fault.
fn merge(found: Vec<(Fault, Verdict)>) -> Vec<(Fault, Verdict)> {
    let mut out: Vec<(Fault, Verdict)> = Vec::new();
    for (fault, verdict) in found {
        match out.iter_mut().find(|(f, _)| *f == fault) {
            Some((_, v)) => *v = (*v).max(verdict),
            None => out.push((fault, verdict)),
        }
    }
    out
}

impl Analysis<'_> {
    /// The abstract state right before an instruction, or before the terminator if `instr` is past
    /// the end. `None` if the analysis says nothing reaches it.
    pub fn state_before(&self, block: usize, instr: usize) -> Option<State> {
        let mut state = self.entry[block].clone()?;
        for (i, _) in self.function.blocks[block].instrs.iter().take(instr) {
            state = transfer(&state, i, &mut |_, _| {})?;
        }
        Some(state)
    }

    pub fn unproved(&self) -> impl Iterator<Item = &Check> {
        self.checks.iter().filter(|c| c.verdict != Verdict::Proved)
    }
}
