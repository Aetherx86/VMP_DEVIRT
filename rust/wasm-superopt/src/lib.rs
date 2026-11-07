//! A WebAssembly deobfuscator and superoptimizer.
//!
//! Each function body is lifted from stack code into a hash-consed SSA graph, one region between
//! control-flow boundaries at a time. The graph is simplified with rules that are proofs, and then
//! lowered back to stack code.
//!
//! - Constant folding, algebraic identities, and propagating constants through locals
//! - Linear Mixed Boolean-Arithmetic simplification, which [`mba`] decides exactly
//! - Opaque predicate removal, which [`opaque`] proves through modular arithmetic and parity, and which
//!   the graph also proves through ranges of possible values
//! - Dead branch and dead code removal, and removal of redundant local and stack shuffling
//!
//! Three things hold however hostile the input is. Everything but the code section is kept byte for
//! byte. A function is only replaced if its encoding gets smaller, so the module never grows. And the
//! result is validated before it is returned.
//!
//! A function using anything this does not model, such as exceptions, garbage collection, tail calls
//! or vector instructions, is left exactly as it was.

pub mod mba;
pub mod opaque;
pub mod ssa;

use crate::ssa::{Bin, Cmp, Graph, Node, Un, V, Width, safe_divisor};
use std::collections::{HashMap, HashSet};
use wasm_encoder::reencode::{Reencode, RoundtripReencoder};
use wasm_encoder::{CodeSection, Function, Instruction, Module, RawSection, SectionId};
use wasmparser::{
    Encoding, FuncValidator, FuncValidatorAllocations, Operator, Parser, Payload, ValType,
    ValidPayload, Validator, ValidatorResources, WasmFeatures,
};

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Invalid(String),
    Unsupported(&'static str),
    /// The optimized module failed validation, which would be a bug in this tool.
    Internal(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Invalid(m) => write!(f, "invalid module: {m}"),
            Error::Unsupported(m) => write!(f, "unsupported: {m}"),
            Error::Internal(m) => write!(f, "internal error, please report this: {m}"),
        }
    }
}

impl std::error::Error for Error {}

fn invalid(e: impl std::fmt::Display) -> Error {
    Error::Invalid(e.to_string())
}

fn internal(e: impl std::fmt::Debug) -> Error {
    Error::Internal(format!("{e:?}"))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub passes: u32,
    pub functions: u32,
    pub functions_improved: u32,
    pub instructions_before: u64,
    pub instructions_after: u64,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub constants_folded: u32,
    pub identities_applied: u32,
    pub opaque_predicates: u32,
    pub mba_simplified: u32,
    pub branches_removed: u32,
}

pub struct Output {
    pub wasm: Vec<u8>,
    pub report: Report,
}

/// Limits on one function. Past any of these it is left alone, which bounds time, memory and
/// recursion on hostile input.
const MAX_BODY_OPERATORS: usize = 500_000;
const MAX_GRAPH_NODES: usize = 200_000;
const MAX_GRAPH_DEPTH: u32 = 2048;
const MAX_LOCALS: u32 = 40_000;

/// The searching proofs share this across a whole run, counted in nodes evaluated.
const WORK_BUDGET: u64 = 200_000_000;

/// Each pass can expose more to the next, such as a local that a removed branch was the last reader of.
const MAX_PASSES: u32 = 8;

/// Optimizes a core module. Everything but the code section is kept byte for byte.
pub fn optimize(wasm: &[u8]) -> Result<Output, Error> {
    let mut work = WORK_BUDGET;
    let mut report = Report {
        bytes_before: wasm.len() as u64,
        ..Report::default()
    };
    let mut improved = HashSet::new();
    let mut current: Option<Vec<u8>> = None;

    for _ in 0..MAX_PASSES {
        let input = current.as_deref().unwrap_or(wasm);
        let pass = run_pass(input, &mut work)?;
        report.passes += 1;
        if report.passes == 1 {
            report.functions = pass.report.functions;
            report.instructions_before = pass.report.instructions_before;
        }
        report.instructions_after = pass.report.instructions_after;
        report.constants_folded += pass.report.constants_folded;
        report.identities_applied += pass.report.identities_applied;
        report.opaque_predicates += pass.report.opaque_predicates;
        report.mba_simplified += pass.report.mba_simplified;
        report.branches_removed += pass.report.branches_removed;
        let progressed = !pass.improved.is_empty();
        improved.extend(pass.improved);
        if !progressed {
            break;
        }
        current = Some(pass.wasm);
    }

    report.functions_improved = improved.len() as u32;
    let bytes = current.unwrap_or_else(|| wasm.to_vec());
    report.bytes_after = bytes.len() as u64;
    Ok(Output {
        wasm: bytes,
        report,
    })
}

struct Pass {
    wasm: Vec<u8>,
    report: Report,
    improved: Vec<u32>,
}

fn run_pass(wasm: &[u8], work: &mut u64) -> Result<Pass, Error> {
    let compact_declared = !has_name_section(wasm)?;
    let mut validator = Validator::new_with_features(WasmFeatures::all());
    let mut allocations = FuncValidatorAllocations::default();
    let mut module = Module::new();
    let mut code = CodeSection::new();
    let mut remaining = 0u32;
    let mut report = Report::default();
    let mut improved = Vec::new();

    for payload in Parser::new(0).parse_all(wasm) {
        let payload = payload.map_err(invalid)?;
        if let Payload::Version {
            encoding: Encoding::Component,
            ..
        } = payload
        {
            return Err(Error::Unsupported(
                "components, as this works on core modules",
            ));
        }
        let validated = validator.payload(&payload).map_err(invalid)?;
        match &payload {
            Payload::CodeSectionStart { count, .. } => {
                remaining = *count;
                if remaining == 0 {
                    module.section(&code);
                }
            }
            Payload::CodeSectionEntry(_) => {}
            other => {
                if let Some((id, range)) = other.as_section() {
                    debug_assert_ne!(id, SectionId::Code as u8);
                    module.section(&RawSection {
                        id,
                        data: &wasm[range.start as usize..range.end as usize],
                    });
                }
            }
        }
        if let ValidPayload::Func(to_validate, body) = validated {
            let func = to_validate.into_validator(std::mem::take(&mut allocations));
            let index = report.functions;
            let result = optimize_function(func, &body, compact_declared, work, &mut report)?;
            allocations = result.allocations;
            match result.function {
                Some(function) => {
                    code.function(&function);
                    improved.push(index);
                }
                None => {
                    let range = body.range();
                    code.raw(&wasm[range.start as usize..range.end as usize]);
                }
            }
            remaining -= 1;
            if remaining == 0 {
                module.section(&code);
            }
        }
    }
    let bytes = module.finish();
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&bytes)
        .map_err(|e| Error::Internal(e.to_string()))?;
    Ok(Pass {
        wasm: bytes,
        report,
        improved,
    })
}

/// Local names refer to local indices, so the indices have to stay if the module has any names.
fn has_name_section(wasm: &[u8]) -> Result<bool, Error> {
    for payload in Parser::new(0).parse_all(wasm) {
        if let Payload::CustomSection(section) = payload.map_err(invalid)?
            && section.name() == "name"
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fold {
    /// An ordinary block, or an `if` whose condition is not known.
    None,
    /// An `if` that always takes its first arm.
    KeepThen,
    /// An `if` that always takes its second arm.
    KeepElse,
}

struct Frame {
    fold: Fold,
    block_type: wasmparser::BlockType,
    /// Whether the frame was entered in dead code, in which case nothing inside is emitted.
    entered_dead: bool,
    /// Whether this frame has an emitted opening instruction that needs a matching `end`.
    emitted: bool,
    /// What was known about locals at the start of an `if`, which is what its `else` starts from.
    known_at_start: HashMap<u32, V>,
}

struct Translator<'a> {
    graph: Graph,
    out: Vec<Instruction<'a>>,
    /// The part of the operand stack not yet emitted, always above whatever is really on the stack.
    stack: Vec<V>,
    /// Deferred writes to locals, which only reach the real locals at region boundaries.
    env: HashMap<u32, V>,
    /// Pure values already computed into a temporary during this region.
    computed: HashMap<V, u32>,
    /// Locals whose constant value is known here, which unlike `env` survives entering a block.
    known: HashMap<u32, V>,
    frames: Vec<Frame>,
    dead: bool,
    param_count: u32,
    /// Locals that some `local.get` reads and that some write changes. Dead code counts too, which is
    /// conservative, and the next pass picks up what that left behind.
    read: HashSet<u32>,
    written: HashSet<u32>,
    next_local: u32,
    new_locals: Vec<ValType>,
    /// How many operands the current instruction has already popped.
    pops: usize,
    /// Where the last branch was emitted, if it goes to the end of its own frame and leaves the stack
    /// exactly as falling through would. If nothing comes between it and that end, it does nothing.
    redundant_branch: Option<usize>,
    /// Set if something happened that this cannot model, so the function has to stay as it was.
    failed: bool,
    branches_removed: u32,
    reencoder: RoundtripReencoder,
}

impl<'a> Translator<'a> {
    fn new_temp(&mut self, ty: ValType) -> u32 {
        let local = self.next_local;
        self.next_local += 1;
        self.new_locals.push(ty);
        if self.next_local > MAX_LOCALS {
            self.failed = true;
        }
        local
    }

    fn input(&mut self, local: u32, ty: ValType) -> V {
        self.graph.intern(Node::Input(local, ty))
    }

    /// Takes one operand. If it is really on the machine stack, it moves to a temporary first.
    fn pop(&mut self, validator: &FuncValidator<ValidatorResources>) -> V {
        let depth = self.pops;
        self.pops += 1;
        if let Some(v) = self.stack.pop() {
            return v;
        }
        // The symbolic stack is the top of what the validator sees, which has not yet run this
        // instruction, so the operand's depth is how many this instruction has popped.
        let Some(ty) = validator.get_operand_type(depth).flatten() else {
            self.failed = true;
            return self.input(0, ValType::I32);
        };
        let temp = self.new_temp(ty);
        self.out.push(Instruction::LocalSet(temp));
        self.input(temp, ty)
    }

    /// Emits pure code that computes a value, leaving it on the machine stack.
    fn emit(&mut self, v: V, uses: &HashMap<V, u32>) {
        if let Some(temp) = self.computed.get(&v) {
            self.out.push(Instruction::LocalGet(*temp));
            return;
        }
        let node = self.graph.node(v);
        match node {
            Node::Const(Width::W32, c) => self.out.push(Instruction::I32Const(c as u32 as i32)),
            Node::Const(Width::W64, c) => self.out.push(Instruction::I64Const(c as i64)),
            Node::Input(local, _) => self.out.push(Instruction::LocalGet(local)),
            Node::Bin(op, width, a, b) => {
                self.emit(a, uses);
                self.emit(b, uses);
                self.out.push(bin_instruction(op, width));
            }
            Node::Un(op, width, a) => {
                self.emit(a, uses);
                self.out.push(un_instruction(op, width));
            }
            Node::Cmp(op, width, a, b) => {
                self.emit(a, uses);
                self.emit(b, uses);
                self.out.push(cmp_instruction(op, width));
            }
            Node::Eqz(width, a) => {
                self.emit(a, uses);
                self.out.push(match width {
                    Width::W32 => Instruction::I32Eqz,
                    Width::W64 => Instruction::I64Eqz,
                });
            }
            Node::ExtendU(a) => {
                self.emit(a, uses);
                self.out.push(Instruction::I64ExtendI32U);
            }
            Node::ExtendS(a) => {
                self.emit(a, uses);
                self.out.push(Instruction::I64ExtendI32S);
            }
            Node::Wrap(a) => {
                self.emit(a, uses);
                self.out.push(Instruction::I32WrapI64);
            }
            Node::Select(a, b, c) => {
                self.emit(a, uses);
                self.emit(b, uses);
                self.emit(c, uses);
                self.out.push(Instruction::Select);
            }
        }
        // A shared computation runs once, and later uses read the temporary. Without this, a chain
        // of doublings like x = x + x would expand exponentially when lowered.
        let shared = uses.get(&v).copied().unwrap_or(0) > 1;
        let trivial = matches!(node, Node::Const(..) | Node::Input(..));
        if shared && !trivial {
            let ty = self.graph.value_type(v);
            let temp = self.new_temp(ty);
            self.out.push(Instruction::LocalTee(temp));
            self.computed.insert(v, temp);
        }
    }

    fn count_uses(&self, v: V, uses: &mut HashMap<V, u32>) {
        let count = uses.entry(v).or_insert(0);
        *count += 1;
        if *count > 1 || self.computed.contains_key(&v) {
            return;
        }
        match self.graph.node(v) {
            Node::Bin(_, _, a, b) | Node::Cmp(_, _, a, b) => {
                self.count_uses(a, uses);
                self.count_uses(b, uses);
            }
            Node::Un(_, _, a)
            | Node::Eqz(_, a)
            | Node::ExtendU(a)
            | Node::ExtendS(a)
            | Node::Wrap(a) => {
                self.count_uses(a, uses);
            }
            Node::Select(a, b, c) => {
                self.count_uses(a, uses);
                self.count_uses(b, uses);
                self.count_uses(c, uses);
            }
            Node::Const(..) | Node::Input(..) => {}
        }
    }

    /// Puts all symbolic operands on the machine stack, then writes all deferred locals.
    ///
    /// Everything computes from the old local values, and only then do the writes happen, in reverse.
    /// This makes it a parallel assignment, so `swap`-like shuffles come out right without temporaries.
    fn flush(&mut self) {
        let mut writes: Vec<(u32, V)> = self.env.drain().collect();
        writes.sort_by_key(|(local, _)| *local);
        // A local assigned its own current value needs no write at all.
        writes
            .retain(|(local, v)| !matches!(self.graph.node(*v), Node::Input(l, _) if l == *local));

        let mut uses = HashMap::new();
        let stack = std::mem::take(&mut self.stack);
        for v in &stack {
            self.count_uses(*v, &mut uses);
        }
        for (_, v) in &writes {
            self.count_uses(*v, &mut uses);
        }
        for v in &stack {
            self.emit(*v, &uses);
        }
        for (_, v) in &writes {
            self.emit(*v, &uses);
        }
        for (local, v) in writes.iter().rev() {
            self.out.push(Instruction::LocalSet(*local));
            if self.graph.as_const(*v).is_some() {
                self.known.insert(*local, *v);
            } else {
                self.known.remove(local);
            }
        }
        // Any computed value might depend on a local that just changed.
        self.computed.clear();
    }

    /// Puts just the symbolic operands on the machine stack, keeping local writes deferred. Values
    /// already emitted still read the old locals, and calls and memory operations can't touch locals.
    fn flush_stack(&mut self) {
        let mut uses = HashMap::new();
        let stack = std::mem::take(&mut self.stack);
        for v in &stack {
            self.count_uses(*v, &mut uses);
        }
        for v in &stack {
            self.emit(*v, &uses);
        }
    }

    fn binary(&mut self, validator: &FuncValidator<ValidatorResources>, op: Bin, width: Width) {
        let b = self.pop(validator);
        let a = self.pop(validator);
        let v = self.graph.bin(op, width, a, b);
        self.stack.push(v);
    }

    fn unary(&mut self, validator: &FuncValidator<ValidatorResources>, op: Un, width: Width) {
        let a = self.pop(validator);
        let v = self.graph.un(op, width, a);
        self.stack.push(v);
    }

    fn compare(&mut self, validator: &FuncValidator<ValidatorResources>, op: Cmp, width: Width) {
        let b = self.pop(validator);
        let a = self.pop(validator);
        let v = self.graph.cmp(op, width, a, b);
        self.stack.push(v);
    }

    fn read_local(&mut self, index: u32, validator: &FuncValidator<ValidatorResources>) -> V {
        if let Some(v) = self.env.get(&index) {
            return *v;
        }
        if let Some(v) = self.known.get(&index) {
            return *v;
        }
        let ty = validator.get_local_type(index).unwrap_or(ValType::I32);
        // A declared local that nothing writes keeps its initial zero.
        if index >= self.param_count && !self.written.contains(&index) {
            match ty {
                ValType::I32 => return self.graph.constant(Width::W32, 0),
                ValType::I64 => return self.graph.constant(Width::W64, 0),
                _ => {}
            }
        }
        self.input(index, ty)
    }

    fn discard_top(&mut self) {
        // A symbolic operand can only be pure, so it can vanish. One on the machine stack has to be
        // dropped for real.
        if self.stack.pop().is_none() {
            self.out.push(Instruction::Drop);
        }
    }

    /// Handles an operator with the graph if it can. Returns false if it has to run as it is.
    fn symbolic(
        &mut self,
        op: &Operator<'a>,
        validator: &FuncValidator<ValidatorResources>,
    ) -> bool {
        use Operator as O;
        use Width::{W32, W64};
        match *op {
            O::Nop => {}
            O::I32Const { value } => {
                let v = self.graph.constant(W32, u64::from(value as u32));
                self.stack.push(v);
            }
            O::I64Const { value } => {
                let v = self.graph.constant(W64, value as u64);
                self.stack.push(v);
            }
            O::LocalGet { local_index } => {
                let v = self.read_local(local_index, validator);
                self.stack.push(v);
            }
            O::LocalSet { local_index } => {
                if self.read.contains(&local_index) {
                    let v = self.pop(validator);
                    self.env.insert(local_index, v);
                } else {
                    self.discard_top();
                }
            }
            O::LocalTee { local_index } => {
                if self.read.contains(&local_index) {
                    let v = self.pop(validator);
                    self.env.insert(local_index, v);
                    self.stack.push(v);
                }
            }
            O::Drop => self.discard_top(),
            O::Select => {
                let c = self.pop(validator);
                let b = self.pop(validator);
                let a = self.pop(validator);
                let v = self.graph.select(a, b, c);
                self.stack.push(v);
            }
            O::I32Add => self.binary(validator, Bin::Add, W32),
            O::I32Sub => self.binary(validator, Bin::Sub, W32),
            O::I32Mul => self.binary(validator, Bin::Mul, W32),
            O::I32And => self.binary(validator, Bin::And, W32),
            O::I32Or => self.binary(validator, Bin::Or, W32),
            O::I32Xor => self.binary(validator, Bin::Xor, W32),
            O::I32Shl => self.binary(validator, Bin::Shl, W32),
            O::I32ShrS => self.binary(validator, Bin::ShrS, W32),
            O::I32ShrU => self.binary(validator, Bin::ShrU, W32),
            O::I32Rotl => self.binary(validator, Bin::Rotl, W32),
            O::I32Rotr => self.binary(validator, Bin::Rotr, W32),
            O::I64Add => self.binary(validator, Bin::Add, W64),
            O::I64Sub => self.binary(validator, Bin::Sub, W64),
            O::I64Mul => self.binary(validator, Bin::Mul, W64),
            O::I64And => self.binary(validator, Bin::And, W64),
            O::I64Or => self.binary(validator, Bin::Or, W64),
            O::I64Xor => self.binary(validator, Bin::Xor, W64),
            O::I64Shl => self.binary(validator, Bin::Shl, W64),
            O::I64ShrS => self.binary(validator, Bin::ShrS, W64),
            O::I64ShrU => self.binary(validator, Bin::ShrU, W64),
            O::I64Rotl => self.binary(validator, Bin::Rotl, W64),
            O::I64Rotr => self.binary(validator, Bin::Rotr, W64),
            O::I32Clz => self.unary(validator, Un::Clz, W32),
            O::I32Ctz => self.unary(validator, Un::Ctz, W32),
            O::I32Popcnt => self.unary(validator, Un::Popcnt, W32),
            O::I64Clz => self.unary(validator, Un::Clz, W64),
            O::I64Ctz => self.unary(validator, Un::Ctz, W64),
            O::I64Popcnt => self.unary(validator, Un::Popcnt, W64),
            O::I32Extend8S => self.unary(validator, Un::Extend8S, W32),
            O::I32Extend16S => self.unary(validator, Un::Extend16S, W32),
            O::I64Extend8S => self.unary(validator, Un::Extend8S, W64),
            O::I64Extend16S => self.unary(validator, Un::Extend16S, W64),
            O::I64Extend32S => self.unary(validator, Un::Extend32S, W64),
            O::I32Eq => self.compare(validator, Cmp::Eq, W32),
            O::I32Ne => self.compare(validator, Cmp::Ne, W32),
            O::I32LtS => self.compare(validator, Cmp::LtS, W32),
            O::I32LtU => self.compare(validator, Cmp::LtU, W32),
            O::I32GtS => self.compare(validator, Cmp::GtS, W32),
            O::I32GtU => self.compare(validator, Cmp::GtU, W32),
            O::I32LeS => self.compare(validator, Cmp::LeS, W32),
            O::I32LeU => self.compare(validator, Cmp::LeU, W32),
            O::I32GeS => self.compare(validator, Cmp::GeS, W32),
            O::I32GeU => self.compare(validator, Cmp::GeU, W32),
            O::I64Eq => self.compare(validator, Cmp::Eq, W64),
            O::I64Ne => self.compare(validator, Cmp::Ne, W64),
            O::I64LtS => self.compare(validator, Cmp::LtS, W64),
            O::I64LtU => self.compare(validator, Cmp::LtU, W64),
            O::I64GtS => self.compare(validator, Cmp::GtS, W64),
            O::I64GtU => self.compare(validator, Cmp::GtU, W64),
            O::I64LeS => self.compare(validator, Cmp::LeS, W64),
            O::I64LeU => self.compare(validator, Cmp::LeU, W64),
            O::I64GeS => self.compare(validator, Cmp::GeS, W64),
            O::I64GeU => self.compare(validator, Cmp::GeU, W64),
            O::I32Eqz | O::I64Eqz => {
                let a = self.pop(validator);
                let width = if matches!(op, O::I32Eqz) { W32 } else { W64 };
                let v = self.graph.eqz(width, a);
                self.stack.push(v);
            }
            O::I64ExtendI32U => {
                let a = self.pop(validator);
                let v = self.graph.extend_u(a);
                self.stack.push(v);
            }
            O::I64ExtendI32S => {
                let a = self.pop(validator);
                let v = self.graph.extend_s(a);
                self.stack.push(v);
            }
            O::I32WrapI64 => {
                let a = self.pop(validator);
                let v = self.graph.wrap(a);
                self.stack.push(v);
            }
            // Division by a suitable constant cannot trap, which makes it pure. Anything else runs in
            // its original order, because whether it traps has to stay the same.
            O::I32DivS
            | O::I32DivU
            | O::I32RemS
            | O::I32RemU
            | O::I64DivS
            | O::I64DivU
            | O::I64RemS
            | O::I64RemU => {
                let (bin, width) = match op {
                    O::I32DivS => (Bin::DivS, W32),
                    O::I32DivU => (Bin::DivU, W32),
                    O::I32RemS => (Bin::RemS, W32),
                    O::I32RemU => (Bin::RemU, W32),
                    O::I64DivS => (Bin::DivS, W64),
                    O::I64DivU => (Bin::DivU, W64),
                    O::I64RemS => (Bin::RemS, W64),
                    _ => (Bin::RemU, W64),
                };
                let top = self.stack.last().and_then(|v| self.graph.as_const(*v));
                if !top.is_some_and(|divisor| safe_divisor(bin, width, divisor)) {
                    return false;
                }
                self.binary(validator, bin, width);
            }
            _ => return false,
        }
        true
    }

    fn known_condition(&mut self) -> Option<u64> {
        let top = self.stack.last_mut()?;
        *top = self.graph.truthy(*top);
        self.graph.as_const(*top)
    }

    fn open_frame(&mut self, fold: Fold, block_type: wasmparser::BlockType, emitted: bool) {
        self.frames.push(Frame {
            fold,
            block_type,
            entered_dead: false,
            emitted,
            known_at_start: self.known.clone(),
        });
    }

    /// Emits an unconditional branch, after which the code is dead. `extra` is how many operands the
    /// original instruction had besides what the branch carries.
    fn branch(&mut self, depth: u32, validator: &FuncValidator<ValidatorResources>, extra: usize) {
        self.redundant_branch = None;
        if depth == 0
            && let Some(frame) = validator.get_control_frame(0)
        {
            let results = match frame.block_type {
                wasmparser::BlockType::Empty => Some(0),
                wasmparser::BlockType::Type(_) => Some(1),
                wasmparser::BlockType::FuncType(_) => None,
            };
            let height = validator.operand_stack_height() as usize;
            let is_exact = results.is_some_and(|results| height == frame.height + results + extra);
            if is_exact && !frame.unreachable && frame.kind != wasmparser::FrameKind::Loop {
                self.redundant_branch = Some(self.out.len());
            }
        }
        self.out.push(Instruction::Br(depth));
        self.dead = true;
    }

    /// Removes the branch just before the end of its own frame, which would only do what falling
    /// through does anyway.
    fn drop_redundant_branch(&mut self) {
        if self
            .redundant_branch
            .take()
            .is_some_and(|at| at + 1 == self.out.len())
        {
            self.out.pop();
        }
    }

    /// Translates one instruction. Returns false if this function has to stay as it was.
    fn translate(
        &mut self,
        op: &Operator<'a>,
        validator: &FuncValidator<ValidatorResources>,
    ) -> Result<bool, Error> {
        use Operator as O;
        self.pops = 0;
        if self.dead {
            return self.translate_dead(op);
        }
        match op {
            O::Block { blockty } => {
                self.flush();
                self.open_frame(Fold::None, *blockty, true);
                self.push_op(op)?;
            }
            O::Loop { blockty } => {
                self.flush();
                // A branch back to here can come from later in the body, so nothing is known.
                self.known.clear();
                self.open_frame(Fold::None, *blockty, true);
                self.push_op(op)?;
            }
            O::If { blockty } => {
                // Folds the condition if it is known. The arm's labels stay valid either way, because
                // the live arm goes in a plain `block` of the same type.
                match self.known_condition() {
                    Some(value) => {
                        self.stack.pop();
                        self.flush();
                        self.branches_removed += 1;
                        if value != 0 {
                            let block = self.block_type(*blockty)?;
                            self.out.push(Instruction::Block(block));
                            self.open_frame(Fold::KeepThen, *blockty, true);
                        } else {
                            // Everything until the `else` is dead, and it opens the block there.
                            self.open_frame(Fold::KeepElse, *blockty, false);
                            self.dead = true;
                        }
                    }
                    None => {
                        self.flush();
                        self.open_frame(Fold::None, *blockty, true);
                        self.push_op(op)?;
                    }
                }
            }
            O::Else => {
                self.flush();
                let frame = self
                    .frames
                    .last()
                    .ok_or_else(|| Error::Internal("else with no frame".into()))?;
                if frame.fold == Fold::KeepThen {
                    // The live arm is finished, and the other arm is dead.
                    self.dead = true;
                } else {
                    self.known = frame.known_at_start.clone();
                    self.out.push(Instruction::Else);
                }
            }
            O::End => {
                // Nothing can see locals after the function returns.
                if self.frames.is_empty() {
                    self.env.clear();
                }
                self.flush();
                // More than one path can reach this point.
                self.known.clear();
                match self.frames.pop() {
                    Some(frame) if !frame.emitted => {}
                    _ => self.out.push(Instruction::End),
                }
            }
            O::Return => {
                self.env.clear();
                self.flush();
                self.out.push(Instruction::Return);
                self.dead = true;
            }
            O::Unreachable => {
                // Execution ends here, so neither pending values nor pending writes matter.
                self.env.clear();
                self.stack.clear();
                self.out.push(Instruction::Unreachable);
                self.dead = true;
            }
            O::Br { relative_depth } => {
                self.flush();
                self.branch(*relative_depth, validator, 0);
            }
            // The operand of a table is an index and not a condition, so nothing may simplify it as one.
            O::BrTable { targets } => match self.stack.last().and_then(|v| self.graph.as_const(*v))
            {
                Some(index) => {
                    self.stack.pop();
                    self.flush();
                    let chosen = targets
                        .targets()
                        .nth(index as usize)
                        .transpose()
                        .map_err(invalid)?;
                    self.branch(chosen.unwrap_or_else(|| targets.default()), validator, 1);
                    self.branches_removed += 1;
                }
                None => {
                    self.flush();
                    self.push_op(op)?;
                    self.dead = true;
                }
            },
            O::BrIf { relative_depth } => match self.known_condition() {
                Some(0) => {
                    self.stack.pop();
                    self.branches_removed += 1;
                }
                Some(_) => {
                    self.stack.pop();
                    self.flush();
                    self.branch(*relative_depth, validator, 1);
                    self.branches_removed += 1;
                }
                None => {
                    self.flush();
                    self.push_op(op)?;
                }
            },
            other => {
                if !self.symbolic(other, validator) {
                    if !runs_in_place(other) {
                        return Ok(false);
                    }
                    // Calls, memory access, floats and so on run in their original order.
                    self.flush_stack();
                    self.push_op(other)?;
                }
            }
        }
        Ok(!self.failed)
    }

    fn translate_dead(&mut self, op: &Operator<'a>) -> Result<bool, Error> {
        use Operator as O;
        match op {
            // These open or close frames in ways the dead code walk does not track.
            O::Try { .. }
            | O::TryTable { .. }
            | O::Catch { .. }
            | O::CatchAll
            | O::Delegate { .. } => return Ok(false),
            O::Block { blockty } | O::Loop { blockty } | O::If { blockty } => {
                self.frames.push(Frame {
                    fold: Fold::None,
                    block_type: *blockty,
                    entered_dead: true,
                    emitted: false,
                    known_at_start: HashMap::new(),
                });
            }
            O::Else => {
                let frame = self
                    .frames
                    .last_mut()
                    .ok_or_else(|| Error::Internal("else with no frame".into()))?;
                if frame.entered_dead {
                    return Ok(true);
                }
                if frame.fold == Fold::KeepThen {
                    self.drop_redundant_branch();
                    return Ok(true);
                }
                let fold = frame.fold;
                let block_type = frame.block_type;
                if fold == Fold::KeepElse {
                    frame.emitted = true;
                }
                self.known = frame.known_at_start.clone();
                self.drop_redundant_branch();
                if fold == Fold::KeepElse {
                    // The dead arm is over, and this one always runs.
                    let block = self.block_type(block_type)?;
                    self.out.push(Instruction::Block(block));
                } else {
                    self.out.push(Instruction::Else);
                }
                self.dead = false;
            }
            O::End => match self.frames.pop() {
                Some(frame) => {
                    if !frame.entered_dead {
                        self.drop_redundant_branch();
                        if frame.emitted {
                            self.out.push(Instruction::End);
                        }
                        self.known.clear();
                        self.dead = false;
                    }
                }
                None => {
                    self.out.push(Instruction::End);
                    self.dead = false;
                }
            },
            _ => {}
        }
        Ok(true)
    }

    fn block_type(
        &mut self,
        block: wasmparser::BlockType,
    ) -> Result<wasm_encoder::BlockType, Error> {
        self.reencoder.block_type(block).map_err(internal)
    }

    fn push_op(&mut self, op: &Operator<'a>) -> Result<(), Error> {
        let instruction = self.reencoder.instruction(op.clone()).map_err(internal)?;
        self.out.push(instruction);
        Ok(())
    }
}

/// Instructions that this understands to have no effect on control flow, so they can be kept in
/// place between the rewritten parts. This is an allow list so that anything it has never heard of,
/// including whatever the format adds later, leaves the function untouched instead of being mistranslated.
fn runs_in_place(op: &Operator<'_>) -> bool {
    use Operator as O;
    matches!(
        op,
        O::Call { .. }
            | O::CallIndirect { .. }
            | O::GlobalGet { .. }
            | O::GlobalSet { .. }
            | O::TypedSelect { .. }
            | O::I32Load { .. }
            | O::I64Load { .. }
            | O::F32Load { .. }
            | O::F64Load { .. }
            | O::I32Load8S { .. }
            | O::I32Load8U { .. }
            | O::I32Load16S { .. }
            | O::I32Load16U { .. }
            | O::I64Load8S { .. }
            | O::I64Load8U { .. }
            | O::I64Load16S { .. }
            | O::I64Load16U { .. }
            | O::I64Load32S { .. }
            | O::I64Load32U { .. }
            | O::I32Store { .. }
            | O::I64Store { .. }
            | O::F32Store { .. }
            | O::F64Store { .. }
            | O::I32Store8 { .. }
            | O::I32Store16 { .. }
            | O::I64Store8 { .. }
            | O::I64Store16 { .. }
            | O::I64Store32 { .. }
            | O::MemorySize { .. }
            | O::MemoryGrow { .. }
            | O::MemoryInit { .. }
            | O::DataDrop { .. }
            | O::MemoryCopy { .. }
            | O::MemoryFill { .. }
            | O::TableInit { .. }
            | O::ElemDrop { .. }
            | O::TableCopy { .. }
            | O::TableFill { .. }
            | O::TableGet { .. }
            | O::TableSet { .. }
            | O::TableGrow { .. }
            | O::TableSize { .. }
            | O::RefNull { .. }
            | O::RefIsNull
            | O::RefFunc { .. }
            | O::F32Const { .. }
            | O::F64Const { .. }
            | O::I32DivS
            | O::I32DivU
            | O::I32RemS
            | O::I32RemU
            | O::I64DivS
            | O::I64DivU
            | O::I64RemS
            | O::I64RemU
            | O::F32Eq
            | O::F32Ne
            | O::F32Lt
            | O::F32Gt
            | O::F32Le
            | O::F32Ge
            | O::F64Eq
            | O::F64Ne
            | O::F64Lt
            | O::F64Gt
            | O::F64Le
            | O::F64Ge
            | O::F32Abs
            | O::F32Neg
            | O::F32Ceil
            | O::F32Floor
            | O::F32Trunc
            | O::F32Nearest
            | O::F32Sqrt
            | O::F32Add
            | O::F32Sub
            | O::F32Mul
            | O::F32Div
            | O::F32Min
            | O::F32Max
            | O::F32Copysign
            | O::F64Abs
            | O::F64Neg
            | O::F64Ceil
            | O::F64Floor
            | O::F64Trunc
            | O::F64Nearest
            | O::F64Sqrt
            | O::F64Add
            | O::F64Sub
            | O::F64Mul
            | O::F64Div
            | O::F64Min
            | O::F64Max
            | O::F64Copysign
            | O::I32TruncF32S
            | O::I32TruncF32U
            | O::I32TruncF64S
            | O::I32TruncF64U
            | O::I64TruncF32S
            | O::I64TruncF32U
            | O::I64TruncF64S
            | O::I64TruncF64U
            | O::F32ConvertI32S
            | O::F32ConvertI32U
            | O::F32ConvertI64S
            | O::F32ConvertI64U
            | O::F32DemoteF64
            | O::F64ConvertI32S
            | O::F64ConvertI32U
            | O::F64ConvertI64S
            | O::F64ConvertI64U
            | O::F64PromoteF32
            | O::I32ReinterpretF32
            | O::I64ReinterpretF64
            | O::F32ReinterpretI32
            | O::F64ReinterpretI64
            | O::I32TruncSatF32S
            | O::I32TruncSatF32U
            | O::I32TruncSatF64S
            | O::I32TruncSatF64U
            | O::I64TruncSatF32S
            | O::I64TruncSatF32U
            | O::I64TruncSatF64S
            | O::I64TruncSatF64U
    )
}

fn bin_instruction(op: Bin, width: Width) -> Instruction<'static> {
    use Instruction as I;
    match (op, width) {
        (Bin::Add, Width::W32) => I::I32Add,
        (Bin::Sub, Width::W32) => I::I32Sub,
        (Bin::Mul, Width::W32) => I::I32Mul,
        (Bin::And, Width::W32) => I::I32And,
        (Bin::Or, Width::W32) => I::I32Or,
        (Bin::Xor, Width::W32) => I::I32Xor,
        (Bin::Shl, Width::W32) => I::I32Shl,
        (Bin::ShrS, Width::W32) => I::I32ShrS,
        (Bin::ShrU, Width::W32) => I::I32ShrU,
        (Bin::Rotl, Width::W32) => I::I32Rotl,
        (Bin::Rotr, Width::W32) => I::I32Rotr,
        (Bin::DivS, Width::W32) => I::I32DivS,
        (Bin::DivU, Width::W32) => I::I32DivU,
        (Bin::RemS, Width::W32) => I::I32RemS,
        (Bin::RemU, Width::W32) => I::I32RemU,
        (Bin::Add, Width::W64) => I::I64Add,
        (Bin::Sub, Width::W64) => I::I64Sub,
        (Bin::Mul, Width::W64) => I::I64Mul,
        (Bin::And, Width::W64) => I::I64And,
        (Bin::Or, Width::W64) => I::I64Or,
        (Bin::Xor, Width::W64) => I::I64Xor,
        (Bin::Shl, Width::W64) => I::I64Shl,
        (Bin::ShrS, Width::W64) => I::I64ShrS,
        (Bin::ShrU, Width::W64) => I::I64ShrU,
        (Bin::Rotl, Width::W64) => I::I64Rotl,
        (Bin::Rotr, Width::W64) => I::I64Rotr,
        (Bin::DivS, Width::W64) => I::I64DivS,
        (Bin::DivU, Width::W64) => I::I64DivU,
        (Bin::RemS, Width::W64) => I::I64RemS,
        (Bin::RemU, Width::W64) => I::I64RemU,
    }
}

fn un_instruction(op: Un, width: Width) -> Instruction<'static> {
    use Instruction as I;
    match (op, width) {
        (Un::Clz, Width::W32) => I::I32Clz,
        (Un::Ctz, Width::W32) => I::I32Ctz,
        (Un::Popcnt, Width::W32) => I::I32Popcnt,
        (Un::Extend8S, Width::W32) => I::I32Extend8S,
        (Un::Extend16S, Width::W32) => I::I32Extend16S,
        (Un::Clz, Width::W64) => I::I64Clz,
        (Un::Ctz, Width::W64) => I::I64Ctz,
        (Un::Popcnt, Width::W64) => I::I64Popcnt,
        (Un::Extend8S, Width::W64) => I::I64Extend8S,
        (Un::Extend16S, Width::W64) => I::I64Extend16S,
        // Only `i64` has this one, so nothing builds it at 32 bits.
        (Un::Extend32S, _) => I::I64Extend32S,
    }
}

fn cmp_instruction(op: Cmp, width: Width) -> Instruction<'static> {
    use Instruction as I;
    match (op, width) {
        (Cmp::Eq, Width::W32) => I::I32Eq,
        (Cmp::Ne, Width::W32) => I::I32Ne,
        (Cmp::LtS, Width::W32) => I::I32LtS,
        (Cmp::LtU, Width::W32) => I::I32LtU,
        (Cmp::GtS, Width::W32) => I::I32GtS,
        (Cmp::GtU, Width::W32) => I::I32GtU,
        (Cmp::LeS, Width::W32) => I::I32LeS,
        (Cmp::LeU, Width::W32) => I::I32LeU,
        (Cmp::GeS, Width::W32) => I::I32GeS,
        (Cmp::GeU, Width::W32) => I::I32GeU,
        (Cmp::Eq, Width::W64) => I::I64Eq,
        (Cmp::Ne, Width::W64) => I::I64Ne,
        (Cmp::LtS, Width::W64) => I::I64LtS,
        (Cmp::LtU, Width::W64) => I::I64LtU,
        (Cmp::GtS, Width::W64) => I::I64GtS,
        (Cmp::GtU, Width::W64) => I::I64GtU,
        (Cmp::LeS, Width::W64) => I::I64LeS,
        (Cmp::LeU, Width::W64) => I::I64LeU,
        (Cmp::GeS, Width::W64) => I::I64GeS,
        (Cmp::GeU, Width::W64) => I::I64GeU,
    }
}

/// Cleans up what the translation leaves between its pieces. All of these hold for any local.
fn peephole(out: &mut Vec<Instruction<'_>>) {
    let mut gets: HashMap<u32, u32> = HashMap::new();
    let mut sets: HashMap<u32, u32> = HashMap::new();
    for instruction in out.iter() {
        match instruction {
            Instruction::LocalGet(l) => *gets.entry(*l).or_insert(0) += 1,
            Instruction::LocalSet(l) | Instruction::LocalTee(l) => {
                *sets.entry(*l).or_insert(0) += 1
            }
            _ => {}
        }
    }
    let read = |l: u32| gets.get(&l).copied().unwrap_or(0);
    let written = |l: u32| sets.get(&l).copied().unwrap_or(0);
    let mut result = Vec::with_capacity(out.len());
    for instruction in out.drain(..) {
        match (result.last(), &instruction) {
            // A value moved into a local only to be read straight back.
            (Some(Instruction::LocalSet(s)), Instruction::LocalGet(g)) if s == g => {
                let local = *s;
                result.pop();
                if read(local) != 1 || written(local) != 1 {
                    result.push(Instruction::LocalTee(local));
                }
                continue;
            }
            // Something pure, computed and then thrown away.
            (
                Some(
                    Instruction::LocalGet(_) | Instruction::I32Const(_) | Instruction::I64Const(_),
                ),
                Instruction::Drop,
            ) => {
                result.pop();
                continue;
            }
            (Some(Instruction::LocalTee(l)), Instruction::Drop) => {
                let local = *l;
                result.pop();
                result.push(Instruction::LocalSet(local));
                continue;
            }
            // An empty `else` is only valid if the `if` leaves the stack alone, which no `else` does too.
            (Some(Instruction::Else), Instruction::End) => {
                result.pop();
                result.push(Instruction::End);
                continue;
            }
            // Both arms are empty, so all that happens is that the condition is popped.
            (Some(Instruction::If(_)), Instruction::End) => {
                result.pop();
                result.push(Instruction::Drop);
                continue;
            }
            _ => {}
        }
        // Nothing ever reads these.
        match &instruction {
            Instruction::LocalSet(l) if read(*l) == 0 => result.push(Instruction::Drop),
            Instruction::LocalTee(l) if read(*l) == 0 => {}
            _ => result.push(instruction),
        }
    }
    *out = result;
}

/// Removes blocks and loops that no branch targets. With no branch to its label, a block is just its
/// contents, and that is what a folded `if` leaves behind. The depth of any branch that crosses one
/// has to go down by one.
fn flatten_blocks(out: &mut Vec<Instruction<'_>>) {
    let mut targeted = vec![false; out.len()];
    let mut open: Vec<usize> = Vec::new();
    let mark = |open: &[usize], targeted: &mut [bool], depth: u32| {
        if let Some(at) = open.len().checked_sub(1 + depth as usize) {
            targeted[open[at]] = true;
        }
    };
    for (i, instruction) in out.iter().enumerate() {
        match instruction {
            Instruction::Block(_) | Instruction::Loop(_) | Instruction::If(_) => open.push(i),
            Instruction::End => {
                open.pop();
            }
            Instruction::Br(depth) | Instruction::BrIf(depth) => mark(&open, &mut targeted, *depth),
            Instruction::BrTable(list, default) => {
                for depth in list.iter().chain(std::iter::once(default)) {
                    mark(&open, &mut targeted, *depth);
                }
            }
            _ => {}
        }
    }
    // For each open frame, whether it was removed, and how many below it were.
    let mut frames: Vec<(bool, u32)> = Vec::new();
    let mut removed = 0u32;
    let mut result = Vec::with_capacity(out.len());
    for (i, instruction) in out.drain(..).enumerate() {
        // A branch crosses the top `depth` frames, so its new depth leaves out any of those removed.
        let new_depth = |frames: &[(bool, u32)], removed: u32, depth: u32| match frames
            .get(frames.len().saturating_sub(depth as usize))
        {
            Some((_, below)) => depth - (removed - below),
            None => depth,
        };
        match instruction {
            Instruction::Block(_) | Instruction::Loop(_) if !targeted[i] => {
                frames.push((true, removed));
                removed += 1;
            }
            Instruction::Block(_) | Instruction::Loop(_) | Instruction::If(_) => {
                frames.push((false, removed));
                result.push(instruction);
            }
            Instruction::End => match frames.pop() {
                Some((true, below)) => removed = below,
                _ => result.push(instruction),
            },
            Instruction::Br(depth) => {
                result.push(Instruction::Br(new_depth(&frames, removed, depth)))
            }
            Instruction::BrIf(depth) => {
                result.push(Instruction::BrIf(new_depth(&frames, removed, depth)))
            }
            Instruction::BrTable(list, default) => {
                let list: Vec<u32> = list
                    .iter()
                    .map(|depth| new_depth(&frames, removed, *depth))
                    .collect();
                result.push(Instruction::BrTable(
                    list.into(),
                    new_depth(&frames, removed, default),
                ));
            }
            other => result.push(other),
        }
    }
    *out = result;
}

/// Renumbers locals after the parameters, leaving out the ones no instruction uses any more. Returns
/// the declarations for what is left.
fn compact_locals(
    out: &mut [Instruction<'_>],
    params: u32,
    declared: &[ValType],
    temps: &[ValType],
    keep_declared: bool,
) -> Vec<ValType> {
    let declared_count = declared.len() as u32;
    let total = (declared.len() + temps.len()) as u32;
    let mut used = vec![false; total as usize];
    for instruction in out.iter() {
        if let Instruction::LocalGet(l) | Instruction::LocalSet(l) | Instruction::LocalTee(l) =
            instruction
            && *l >= params
        {
            used[(*l - params) as usize] = true;
        }
    }
    let mut mapping = vec![0u32; total as usize];
    let mut types = Vec::new();
    for slot in 0..total {
        let is_declared = slot < declared_count;
        if used[slot as usize] || (is_declared && keep_declared) {
            mapping[slot as usize] = params + types.len() as u32;
            types.push(if is_declared {
                declared[slot as usize]
            } else {
                temps[(slot - declared_count) as usize]
            });
        }
    }
    for instruction in out.iter_mut() {
        if let Instruction::LocalGet(l) | Instruction::LocalSet(l) | Instruction::LocalTee(l) =
            instruction
            && *l >= params
        {
            *l = mapping[(*l - params) as usize];
        }
    }
    types
}

struct Rewritten {
    function: Option<Function>,
    allocations: FuncValidatorAllocations,
}

fn optimize_function<'a>(
    mut validator: FuncValidator<ValidatorResources>,
    body: &wasmparser::FunctionBody<'a>,
    compact_declared: bool,
    work: &mut u64,
    report: &mut Report,
) -> Result<Rewritten, Error> {
    let params = validator.len_locals();

    let mut declared: Vec<ValType> = Vec::new();
    let mut local_reader = body.get_locals_reader().map_err(invalid)?;
    for _ in 0..local_reader.get_count() {
        let offset = local_reader.original_position();
        let (count, ty) = local_reader.read().map_err(invalid)?;
        validator
            .define_locals(offset, count, ty)
            .map_err(invalid)?;
        declared.extend(std::iter::repeat_n(ty, count as usize));
    }

    let mut read = HashSet::new();
    let mut written = HashSet::new();
    let mut scan = body.get_operators_reader().map_err(invalid)?;
    while !scan.eof() {
        match scan.read().map_err(invalid)? {
            Operator::LocalGet { local_index } => {
                read.insert(local_index);
            }
            Operator::LocalSet { local_index } | Operator::LocalTee { local_index } => {
                written.insert(local_index);
            }
            _ => {}
        }
    }

    let total_locals = params + declared.len() as u32;
    let mut translator = Translator {
        graph: Graph::new(*work),
        out: Vec::new(),
        stack: Vec::new(),
        env: HashMap::new(),
        computed: HashMap::new(),
        known: HashMap::new(),
        frames: Vec::new(),
        dead: false,
        param_count: params,
        read,
        written,
        next_local: total_locals,
        new_locals: Vec::new(),
        pops: 0,
        redundant_branch: None,
        failed: false,
        branches_removed: 0,
        reencoder: RoundtripReencoder,
    };

    let mut operators = body.get_operators_reader().map_err(invalid)?;
    let mut before = 0u64;
    let mut give_up = total_locals > MAX_LOCALS;
    while !operators.eof() {
        let offset = operators.original_position();
        let op = operators.read().map_err(invalid)?;
        before += 1;
        if !give_up {
            let carried_on = translator.translate(&op, &validator)?;
            if !carried_on
                || before as usize > MAX_BODY_OPERATORS
                || translator.graph.len() > MAX_GRAPH_NODES
                || translator.graph.deepest() > MAX_GRAPH_DEPTH
            {
                give_up = true;
            }
        }
        validator.op(offset, &op).map_err(invalid)?;
    }
    operators.finish().map_err(invalid)?;
    *work = translator.graph.work.get();

    report.functions += 1;
    report.instructions_before += before;

    let mut rewritten = None;
    let mut after = before;
    if !give_up {
        peephole(&mut translator.out);
        flatten_blocks(&mut translator.out);
        peephole(&mut translator.out);
        let types = compact_locals(
            &mut translator.out,
            params,
            &declared,
            &translator.new_locals,
            !compact_declared,
        );
        let mut groups: Vec<(u32, wasm_encoder::ValType)> = Vec::new();
        for ty in types {
            let ty = translator.reencoder.val_type(ty).map_err(internal)?;
            match groups.last_mut() {
                Some((count, last)) if *last == ty => *count += 1,
                _ => groups.push((1, ty)),
            }
        }
        let mut function = Function::new(groups);
        for instruction in &translator.out {
            function.instruction(instruction);
        }
        // A tie keeps the original, because a rewrite that gains nothing only makes the result
        // harder to compare, and could otherwise repeat forever.
        let original = body.range();
        if (function.byte_len() as u64) < original.end - original.start {
            after = translator.out.len() as u64;
            report.functions_improved += 1;
            report.constants_folded += translator.graph.stats.constants_folded;
            report.identities_applied += translator.graph.stats.identities_applied;
            report.opaque_predicates += translator.graph.stats.opaque_predicates;
            report.mba_simplified += translator.graph.stats.mba_simplified;
            report.branches_removed += translator.branches_removed;
            rewritten = Some(function);
        }
    }
    report.instructions_after += after;
    Ok(Rewritten {
        function: rewritten,
        allocations: validator.into_allocations(),
    })
}
