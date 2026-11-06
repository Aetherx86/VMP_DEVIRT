//! Stack analysis of x86-64 machine code, using a separate engine over decoded instructions.
//!
//! Each register is tracked as an integer range, or as a range of offsets from the stack pointer on
//! entry. The return address is at offset 0. So a write through a stack-relative pointer whose range
//! stays below 0 is proved unable to overwrite the return address, which is the classic stack smash. A
//! write that may reach it is reported.
//!
//! Stack slots written with a known offset are remembered, which is what makes unoptimized code work.
//!
//! Assumptions, stated plainly: a call returns normally, restores the stack pointer and the registers
//! the System V ABI makes it preserve, and writes nothing into this frame unless a pointer to the frame
//! has been passed to it, which is not tracked. A write through an unknown pointer is assumed to
//! miss this frame too, and is reported as unknown and not as proved. Anything the decoder knows this
//! does not model has its written registers forgotten.

use crate::domain::{INF, Interval};
use iced_x86::{
    ConditionCode, Decoder, DecoderOptions, FlowControl, Instruction, InstructionInfoFactory,
    Mnemonic, OpAccess, OpKind, Register,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegValue {
    Top,
    Int(Interval),
    /// An offset from the stack pointer on entry.
    Stack(Interval),
}

impl RegValue {
    fn join(self, o: RegValue) -> RegValue {
        match (self, o) {
            (RegValue::Int(a), RegValue::Int(b)) => RegValue::Int(a.join(b)),
            (RegValue::Stack(a), RegValue::Stack(b)) => RegValue::Stack(a.join(b)),
            _ => RegValue::Top,
        }
    }

    fn widen(self, o: RegValue, thresholds: &[i128]) -> RegValue {
        match (self, o) {
            (RegValue::Int(a), RegValue::Int(b)) => RegValue::Int(a.widen(b, thresholds)),
            (RegValue::Stack(a), RegValue::Stack(b)) => RegValue::Stack(a.widen(b, thresholds)),
            _ => RegValue::Top,
        }
    }
}

const REGS: usize = 16;

fn index_of(reg: Register) -> Option<usize> {
    let full = reg.full_register();
    match full {
        Register::RAX => Some(0),
        Register::RCX => Some(1),
        Register::RDX => Some(2),
        Register::RBX => Some(3),
        Register::RSP => Some(4),
        Register::RBP => Some(5),
        Register::RSI => Some(6),
        Register::RDI => Some(7),
        Register::R8 => Some(8),
        Register::R9 => Some(9),
        Register::R10 => Some(10),
        Register::R11 => Some(11),
        Register::R12 => Some(12),
        Register::R13 => Some(13),
        Register::R14 => Some(14),
        Register::R15 => Some(15),
        _ => None,
    }
}

const RSP: usize = 4;

/// What the last flag-setting comparison compared: a register or stack slot, against a constant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Compared {
    target: Target,
    bits: u32,
    value: i128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Reg(usize),
    Slot(i128),
}

#[derive(Clone, Debug, PartialEq)]
struct State {
    regs: [RegValue; REGS],
    /// Known contents of the stack: an offset, its size in bytes, and the value there.
    slots: BTreeMap<i128, (u32, RegValue)>,
    compared: Option<Compared>,
}

impl State {
    fn initial() -> State {
        let mut regs = [RegValue::Top; REGS];
        regs[RSP] = RegValue::Stack(Interval::exact(0));
        State {
            regs,
            slots: BTreeMap::new(),
            compared: None,
        }
    }

    fn join(&self, o: &State) -> State {
        let mut regs = self.regs;
        for (r, other) in regs.iter_mut().zip(o.regs) {
            *r = r.join(other);
        }
        let mut slots = BTreeMap::new();
        for (k, (size, v)) in &self.slots {
            if let Some((s2, v2)) = o.slots.get(k)
                && s2 == size
            {
                slots.insert(*k, (*size, v.join(*v2)));
            }
        }
        State {
            regs,
            slots,
            compared: if self.compared == o.compared {
                self.compared
            } else {
                None
            },
        }
    }

    fn widen(&self, next: &State, thresholds: &[i128]) -> State {
        let joined = self.join(next);
        let mut regs = self.regs;
        for (r, n) in regs.iter_mut().zip(joined.regs) {
            *r = r.widen(n, thresholds);
        }
        let mut slots = BTreeMap::new();
        for (k, (size, v)) in &self.slots {
            if let Some((s2, v2)) = joined.slots.get(k)
                && s2 == size
            {
                slots.insert(*k, (*size, v.widen(*v2, thresholds)));
            }
        }
        State {
            regs,
            slots,
            compared: joined.compared,
        }
    }

    /// Forgets every slot a write to this range of offsets could touch.
    fn clobber(&mut self, lo: i128, hi_exclusive: i128) {
        self.slots.retain(|offset, (size, _)| {
            *offset + i128::from(*size) <= lo || *offset >= hi_exclusive
        });
        if let Some(Compared {
            target: Target::Slot(at),
            ..
        }) = self.compared
            && !self.slots.contains_key(&at)
        {
            self.compared = None;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackVerdict {
    /// The whole access stays below the return address, and at or above the lowest stack pointer seen.
    Safe,
    /// It may touch the return address or beyond.
    MayOverwriteReturnAddress,
    /// It stays clear of the return address when writing, but may go outside the frame.
    MayLeaveFrame,
    /// A pointer this analysis knows nothing about, so it claims nothing.
    UnknownPointer,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Access {
    pub address: u64,
    pub text: String,
    pub write: bool,
    pub verdict: StackVerdict,
    pub range: Option<Interval>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub accesses: Vec<Access>,
    /// Addresses that return with the stack pointer not provably back where it started.
    pub unbalanced_returns: Vec<u64>,
    /// Addresses of indirect jumps, past which nothing was analyzed.
    pub unresolved_jumps: Vec<u64>,
    /// Whether the analysis reached something it could not decode.
    pub incomplete: bool,
    pub instructions: usize,
}

impl Report {
    pub fn safe_writes(&self) -> usize {
        self.accesses
            .iter()
            .filter(|a| a.write && a.verdict == StackVerdict::Safe)
            .count()
    }

    pub fn dangerous(&self) -> Vec<&Access> {
        self.accesses
            .iter()
            .filter(|a| a.verdict == StackVerdict::MayOverwriteReturnAddress)
            .collect()
    }
}

fn wrap_to(bits: u32, v: RegValue) -> RegValue {
    match (bits, v) {
        (64, v) => v,
        (bits, RegValue::Int(i)) => RegValue::Int(i.wrap(bits)),
        (bits, _) => RegValue::Int(Interval::unsigned_bits(bits)),
    }
}

fn register_bits(reg: Register) -> u32 {
    (reg.size() * 8) as u32
}

fn read_reg(state: &State, reg: Register) -> RegValue {
    let Some(i) = index_of(reg) else {
        return RegValue::Top;
    };
    match register_bits(reg) {
        64 => state.regs[i],
        bits => match state.regs[i] {
            RegValue::Int(v) => RegValue::Int(v.wrap(bits)),
            _ => RegValue::Int(Interval::unsigned_bits(bits)),
        },
    }
}

fn write_reg(state: &mut State, reg: Register, value: RegValue) {
    let Some(i) = index_of(reg) else { return };
    match register_bits(reg) {
        64 => state.regs[i] = value,
        // Writing 32 bits clears the top half.
        32 => state.regs[i] = wrap_to(32, value),
        // Writing fewer keeps the rest, which this does not track.
        _ => state.regs[i] = RegValue::Top,
    }
    if state.compared.is_some_and(|c| c.target == Target::Reg(i)) {
        state.compared = None;
    }
}

fn add_values(a: RegValue, b: RegValue) -> RegValue {
    match (a, b) {
        (RegValue::Int(x), RegValue::Int(y)) => RegValue::Int(x.add(y).wrap(64)),
        (RegValue::Stack(x), RegValue::Int(y)) | (RegValue::Int(y), RegValue::Stack(x)) => {
            // An offset can be stored as a very large unsigned number, when it is really negative.
            let y = signed_view(y);
            RegValue::Stack(x.add(y))
        }
        _ => RegValue::Top,
    }
}

/// Reads a 64-bit range as signed, if it does not straddle the wrap-around.
fn signed_view(i: Interval) -> Interval {
    let half = 1i128 << 63;
    if i.lo >= half && i.hi < 1i128 << 64 {
        Interval {
            lo: i.lo - (1i128 << 64),
            hi: i.hi - (1i128 << 64),
        }
    } else if i.hi < half {
        i
    } else {
        Interval::TOP
    }
}

fn effective_address(state: &State, instr: &Instruction) -> RegValue {
    let mut value = RegValue::Int(Interval::exact(0));
    if instr.memory_base() == Register::RIP {
        return RegValue::Int(Interval::exact(i128::from(instr.memory_displacement64())));
    }
    if instr.memory_base() != Register::None {
        value = add_values(read_reg(state, instr.memory_base()), value);
    }
    if instr.memory_index() != Register::None {
        let scale = Interval::exact(i128::from(instr.memory_index_scale()));
        let index = match read_reg(state, instr.memory_index()) {
            RegValue::Int(i) => RegValue::Int(i.mul(scale)),
            _ => RegValue::Top,
        };
        value = add_values(value, index);
    }
    let displacement = instr.memory_displacement64() as i64;
    add_values(
        value,
        RegValue::Int(Interval::exact(i128::from(displacement))),
    )
}

fn immediate(instr: &Instruction, operand: u32) -> Option<i128> {
    match instr.op_kind(operand) {
        OpKind::Immediate8
        | OpKind::Immediate16
        | OpKind::Immediate32
        | OpKind::Immediate64
        | OpKind::Immediate8to16
        | OpKind::Immediate8to32
        | OpKind::Immediate8to64
        | OpKind::Immediate32to64 => Some(i128::from(instr.immediate(operand) as i64)),
        _ => None,
    }
}

/// What this instruction's memory operand is, as a value, read from the state.
fn read_operand(state: &State, instr: &Instruction, operand: u32) -> RegValue {
    match instr.op_kind(operand) {
        OpKind::Register => read_reg(state, instr.op_register(operand)),
        OpKind::Memory => {
            let bytes = instr.memory_size().size() as u32;
            if let RegValue::Stack(at) = effective_address(state, instr)
                && at.lo == at.hi
                && let Some((size, v)) = state.slots.get(&at.lo)
                && *size == bytes
            {
                return *v;
            }
            match bytes {
                1 | 2 | 4 => RegValue::Int(Interval::unsigned_bits(bytes * 8)),
                _ => RegValue::Top,
            }
        }
        _ => match immediate(instr, operand) {
            Some(v) => RegValue::Int(Interval::exact(v).wrap(64)),
            None => RegValue::Top,
        },
    }
}

fn write_operand(state: &mut State, instr: &Instruction, operand: u32, value: RegValue) {
    match instr.op_kind(operand) {
        OpKind::Register => write_reg(state, instr.op_register(operand), value),
        OpKind::Memory => {
            let bytes = instr.memory_size().size() as u32;
            if let RegValue::Stack(at) = effective_address(state, instr) {
                state.clobber(at.lo, at.hi.saturating_add(i128::from(bytes)));
                if at.lo == at.hi {
                    state.slots.insert(at.lo, (bytes, value));
                }
            }
        }
        _ => {}
    }
}

fn arithmetic(instr: &Instruction, a: RegValue, b: RegValue) -> RegValue {
    let bits = match instr.op_kind(0) {
        OpKind::Register => register_bits(instr.op_register(0)),
        _ => instr.memory_size().size() as u32 * 8,
    };
    let result = match instr.mnemonic() {
        Mnemonic::Add => add_values(a, b),
        Mnemonic::Sub => match (a, b) {
            (RegValue::Int(x), RegValue::Int(y)) => RegValue::Int(x.sub(y).wrap(64)),
            (RegValue::Stack(x), RegValue::Int(y)) => RegValue::Stack(x.sub(signed_view(y))),
            _ => RegValue::Top,
        },
        Mnemonic::And => match (a, b) {
            (RegValue::Int(x), RegValue::Int(y)) => RegValue::Int(x.bit_and(y)),
            // Aligning the stack pointer: this can only move down, by less than the alignment.
            (RegValue::Stack(x), RegValue::Int(y)) => {
                let mask = signed_view(y);
                if mask.lo == mask.hi && mask.lo < 0 && (-mask.lo).count_ones() == 1 {
                    RegValue::Stack(Interval {
                        lo: x.lo.saturating_add(mask.lo + 1).max(-INF),
                        hi: x.hi,
                    })
                } else {
                    RegValue::Top
                }
            }
            _ => RegValue::Top,
        },
        Mnemonic::Or => match (a, b) {
            (RegValue::Int(x), RegValue::Int(y)) => RegValue::Int(x.bit_or(y)),
            _ => RegValue::Top,
        },
        Mnemonic::Xor => match (a, b) {
            (RegValue::Int(x), RegValue::Int(y)) => RegValue::Int(x.bit_xor(y)),
            _ => RegValue::Top,
        },
        _ => RegValue::Top,
    };
    wrap_to(bits, result)
}

/// Narrows the compared register along a conditional jump. `None` if this edge cannot be taken.
fn refine(state: &State, condition: ConditionCode, taken: bool) -> Option<State> {
    let Some(compared) = state.compared else {
        return Some(state.clone());
    };
    let current = match compared.target {
        Target::Reg(r) => state.regs[r],
        Target::Slot(at) => state.slots.get(&at).map_or(RegValue::Top, |(_, v)| *v),
    };
    let RegValue::Int(value) = current else {
        return Some(state.clone());
    };
    let (mask, half) = (1i128 << compared.bits, 1i128 << (compared.bits - 1));
    // A comparison of part of a register says nothing about a value with bits above that part.
    if value.lo < 0 || value.hi >= mask {
        return Some(state.clone());
    }
    let c = compared.value.rem_euclid(mask);
    let inverse = |cc: ConditionCode| match cc {
        ConditionCode::b => ConditionCode::ae,
        ConditionCode::ae => ConditionCode::b,
        ConditionCode::be => ConditionCode::a,
        ConditionCode::a => ConditionCode::be,
        ConditionCode::l => ConditionCode::ge,
        ConditionCode::ge => ConditionCode::l,
        ConditionCode::le => ConditionCode::g,
        ConditionCode::g => ConditionCode::le,
        ConditionCode::e => ConditionCode::ne,
        ConditionCode::ne => ConditionCode::e,
        other => other,
    };
    let holds = if taken { condition } else { inverse(condition) };
    // The signed conditions only match unsigned reasoning when neither side has the sign bit.
    let both_small = value.hi < half && c < half;
    let limit = match holds {
        ConditionCode::b => Some(Interval { lo: 0, hi: c - 1 }),
        ConditionCode::be => Some(Interval { lo: 0, hi: c }),
        ConditionCode::a => Some(Interval {
            lo: c + 1,
            hi: mask - 1,
        }),
        ConditionCode::ae => Some(Interval {
            lo: c,
            hi: mask - 1,
        }),
        ConditionCode::l if both_small => Some(Interval { lo: 0, hi: c - 1 }),
        ConditionCode::le if both_small => Some(Interval { lo: 0, hi: c }),
        ConditionCode::g if both_small => Some(Interval {
            lo: c + 1,
            hi: mask - 1,
        }),
        ConditionCode::ge if both_small => Some(Interval {
            lo: c,
            hi: mask - 1,
        }),
        ConditionCode::e => Some(Interval::exact(c)),
        _ => None,
    };
    let Some(limit) = limit else {
        return Some(state.clone());
    };
    if limit.lo > limit.hi {
        return None;
    }
    let narrowed = value.meet(limit)?;
    let mut out = state.clone();
    match compared.target {
        Target::Reg(r) => out.regs[r] = RegValue::Int(narrowed),
        Target::Slot(at) => {
            if let Some((_, v)) = out.slots.get_mut(&at) {
                *v = RegValue::Int(narrowed);
            }
        }
    }
    Some(out)
}

fn check_memory(state: &State, instr: &Instruction, write: bool, lowest: i128) -> Access {
    let bytes = i128::from(instr.memory_size().size() as u32);
    let (verdict, range) = match effective_address(state, instr) {
        RegValue::Stack(at) => {
            // The 128 bytes below the stack pointer are the red zone, which leaf functions may use.
            let verdict = if at.hi.saturating_add(bytes) <= 0 && at.lo >= lowest - 128 {
                StackVerdict::Safe
            } else if !write {
                // Reading the return address, or the caller's frame, is not a smash.
                StackVerdict::MayLeaveFrame
            } else if at.hi.saturating_add(bytes) <= 0 {
                // Only ever below the return address, but possibly below the frame.
                StackVerdict::MayLeaveFrame
            } else {
                StackVerdict::MayOverwriteReturnAddress
            };
            (verdict, Some(at))
        }
        _ => (StackVerdict::UnknownPointer, None),
    };
    Access {
        address: instr.ip(),
        text: format!("{instr}"),
        write,
        verdict,
        range,
    }
}

/// One instruction's effect. It returns where control goes next, each with a state.
fn step(
    state: &State,
    instr: &Instruction,
    factory: &mut InstructionInfoFactory,
    report: Option<&mut Report>,
    lowest: i128,
) -> Vec<(u64, State)> {
    let mut next = state.clone();
    let mut accesses: Vec<Access> = Vec::new();
    let mut unbalanced = false;
    let mut unresolved = false;
    let falls_through = |s: State| vec![(instr.next_ip(), s)];

    let has_memory = (0..instr.op_count()).any(|i| instr.op_kind(i) == OpKind::Memory);
    let lea = instr.mnemonic() == Mnemonic::Lea;
    let flow = instr.flow_control();

    if has_memory && !lea {
        let info = factory.info(instr);
        let mut writes = false;
        let mut reads = false;
        for used in info.used_memory() {
            match used.access() {
                OpAccess::Write
                | OpAccess::CondWrite
                | OpAccess::ReadWrite
                | OpAccess::ReadCondWrite => writes = true,
                OpAccess::Read | OpAccess::CondRead => reads = true,
                _ => {}
            }
        }
        if writes {
            accesses.push(check_memory(state, instr, true, lowest));
        } else if reads {
            accesses.push(check_memory(state, instr, false, lowest));
        }
    }

    let mut outcome: Vec<(u64, State)> = Vec::new();
    match (instr.mnemonic(), flow) {
        (Mnemonic::Push, _) => {
            let value = read_operand(state, instr, 0);
            let bytes = instr.stack_pointer_increment();
            let rsp = add_values(
                state.regs[RSP],
                RegValue::Int(Interval::exact(i128::from(bytes)).wrap(64)),
            );
            next.regs[RSP] = rsp;
            if let RegValue::Stack(at) = rsp {
                next.clobber(at.lo, at.hi.saturating_add(-i128::from(bytes)));
                if at.lo == at.hi {
                    next.slots.insert(at.lo, (-bytes as u32, value));
                }
                let verdict = if at.hi - i128::from(bytes) <= 0 {
                    StackVerdict::Safe
                } else {
                    StackVerdict::MayOverwriteReturnAddress
                };
                accesses.push(Access {
                    address: instr.ip(),
                    text: format!("{instr}"),
                    write: true,
                    verdict,
                    range: Some(at),
                });
            }
            outcome = falls_through(next);
        }
        (Mnemonic::Pop, _) => {
            let bytes = instr.stack_pointer_increment();
            let mut value = RegValue::Top;
            if let RegValue::Stack(at) = state.regs[RSP]
                && at.lo == at.hi
                && let Some((size, v)) = state.slots.get(&at.lo)
                && i128::from(*size) == i128::from(bytes)
            {
                value = *v;
            }
            next.regs[RSP] = add_values(
                state.regs[RSP],
                RegValue::Int(Interval::exact(i128::from(bytes))),
            );
            write_operand(&mut next, instr, 0, value);
            outcome = falls_through(next);
        }
        (Mnemonic::Leave, _) => {
            let base = state.regs[5];
            let mut value = RegValue::Top;
            if let RegValue::Stack(at) = base
                && at.lo == at.hi
                && let Some((8, v)) = state.slots.get(&at.lo)
            {
                value = *v;
            }
            next.regs[RSP] = add_values(base, RegValue::Int(Interval::exact(8)));
            next.regs[5] = value;
            outcome = falls_through(next);
        }
        (Mnemonic::Mov | Mnemonic::Movzx, _) => {
            let value = read_operand(state, instr, 1);
            write_operand(&mut next, instr, 0, value);
            outcome = falls_through(next);
        }
        (Mnemonic::Lea, _) => {
            let value = effective_address(state, instr);
            write_operand(&mut next, instr, 0, value);
            outcome = falls_through(next);
        }
        (Mnemonic::Add | Mnemonic::Sub | Mnemonic::And | Mnemonic::Or | Mnemonic::Xor, _) => {
            let same = instr.op_kind(0) == OpKind::Register
                && instr.op_kind(1) == OpKind::Register
                && instr.op_register(0) == instr.op_register(1);
            let value = if same && matches!(instr.mnemonic(), Mnemonic::Xor | Mnemonic::Sub) {
                RegValue::Int(Interval::exact(0))
            } else {
                arithmetic(
                    instr,
                    read_operand(state, instr, 0),
                    read_operand(state, instr, 1),
                )
            };
            write_operand(&mut next, instr, 0, value);
            next.compared = None;
            outcome = falls_through(next);
        }
        (Mnemonic::Inc | Mnemonic::Dec, _) => {
            let delta = if instr.mnemonic() == Mnemonic::Inc {
                1
            } else {
                -1
            };
            let value = match read_operand(state, instr, 0) {
                RegValue::Int(i) => {
                    let bits = match instr.op_kind(0) {
                        OpKind::Register => register_bits(instr.op_register(0)),
                        _ => instr.memory_size().size() as u32 * 8,
                    };
                    RegValue::Int(i.add(Interval::exact(delta)).wrap(bits))
                }
                RegValue::Stack(i) => RegValue::Stack(i.add(Interval::exact(delta))),
                RegValue::Top => RegValue::Top,
            };
            write_operand(&mut next, instr, 0, value);
            next.compared = None;
            outcome = falls_through(next);
        }
        (Mnemonic::Cmp, _) => {
            next.compared = None;
            if let Some(value) = immediate(instr, 1) {
                if instr.op_kind(0) == OpKind::Register {
                    if let Some(reg) = index_of(instr.op_register(0)) {
                        next.compared = Some(Compared {
                            target: Target::Reg(reg),
                            bits: register_bits(instr.op_register(0)),
                            value,
                        });
                    }
                } else if let (OpKind::Memory, RegValue::Stack(at)) =
                    (instr.op_kind(0), effective_address(state, instr))
                {
                    let bytes = instr.memory_size().size() as u32;
                    if at.lo == at.hi
                        && state
                            .slots
                            .get(&at.lo)
                            .is_some_and(|(size, _)| *size == bytes)
                    {
                        next.compared = Some(Compared {
                            target: Target::Slot(at.lo),
                            bits: bytes * 8,
                            value,
                        });
                    }
                }
            }
            outcome = falls_through(next);
        }
        (Mnemonic::Nop | Mnemonic::Endbr64, _) => outcome = falls_through(next),
        (_, FlowControl::Return) => {
            if state.regs[RSP] != RegValue::Stack(Interval::exact(0)) {
                unbalanced = true;
            }
        }
        (_, FlowControl::UnconditionalBranch) => {
            next.compared = None;
            outcome = vec![(instr.near_branch_target(), next)];
        }
        (_, FlowControl::ConditionalBranch) => {
            if let Some(taken) = refine(state, instr.condition_code(), true) {
                outcome.push((instr.near_branch_target(), taken));
            }
            if let Some(fall) = refine(state, instr.condition_code(), false) {
                outcome.push((instr.next_ip(), fall));
            }
        }
        (_, FlowControl::IndirectBranch) => unresolved = true,
        (_, FlowControl::Call | FlowControl::IndirectCall) => {
            // The System V registers a callee may change. The stack pointer comes back.
            for r in [0usize, 1, 2, 6, 7, 8, 9, 10, 11] {
                next.regs[r] = RegValue::Top;
            }
            next.compared = None;
            outcome = falls_through(next);
        }
        (_, FlowControl::Exception | FlowControl::Interrupt) => {}
        _ => {
            // Not modelled, so what it writes is forgotten, including anywhere in memory on the stack.
            let info = factory.info(instr);
            for used in info.used_registers() {
                if matches!(
                    used.access(),
                    OpAccess::Write
                        | OpAccess::CondWrite
                        | OpAccess::ReadWrite
                        | OpAccess::ReadCondWrite
                ) && let Some(i) = index_of(used.register())
                {
                    next.regs[i] = RegValue::Top;
                }
            }
            let writes_memory = factory.info(instr).used_memory().iter().any(|m| {
                matches!(
                    m.access(),
                    OpAccess::Write
                        | OpAccess::CondWrite
                        | OpAccess::ReadWrite
                        | OpAccess::ReadCondWrite
                )
            });
            if writes_memory {
                match effective_address(state, instr) {
                    RegValue::Stack(at) => next.clobber(
                        at.lo,
                        at.hi
                            .saturating_add(i128::from(instr.memory_size().size() as u32)),
                    ),
                    _ => next.slots.clear(),
                }
            }
            next.compared = None;
            outcome = falls_through(next);
        }
    }
    if let Some(report) = report {
        report.accesses.extend(accesses);
        if unbalanced {
            report.unbalanced_returns.push(instr.ip());
        }
        if unresolved {
            report.unresolved_jumps.push(instr.ip());
        }
    }
    outcome
}

fn stack_offsets(state: &State) -> i128 {
    match state.regs[RSP] {
        RegValue::Stack(at) => at.lo,
        _ => -INF,
    }
}

const MAX_INSTRUCTIONS: usize = 200_000;
const WIDEN_AFTER: u32 = 3;

/// Analyzes one function: raw code loaded at `base`, starting at `entry`.
pub fn analyze(code: &[u8], base: u64, entry: u64) -> Report {
    let mut report = Report::default();
    let mut decoded: HashMap<u64, Instruction> = HashMap::new();
    let mut thresholds: BTreeSet<i128> = BTreeSet::from([0, 8, -8]);
    let mut states: BTreeMap<u64, State> = BTreeMap::new();
    let mut visits: HashMap<u64, u32> = HashMap::new();
    let mut work: BTreeSet<u64> = BTreeSet::from([entry]);
    states.insert(entry, State::initial());
    let mut factory = InstructionInfoFactory::new();

    let decode = |at: u64,
                  decoded: &mut HashMap<u64, Instruction>,
                  thresholds: &mut BTreeSet<i128>,
                  report: &mut Report| {
        if let Some(instr) = decoded.get(&at) {
            return Some(*instr);
        }
        let offset = at.checked_sub(base)? as usize;
        if offset >= code.len() || decoded.len() >= MAX_INSTRUCTIONS {
            report.incomplete = true;
            return None;
        }
        let mut decoder = Decoder::with_ip(64, &code[offset..], at, DecoderOptions::NONE);
        let instr = decoder.decode();
        if instr.is_invalid() {
            report.incomplete = true;
            return None;
        }
        for i in 0..instr.op_count() {
            if let Some(v) = immediate(&instr, i) {
                for t in [v - 1, v, v + 1, -v - 1, -v, -v + 1] {
                    thresholds.insert(t);
                }
            }
        }
        if instr.memory_displacement64() != 0 {
            let d = i128::from(instr.memory_displacement64() as i64);
            for t in [d - 1, d, d + 1] {
                thresholds.insert(t);
            }
        }
        decoded.insert(at, instr);
        Some(instr)
    };

    // The lowest the stack pointer ever gets is what bounds the frame, so it is found as this goes.
    let mut lowest: i128 = 0;
    while let Some(at) = work.pop_first() {
        let Some(instr) = decode(at, &mut decoded, &mut thresholds, &mut report) else {
            continue;
        };
        let state = states[&at].clone();
        lowest = lowest.min(stack_offsets(&state));
        let list: Vec<i128> = thresholds.iter().copied().collect();
        for (target, out) in step(&state, &instr, &mut factory, None, lowest) {
            let visit = visits.entry(target).or_insert(0);
            let updated = match states.get(&target) {
                None => Some(out),
                Some(old) => {
                    let joined = old.join(&out);
                    if joined == *old {
                        None
                    } else {
                        *visit += 1;
                        if *visit > WIDEN_AFTER {
                            Some(old.widen(&joined, &list))
                        } else {
                            Some(joined)
                        }
                    }
                }
            };
            if let Some(s) = updated {
                states.insert(target, s);
                work.insert(target);
            }
        }
    }

    for (at, state) in &states {
        let Some(instr) = decoded.get(at).copied() else {
            continue;
        };
        report.instructions += 1;
        step(state, &instr, &mut factory, Some(&mut report), lowest);
    }
    report.accesses.sort_by_key(|a| a.address);
    report
}
