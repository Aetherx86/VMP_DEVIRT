//! The property the whole analyzer rests on: nothing it claims can be contradicted by running the code.
//!
//! Random programs run concretely on random inputs. At every step, each variable's value has to be
//! inside its abstract value. A fault may only occur where the analysis did not prove it impossible, and
//! wherever it said a fault was definite, reaching that point has to fault.

use absint::analysis::{Analysis, Verdict, analyze};
use absint::domain::{Interval, Nullness, Value};
use absint::ir::{
    BinOp, Block, CmpOp, Concrete, Cond, Function, Instr, Operand, Outcome, Term, ThreadKind, run,
};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        (self.next() >> 16) % n
    }
}

const VARS: u32 = 8;

fn operand(rng: &mut Rng) -> Operand {
    if rng.below(3) == 0 {
        Operand::Const(
            *[0, 1, -1, 2, 3, 8, 16, 100, -7]
                .get(rng.below(9) as usize)
                .unwrap(),
        )
    } else {
        Operand::Var(rng.below(u64::from(VARS)) as u32)
    }
}

fn var(rng: &mut Rng) -> u32 {
    rng.below(u64::from(VARS)) as u32
}

fn cond(rng: &mut Rng) -> Cond {
    let op = *[
        CmpOp::Lt,
        CmpOp::Le,
        CmpOp::Gt,
        CmpOp::Ge,
        CmpOp::Eq,
        CmpOp::Ne,
    ]
    .get(rng.below(6) as usize)
    .unwrap();
    let signed_bits = if rng.below(8) == 0 { Some(32) } else { None };
    Cond {
        op,
        a: operand(rng),
        b: operand(rng),
        signed_bits,
    }
}

fn instr(rng: &mut Rng) -> Instr {
    match rng.below(20) {
        0..=4 => Instr::Copy(var(rng), operand(rng)),
        5..=9 => {
            let op = *[
                BinOp::Add,
                BinOp::Sub,
                BinOp::Mul,
                BinOp::Div,
                BinOp::Rem,
                BinOp::And,
                BinOp::Or,
                BinOp::Xor,
                BinOp::Shl,
                BinOp::Shr,
            ]
            .get(rng.below(10) as usize)
            .unwrap();
            Instr::Bin(var(rng), op, operand(rng), operand(rng))
        }
        10 => Instr::Wrap(
            var(rng),
            operand(rng),
            if rng.below(2) == 0 { 8 } else { 32 },
        ),
        11 => {
            let lo = rng.below(40) as i128 - 20;
            Instr::Havoc(var(rng), Interval::new(lo, lo + rng.below(30) as i128))
        }
        12 | 13 => Instr::Alloc {
            dst: var(rng),
            size: operand(rng),
            nullable: rng.below(2) == 0,
        },
        14 => Instr::Null(var(rng)),
        15 => Instr::Load {
            dst: var(rng),
            ptr: var(rng),
            index: operand(rng),
        },
        16 => Instr::Store {
            ptr: var(rng),
            index: operand(rng),
            value: operand(rng),
        },
        17 => Instr::Assert(cond(rng)),
        18 => Instr::Assume(cond(rng)),
        _ => Instr::MemAccess {
            addr: operand(rng),
            offset: rng.below(16),
            size: 1 << rng.below(4),
            limit: 64,
        },
    }
}

fn function(rng: &mut Rng) -> Function {
    let count = 1 + rng.below(8) as usize;
    let mut blocks = Vec::new();
    for b in 0..count {
        let instrs: Vec<(Instr, u32)> = (0..rng.below(6)).map(|_| (instr(rng), 0)).collect();
        let term = match rng.below(5) {
            0 => Term::Return,
            1 | 2 => Term::Goto(rng.below(count as u64) as usize),
            _ => Term::Branch(
                cond(rng),
                rng.below(count as u64) as usize,
                rng.below(count as u64) as usize,
            ),
        };
        blocks.push(Block {
            name: format!("b{b}"),
            instrs,
            term,
        });
    }
    Function {
        name: "f".into(),
        thread: ThreadKind::None,
        params: vec![(0, Interval::new(-5, 20)), (1, Interval::TOP)],
        blocks,
        var_names: (0..VARS).map(|v| format!("v{v}")).collect(),
    }
}

fn inside(concrete: Concrete, value: Value) -> bool {
    match (concrete, value) {
        (_, Value::Top) => true,
        (Concrete::Int(v), Value::Int(i)) => i.contains(v),
        (Concrete::Null, Value::Ptr { null, .. }) => null != Nullness::NonNull,
        (Concrete::Ptr(_, size), Value::Ptr { null, size: s }) => {
            null != Nullness::Null && s.contains(size)
        }
        _ => false,
    }
}

fn check_run(analysis: &Analysis<'_>, seed: u64, rng: &mut Rng) -> Option<bool> {
    let f = analysis.function;
    let args = [
        rng.below(26) as i128 - 5,
        rng.next() as i64 as i128 >> rng.below(64),
    ];
    let mut chooser = Rng(rng.next() | 1);
    let mut choose = |r: Interval| {
        let span = (r.hi.min(1000) - r.lo.max(-1000)).max(0) as u64 + 1;
        r.lo.max(-1000) + chooser.below(span) as i128
    };
    let mut reached_all_definite = true;
    let outcome = run(f, &args, 2_000, &mut choose, &mut |block, i, vars| {
        let Some(state) = analysis.state_before(block, i) else {
            panic!(
                "seed {seed}: block {block} instruction {i} runs, but was said to be unreachable\n{f:#?}"
            );
        };
        for (v, (abs, concrete)) in state.iter().zip(vars).enumerate() {
            if let Some(a) = abs {
                let Some(c) = concrete else {
                    panic!(
                        "seed {seed}: v{v} is uninitialized at block {block} instruction {i}, and was said to hold {a:?}\n{f:#?}"
                    );
                };
                assert!(
                    inside(*c, *a),
                    "seed {seed}: v{v} is {c:?} and was said to be {a:?} at block {block} instruction {i}\n{f:#?}"
                );
            }
        }
        let _ = &mut reached_all_definite;
    });
    match outcome {
        Outcome::Fault {
            fault,
            block,
            instr,
        } => {
            let check = analysis
                .checks
                .iter()
                .find(|c| c.block == block && c.instr == instr && c.fault == fault);
            let check = check.unwrap_or_else(|| {
                panic!("seed {seed}: {fault:?} at {block}:{instr} was never checked\n{f:#?}")
            });
            assert_ne!(
                check.verdict,
                Verdict::Proved,
                "seed {seed}: {fault:?} happened at {block}:{instr} after being proved impossible\n{f:#?}"
            );
            Some(true)
        }
        Outcome::Returned | Outcome::Infeasible | Outcome::OutOfSteps => Some(false),
    }
}

#[test]
fn no_run_contradicts_the_analysis() {
    let mut rng = Rng(0xA5A5_5A5A_1234_8765);
    let (mut runs, mut faults, mut proved) = (0, 0, 0);
    for seed in 0..6_000 {
        let f = function(&mut rng);
        let analysis = analyze(&f);
        proved += analysis
            .checks
            .iter()
            .filter(|c| c.verdict == Verdict::Proved && !c.dead)
            .count();
        for _ in 0..12 {
            runs += 1;
            if check_run(&analysis, seed, &mut rng) == Some(true) {
                faults += 1;
            }
        }
    }
    assert!(
        faults > 1_000,
        "too few faults to mean anything: {faults} of {runs}"
    );
    assert!(
        proved > 2_000,
        "it should prove a good amount too: {proved}"
    );
}

/// A definite verdict says that reaching the instruction faults every time.
#[test]
fn definite_verdicts_always_fault_when_reached() {
    let mut rng = Rng(0x0F0F_F0F0_DEAD_10CC);
    let mut seen = 0;
    for seed in 0..6_000 {
        let f = function(&mut rng);
        let analysis = analyze(&f);
        let definite: Vec<(usize, usize)> = analysis
            .checks
            .iter()
            .filter(|c| c.verdict == Verdict::Definite)
            .map(|c| (c.block, c.instr))
            .collect();
        if definite.is_empty() {
            continue;
        }
        for _ in 0..12 {
            let args = [
                rng.below(26) as i128 - 5,
                rng.next() as i64 as i128 >> rng.below(64),
            ];
            let mut chooser = Rng(rng.next() | 1);
            let mut choose = |r: Interval| {
                let span = (r.hi.min(1000) - r.lo.max(-1000)).max(0) as u64 + 1;
                r.lo.max(-1000) + chooser.below(span) as i128
            };
            let mut last = (0, 0);
            let mut hit = false;
            let outcome = run(&f, &args, 2_000, &mut choose, &mut |b, i, _| {
                last = (b, i);
                hit = hit || definite.contains(&(b, i));
            });
            if hit {
                seen += 1;
                assert!(
                    matches!(outcome, Outcome::Fault { .. }),
                    "seed {seed}: reached a definite fault at {last:?}, and it ended with {outcome:?}\n{f:#?}"
                );
            }
        }
    }
    assert!(
        seen > 100,
        "too few reached a definite fault to mean anything: {seen}"
    );
}
