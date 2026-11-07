//! Opaque predicate detection: conditions that look like they depend on input, but never do.
//!
//! Two proofs are used, and neither is a sample.
//!
//! The first is a property of T-functions. In an expression built from addition, subtraction,
//! multiplication, bitwise operators and left shifts by constants, the low `k` bits of the result
//! depend only on the low `k` bits of the inputs. So if two such expressions differ for every assignment
//! of the low bits, they can never be equal, whatever the full values are. This proves the classic
//! `7 * y * y - 1 != x * x` by checking only the residues modulo 8.
//!
//! The second is arithmetic over GF(2) on the lowest bit, where addition is XOR and multiplication is
//! AND, and where `x * x` collapses to `x`. So `x * x + x` comes out as exactly zero, which proves it is
//! always even. Unlike the first, this can also prove that a condition is always true.
//!
//! Anything else in an expression counts as an unknown input, which is safe, because a fact that holds
//! for every value of an unknown holds for whatever value it really has.

use crate::ssa::{Bin, Graph, Node, V, Width, eval_bin};
use std::collections::BTreeSet;

/// The low bit counts tried, in order. Most predicates are decided by a very small modulus, and a
/// normal comparison is ruled out at the first collision, so this rarely gets near the last level.
const LEVELS: [u32; 5] = [1, 2, 3, 4, 8];

/// Two unknowns at 8 bits is 65,536 evaluations, which is why the search stops at two.
const MAX_LEAVES: usize = 2;
const MAX_NODES: usize = 128;
const MAX_PARITY_VISITS: u32 = 2048;
const MAX_PARITY_TERMS: usize = 256;

fn t_function_leaves(g: &Graph, v: V, leaves: &mut Vec<V>, visited: &mut usize) -> Option<()> {
    *visited += 1;
    if *visited > MAX_NODES {
        return None;
    }
    match g.node(v) {
        Node::Const(..) => Some(()),
        Node::Bin(Bin::Add | Bin::Sub | Bin::Mul | Bin::And | Bin::Or | Bin::Xor, _, a, b) => {
            t_function_leaves(g, a, leaves, visited)?;
            t_function_leaves(g, b, leaves, visited)
        }
        Node::Bin(Bin::Shl, _, a, c) if g.as_const(c).is_some() => {
            t_function_leaves(g, a, leaves, visited)
        }
        _ => {
            if !leaves.contains(&v) {
                if leaves.len() == MAX_LEAVES {
                    return None;
                }
                leaves.push(v);
            }
            Some(())
        }
    }
}

fn t_function_eval(g: &Graph, v: V, width: Width, leaves: &[V], values: &[u64; MAX_LEAVES]) -> u64 {
    if let Some(i) = leaves.iter().position(|leaf| *leaf == v) {
        return values[i];
    }
    match g.node(v) {
        Node::Const(_, c) => c,
        Node::Bin(op, _, a, b) => eval_bin(
            op,
            width,
            t_function_eval(g, a, width, leaves, values),
            t_function_eval(g, b, width, leaves, values),
        ),
        _ => 0,
    }
}

/// Tries to prove that `a` and `b` never agree in their low `max_bits` bits, which also proves that
/// they are never equal.
fn never_equal_low(g: &Graph, width: Width, a: V, b: V, max_bits: u32) -> bool {
    let mut leaves = Vec::new();
    let mut visited = 0;
    if t_function_leaves(g, a, &mut leaves, &mut visited).is_none()
        || t_function_leaves(g, b, &mut leaves, &mut visited).is_none()
        || leaves.is_empty()
    {
        return false;
    }
    let cost = visited as u64;
    'levels: for bits in LEVELS {
        if bits > max_bits {
            break;
        }
        let range = 1u64 << bits;
        let mask = range - 1;
        let outer = range;
        let inner = if leaves.len() == 2 { range } else { 1 };
        for first in 0..outer {
            for second in 0..inner {
                if !g.spend(cost) {
                    return false;
                }
                let values = [first, second];
                let x = t_function_eval(g, a, width, &leaves, &values) & mask;
                let y = t_function_eval(g, b, width, &leaves, &values) & mask;
                if x == y {
                    continue 'levels;
                }
            }
        }
        return true;
    }
    false
}

/// A polynomial over GF(2), as a set of monomials that are each a set of unknowns. The empty monomial
/// is the constant 1, and an empty polynomial is the constant 0.
type Polynomial = BTreeSet<BTreeSet<V>>;

fn plus(a: &Polynomial, b: &Polynomial) -> Polynomial {
    a.symmetric_difference(b).cloned().collect()
}

fn times(a: &Polynomial, b: &Polynomial) -> Option<Polynomial> {
    if a.len().saturating_mul(b.len()) > MAX_PARITY_TERMS * 16 {
        return None;
    }
    let mut out = Polynomial::new();
    for x in a {
        for y in b {
            // A variable times itself is itself, so a monomial is only a set.
            let product: BTreeSet<V> = x.union(y).copied().collect();
            if !out.remove(&product) {
                out.insert(product);
            }
        }
    }
    (out.len() <= MAX_PARITY_TERMS).then_some(out)
}

fn lowest_bit(g: &Graph, v: V, visits: &mut u32) -> Option<Polynomial> {
    *visits += 1;
    if *visits > MAX_PARITY_VISITS {
        return None;
    }
    Some(match g.node(v) {
        Node::Const(_, c) => {
            let mut out = Polynomial::new();
            if c & 1 == 1 {
                out.insert(BTreeSet::new());
            }
            out
        }
        Node::Bin(Bin::Add | Bin::Sub | Bin::Xor, _, a, b) => {
            let (pa, pb) = (lowest_bit(g, a, visits)?, lowest_bit(g, b, visits)?);
            plus(&pa, &pb)
        }
        Node::Bin(Bin::Mul | Bin::And, _, a, b) => {
            let (pa, pb) = (lowest_bit(g, a, visits)?, lowest_bit(g, b, visits)?);
            times(&pa, &pb)?
        }
        Node::Bin(Bin::Or, _, a, b) => {
            // a | b is a + b + a*b over GF(2).
            let (pa, pb) = (lowest_bit(g, a, visits)?, lowest_bit(g, b, visits)?);
            plus(&plus(&pa, &pb), &times(&pa, &pb)?)
        }
        // A left shift by a non-zero amount brings a zero into the lowest bit. An amount of zero has
        // already been simplified away, but the width is not known here, so it stays an unknown.
        Node::Bin(Bin::Shl, width, _, c)
            if g.as_const(c)
                .is_some_and(|c| c % u64::from(width.bits()) != 0) =>
        {
            Polynomial::new()
        }
        // A remainder by an even number differs from the dividend by a multiple of an even number.
        Node::Bin(Bin::RemS | Bin::RemU, _, a, m) if g.as_const(m).is_some_and(|m| m & 1 == 0) => {
            lowest_bit(g, a, visits)?
        }
        Node::ExtendU(a) | Node::ExtendS(a) | Node::Wrap(a) => lowest_bit(g, a, visits)?,
        _ => Polynomial::from([BTreeSet::from([v])]),
    })
}

/// The value of the lowest bit, if the arithmetic settles it.
pub fn known_parity(g: &Graph, v: V) -> Option<bool> {
    let mut visits = 0;
    let polynomial = lowest_bit(g, v, &mut visits)?;
    if polynomial.is_empty() {
        Some(false)
    } else if polynomial.len() == 1 && polynomial.iter().next().is_some_and(BTreeSet::is_empty) {
        Some(true)
    } else {
        None
    }
}

/// Decides a comparison between a remainder by a power of two and a constant.
///
/// For both the signed and unsigned forms, `x rem 2^k` agrees with `x` in its low `k` bits, even when
/// `x` is negative, because the result differs from `x` by a multiple of `2^k`.
fn remainder_against_constant(g: &Graph, width: Width, a: V, b: V, c: u64) -> Option<bool> {
    let Node::Bin(op @ (Bin::RemS | Bin::RemU), _, x, m) = g.node(a) else {
        return None;
    };
    let m = g.as_const(m).filter(|m| m.is_power_of_two() && *m > 1)?;
    let k = m.trailing_zeros();
    if k == 1 {
        match known_parity(g, x) {
            Some(false) => return Some(c == 0),
            Some(true) if op == Bin::RemU => return Some(c == 1),
            // Signed, an odd dividend has a remainder of 1 or -1, depending on its sign.
            Some(true) if c != 1 && c != width.mask() => return Some(false),
            _ => {}
        }
    }
    if never_equal_low(g, width, x, b, k.min(8)) {
        return Some(false);
    }
    None
}

/// Decides equality when that is provable.
///
/// Returns `Some(true)` if the two are always equal, `Some(false)` if they are never equal, and `None`
/// if it could go either way, or if the proofs could not tell.
pub fn decide_equality(g: &Graph, width: Width, a: V, b: V) -> Option<bool> {
    if a == b {
        return Some(true);
    }
    if let Some(c) = g.as_const(b) {
        if let Some(verdict) = remainder_against_constant(g, width, a, b, c) {
            return Some(verdict);
        }
        // `(x & 1) == c`, whose value is exactly the lowest bit.
        if let Node::Bin(Bin::And, _, x, m) = g.node(a)
            && g.as_const(m) == Some(1)
        {
            if c > 1 {
                return Some(false);
            }
            if let Some(bit) = known_parity(g, x) {
                return Some(bit == (c == 1));
            }
        }
        // A value with a known parity can never equal a constant of the other parity.
        if let Some(bit) = known_parity(g, a)
            && bit != (c & 1 == 1)
        {
            return Some(false);
        }
    }
    if never_equal_low(g, width, a, b, 8) {
        return Some(false);
    }
    None
}
