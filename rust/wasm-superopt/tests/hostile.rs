//! Input built to make this tool crash, run out of memory, or take forever.

mod common;

use common::{Rng, bodies};
use std::time::{Duration, Instant};
use wasm_superopt::{Error, optimize};

fn quickly<T>(limit: Duration, what: &str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let out = f();
    assert!(start.elapsed() < limit, "{what} took {:?}", start.elapsed());
    out
}

const LIMIT: Duration = Duration::from_secs(120);

fn module_from(signature: &str, body: &str) -> Vec<u8> {
    wat::parse_str(format!("(module (memory 1) (func {signature} {body}))")).unwrap()
}

#[test]
fn a_very_long_chain_of_operations_does_not_overflow_the_stack() {
    // Nothing folds here, so this builds one enormous expression.
    let body = "local.get 0 local.get 1 i32.mul local.set 0 ".repeat(100_000);
    let input = module_from(
        "(param i32 i32) (result i32)",
        &format!("{body} local.get 0"),
    );
    let out = quickly(LIMIT, "the long chain", || optimize(&input)).unwrap();
    assert_eq!(
        out.wasm, input,
        "past the depth limit, the function stays as it was"
    );
}

#[test]
fn a_doubling_chain_does_not_blow_up_exponentially() {
    // 2^40 if anything walks this as a tree, whether when simplifying or when lowering.
    let body = "local.get 0 local.get 0 i32.add local.set 0 ".repeat(40);
    let input = module_from("(param i32) (result i32)", &format!("{body} local.get 0"));
    let out = quickly(LIMIT, "the doubling chain", || optimize(&input)).unwrap();
    assert!(out.wasm.len() < input.len());

    let squares = "local.get 0 local.get 0 i32.mul local.get 0 i32.add local.set 0 ".repeat(64);
    let test = "local.get 0 i32.const 1 i32.and i32.eqz if unreachable end local.get 0 i32.eqz";
    let input = module_from("(param i32) (result i32)", &format!("{squares} {test}"));
    let out = quickly(LIMIT, "the squaring chain", || optimize(&input)).unwrap();
    assert!(out.wasm.len() < input.len());
}

#[test]
fn deep_nesting_is_fine() {
    let depth = 20_000;
    let input = module_from(
        "",
        &format!("{}{}", "block ".repeat(depth), "end ".repeat(depth)),
    );
    let out = quickly(LIMIT, "deep nesting", || optimize(&input)).unwrap();
    assert_eq!(
        bodies(&out.wasm)[0].len(),
        1,
        "unused blocks all go away, leaving only the final end"
    );
}

#[test]
fn a_flood_of_hard_predicates_is_stopped_by_the_budget() {
    // Each of these takes the most work the proofs can do, and there are a lot of them.
    let hard = "local.get 0 local.get 0 i32.mul i32.const 7 i32.mul  local.get 1 local.get 1 i32.mul i32.const 3 i32.mul i32.add  i32.const 5 i32.add  local.get 0 local.get 1 i32.mul i32.const 9 i32.mul  i32.eq drop ";
    let input = module_from("(param i32 i32)", &hard.repeat(4_000));
    let out = quickly(LIMIT, "the hard predicates", || optimize(&input)).unwrap();
    assert!(out.wasm.len() <= input.len());
}

#[test]
fn many_locals_and_many_temporaries_are_fine() {
    // The limit on the whole module's locals must not turn into an internal error.
    let input = wat::parse_str(format!(
        "(module (func (local {}) i32.const 1 local.set 0))",
        "i32 ".repeat(45_000)
    ))
    .unwrap();
    optimize(&input).unwrap();
    // A lot of values on the real stack at once, which all have to spill.
    let calls = "call $v ".repeat(30_000);
    let adds = "i32.const 1 i32.add drop ".repeat(30_000);
    let input = wat::parse_str(format!(
        "(module (func $v (result i32) i32.const 1) (func {calls} {adds}))"
    ))
    .unwrap();
    quickly(LIMIT, "the spills", || optimize(&input)).unwrap();
}

fn not_a_bug(result: Result<wasm_superopt::Output, Error>) {
    if let Err(Error::Internal(message)) = result {
        panic!("a damaged module made it produce an invalid one: {message}");
    }
}

#[test]
fn garbage_is_rejected_and_never_panics() {
    let mut rng = Rng(0x0123_4567_89AB_CDEF);
    assert!(matches!(optimize(b""), Err(Error::Invalid(_))));
    assert!(matches!(optimize(b"not a module"), Err(Error::Invalid(_))));
    assert!(matches!(optimize(b"\0asm"), Err(Error::Invalid(_))));
    for _ in 0..2_000 {
        let length = rng.below(256) as usize;
        let mut bytes: Vec<u8> = (0..length).map(|_| rng.next() as u8).collect();
        if rng.chance(2) && bytes.len() >= 8 {
            bytes[..8].copy_from_slice(b"\0asm\x01\0\0\0");
        }
        not_a_bug(optimize(&bytes));
    }
}

#[test]
fn damaged_modules_never_panic_or_produce_anything_invalid() {
    let original = wat::parse_str(
        r#"(module
            (import "env" "log" (func $log (param i32)))
            (memory 1)
            (global $g (mut i32) (i32.const 7))
            (table 2 funcref)
            (elem (i32.const 0) $f)
            (data (i32.const 16) "hello")
            (func $f (param i32 i32) (result i32) (local i32 i64 f64)
              block (result i32)
                local.get 0
                local.get 0 local.get 1 i32.xor local.get 0 local.get 1 i32.and i32.const 2 i32.mul i32.add
                local.tee 2 local.get 2 i32.mul local.get 2 i32.add i32.const 1 i32.and i32.eqz br_if 0
                local.get 0 i32.load global.get $g i32.add call $log
                loop
                  local.get 2 i32.const 1 i32.sub local.tee 2 br_if 0
                end
                drop
                local.get 0 i32.const 3 i32.div_s
                i32.const 1 if (result i32) i32.const 4 else unreachable end
                local.get 1 select
              end
              block block local.get 1 br_table 0 1 end end
              f64.const 1.5 drop
              i64.const 9 local.set 3))"#,
    )
    .unwrap();
    optimize(&original).expect("the module has to be valid before it gets damaged");

    // Damage to the code section is the kind that most often leaves a module valid.
    let code = wasmparser::Parser::new(0)
        .parse_all(&original)
        .find_map(|payload| {
            let payload = payload.unwrap();
            match payload {
                wasmparser::Payload::CodeSectionStart { .. } => {
                    payload.as_section().map(|(_, range)| range)
                }
                _ => None,
            }
        })
        .unwrap();
    let (first, count) = (code.start, code.end - code.start);

    let mut rng = Rng(0xFEED_FACE_0BAD_F00D);
    let mut accepted = 0;
    for _ in 0..30_000 {
        let mut bytes = original.clone();
        for _ in 0..1 + rng.below(2) {
            let at = (first + rng.below(count)) as usize;
            match rng.below(3) {
                0 => bytes[at] = rng.next() as u8,
                1 => bytes[at] ^= 1 << rng.below(8),
                _ => bytes[at] = bytes[at].wrapping_add(1),
            }
        }
        // Now and then, damage anything at all, or cut it short.
        if rng.chance(8) {
            let at = rng.below(bytes.len() as u64) as usize;
            bytes[at] = rng.next() as u8;
        }
        if rng.chance(16) {
            bytes.truncate(rng.below(bytes.len() as u64 + 1) as usize);
        }
        let result = optimize(&bytes);
        if result.is_ok() {
            accepted += 1;
        }
        not_a_bug(result);
    }
    assert!(
        accepted > 1_000,
        "too few damaged modules were still valid to mean anything: {accepted}"
    );
}

#[test]
fn components_are_refused_clearly() {
    let component = wat::parse_str("(component)").unwrap();
    assert!(matches!(optimize(&component), Err(Error::Unsupported(_))));
}
