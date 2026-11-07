//! Simplification of linear Mixed Boolean-Arithmetic (MBA) expressions.
//!
//! Obfuscators hide `x + y` behind forms such as `(x ^ y) + 2 * (x & y)`. A linear MBA expression is
//! an integer linear combination of bitwise functions of some variables. For these there is a complete
//! decision method. Each bit position of a bitwise function depends only on that position of its
//! inputs, so writing `S(p)` for the sum of the coefficients of the functions that are true on the bit
//! pattern `p`, the expression's value is the sum over bit positions `b` of `2^b * S(pattern at b)`.
//!
//! Set every variable to all zeros or all ones, and every bit position has the same pattern `p`. The
//! value is then `(2^w - 1) * S(p)`, which is `-S(p)` modulo `2^w`. So the `2^k` values on those inputs,
//! called the signature, determine every `S(p)`, and through them the whole expression.
//!
//! The simplifier computes the signature and finds the cheapest expression that has the same one. This
//! is a proof for all inputs, and not a rule that happens to work on samples.
//!
//! Any subterm that is not linear is treated as one more variable. If an identity holds for
//! independent variables, it still holds when some of them are replaced with real expressions. That
//! keeps it sound, but it does mean that a nonlinear MBA is only simplified where its linear parts allow.

use crate::ssa::{Bin, Graph, Node, V, Width, eval_bin};
use std::collections::BTreeMap;

/// More variables than this means a signature of over 64 entries, which is not worth the search.
const MAX_VARIABLES: usize = 6;

/// How many nodes an expression may hold before the analysis gives up. This keeps it cheap, and it
/// also bounds every later walk over the expression, however much the graph shares.
const MAX_NODES: usize = 256;

pub fn is_candidate(g: &Graph, v: V) -> bool {
    matches!(
        g.node(v),
        Node::Bin(
            Bin::Add | Bin::Sub | Bin::Mul | Bin::And | Bin::Or | Bin::Xor | Bin::Shl,
            ..
        )
    )
}

/// Finds the variables of a linear MBA expression.
struct Analysis<'a> {
    g: &'a Graph,
    variables: BTreeMap<V, usize>,
    visited: usize,
}

impl Analysis<'_> {
    fn variable(&mut self, v: V) -> bool {
        if self.variables.contains_key(&v) {
            return true;
        }
        if self.variables.len() >= MAX_VARIABLES {
            return false;
        }
        let index = self.variables.len();
        self.variables.insert(v, index);
        true
    }

    /// Walks the arithmetic part, where everything must be linear.
    fn arithmetic(&mut self, v: V) -> bool {
        self.visited += 1;
        if self.visited > MAX_NODES {
            return false;
        }
        match self.g.node(v) {
            Node::Const(..) => true,
            Node::Bin(Bin::Add | Bin::Sub, _, a, b) => self.arithmetic(a) && self.arithmetic(b),
            Node::Bin(Bin::Mul, _, a, b) => {
                if self.g.as_const(b).is_some() {
                    self.arithmetic(a)
                } else if self.g.as_const(a).is_some() {
                    self.arithmetic(b)
                } else {
                    // A product of two unknowns is not linear, so it is one opaque variable.
                    self.variable(v)
                }
            }
            // A shift by a constant multiplies by a power of two.
            Node::Bin(Bin::Shl, _, a, c) if self.g.as_const(c).is_some() => self.arithmetic(a),
            // ~a is -a - 1, whatever a is, so this stays linear even over arithmetic.
            Node::Bin(Bin::Xor, width, a, c) if self.g.as_const(c) == Some(width.mask()) => {
                self.arithmetic(a)
            }
            Node::Bin(Bin::And | Bin::Or | Bin::Xor, _, a, b) => {
                // A bitwise term with an ordinary constant in it is not in the linear form, but the
                // whole term can still be one variable.
                let saved = self.variables.clone();
                if self.bitwise(a) && self.bitwise(b) {
                    true
                } else {
                    self.variables = saved;
                    self.variable(v)
                }
            }
            _ => self.variable(v),
        }
    }

    /// Walks a bitwise subtree. Only bitwise operators and all-zeros or all-ones constants are allowed
    /// here, and anything else is a variable.
    fn bitwise(&mut self, v: V) -> bool {
        self.visited += 1;
        if self.visited > MAX_NODES {
            return false;
        }
        match self.g.node(v) {
            Node::Const(width, c) => c == 0 || c == width.mask(),
            Node::Bin(Bin::And | Bin::Or | Bin::Xor, _, a, b) => self.bitwise(a) && self.bitwise(b),
            _ => self.variable(v),
        }
    }
}

fn signature_entry(
    g: &Graph,
    v: V,
    width: Width,
    variables: &BTreeMap<V, usize>,
    pattern: usize,
) -> u64 {
    if let Some(index) = variables.get(&v) {
        return if pattern >> index & 1 == 1 {
            width.mask()
        } else {
            0
        };
    }
    match g.node(v) {
        Node::Const(_, c) => c,
        Node::Bin(op, _, a, b) => eval_bin(
            op,
            width,
            signature_entry(g, a, width, variables, pattern),
            signature_entry(g, b, width, variables, pattern),
        ),
        // The analysis made anything else into a variable.
        _ => 0,
    }
}

/// The number of operators above the variables, which the analysis has already bounded.
fn cost(g: &Graph, v: V, variables: &BTreeMap<V, usize>) -> u32 {
    if variables.contains_key(&v) {
        return 0;
    }
    match g.node(v) {
        Node::Bin(_, _, a, b) => cost(g, a, variables) + cost(g, b, variables) + 1,
        _ => 0,
    }
}

/// The Möbius transform: rewrites a signature as coefficients over ANDs of variables. Entry `s` of
/// the result gives the coefficient of the AND of the variables in the bit set `s`.
fn conjunction_basis(signature: &[u64], width: Width) -> Vec<u64> {
    let mut out = signature.to_vec();
    let k = signature.len().trailing_zeros();
    for bit in 0..k {
        for s in 0..signature.len() {
            if s >> bit & 1 == 1 {
                out[s] = width.truncate(out[s].wrapping_sub(out[s ^ (1 << bit)]));
            }
        }
    }
    out
}

/// Builds an expression with a given signature, from coefficients over conjunctions.
fn build_from_basis(g: &mut Graph, width: Width, variables: &[V], basis: &[u64]) -> V {
    let mut sum = g.constant(width, basis[0]);
    for (set, raw) in basis.iter().enumerate().skip(1) {
        // A conjunction is -1 on all-ones inputs, so its coefficient is the negated entry.
        let coefficient = width.truncate(raw.wrapping_neg());
        if coefficient == 0 {
            continue;
        }
        let mut term: Option<V> = None;
        for (i, variable) in variables.iter().enumerate() {
            if set >> i & 1 == 1 {
                term = Some(match term {
                    Some(t) => g.intern_bin(Bin::And, width, t, *variable),
                    None => *variable,
                });
            }
        }
        let Some(term) = term else { continue };
        let negated = width.sign_extend(coefficient) < 0;
        let magnitude = if negated {
            width.truncate(coefficient.wrapping_neg())
        } else {
            coefficient
        };
        let scaled = if magnitude == 1 {
            term
        } else {
            let c = g.constant(width, magnitude);
            g.intern_bin(Bin::Mul, width, term, c)
        };
        sum = if negated {
            g.intern_bin(Bin::Sub, width, sum, scaled)
        } else if g.as_const(sum) == Some(0) {
            scaled
        } else {
            g.intern_bin(Bin::Add, width, sum, scaled)
        };
    }
    sum
}

/// Short forms for one or two variables, which come out cheaper than the conjunction basis.
#[derive(Clone, Copy)]
enum Shape {
    X,
    Y,
    NotX,
    NotY,
    NegX,
    And,
    Or,
    Xor,
    Add,
    SubXY,
    SubYX,
    AndNotY,
    AndNotX,
    OrNotY,
    OrNotX,
    Nand,
    Nor,
    Xnor,
}

const SHAPES: [Shape; 18] = [
    Shape::X,
    Shape::Y,
    Shape::NotX,
    Shape::NotY,
    Shape::NegX,
    Shape::And,
    Shape::Or,
    Shape::Xor,
    Shape::Add,
    Shape::SubXY,
    Shape::SubYX,
    Shape::AndNotY,
    Shape::AndNotX,
    Shape::OrNotY,
    Shape::OrNotX,
    Shape::Nand,
    Shape::Nor,
    Shape::Xnor,
];

impl Shape {
    fn eval(self, x: u64, y: u64, width: Width) -> u64 {
        width.truncate(match self {
            Shape::X => x,
            Shape::Y => y,
            Shape::NotX => !x,
            Shape::NotY => !y,
            Shape::NegX => x.wrapping_neg(),
            Shape::And => x & y,
            Shape::Or => x | y,
            Shape::Xor => x ^ y,
            Shape::Add => x.wrapping_add(y),
            Shape::SubXY => x.wrapping_sub(y),
            Shape::SubYX => y.wrapping_sub(x),
            Shape::AndNotY => x & !y,
            Shape::AndNotX => !x & y,
            Shape::OrNotY => x | !y,
            Shape::OrNotX => !x | y,
            Shape::Nand => !(x & y),
            Shape::Nor => !(x | y),
            Shape::Xnor => !(x ^ y),
        })
    }

    fn build(self, g: &mut Graph, width: Width, x: V, y: V) -> V {
        let ones = g.constant(width, width.mask());
        let zero = g.constant(width, 0);
        let not = |g: &mut Graph, v: V| g.intern_bin(Bin::Xor, width, v, ones);
        match self {
            Shape::X => x,
            Shape::Y => y,
            Shape::NotX => not(g, x),
            Shape::NotY => not(g, y),
            Shape::NegX => g.intern_bin(Bin::Sub, width, zero, x),
            Shape::And => g.intern_bin(Bin::And, width, x, y),
            Shape::Or => g.intern_bin(Bin::Or, width, x, y),
            Shape::Xor => g.intern_bin(Bin::Xor, width, x, y),
            Shape::Add => g.intern_bin(Bin::Add, width, x, y),
            Shape::SubXY => g.intern_bin(Bin::Sub, width, x, y),
            Shape::SubYX => g.intern_bin(Bin::Sub, width, y, x),
            Shape::AndNotY => {
                let n = not(g, y);
                g.intern_bin(Bin::And, width, x, n)
            }
            Shape::AndNotX => {
                let n = not(g, x);
                g.intern_bin(Bin::And, width, n, y)
            }
            Shape::OrNotY => {
                let n = not(g, y);
                g.intern_bin(Bin::Or, width, x, n)
            }
            Shape::OrNotX => {
                let n = not(g, x);
                g.intern_bin(Bin::Or, width, n, y)
            }
            Shape::Nand => {
                let inner = g.intern_bin(Bin::And, width, x, y);
                not(g, inner)
            }
            Shape::Nor => {
                let inner = g.intern_bin(Bin::Or, width, x, y);
                not(g, inner)
            }
            Shape::Xnor => {
                let inner = g.intern_bin(Bin::Xor, width, x, y);
                not(g, inner)
            }
        }
    }
}

/// The shapes that match a signature. Nothing joins the graph unless it is actually chosen, because a
/// failed search over every expression would otherwise fill the graph with unused nodes.
fn matching_shape(g: &mut Graph, width: Width, variables: &[V], signature: &[u64]) -> Vec<V> {
    if variables.is_empty() || variables.len() > 2 {
        return Vec::new();
    }
    let x_index = 0;
    let y_index = variables.len() - 1;
    let value = |pattern: usize, index: usize| {
        if pattern >> index & 1 == 1 {
            width.mask()
        } else {
            0
        }
    };
    let mut out = Vec::new();
    for shape in SHAPES {
        let matches = signature.iter().enumerate().all(|(pattern, want)| {
            shape.eval(value(pattern, x_index), value(pattern, y_index), width) == *want
        });
        if matches {
            out.push(shape.build(g, width, variables[x_index], variables[y_index]));
        }
    }
    out
}

/// Returns a strictly cheaper equal expression, if there is one.
pub fn simplify(g: &mut Graph, v: V, width: Width) -> Option<V> {
    let mut analysis = Analysis {
        g,
        variables: BTreeMap::new(),
        visited: 0,
    };
    if !analysis.arithmetic(v) {
        return None;
    }
    let visited = analysis.visited as u64;
    let variables = analysis.variables;
    let before = cost(g, v, &variables);
    if before < 2 {
        return None;
    }
    let size = 1usize << variables.len();
    if !g.spend(visited * size as u64) {
        return None;
    }
    // Variables keep the order they were found in, so results are reproducible.
    let mut ordered: Vec<(usize, V)> = variables.iter().map(|(v, i)| (*i, *v)).collect();
    ordered.sort_unstable();
    let ordered: Vec<V> = ordered.into_iter().map(|(_, v)| v).collect();

    let signature: Vec<u64> = (0..size)
        .map(|pattern| signature_entry(g, v, width, &variables, pattern))
        .collect();

    let mut candidates = matching_shape(g, width, &ordered, &signature);
    let basis = conjunction_basis(&signature, width);
    candidates.push(build_from_basis(g, width, &ordered, &basis));

    let best = candidates
        .into_iter()
        .min_by_key(|c| cost(g, *c, &variables))?;
    (cost(g, best, &variables) < before).then_some(best)
}
