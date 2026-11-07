//! Lowers WebAssembly to the analyzer's representation, to prove memory accesses in bounds.
//!
//! Machine integers are kept as their unsigned values, with wrapping explicit. The analysis then proves
//! that a load or store cannot run past the memory's declared minimum size, and that a division cannot
//! have a zero divisor. Anything else is havoc: a call may return anything, and so may an operation
//! with no model here. Memory contents and globals are not tracked at all.
//!
//! Vector, atomic and bulk memory accesses are not checked. A function using control flow that this
//! does not model, such as exceptions or tail calls, is listed as skipped and not analyzed at all.

use crate::domain::Interval;
use crate::ir::{
    BinOp, Block, CmpOp, Cond, Function, Instr, Operand, Program, Term, ThreadKind, Var,
};
use wasmparser::{
    BlockType, FuncValidator, FuncValidatorAllocations, Operator, Parser, Payload, TypeRef,
    ValType, ValidPayload, Validator, ValidatorResources, WasmFeatures, WasmModuleResources,
};

#[derive(Debug, PartialEq, Eq)]
pub struct WasmError(pub String);

impl std::fmt::Display for WasmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid module: {}", self.0)
    }
}

impl std::error::Error for WasmError {}

fn bad(e: impl std::fmt::Display) -> WasmError {
    WasmError(e.to_string())
}

#[derive(Debug, Default)]
pub struct Lowered {
    pub program: Program,
    /// Functions left out, and why. Nothing is claimed about them.
    pub skipped: Vec<(String, String)>,
}

const MAX_OPERATORS: usize = 100_000;

#[derive(Clone, Copy, Debug)]
enum Item {
    Const(i128),
    /// The current value of a local, kept as a reference so that a comparison narrows the local.
    Local(u32),
    Temp(Var),
    Cmp(Cond),
}

enum Kind {
    Block,
    Loop,
    If { else_block: Option<usize> },
}

struct Frame {
    kind: Kind,
    /// Where a branch to this frame goes: the head of a loop, otherwise its end.
    target: usize,
    end: usize,
    params: Vec<Var>,
    results: Vec<Var>,
    height: usize,
}

struct Builder {
    blocks: Vec<Block>,
    current: usize,
    var_names: Vec<String>,
    stack: Vec<Item>,
    frames: Vec<Frame>,
    dead: bool,
    dead_nesting: u32,
    memories: Vec<u128>,
    line: u32,
    /// Results of the last operator, by block and instruction, that still need their types.
    untyped: Vec<(usize, usize)>,
}

/// Which memory an access goes to. An access to anything else counts as the first memory, which the
/// bounds check against a memory of size 0 would flag anyway if there is none.
fn memory_of(memarg: &wasmparser::MemArg) -> u32 {
    match memarg.target {
        wasmparser::MemArgTarget::Memory(index) => index,
        _ => u32::MAX,
    }
}

fn range_of(ty: Option<ValType>) -> Interval {
    match ty {
        Some(ValType::I32) => Interval::unsigned_bits(32),
        Some(ValType::I64) => Interval::unsigned_bits(64),
        _ => Interval::TOP,
    }
}

impl Builder {
    fn new_var(&mut self) -> Var {
        let v = self.var_names.len() as Var;
        self.var_names.push(format!("t{v}"));
        v
    }

    fn new_block(&mut self) -> usize {
        self.blocks.push(Block {
            name: format!("b{}", self.blocks.len()),
            instrs: Vec::new(),
            term: Term::Return,
        });
        self.blocks.len() - 1
    }

    fn emit(&mut self, instr: Instr) {
        let line = self.line;
        self.blocks[self.current].instrs.push((instr, line));
    }

    fn operand(&mut self, item: Item) -> Operand {
        match item {
            Item::Const(c) => Operand::Const(c),
            Item::Local(l) => Operand::Var(l),
            Item::Temp(v) => Operand::Var(v),
            Item::Cmp(c) => {
                // A truth value kept as a number, so the link back to what it tested is lost.
                let v = self.new_var();
                let _ = c;
                self.emit(Instr::Havoc(v, Interval::new(0, 1)));
                Operand::Var(v)
            }
        }
    }

    fn pop(&mut self) -> Result<Item, WasmError> {
        self.stack
            .pop()
            .ok_or_else(|| WasmError("the operand stack ran out".into()))
    }

    fn pop_operand(&mut self) -> Result<Operand, WasmError> {
        let item = self.pop()?;
        Ok(self.operand(item))
    }

    /// Copies a stack entry out of anything that could change under it.
    fn settle(&mut self, at: usize) {
        match self.stack[at] {
            Item::Local(l) => {
                let v = self.new_var();
                self.emit(Instr::Copy(v, Operand::Var(l)));
                self.stack[at] = Item::Temp(v);
            }
            Item::Cmp(_) => {
                let v = self.new_var();
                self.emit(Instr::Havoc(v, Interval::new(0, 1)));
                self.stack[at] = Item::Temp(v);
            }
            _ => {}
        }
    }

    /// Before a local changes, every waiting reference to it has to take its current value.
    fn settle_local(&mut self, local: u32) {
        for at in 0..self.stack.len() {
            let refers = match self.stack[at] {
                Item::Local(l) => l == local,
                Item::Cmp(c) => c.a == Operand::Var(local) || c.b == Operand::Var(local),
                _ => false,
            };
            if refers {
                self.settle(at);
            }
        }
    }

    fn settle_all(&mut self) {
        for at in 0..self.stack.len() {
            self.settle(at);
        }
    }

    fn bin(&mut self, op: BinOp, bits: u32) -> Result<(), WasmError> {
        let b = self.pop_operand()?;
        let a = self.pop_operand()?;
        let raw = self.new_var();
        self.emit(Instr::Bin(raw, op, a, b));
        let out = self.new_var();
        self.emit(Instr::Wrap(out, Operand::Var(raw), bits));
        self.stack.push(Item::Temp(out));
        Ok(())
    }

    fn compare(&mut self, op: CmpOp, signed: Option<u32>) -> Result<(), WasmError> {
        let b = self.pop_operand()?;
        let a = self.pop_operand()?;
        self.stack.push(Item::Cmp(Cond {
            op,
            a,
            b,
            signed_bits: signed,
        }));
        Ok(())
    }

    /// A shift by a constant is modelled, and otherwise the amount is unknown.
    fn shift(&mut self, op: BinOp, bits: u32) -> Result<(), WasmError> {
        let amount = self.pop()?;
        match amount {
            Item::Const(c) => {
                self.stack.push(Item::Const(c.rem_euclid(i128::from(bits))));
                self.bin(op, bits)
            }
            _ => {
                self.pop()?;
                let out = self.new_var();
                self.emit(Instr::Havoc(out, Interval::unsigned_bits(bits)));
                self.stack.push(Item::Temp(out));
                Ok(())
            }
        }
    }

    /// Division on the unsigned values is exact for the unsigned forms. For the signed forms, the
    /// division is only there to check the divisor, and the result is treated as unknown.
    fn divide(&mut self, op: BinOp, bits: u32, signed: bool) -> Result<(), WasmError> {
        if !signed {
            return self.bin(op, bits);
        }
        let b = self.pop_operand()?;
        let a = self.pop_operand()?;
        let scratch = self.new_var();
        self.emit(Instr::Bin(scratch, op, a, b));
        let out = self.new_var();
        self.emit(Instr::Havoc(out, Interval::unsigned_bits(bits)));
        self.stack.push(Item::Temp(out));
        Ok(())
    }

    fn access(
        &mut self,
        memory: u32,
        offset: u64,
        size: u32,
        load: Option<Interval>,
    ) -> Result<(), WasmError> {
        let limit = self.memories.get(memory as usize).copied().unwrap_or(0);
        if load.is_none() {
            self.pop()?;
        }
        let addr = self.pop_operand()?;
        self.emit(Instr::MemAccess {
            addr,
            offset,
            size,
            limit,
        });
        if let Some(range) = load {
            let out = self.new_var();
            self.emit(Instr::Havoc(out, range));
            self.stack.push(Item::Temp(out));
        }
        Ok(())
    }

    fn top_cond(&mut self) -> Result<Cond, WasmError> {
        let item = self.pop()?;
        Ok(match item {
            Item::Cmp(c) => c,
            other => {
                let a = self.operand(other);
                Cond {
                    op: CmpOp::Ne,
                    a,
                    b: Operand::Const(0),
                    signed_bits: None,
                }
            }
        })
    }

    /// Moves the top `count` values into `vars`, in two steps so that one cannot overwrite another.
    fn assign_top(&mut self, vars: &[Var], keep: bool) -> Result<(), WasmError> {
        let at = self
            .stack
            .len()
            .checked_sub(vars.len())
            .ok_or_else(|| WasmError("too few operands for a branch".into()))?;
        let items: Vec<Item> = self.stack[at..].to_vec();
        let mut temps = Vec::with_capacity(items.len());
        for item in items {
            let o = self.operand(item);
            if vars.len() > 1 {
                let t = self.new_var();
                self.emit(Instr::Copy(t, o));
                temps.push(Operand::Var(t));
            } else {
                temps.push(o);
            }
        }
        for (v, o) in vars.iter().zip(temps) {
            self.emit(Instr::Copy(*v, o));
        }
        if !keep {
            self.stack.truncate(at);
        }
        Ok(())
    }

    fn branch_vars(&self, depth: u32) -> Result<(usize, Vec<Var>), WasmError> {
        let at = self
            .frames
            .len()
            .checked_sub(1 + depth as usize)
            .ok_or_else(|| WasmError("a branch goes too far".into()))?;
        let frame = &self.frames[at];
        Ok(match frame.kind {
            Kind::Loop => (frame.target, frame.params.clone()),
            _ => (frame.target, frame.results.clone()),
        })
    }

    fn go(&mut self, target: usize) {
        self.blocks[self.current].term = Term::Goto(target);
    }

    fn signature(
        &self,
        block: BlockType,
        validator: &FuncValidator<ValidatorResources>,
    ) -> (usize, usize) {
        match block {
            BlockType::Empty => (0, 0),
            BlockType::Type(_) => (0, 1),
            BlockType::FuncType(index) => validator
                .resources()
                .sub_type_at(index)
                .and_then(|t| match &t.composite_type.inner {
                    wasmparser::CompositeInnerType::Func(f) => {
                        Some((f.params().len(), f.results().len()))
                    }
                    _ => None,
                })
                .unwrap_or((0, 0)),
        }
    }

    fn open(
        &mut self,
        kind: Kind,
        block: BlockType,
        validator: &FuncValidator<ValidatorResources>,
        target_is_head: bool,
    ) {
        let (params, results) = self.signature(block, validator);
        self.settle_all();
        let param_vars: Vec<Var> = (0..params).map(|_| self.new_var()).collect();
        let result_vars: Vec<Var> = (0..results).map(|_| self.new_var()).collect();
        // The stack has enough operands, because the module validates.
        let _ = self.assign_top(&param_vars, false);
        let end = self.new_block();
        let target = if target_is_head {
            let head = self.new_block();
            self.go(head);
            self.current = head;
            head
        } else {
            end
        };
        let height = self.stack.len();
        self.stack.extend(param_vars.iter().map(|v| Item::Temp(*v)));
        self.frames.push(Frame {
            kind,
            target,
            end,
            params: param_vars,
            results: result_vars,
            height,
        });
    }

    /// Ends the arm that is running, with its values going to the frame's results.
    fn finish_arm(&mut self) -> Result<(), WasmError> {
        let frame = self
            .frames
            .last()
            .ok_or_else(|| WasmError("end with no frame".into()))?;
        let (results, end) = (frame.results.clone(), frame.end);
        self.assign_top(&results, false)?;
        self.go(end);
        Ok(())
    }

    fn lower(
        &mut self,
        op: &Operator<'_>,
        validator: &FuncValidator<ValidatorResources>,
    ) -> Result<bool, WasmError> {
        use Operator as O;
        if self.dead {
            match op {
                O::Block { .. } | O::Loop { .. } | O::If { .. } => self.dead_nesting += 1,
                O::End if self.dead_nesting > 0 => self.dead_nesting -= 1,
                O::Else if self.dead_nesting == 0 => self.begin_else()?,
                O::End => self.close_frame(true)?,
                O::Try { .. } | O::TryTable { .. } => return Ok(false),
                _ => {}
            }
            return Ok(true);
        }
        match *op {
            O::Nop => {}
            O::I32Const { value } => self.stack.push(Item::Const(i128::from(value as u32))),
            O::I64Const { value } => self.stack.push(Item::Const(i128::from(value as u64))),
            O::LocalGet { local_index } => self.stack.push(Item::Local(local_index)),
            O::LocalSet { local_index } => {
                self.settle_local(local_index);
                let o = self.pop_operand()?;
                self.emit(Instr::Copy(local_index, o));
            }
            O::LocalTee { local_index } => {
                self.settle_local(local_index);
                let o = self.pop_operand()?;
                self.emit(Instr::Copy(local_index, o));
                self.stack.push(Item::Local(local_index));
            }
            O::Drop => {
                self.pop()?;
            }
            O::I32Add => self.bin(BinOp::Add, 32)?,
            O::I32Sub => self.bin(BinOp::Sub, 32)?,
            O::I32Mul => self.bin(BinOp::Mul, 32)?,
            O::I32And => self.bin(BinOp::And, 32)?,
            O::I32Or => self.bin(BinOp::Or, 32)?,
            O::I32Xor => self.bin(BinOp::Xor, 32)?,
            O::I64Add => self.bin(BinOp::Add, 64)?,
            O::I64Sub => self.bin(BinOp::Sub, 64)?,
            O::I64Mul => self.bin(BinOp::Mul, 64)?,
            O::I64And => self.bin(BinOp::And, 64)?,
            O::I64Or => self.bin(BinOp::Or, 64)?,
            O::I64Xor => self.bin(BinOp::Xor, 64)?,
            O::I32Shl => self.shift(BinOp::Shl, 32)?,
            O::I32ShrU => self.shift(BinOp::Shr, 32)?,
            O::I64Shl => self.shift(BinOp::Shl, 64)?,
            O::I64ShrU => self.shift(BinOp::Shr, 64)?,
            O::I32DivU => self.divide(BinOp::Div, 32, false)?,
            O::I32RemU => self.divide(BinOp::Rem, 32, false)?,
            O::I64DivU => self.divide(BinOp::Div, 64, false)?,
            O::I64RemU => self.divide(BinOp::Rem, 64, false)?,
            O::I32DivS => self.divide(BinOp::Div, 32, true)?,
            O::I32RemS => self.divide(BinOp::Rem, 32, true)?,
            O::I64DivS => self.divide(BinOp::Div, 64, true)?,
            O::I64RemS => self.divide(BinOp::Rem, 64, true)?,
            O::I32Eq | O::I64Eq => self.compare(CmpOp::Eq, None)?,
            O::I32Ne | O::I64Ne => self.compare(CmpOp::Ne, None)?,
            O::I32LtU | O::I64LtU => self.compare(CmpOp::Lt, None)?,
            O::I32LeU | O::I64LeU => self.compare(CmpOp::Le, None)?,
            O::I32GtU | O::I64GtU => self.compare(CmpOp::Gt, None)?,
            O::I32GeU | O::I64GeU => self.compare(CmpOp::Ge, None)?,
            O::I32LtS => self.compare(CmpOp::Lt, Some(32))?,
            O::I32LeS => self.compare(CmpOp::Le, Some(32))?,
            O::I32GtS => self.compare(CmpOp::Gt, Some(32))?,
            O::I32GeS => self.compare(CmpOp::Ge, Some(32))?,
            O::I64LtS => self.compare(CmpOp::Lt, Some(64))?,
            O::I64LeS => self.compare(CmpOp::Le, Some(64))?,
            O::I64GtS => self.compare(CmpOp::Gt, Some(64))?,
            O::I64GeS => self.compare(CmpOp::Ge, Some(64))?,
            O::I32Eqz | O::I64Eqz => {
                let item = self.pop()?;
                let cond = match item {
                    Item::Cmp(c) => Cond {
                        op: c.op.negate(),
                        ..c
                    },
                    other => {
                        let a = self.operand(other);
                        Cond {
                            op: CmpOp::Eq,
                            a,
                            b: Operand::Const(0),
                            signed_bits: None,
                        }
                    }
                };
                self.stack.push(Item::Cmp(cond));
            }
            O::I64ExtendI32U => {}
            O::I32WrapI64 => {
                let o = self.pop_operand()?;
                let out = self.new_var();
                self.emit(Instr::Wrap(out, o, 32));
                self.stack.push(Item::Temp(out));
            }
            O::I32Load { memarg } | O::F32Load { memarg } => self.access(
                memory_of(&memarg),
                memarg.offset,
                4,
                Some(range_of(Some(ValType::I32))),
            )?,
            O::I64Load { memarg } | O::F64Load { memarg } => self.access(
                memory_of(&memarg),
                memarg.offset,
                8,
                Some(range_of(Some(ValType::I64))),
            )?,
            O::I32Load8U { memarg } | O::I64Load8U { memarg } => self.access(
                memory_of(&memarg),
                memarg.offset,
                1,
                Some(Interval::unsigned_bits(8)),
            )?,
            O::I32Load16U { memarg } | O::I64Load16U { memarg } => self.access(
                memory_of(&memarg),
                memarg.offset,
                2,
                Some(Interval::unsigned_bits(16)),
            )?,
            O::I64Load32U { memarg } => self.access(
                memory_of(&memarg),
                memarg.offset,
                4,
                Some(Interval::unsigned_bits(32)),
            )?,
            O::I32Load8S { memarg } => self.access(
                memory_of(&memarg),
                memarg.offset,
                1,
                Some(Interval::unsigned_bits(32)),
            )?,
            O::I32Load16S { memarg } => self.access(
                memory_of(&memarg),
                memarg.offset,
                2,
                Some(Interval::unsigned_bits(32)),
            )?,
            O::I64Load8S { memarg } => self.access(
                memory_of(&memarg),
                memarg.offset,
                1,
                Some(Interval::unsigned_bits(64)),
            )?,
            O::I64Load16S { memarg } => self.access(
                memory_of(&memarg),
                memarg.offset,
                2,
                Some(Interval::unsigned_bits(64)),
            )?,
            O::I64Load32S { memarg } => self.access(
                memory_of(&memarg),
                memarg.offset,
                4,
                Some(Interval::unsigned_bits(64)),
            )?,
            O::I32Store { memarg } | O::F32Store { memarg } | O::I64Store32 { memarg } => {
                self.access(memory_of(&memarg), memarg.offset, 4, None)?
            }
            O::I64Store { memarg } | O::F64Store { memarg } => {
                self.access(memory_of(&memarg), memarg.offset, 8, None)?
            }
            O::I32Store8 { memarg } | O::I64Store8 { memarg } => {
                self.access(memory_of(&memarg), memarg.offset, 1, None)?
            }
            O::I32Store16 { memarg } | O::I64Store16 { memarg } => {
                self.access(memory_of(&memarg), memarg.offset, 2, None)?
            }
            O::Block { blockty } => self.open(Kind::Block, blockty, validator, false),
            O::Loop { blockty } => self.open(Kind::Loop, blockty, validator, true),
            O::If { blockty } => {
                let cond = self.top_cond()?;
                let (params, results) = self.signature(blockty, validator);
                // All of this happens before the branch, so that both arms see it.
                self.settle_all();
                let param_vars: Vec<Var> = (0..params).map(|_| self.new_var()).collect();
                let result_vars: Vec<Var> = (0..results).map(|_| self.new_var()).collect();
                self.assign_top(&param_vars, false)?;
                let (then_block, else_block, end) =
                    (self.new_block(), self.new_block(), self.new_block());
                self.blocks[self.current].term = Term::Branch(cond, then_block, else_block);
                self.current = then_block;
                let height = self.stack.len();
                self.stack.extend(param_vars.iter().map(|v| Item::Temp(*v)));
                self.frames.push(Frame {
                    kind: Kind::If {
                        else_block: Some(else_block),
                    },
                    target: end,
                    end,
                    params: param_vars,
                    results: result_vars,
                    height,
                });
            }
            O::Else => {
                self.finish_arm()?;
                self.begin_else()?;
            }
            O::End => self.close_frame(false)?,
            O::Br { relative_depth } => {
                let (target, vars) = self.branch_vars(relative_depth)?;
                self.assign_top(&vars, false)?;
                self.go(target);
                self.dead = true;
            }
            O::BrIf { relative_depth } => {
                let cond = self.top_cond()?;
                let (target, vars) = self.branch_vars(relative_depth)?;
                let (taken, next) = (self.new_block(), self.new_block());
                self.blocks[self.current].term = Term::Branch(cond, taken, next);
                self.current = taken;
                self.assign_top(&vars, true)?;
                self.go(target);
                self.current = next;
            }
            O::BrTable { ref targets } => {
                let index = self.pop_operand()?;
                let mut depths: Vec<u32> =
                    targets.targets().collect::<Result<_, _>>().map_err(bad)?;
                let default = targets.default();
                // Each entry in turn, so the index is narrowed along each. A huge table just becomes
                // any target at all, with no narrowing.
                if depths.len() > 64 {
                    depths.clear();
                    let all: Vec<u32> = targets.targets().collect::<Result<_, _>>().map_err(bad)?;
                    let selector = self.new_var();
                    self.emit(Instr::Havoc(selector, Interval::TOP));
                    for (at, depth) in all.iter().enumerate() {
                        self.table_arm(
                            Cond {
                                op: CmpOp::Eq,
                                a: Operand::Var(selector),
                                b: Operand::Const(at as i128),
                                signed_bits: None,
                            },
                            *depth,
                        )?;
                    }
                } else {
                    for (at, depth) in depths.iter().enumerate() {
                        self.table_arm(
                            Cond {
                                op: CmpOp::Eq,
                                a: index,
                                b: Operand::Const(at as i128),
                                signed_bits: None,
                            },
                            *depth,
                        )?;
                    }
                }
                let (target, vars) = self.branch_vars(default)?;
                self.assign_top(&vars, false)?;
                self.go(target);
                self.dead = true;
            }
            O::Return | O::Unreachable => {
                // Both end the function, and the only difference is whether it counts as a trap.
                self.blocks[self.current].term = Term::Return;
                self.dead = true;
            }
            ref other => return self.generic(other, validator),
        }
        Ok(true)
    }

    fn table_arm(&mut self, cond: Cond, depth: u32) -> Result<(), WasmError> {
        let (target, vars) = self.branch_vars(depth)?;
        let (taken, next) = (self.new_block(), self.new_block());
        self.blocks[self.current].term = Term::Branch(cond, taken, next);
        self.current = taken;
        self.assign_top(&vars, true)?;
        self.go(target);
        self.current = next;
        Ok(())
    }

    fn begin_else(&mut self) -> Result<(), WasmError> {
        let frame = self
            .frames
            .last_mut()
            .ok_or_else(|| WasmError("else with no frame".into()))?;
        let Kind::If { else_block } = &mut frame.kind else {
            return Err(WasmError("else outside an if".into()));
        };
        let block = else_block
            .take()
            .ok_or_else(|| WasmError("a second else".into()))?;
        let (height, params) = (frame.height, frame.params.clone());
        self.stack.truncate(height);
        self.stack.extend(params.iter().map(|v| Item::Temp(*v)));
        self.current = block;
        self.dead = false;
        Ok(())
    }

    fn close_frame(&mut self, was_dead: bool) -> Result<(), WasmError> {
        if !was_dead {
            self.finish_arm()?;
        }
        let frame = self
            .frames
            .pop()
            .ok_or_else(|| WasmError("end with no frame".into()))?;
        // An `if` with no `else` passes its parameters through as its results.
        if let Kind::If {
            else_block: Some(block),
        } = frame.kind
        {
            let line = self.line;
            for (r, p) in frame.results.iter().zip(&frame.params) {
                self.blocks[block]
                    .instrs
                    .push((Instr::Copy(*r, Operand::Var(*p)), line));
            }
            self.blocks[block].term = Term::Goto(frame.end);
        }
        self.stack.truncate(frame.height);
        self.stack
            .extend(frame.results.iter().map(|v| Item::Temp(*v)));
        self.current = frame.end;
        self.dead = false;
        Ok(())
    }

    /// Any other operator: it pops and pushes, and what it pushes can be anything of its type.
    fn generic(
        &mut self,
        op: &Operator<'_>,
        validator: &FuncValidator<ValidatorResources>,
    ) -> Result<bool, WasmError> {
        use Operator as O;
        if matches!(
            op,
            O::Try { .. }
                | O::TryTable { .. }
                | O::Catch { .. }
                | O::CatchAll
                | O::Delegate { .. }
                | O::Throw { .. }
                | O::ThrowRef
                | O::Rethrow { .. }
                | O::ReturnCall { .. }
                | O::ReturnCallIndirect { .. }
                | O::ReturnCallRef { .. }
                | O::BrOnNull { .. }
                | O::BrOnNonNull { .. }
                | O::BrOnCast { .. }
                | O::BrOnCastFail { .. }
        ) {
            return Ok(false);
        }
        let Some((pops, pushes)) = op.operator_arity(validator) else {
            return Ok(false);
        };
        for _ in 0..pops {
            self.pop()?;
        }
        // The types are not known until the validator has run this, so these get narrowed after that.
        for _ in 0..pushes {
            let out = self.new_var();
            self.emit(Instr::Havoc(out, Interval::TOP));
            self.untyped
                .push((self.current, self.blocks[self.current].instrs.len() - 1));
            self.stack.push(Item::Temp(out));
        }
        Ok(true)
    }
}

fn lower_function(
    name: String,
    mut validator: FuncValidator<ValidatorResources>,
    body: &wasmparser::FunctionBody<'_>,
    memories: &[u128],
) -> Result<(Result<Function, String>, FuncValidatorAllocations), WasmError> {
    let params = validator.len_locals();
    let mut declared: Vec<ValType> = Vec::new();
    let mut reader = body.get_locals_reader().map_err(bad)?;
    for _ in 0..reader.get_count() {
        let offset = reader.original_position();
        let (count, ty) = reader.read().map_err(bad)?;
        validator.define_locals(offset, count, ty).map_err(bad)?;
        declared.extend(std::iter::repeat_n(ty, count as usize));
    }
    let locals = validator.len_locals();
    let mut builder = Builder {
        blocks: Vec::new(),
        current: 0,
        var_names: (0..locals).map(|i| format!("local{i}")).collect(),
        stack: Vec::new(),
        frames: Vec::new(),
        dead: false,
        dead_nesting: 0,
        memories: memories.to_vec(),
        line: 0,
        untyped: Vec::new(),
    };
    let entry = builder.new_block();
    builder.current = entry;
    // Declared locals start at zero, and references start null, which nothing here treats as a number.
    for (i, ty) in declared.iter().enumerate() {
        if matches!(ty, ValType::I32 | ValType::I64) {
            builder.blocks[entry]
                .instrs
                .push((Instr::Copy(params + i as u32, Operand::Const(0)), 0));
        } else {
            builder.blocks[entry]
                .instrs
                .push((Instr::Havoc(params + i as u32, Interval::TOP), 0));
        }
    }
    let mut param_ranges = Vec::new();
    for p in 0..params {
        param_ranges.push((p, range_of(validator.get_local_type(p))));
    }
    // The function body is a frame whose end returns.
    let exit = builder.new_block();
    builder.frames.push(Frame {
        kind: Kind::Block,
        target: exit,
        end: exit,
        params: Vec::new(),
        results: Vec::new(),
        height: 0,
    });

    let mut operators = body.get_operators_reader().map_err(bad)?;
    let mut failure: Option<String> = None;
    let mut count = 0;
    while !operators.eof() {
        let offset = operators.original_position();
        let op = operators.read().map_err(bad)?;
        count += 1;
        if failure.is_none() {
            if count > MAX_OPERATORS {
                failure = Some(format!("it has over {MAX_OPERATORS} instructions"));
            } else {
                builder.line = offset as u32;
                // The final `end` closes the body, whose values are the function's results.
                let is_last = matches!(op, Operator::End)
                    && builder.frames.len() == 1
                    && !builder.dead
                    && builder.dead_nesting == 0;
                let is_last_dead = matches!(op, Operator::End)
                    && builder.frames.len() == 1
                    && builder.dead
                    && builder.dead_nesting == 0;
                if is_last || is_last_dead {
                    if is_last {
                        builder.go(exit);
                    }
                    builder.frames.pop();
                    builder.stack.clear();
                    builder.dead = true;
                } else if !builder.lower(&op, &validator)? {
                    failure = Some(format!("it uses {op:?}, which is not modelled"));
                }
            }
        }
        validator.op(offset, &op).map_err(bad)?;
        for (depth, (block, instr)) in builder.untyped.drain(..).rev().enumerate() {
            if let Instr::Havoc(_, range) = &mut builder.blocks[block].instrs[instr].0 {
                *range = range_of(validator.get_operand_type(depth).flatten());
            }
        }
    }
    operators.finish().map_err(bad)?;
    let result = match failure {
        Some(reason) => Err(reason),
        None => Ok(Function {
            name,
            thread: ThreadKind::None,
            params: param_ranges,
            blocks: builder.blocks,
            var_names: builder.var_names,
        }),
    };
    Ok((result, validator.into_allocations()))
}

pub fn lower(wasm: &[u8]) -> Result<Lowered, WasmError> {
    let mut validator = Validator::new_with_features(WasmFeatures::all());
    let mut allocations = FuncValidatorAllocations::default();
    let mut memories: Vec<u128> = Vec::new();
    let mut lowered = Lowered::default();
    let mut imported_functions = 0u32;
    let mut defined = 0u32;
    let memory_size =
        |m: &wasmparser::MemoryType| u128::from(m.initial) << m.page_size_log2.unwrap_or(16);
    for payload in Parser::new(0).parse_all(wasm) {
        let payload = payload.map_err(bad)?;
        let validated = validator.payload(&payload).map_err(bad)?;
        match &payload {
            Payload::ImportSection(reader) => {
                for import in reader.clone().into_imports() {
                    match import.map_err(bad)?.ty {
                        TypeRef::Func(_) => imported_functions += 1,
                        TypeRef::Memory(m) => memories.push(memory_size(&m)),
                        _ => {}
                    }
                }
            }
            Payload::MemorySection(reader) => {
                for memory in reader.clone() {
                    memories.push(memory_size(&memory.map_err(bad)?));
                }
            }
            _ => {}
        }
        if let ValidPayload::Func(to_validate, body) = validated {
            let func = to_validate.into_validator(std::mem::take(&mut allocations));
            let name = format!("func{}", imported_functions + defined);
            defined += 1;
            let (result, reused) = lower_function(name.clone(), func, &body, &memories)?;
            allocations = reused;
            match result {
                Ok(function) => lowered.program.functions.push(function),
                Err(reason) => lowered.skipped.push((name, reason)),
            }
        }
    }
    Ok(lowered)
}
