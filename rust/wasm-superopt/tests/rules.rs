//! Every simplification has to agree with plain evaluation, on inputs chosen to find the edge cases.
//!
//! Each random expression is built twice in one graph: once exactly as written, and once through the
//! simplifier. The two then have to compute the same value on every input tried.

mod common;

use common::Rng;
use wasm_superopt::ssa::{Bin, Cmp, Graph, Node, Un, V, Width, eval_bin, safe_divisor};
use wasmparser::ValType;

const I32_INPUTS: u32 = 3;
const I64_INPUTS: u32 = 2;

fn interesting(rng: &mut Rng) -> u64 {
    match rng.below(8) {
        0 => 0,
        1 => 1,
        2 => u64::MAX,
        3 => 0x7FFF_FFFF,
        4 => 0x8000_0000,
        5 => u64::MAX << 63,
        6 => 1 << rng.below(64),
        _ => rng.next(),
    }
}

/// One expression, built both ways.
fn pair(g: &mut Graph, rng: &mut Rng, width: Width, depth: u32) -> (V, V) {
    if depth == 0 || rng.chance(6) {
        if rng.chance(3) {
            let c = g.constant(width, interesting(rng));
            return (c, c);
        }
        let input = match width {
            Width::W32 => g.intern(Node::Input(
                rng.below(u64::from(I32_INPUTS)) as u32,
                ValType::I32,
            )),
            Width::W64 => g.intern(Node::Input(
                I32_INPUTS + rng.below(u64::from(I64_INPUTS)) as u32,
                ValType::I64,
            )),
        };
        return (input, input);
    }
    match rng.below(20) {
        // The forms obfuscators use, so that the proofs get exercised far more often than chance would.
        0 => {
            let (xr, xs) = pair(g, rng, width, depth - 1);
            let (yr, ys) = pair(g, rng, width, depth - 1);
            let two = g.constant(width, 2);
            let raw = {
                let a = g.intern(Node::Bin(Bin::Xor, width, xr, yr));
                let b = g.intern(Node::Bin(Bin::And, width, xr, yr));
                let c = g.intern(Node::Bin(Bin::Mul, width, b, two));
                g.intern(Node::Bin(Bin::Add, width, a, c))
            };
            let simple = {
                let a = g.bin(Bin::Xor, width, xs, ys);
                let b = g.bin(Bin::And, width, xs, ys);
                let c = g.bin(Bin::Mul, width, b, two);
                g.bin(Bin::Add, width, a, c)
            };
            (raw, simple)
        }
        1 => {
            // A random linear combination of bitwise functions of the same few values.
            let (xr, xs) = pair(g, rng, width, depth - 1);
            let (yr, ys) = pair(g, rng, width, depth - 1);
            let mut raw = g.constant(width, rng.below(5));
            let mut simple = raw;
            for _ in 0..1 + rng.below(4) {
                let op = *rng.pick(&[Bin::And, Bin::Or, Bin::Xor]);
                let factor = g.constant(width, rng.below(9).wrapping_sub(4));
                let (tr, ts) = (
                    g.intern(Node::Bin(op, width, xr, yr)),
                    g.bin(op, width, xs, ys),
                );
                let (mr, ms) = (
                    g.intern(Node::Bin(Bin::Mul, width, tr, factor)),
                    g.bin(Bin::Mul, width, ts, factor),
                );
                raw = g.intern(Node::Bin(Bin::Add, width, raw, mr));
                simple = g.bin(Bin::Add, width, simple, ms);
            }
            (raw, simple)
        }
        2 => {
            // Division, by a constant that the translator would accept as unable to trap.
            let (xr, xs) = pair(g, rng, width, depth - 1);
            let op = *rng.pick(&[Bin::DivS, Bin::DivU, Bin::RemS, Bin::RemU]);
            let mut divisor = width.truncate(*rng.pick(&[
                1,
                2,
                3,
                4,
                7,
                8,
                100,
                0x8000_0000,
                u64::MAX,
                u64::MAX - 1,
            ]));
            if !safe_divisor(op, width, divisor) {
                divisor = 3;
            }
            let c = g.constant(width, divisor);
            (
                g.intern(Node::Bin(op, width, xr, c)),
                g.bin(op, width, xs, c),
            )
        }
        3 => {
            let (xr, xs) = pair(g, rng, width, depth - 1);
            let mut ops = vec![Un::Clz, Un::Ctz, Un::Popcnt, Un::Extend8S, Un::Extend16S];
            if width == Width::W64 {
                ops.push(Un::Extend32S);
            }
            let op = *rng.pick(&ops);
            (g.intern(Node::Un(op, width, xr)), g.un(op, width, xs))
        }
        4 | 5 => {
            // A comparison at either width, widened or not to fit.
            let inner = if rng.chance(2) {
                Width::W32
            } else {
                Width::W64
            };
            let (ar, asimple) = pair(g, rng, inner, depth - 1);
            let (br, bsimple) = pair(g, rng, inner, depth - 1);
            let op = *rng.pick(&[
                Cmp::Eq,
                Cmp::Ne,
                Cmp::LtS,
                Cmp::LtU,
                Cmp::GtS,
                Cmp::GtU,
                Cmp::LeS,
                Cmp::LeU,
                Cmp::GeS,
                Cmp::GeU,
            ]);
            let raw = g.intern(Node::Cmp(op, inner, ar, br));
            let simple = g.cmp(op, inner, asimple, bsimple);
            widen(g, width, raw, simple)
        }
        6 => {
            let inner = if rng.chance(2) {
                Width::W32
            } else {
                Width::W64
            };
            let (xr, xs) = pair(g, rng, inner, depth - 1);
            let raw = g.intern(Node::Eqz(inner, xr));
            let simple = g.eqz(inner, xs);
            widen(g, width, raw, simple)
        }
        7 => {
            let (ar, asimple) = pair(g, rng, width, depth - 1);
            let (br, bsimple) = pair(g, rng, width, depth - 1);
            let (cr, csimple) = pair(g, rng, Width::W32, depth - 1);
            (
                g.intern(Node::Select(ar, br, cr)),
                g.select(asimple, bsimple, csimple),
            )
        }
        8 => match width {
            Width::W32 => {
                let (xr, xs) = pair(g, rng, Width::W64, depth - 1);
                (g.intern(Node::Wrap(xr)), g.wrap(xs))
            }
            Width::W64 => {
                let (xr, xs) = pair(g, rng, Width::W32, depth - 1);
                if rng.chance(2) {
                    (g.intern(Node::ExtendU(xr)), g.extend_u(xs))
                } else {
                    (g.intern(Node::ExtendS(xr)), g.extend_s(xs))
                }
            }
        },
        _ => {
            let op = *rng.pick(&[
                Bin::Add,
                Bin::Sub,
                Bin::Mul,
                Bin::And,
                Bin::Or,
                Bin::Xor,
                Bin::Shl,
                Bin::ShrS,
                Bin::ShrU,
                Bin::Rotl,
                Bin::Rotr,
            ]);
            let (ar, asimple) = pair(g, rng, width, depth - 1);
            let (br, bsimple) = pair(g, rng, width, depth - 1);
            (
                g.intern(Node::Bin(op, width, ar, br)),
                g.bin(op, width, asimple, bsimple),
            )
        }
    }
}

/// Fits the `i32` a comparison produces into the width the caller wanted.
fn widen(g: &mut Graph, width: Width, raw: V, simple: V) -> (V, V) {
    match width {
        Width::W32 => (raw, simple),
        Width::W64 => (g.intern(Node::ExtendU(raw)), g.extend_u(simple)),
    }
}

fn inputs_for(rng: &mut Rng) -> [u64; (I32_INPUTS + I64_INPUTS) as usize] {
    let mut out = [0; (I32_INPUTS + I64_INPUTS) as usize];
    for (i, slot) in out.iter_mut().enumerate() {
        let value = interesting(rng);
        *slot = if (i as u32) < I32_INPUTS {
            value & 0xFFFF_FFFF
        } else {
            value
        };
    }
    out
}

#[test]
fn simplified_expressions_compute_the_same_values() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut simpler = 0;
    for tree in 0..40_000 {
        let mut g = Graph::default();
        let width = if rng.chance(2) {
            Width::W32
        } else {
            Width::W64
        };
        let depth = 1 + rng.below(5) as u32;
        let (raw, simple) = pair(&mut g, &mut rng, width, depth);
        if raw != simple {
            simpler += 1;
        }
        for _ in 0..12 {
            let inputs = inputs_for(&mut rng);
            let get = |local: u32| inputs.get(local as usize).copied();
            let (a, b) = (g.eval(raw, &get).unwrap(), g.eval(simple, &get).unwrap());
            assert_eq!(
                a,
                b,
                "tree {tree} differs on {inputs:x?}:\n raw {:?}\n simplified {:?}",
                g.node(raw),
                g.node(simple)
            );
        }
    }
    assert!(
        simpler > 10_000,
        "the test only means something if the simplifier did something, and it changed {simpler}"
    );
}

#[test]
fn ranges_hold_for_the_values_that_actually_occur() {
    let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
    let mut narrow = 0;
    for _ in 0..20_000 {
        let mut g = Graph::default();
        let width = if rng.chance(2) {
            Width::W32
        } else {
            Width::W64
        };
        let (raw, _) = pair(&mut g, &mut rng, width, 4);
        let mut budget = 256;
        let (lo, hi) = g.range(raw, width, &mut budget);
        assert!(lo <= hi);
        if (lo, hi) != (0, width.mask()) {
            narrow += 1;
        }
        for _ in 0..12 {
            let inputs = inputs_for(&mut rng);
            let value = g
                .eval(raw, &|local| inputs.get(local as usize).copied())
                .unwrap();
            assert!(
                (lo..=hi).contains(&value),
                "{value:#x} is outside {lo:#x}..={hi:#x} for {:?}",
                g.node(raw)
            );
        }
    }
    assert!(narrow > 2_000, "ranges were almost never narrow: {narrow}");
}

#[test]
fn every_two_variable_bitwise_function_is_found_by_mba() {
    // There are 16 bitwise functions of two values, and each written as a sum of conjunctions has to
    // come back as something that evaluates the same.
    let width = Width::W32;
    for table in 0u32..16 {
        let mut g = Graph::default();
        let x = g.intern(Node::Input(0, ValType::I32));
        let y = g.intern(Node::Input(1, ValType::I32));
        let ones = g.constant(width, width.mask());
        let not_x = g.bin(Bin::Xor, width, x, ones);
        let not_y = g.bin(Bin::Xor, width, y, ones);
        let minterms = [(not_x, not_y), (x, not_y), (not_x, y), (x, y)];
        let mut sum = g.constant(width, 0);
        for (bit, (a, b)) in minterms.into_iter().enumerate() {
            if table >> bit & 1 == 1 {
                let term = g.bin(Bin::And, width, a, b);
                sum = g.bin(Bin::Add, width, sum, term);
            }
        }
        let mut rng = Rng(u64::from(table) + 1);
        for _ in 0..64 {
            let (a, b) = (rng.next() & 0xFFFF_FFFF, rng.next() & 0xFFFF_FFFF);
            let mut want = 0;
            for position in 0..32 {
                let index = (a >> position & 1) | (b >> position & 1) << 1;
                want |= u64::from(table >> index & 1) << position;
            }
            let values = [a, b];
            assert_eq!(
                g.eval(sum, &|local| values.get(local as usize).copied()),
                Some(want),
                "table {table:04b}"
            );
        }
    }
}

#[test]
fn the_helpers_agree_with_wasm_on_the_cases_that_differ_between_widths() {
    use Width::{W32, W64};
    assert_eq!(
        eval_bin(Bin::Shl, W32, 1, 33),
        2,
        "shift amounts wrap at the width"
    );
    assert_eq!(eval_bin(Bin::Shl, W64, 1, 33), 1 << 33);
    assert_eq!(eval_bin(Bin::ShrS, W32, 0x8000_0000, 31), 0xFFFF_FFFF);
    assert_eq!(eval_bin(Bin::ShrU, W32, 0x8000_0000, 31), 1);
    assert_eq!(eval_bin(Bin::Rotl, W32, 0x8000_0001, 1), 3);
    assert_eq!(eval_bin(Bin::Rotr, W32, 3, 1), 0x8000_0001);
    assert_eq!(
        eval_bin(Bin::DivS, W32, 0xFFFF_FFF9, 2),
        0xFFFF_FFFD,
        "signed division truncates toward zero"
    );
    assert_eq!(
        eval_bin(Bin::RemS, W32, 0xFFFF_FFF9, 2),
        0xFFFF_FFFF,
        "and the remainder takes the dividend's sign"
    );
    assert_eq!(eval_bin(Bin::DivU, W32, 0xFFFF_FFF9, 2), 0x7FFF_FFFC);
    assert!(
        !safe_divisor(Bin::DivS, W32, 0xFFFF_FFFF),
        "the most negative value divided by -1 traps"
    );
    assert!(
        safe_divisor(Bin::RemS, W32, 0xFFFF_FFFF),
        "but its remainder is just 0"
    );
    assert!(!safe_divisor(Bin::RemU, W64, 0));
}
