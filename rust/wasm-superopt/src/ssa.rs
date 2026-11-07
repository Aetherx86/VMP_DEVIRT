//! The value graph: hash-consed SSA nodes for pure computations, and the simplifier.
//!
//! Every rule here is an identity that holds for all inputs, because a deobfuscator that is
//! occasionally wrong is worse than useless when reverse engineering. The tests check the rules
//! against concrete evaluation on random values.

use crate::{mba, opaque};
use std::cell::Cell;
use std::collections::HashMap;
use wasmparser::ValType;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct V(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Width {
    W32,
    W64,
}

impl Width {
    pub fn bits(self) -> u32 {
        match self {
            Width::W32 => 32,
            Width::W64 => 64,
        }
    }

    pub fn mask(self) -> u64 {
        match self {
            Width::W32 => 0xFFFF_FFFF,
            Width::W64 => u64::MAX,
        }
    }

    pub fn sign_bit(self) -> u64 {
        1u64 << (self.bits() - 1)
    }

    pub fn truncate(self, value: u64) -> u64 {
        value & self.mask()
    }

    pub fn sign_extend(self, value: u64) -> i64 {
        match self {
            Width::W32 => i64::from(value as u32 as i32),
            Width::W64 => value as i64,
        }
    }

    pub fn val_type(self) -> ValType {
        match self {
            Width::W32 => ValType::I32,
            Width::W64 => ValType::I64,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bin {
    Add,
    Sub,
    Mul,
    And,
    Or,
    Xor,
    Shl,
    ShrS,
    ShrU,
    Rotl,
    Rotr,
    /// Division and remainder only join the graph when the divisor makes them unable to trap. See
    /// [`safe_divisor`].
    DivS,
    DivU,
    RemS,
    RemU,
}

impl Bin {
    fn commutative(self) -> bool {
        matches!(self, Bin::Add | Bin::Mul | Bin::And | Bin::Or | Bin::Xor)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Un {
    Clz,
    Ctz,
    Popcnt,
    Extend8S,
    Extend16S,
    Extend32S,
}

/// A comparison, which always produces an `i32` of 0 or 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cmp {
    Eq,
    Ne,
    LtS,
    LtU,
    GtS,
    GtU,
    LeS,
    LeU,
    GeS,
    GeU,
}

impl Cmp {
    fn inverse(self) -> Cmp {
        match self {
            Cmp::Eq => Cmp::Ne,
            Cmp::Ne => Cmp::Eq,
            Cmp::LtS => Cmp::GeS,
            Cmp::LtU => Cmp::GeU,
            Cmp::GtS => Cmp::LeS,
            Cmp::GtU => Cmp::LeU,
            Cmp::LeS => Cmp::GtS,
            Cmp::LeU => Cmp::GtU,
            Cmp::GeS => Cmp::LtS,
            Cmp::GeU => Cmp::LtU,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Node {
    Const(Width, u64),
    /// The current value of a local variable at the start of the region being translated.
    Input(u32, ValType),
    Bin(Bin, Width, V, V),
    Un(Un, Width, V),
    Cmp(Cmp, Width, V, V),
    Eqz(Width, V),
    /// `i64.extend_i32_u`, `i64.extend_i32_s` and `i32.wrap_i64`.
    ExtendU(V),
    ExtendS(V),
    Wrap(V),
    /// `select`: the first value if the condition is not zero, and otherwise the second.
    Select(V, V, V),
}

fn children(node: Node) -> [Option<V>; 3] {
    match node {
        Node::Const(..) | Node::Input(..) => [None, None, None],
        Node::Bin(_, _, a, b) | Node::Cmp(_, _, a, b) => [Some(a), Some(b), None],
        Node::Un(_, _, a)
        | Node::Eqz(_, a)
        | Node::ExtendU(a)
        | Node::ExtendS(a)
        | Node::Wrap(a) => [Some(a), None, None],
        Node::Select(a, b, c) => [Some(a), Some(b), Some(c)],
    }
}

#[derive(Default, Clone, Copy)]
pub struct Stats {
    pub constants_folded: u32,
    pub identities_applied: u32,
    pub opaque_predicates: u32,
    pub mba_simplified: u32,
}

pub struct Graph {
    nodes: Vec<Node>,
    depths: Vec<u32>,
    index: HashMap<Node, V>,
    deepest: u32,
    pub stats: Stats,
    /// What is left of the budget for the searching proofs. It is shared across a whole run, so that
    /// a hostile module cannot make this tool take long by repeating an expensive expression.
    pub work: Cell<u64>,
}

impl Default for Graph {
    fn default() -> Self {
        Graph::new(u64::MAX)
    }
}

impl Graph {
    pub fn new(work: u64) -> Self {
        Graph {
            nodes: Vec::new(),
            depths: Vec::new(),
            index: HashMap::new(),
            deepest: 0,
            stats: Stats::default(),
            work: Cell::new(work),
        }
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The length of the longest chain of nodes, which bounds recursion when lowering.
    pub fn deepest(&self) -> u32 {
        self.deepest
    }

    pub fn node(&self, v: V) -> Node {
        self.nodes[v.0 as usize]
    }

    /// Takes from the work budget. Returns false if there is not enough left.
    pub fn spend(&self, amount: u64) -> bool {
        match self.work.get().checked_sub(amount) {
            Some(left) => {
                self.work.set(left);
                true
            }
            None => false,
        }
    }

    /// Adds a node as is, reusing an identical one if it exists.
    pub fn intern(&mut self, node: Node) -> V {
        if let Some(existing) = self.index.get(&node) {
            return *existing;
        }
        let v = V(self.nodes.len() as u32);
        let depth = children(node)
            .iter()
            .flatten()
            .map(|c| self.depths[c.0 as usize])
            .max()
            .map_or(1, |d| d.saturating_add(1));
        self.deepest = self.deepest.max(depth);
        self.nodes.push(node);
        self.depths.push(depth);
        self.index.insert(node, v);
        v
    }

    /// Whether two commutative operands should swap to reach the canonical order: constants on the
    /// right, and otherwise a fixed order, so that `a + b` and `b + a` become one node.
    fn should_swap(&self, a: V, b: V) -> bool {
        self.as_const(a).is_some() || (self.as_const(b).is_none() && a > b)
    }

    /// Adds a binary node in canonical operand order, without simplifying it.
    pub fn intern_bin(&mut self, op: Bin, width: Width, a: V, b: V) -> V {
        let (a, b) = if op.commutative() && self.should_swap(a, b) {
            (b, a)
        } else {
            (a, b)
        };
        self.intern(Node::Bin(op, width, a, b))
    }

    pub fn constant(&mut self, width: Width, value: u64) -> V {
        self.intern(Node::Const(width, width.truncate(value)))
    }

    pub fn as_const(&self, v: V) -> Option<u64> {
        match self.node(v) {
            Node::Const(_, value) => Some(value),
            _ => None,
        }
    }

    pub fn value_type(&self, mut v: V) -> ValType {
        loop {
            match self.node(v) {
                Node::Const(width, _) | Node::Bin(_, width, _, _) | Node::Un(_, width, _) => {
                    return width.val_type();
                }
                Node::Input(_, ty) => return ty,
                Node::Cmp(..) | Node::Eqz(..) | Node::Wrap(_) => return ValType::I32,
                Node::ExtendU(_) | Node::ExtendS(_) => return ValType::I64,
                Node::Select(a, _, _) => v = a,
            }
        }
    }

    /// Evaluates a value given the values of its inputs. `None` if an input has no value.
    pub fn eval(&self, v: V, inputs: &dyn Fn(u32) -> Option<u64>) -> Option<u64> {
        let mut memo: HashMap<V, u64> = HashMap::new();
        self.eval_memo(v, inputs, &mut memo)
    }

    fn eval_memo(
        &self,
        v: V,
        inputs: &dyn Fn(u32) -> Option<u64>,
        memo: &mut HashMap<V, u64>,
    ) -> Option<u64> {
        if let Some(known) = memo.get(&v) {
            return Some(*known);
        }
        let out = match self.node(v) {
            Node::Const(_, value) => value,
            Node::Input(local, _) => inputs(local)?,
            Node::Bin(op, width, a, b) => {
                let (a, b) = (
                    self.eval_memo(a, inputs, memo)?,
                    self.eval_memo(b, inputs, memo)?,
                );
                eval_bin(op, width, a, b)
            }
            Node::Un(op, width, a) => eval_un(op, width, self.eval_memo(a, inputs, memo)?),
            Node::Cmp(op, width, a, b) => {
                let (a, b) = (
                    self.eval_memo(a, inputs, memo)?,
                    self.eval_memo(b, inputs, memo)?,
                );
                u64::from(eval_cmp(op, width, a, b))
            }
            Node::Eqz(_, a) => u64::from(self.eval_memo(a, inputs, memo)? == 0),
            Node::ExtendU(a) | Node::Wrap(a) => self.eval_memo(a, inputs, memo)? & 0xFFFF_FFFF,
            Node::ExtendS(a) => i64::from(self.eval_memo(a, inputs, memo)? as u32 as i32) as u64,
            Node::Select(a, b, c) => {
                if self.eval_memo(c, inputs, memo)? != 0 {
                    self.eval_memo(a, inputs, memo)?
                } else {
                    self.eval_memo(b, inputs, memo)?
                }
            }
        };
        memo.insert(v, out);
        Some(out)
    }

    /// Builds a binary operation and simplifies it as far as it can be proved.
    pub fn bin(&mut self, op: Bin, width: Width, a: V, b: V) -> V {
        if let (Some(x), Some(y)) = (self.as_const(a), self.as_const(b)) {
            self.stats.constants_folded += 1;
            return self.constant(width, eval_bin(op, width, x, y));
        }
        // Canonical operand order, so the rules only look at one arrangement.
        let (a, b) = if op.commutative() && self.should_swap(a, b) {
            (b, a)
        } else {
            (a, b)
        };
        if let Some(simple) = self.identity(op, width, a, b) {
            self.stats.identities_applied += 1;
            return simple;
        }
        // Reassociates a chain of constants: (x + 1) + 2 becomes x + 3.
        if op.commutative()
            && let (Some(c2), Node::Bin(inner, _, x, c1)) = (self.as_const(b), self.node(a))
            && inner == op
            && let Some(c1) = self.as_const(c1)
        {
            let folded = self.constant(width, eval_bin(op, width, c1, c2));
            self.stats.constants_folded += 1;
            return self.bin(op, width, x, folded);
        }
        // `x & 1` is exactly the lowest bit, which arithmetic alone can settle: x * x + x is always even.
        if op == Bin::And
            && self.as_const(b) == Some(1)
            && let Some(bit) = opaque::known_parity(self, a)
        {
            self.stats.opaque_predicates += 1;
            return self.constant(width, u64::from(bit));
        }
        let node = self.intern_bin(op, width, a, b);
        if mba::is_candidate(self, node)
            && let Some(simpler) = mba::simplify(self, node, width)
        {
            self.stats.mba_simplified += 1;
            return simpler;
        }
        node
    }

    /// Algebraic identities that hold for every input.
    fn identity(&mut self, op: Bin, width: Width, a: V, b: V) -> Option<V> {
        let cb = self.as_const(b);
        let ones = width.mask();
        match (op, cb) {
            (Bin::Add | Bin::Sub | Bin::Or | Bin::Xor, Some(0)) => return Some(a),
            (Bin::Mul | Bin::DivS | Bin::DivU, Some(1)) => return Some(a),
            (Bin::Mul | Bin::And, Some(0)) => return Some(self.constant(width, 0)),
            (Bin::RemS | Bin::RemU, Some(1)) => return Some(self.constant(width, 0)),
            (Bin::RemS, Some(c)) if c == ones => return Some(self.constant(width, 0)),
            (Bin::And, Some(c)) if c == ones => return Some(a),
            (Bin::Or, Some(c)) if c == ones => return Some(self.constant(width, ones)),
            // Shift amounts are taken modulo the width.
            (Bin::Shl | Bin::ShrS | Bin::ShrU | Bin::Rotl | Bin::Rotr, Some(c))
                if c.is_multiple_of(u64::from(width.bits())) =>
            {
                return Some(a);
            }
            // Unsigned division by a power of two is a shift, and the remainder is a mask. Those forms
            // are what the other rules understand.
            (Bin::DivU, Some(c)) if c.is_power_of_two() => {
                let shift = self.constant(width, u64::from(c.trailing_zeros()));
                return Some(self.bin(Bin::ShrU, width, a, shift));
            }
            (Bin::RemU, Some(c)) if c.is_power_of_two() => {
                let mask = self.constant(width, c - 1);
                return Some(self.bin(Bin::And, width, a, mask));
            }
            _ => {}
        }
        if a == b {
            match op {
                Bin::Sub | Bin::Xor => return Some(self.constant(width, 0)),
                Bin::And | Bin::Or => return Some(a),
                _ => {}
            }
        }
        // Absorption, with the matching operand in either position: (x | y) & x is x.
        if matches!(op, Bin::And | Bin::Or) {
            let dual = if op == Bin::And { Bin::Or } else { Bin::And };
            for (outer, other) in [(a, b), (b, a)] {
                if let Node::Bin(inner, _, x, y) = self.node(outer)
                    && inner == dual
                    && (x == other || y == other)
                {
                    return Some(other);
                }
            }
        }
        if let Some(simple) = self.complement(op, width, a, b) {
            return Some(simple);
        }
        // Undoing an operation.
        match (op, self.node(a), self.node(b)) {
            (Bin::Add, Node::Bin(Bin::Sub, _, x, y), _) if y == b => return Some(x),
            (Bin::Add, _, Node::Bin(Bin::Sub, _, x, y)) if y == a => return Some(x),
            (Bin::Sub, Node::Bin(Bin::Add, _, x, y), _) if y == b => return Some(x),
            (Bin::Sub, Node::Bin(Bin::Add, _, x, y), _) if x == b => return Some(y),
            (Bin::Xor, Node::Bin(Bin::Xor, _, x, y), _) if y == b => return Some(x),
            (Bin::Xor, Node::Bin(Bin::Xor, _, x, y), _) if x == b => return Some(y),
            _ => {}
        }
        None
    }

    /// `x & ~x` is 0, and `x | ~x` and `x ^ ~x` are all ones. Here `~x` is spelled `x ^ -1`.
    fn complement(&mut self, op: Bin, width: Width, a: V, b: V) -> Option<V> {
        let is_not_of = |g: &Graph, n: V, of: V| match g.node(n) {
            Node::Bin(Bin::Xor, _, x, c) => x == of && g.as_const(c) == Some(width.mask()),
            _ => false,
        };
        if is_not_of(self, a, b) || is_not_of(self, b, a) {
            return match op {
                Bin::And => Some(self.constant(width, 0)),
                Bin::Or | Bin::Xor => Some(self.constant(width, width.mask())),
                _ => None,
            };
        }
        None
    }

    pub fn un(&mut self, op: Un, width: Width, a: V) -> V {
        if let Some(x) = self.as_const(a) {
            self.stats.constants_folded += 1;
            return self.constant(width, eval_un(op, width, x));
        }
        self.intern(Node::Un(op, width, a))
    }

    pub fn cmp(&mut self, op: Cmp, width: Width, a: V, b: V) -> V {
        if let (Some(x), Some(y)) = (self.as_const(a), self.as_const(b)) {
            self.stats.constants_folded += 1;
            return self.constant(Width::W32, u64::from(eval_cmp(op, width, x, y)));
        }
        if a == b {
            self.stats.identities_applied += 1;
            let always = matches!(op, Cmp::Eq | Cmp::LeS | Cmp::LeU | Cmp::GeS | Cmp::GeU);
            return self.constant(Width::W32, u64::from(always));
        }
        // Equality does not care about order, and the proofs look for a constant on the right.
        let (a, b) = if matches!(op, Cmp::Eq | Cmp::Ne) && self.should_swap(a, b) {
            (b, a)
        } else {
            (a, b)
        };
        if let Some(verdict) = self.compare_ranges(op, width, a, b) {
            self.stats.opaque_predicates += 1;
            return self.constant(Width::W32, u64::from(verdict));
        }
        if matches!(op, Cmp::Eq | Cmp::Ne) {
            if let Some(always_equal) = opaque::decide_equality(self, width, a, b) {
                self.stats.opaque_predicates += 1;
                return self.constant(Width::W32, u64::from(always_equal == (op == Cmp::Eq)));
            }
            if op == Cmp::Eq && self.as_const(b) == Some(0) {
                return self.intern(Node::Eqz(width, a));
            }
        }
        self.intern(Node::Cmp(op, width, a, b))
    }

    /// Decides a comparison if the two ranges of possible values settle it.
    fn compare_ranges(&self, op: Cmp, width: Width, a: V, b: V) -> Option<bool> {
        let mut budget = RANGE_VISITS;
        let (alo, ahi) = self.range(a, width, &mut budget);
        let (blo, bhi) = self.range(b, width, &mut budget);
        // Signed comparisons agree with unsigned ones when neither side can have the sign bit set.
        let non_negative = ahi < width.sign_bit() && bhi < width.sign_bit();
        let settled = |always: bool, never: bool| {
            if always {
                Some(true)
            } else if never {
                Some(false)
            } else {
                None
            }
        };
        let apart = ahi < blo || bhi < alo;
        match op {
            Cmp::Eq => settled(false, apart),
            Cmp::Ne => settled(apart, false),
            Cmp::LtU => settled(ahi < blo, bhi <= alo),
            Cmp::LeU => settled(ahi <= blo, bhi < alo),
            Cmp::GtU => settled(bhi < alo, ahi <= blo),
            Cmp::GeU => settled(bhi <= alo, ahi < blo),
            Cmp::LtS if non_negative => settled(ahi < blo, bhi <= alo),
            Cmp::LeS if non_negative => settled(ahi <= blo, bhi < alo),
            Cmp::GtS if non_negative => settled(bhi < alo, ahi <= blo),
            Cmp::GeS if non_negative => settled(bhi <= alo, ahi < blo),
            _ => None,
        }
    }

    /// The lowest and highest unsigned value something can have. Always an over-approximation, and
    /// the whole range when it knows nothing.
    pub fn range(&self, v: V, width: Width, budget: &mut u32) -> (u64, u64) {
        let full = (0, width.mask());
        if *budget == 0 {
            return full;
        }
        *budget -= 1;
        let fill = |x: u64| {
            if x == 0 {
                0
            } else {
                u64::MAX >> x.leading_zeros()
            }
        };
        let fits = |x: Option<u64>| x.filter(|value| *value <= width.mask());
        match self.node(v) {
            Node::Const(_, c) => (c, c),
            Node::Input(..) => full,
            Node::Cmp(..) | Node::Eqz(..) => (0, 1),
            Node::Un(Un::Clz | Un::Ctz | Un::Popcnt, _, _) => (0, u64::from(width.bits())),
            Node::Un(..) => full,
            Node::ExtendU(a) => self.range(a, Width::W32, budget),
            Node::ExtendS(a) => {
                let (lo, hi) = self.range(a, Width::W32, budget);
                if hi < Width::W32.sign_bit() {
                    (lo, hi)
                } else {
                    full
                }
            }
            Node::Wrap(a) => {
                let (lo, hi) = self.range(a, Width::W64, budget);
                if hi <= Width::W32.mask() {
                    (lo, hi)
                } else {
                    full
                }
            }
            Node::Select(a, b, _) => {
                let (alo, ahi) = self.range(a, width, budget);
                let (blo, bhi) = self.range(b, width, budget);
                (alo.min(blo), ahi.max(bhi))
            }
            Node::Bin(op, _, a, b) => {
                let (alo, ahi) = self.range(a, width, budget);
                let (blo, bhi) = self.range(b, width, budget);
                let amount = self
                    .as_const(b)
                    .map(|c| (c % u64::from(width.bits())) as u32);
                match op {
                    Bin::And => (0, ahi.min(bhi)),
                    Bin::Or => (alo.max(blo), fill(ahi.max(bhi)) & width.mask()),
                    Bin::Xor => (0, fill(ahi.max(bhi)) & width.mask()),
                    Bin::Add => match fits(ahi.checked_add(bhi)) {
                        Some(hi) => (alo + blo, hi),
                        None => full,
                    },
                    Bin::Sub if alo >= bhi => (alo - bhi, ahi - blo),
                    Bin::Mul => match fits(ahi.checked_mul(bhi)) {
                        Some(hi) => (alo * blo, hi),
                        None => full,
                    },
                    Bin::Shl => match amount {
                        Some(s) if ahi.leading_zeros() >= s && fits(Some(ahi << s)).is_some() => {
                            (alo << s, ahi << s)
                        }
                        _ => full,
                    },
                    Bin::ShrU => match amount {
                        Some(s) => (alo >> s, ahi >> s),
                        None => full,
                    },
                    Bin::ShrS if ahi < width.sign_bit() => match amount {
                        Some(s) => (alo >> s, ahi >> s),
                        None => full,
                    },
                    Bin::DivU if blo > 0 => (alo / bhi, ahi / blo),
                    Bin::RemU if blo > 0 => (0, ahi.min(bhi - 1)),
                    Bin::RemS if blo > 0 && bhi < width.sign_bit() && ahi < width.sign_bit() => {
                        (0, ahi.min(bhi - 1))
                    }
                    _ => full,
                }
            }
        }
    }

    pub fn eqz(&mut self, width: Width, a: V) -> V {
        if let Some(x) = self.as_const(a) {
            self.stats.constants_folded += 1;
            return self.constant(Width::W32, u64::from(x == 0));
        }
        match self.node(a) {
            // A comparison yields 0 or 1, so eqz can just flip it.
            Node::Cmp(op, w, x, y) => {
                self.stats.identities_applied += 1;
                self.cmp(op.inverse(), w, x, y)
            }
            // eqz(eqz(x)) is x only if x is known to be 0 or 1, and is otherwise x != 0.
            Node::Eqz(w, x) if w == Width::W32 && self.is_boolean(x) => {
                self.stats.identities_applied += 1;
                x
            }
            _ => {
                let zero = self.constant(width, 0);
                self.cmp(Cmp::Eq, width, a, zero)
            }
        }
    }

    /// Whether a value is always exactly 0 or 1.
    pub fn is_boolean(&self, v: V) -> bool {
        match self.node(v) {
            Node::Cmp(..) | Node::Eqz(..) => true,
            Node::Const(_, c) => c <= 1,
            Node::Bin(Bin::And, _, _, c) => self.as_const(c).is_some_and(|c| c <= 1),
            _ => false,
        }
    }

    /// Removes the parts of an `i32` condition that make no difference to what is zero and what is not.
    pub fn truthy(&self, mut v: V) -> V {
        loop {
            match self.node(v) {
                Node::Cmp(Cmp::Ne, Width::W32, x, zero) if self.as_const(zero) == Some(0) => v = x,
                Node::Eqz(Width::W32, inner) => match self.node(inner) {
                    Node::Eqz(Width::W32, x) => v = x,
                    _ => return v,
                },
                _ => return v,
            }
        }
    }

    pub fn select(&mut self, a: V, b: V, condition: V) -> V {
        let condition = self.truthy(condition);
        if let Some(c) = self.as_const(condition) {
            self.stats.constants_folded += 1;
            return if c != 0 { a } else { b };
        }
        if a == b {
            self.stats.identities_applied += 1;
            return a;
        }
        self.intern(Node::Select(a, b, condition))
    }

    pub fn extend_u(&mut self, a: V) -> V {
        if let Some(x) = self.as_const(a) {
            return self.constant(Width::W64, x & 0xFFFF_FFFF);
        }
        self.intern(Node::ExtendU(a))
    }

    pub fn extend_s(&mut self, a: V) -> V {
        if let Some(x) = self.as_const(a) {
            return self.constant(Width::W64, i64::from(x as u32 as i32) as u64);
        }
        self.intern(Node::ExtendS(a))
    }

    pub fn wrap(&mut self, a: V) -> V {
        if let Some(x) = self.as_const(a) {
            return self.constant(Width::W32, x);
        }
        match self.node(a) {
            Node::ExtendU(x) | Node::ExtendS(x) => {
                self.stats.identities_applied += 1;
                x
            }
            _ => self.intern(Node::Wrap(a)),
        }
    }
}

/// How many nodes one comparison looks at when working out ranges.
const RANGE_VISITS: u32 = 64;

/// Whether dividing by this constant can never trap, so that the operation is pure.
pub fn safe_divisor(op: Bin, width: Width, divisor: u64) -> bool {
    match op {
        // The most negative value divided by -1 overflows, and that traps.
        Bin::DivS => divisor != 0 && divisor != width.mask(),
        Bin::DivU | Bin::RemS | Bin::RemU => divisor != 0,
        _ => false,
    }
}

pub fn eval_un(op: Un, width: Width, a: u64) -> u64 {
    let a = width.truncate(a);
    let out = match (op, width) {
        (Un::Clz, Width::W32) => u64::from((a as u32).leading_zeros()),
        (Un::Clz, Width::W64) => u64::from(a.leading_zeros()),
        (Un::Ctz, Width::W32) => u64::from((a as u32).trailing_zeros()),
        (Un::Ctz, Width::W64) => u64::from(a.trailing_zeros()),
        (Un::Popcnt, _) => u64::from(a.count_ones()),
        (Un::Extend8S, _) => i64::from(a as u8 as i8) as u64,
        (Un::Extend16S, _) => i64::from(a as u16 as i16) as u64,
        (Un::Extend32S, _) => i64::from(a as u32 as i32) as u64,
    };
    width.truncate(out)
}

pub fn eval_bin(op: Bin, width: Width, a: u64, b: u64) -> u64 {
    let bits = u64::from(width.bits());
    let shift = (b % bits) as u32;
    let (sa, sb) = (width.sign_extend(a), width.sign_extend(b));
    width.truncate(match op {
        Bin::Add => a.wrapping_add(b),
        Bin::Sub => a.wrapping_sub(b),
        Bin::Mul => a.wrapping_mul(b),
        Bin::And => a & b,
        Bin::Or => a | b,
        Bin::Xor => a ^ b,
        Bin::Shl => a.wrapping_shl(shift),
        Bin::ShrS => (sa >> shift) as u64,
        Bin::ShrU => width.truncate(a) >> shift,
        Bin::Rotl => match width {
            Width::W32 => u64::from((a as u32).rotate_left(shift)),
            Width::W64 => a.rotate_left(shift),
        },
        Bin::Rotr => match width {
            Width::W32 => u64::from((a as u32).rotate_right(shift)),
            Width::W64 => a.rotate_right(shift),
        },
        // A zero divisor traps and never gets this far, because of `safe_divisor`.
        Bin::DivS => sa.checked_div(sb).unwrap_or(0) as u64,
        Bin::DivU => width
            .truncate(a)
            .checked_div(width.truncate(b))
            .unwrap_or(0),
        Bin::RemS => sa.checked_rem(sb).unwrap_or(0) as u64,
        Bin::RemU => width
            .truncate(a)
            .checked_rem(width.truncate(b))
            .unwrap_or(0),
    })
}

pub fn eval_cmp(op: Cmp, width: Width, a: u64, b: u64) -> bool {
    let (sa, sb) = (width.sign_extend(a), width.sign_extend(b));
    match op {
        Cmp::Eq => a == b,
        Cmp::Ne => a != b,
        Cmp::LtS => sa < sb,
        Cmp::LtU => a < b,
        Cmp::GtS => sa > sb,
        Cmp::GtU => a > b,
        Cmp::LeS => sa <= sb,
        Cmp::LeU => a <= b,
        Cmp::GeS => sa >= sb,
        Cmp::GeU => a >= b,
    }
}
