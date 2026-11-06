//! Proving WebAssembly memory accesses in bounds, and whether those proofs hold on a real engine.

use absint::analysis::{Verdict, analyze};
use absint::ir::Fault;
use absint::wasm::lower;

fn verdicts(text: &str) -> Vec<(Fault, Verdict)> {
    let wasm = wat::parse_str(text).unwrap();
    let lowered = lower(&wasm).unwrap();
    assert_eq!(lowered.skipped, [], "{text}");
    let mut out = Vec::new();
    for function in &lowered.program.functions {
        let result = analyze(function);
        out.extend(
            result
                .checks
                .iter()
                .filter(|c| {
                    !c.dead && matches!(c.fault, Fault::OutOfBounds | Fault::DivisionByZero)
                })
                .map(|c| (c.fault, c.verdict)),
        );
    }
    out
}

fn module(body: &str) -> String {
    format!("(module (memory 1) (func (param i32 i64) (result i32) (local i32 i64) {body}))")
}

fn all_proved(body: &str) {
    let found = verdicts(&module(body));
    assert!(!found.is_empty(), "nothing was checked: {body}");
    assert!(
        found.iter().all(|(_, v)| *v == Verdict::Proved),
        "{found:?}\n{body}"
    );
}

fn not_proved(body: &str) {
    let found = verdicts(&module(body));
    assert!(
        found.iter().any(|(_, v)| *v == Verdict::Possible),
        "this should not be proved: {found:?}\n{body}"
    );
}

#[test]
fn constant_addresses_are_checked_against_the_memory_size() {
    all_proved("i32.const 0 i32.load");
    all_proved("i32.const 65532 i32.load");
    not_proved("i32.const 65533 i32.load");
    all_proved("i32.const 65535 i32.load8_u");
    not_proved("i32.const 65536 i32.load8_u");
    all_proved("i32.const 100 i64.load offset=65428 i32.wrap_i64");
    not_proved("i32.const 100 i64.load offset=65429 i32.wrap_i64");
    all_proved("i32.const 65528 i64.const 1 i64.store i32.const 0");
    not_proved("i32.const 65529 i64.const 1 i64.store i32.const 0");
}

#[test]
fn a_mask_proves_an_address() {
    all_proved("local.get 0 i32.const 4095 i32.and i32.load");
    not_proved("local.get 0 i32.const 65535 i32.and i32.load");
    all_proved("local.get 0 i32.const 65532 i32.and i32.load");
    not_proved("local.get 0 i32.load");
}

#[test]
fn a_bounds_check_proves_an_address() {
    all_proved(
        "block (result i32) i32.const 0 local.get 0 i32.const 100 i32.ge_u br_if 0 drop local.get 0 i32.load end",
    );
    all_proved(
        "local.get 0 i32.const 100 i32.lt_u if (result i32) local.get 0 i32.load else i32.const 0 end",
    );
    all_proved(
        "local.get 0 i32.const 100 i32.ge_u if (result i32) i32.const 0 else local.get 0 i32.load end",
    );
    // An inverted test swaps which arm is safe.
    all_proved(
        "local.get 0 i32.const 100 i32.lt_u i32.eqz if (result i32) i32.const 0 else local.get 0 i32.load end",
    );
    not_proved(
        "local.get 0 i32.const 100 i32.lt_u i32.eqz if (result i32) local.get 0 i32.load else i32.const 0 end",
    );
    // The other arm of the same test.
    not_proved(
        "local.get 0 i32.const 100 i32.lt_u if (result i32) i32.const 0 else local.get 0 i32.load end",
    );
    // A signed check says nothing, since a negative number is a huge address.
    not_proved(
        "local.get 0 i32.const 100 i32.lt_s if (result i32) local.get 0 i32.load else i32.const 0 end",
    );
}

#[test]
fn signed_checks_prove_things_once_a_value_is_known_to_be_small() {
    all_proved(
        "local.get 0 i32.const 255 i32.and local.set 2 local.get 2 i32.const 100 i32.lt_s if (result i32) local.get 2 i32.load else i32.const 0 end",
    );
}

const COPY: &str = "
  block
    loop
      local.get 2 i32.const 1024 i32.ge_u br_if 1
      local.get 2 local.get 2 i32.store8
      local.get 2 i32.const 1 i32.add local.set 2
      br 0
    end
  end
  i32.const 0";

#[test]
fn a_counted_loop_is_proved_in_bounds() {
    all_proved(COPY);
    not_proved(&COPY.replace("1024", "70000"));
}

#[test]
fn local_writes_and_tee_are_followed() {
    all_proved("i32.const 8 local.set 2 local.get 2 i32.load");
    all_proved("i32.const 8 local.tee 2 i32.load");
    // The old value was already on the stack when the local changed.
    all_proved("i32.const 8 local.set 2 local.get 2 i32.const 99999 local.set 2 i32.load");
    not_proved("i32.const 99999 local.set 2 local.get 2 i32.const 8 local.set 2 i32.load");
}

#[test]
fn merged_paths_take_the_worst_of_both() {
    not_proved("local.get 0 if (result i32) i32.const 10 else i32.const 70000 end i32.load");
    all_proved("local.get 0 if (result i32) i32.const 10 else i32.const 7000 end i32.load");
}

#[test]
fn branch_tables_narrow_their_index() {
    all_proved(
        "block block local.get 0 i32.const 3 i32.and br_table 0 1 1 1 end i32.const 4 i32.load return end i32.const 8 i32.load",
    );
}

#[test]
fn wraparound_is_not_a_proof() {
    // Adding to a masked value is fine, and adding beyond 32 bits wraps to anything.
    all_proved("local.get 0 i32.const 255 i32.and i32.const 1000 i32.add i32.load");
    not_proved("local.get 0 i32.const 1 i32.add i32.load");
    // The largest value plus one wraps around to zero, which is a perfectly good address.
    all_proved("i32.const -1 i32.const 1 i32.add i32.load");
    all_proved("i64.const -1 i64.const 1 i64.add i32.wrap_i64 i32.load");
    not_proved("local.get 0 i32.const 2147483647 i32.and i32.const 2147483647 i32.mul i32.load");
}

#[test]
fn anything_unknown_has_the_full_range_of_its_type() {
    let found = verdicts(
        "(module (import \"a\" \"b\" (func (result i32))) (memory 1) (global (mut i32) (i32.const 0))
          (func (result i32)  global.get 0 i32.const 100 i32.lt_u if (result i32) global.get 0 i32.load else i32.const 0 end)
          (func (result i32)  call 0 i32.load))",
    );
    assert_eq!(
        found
            .iter()
            .filter(|(_, v)| *v == Verdict::Possible)
            .count(),
        2,
        "{found:?}"
    );
    let found = verdicts(
        "(module (import \"a\" \"b\" (func (result i32))) (memory 1)
          (func (result i32) (local i32)  call 0 local.set 0 local.get 0 i32.const 100 i32.lt_u if (result i32) local.get 0 i32.load else i32.const 0 end))",
    );
    assert!(
        found.iter().all(|(_, v)| *v == Verdict::Proved),
        "a checked call result is proved: {found:?}"
    );
}

#[test]
fn division_by_zero_is_checked() {
    all_proved("i32.const 10 local.get 0 i32.const 1 i32.or i32.div_u");
    not_proved("i32.const 10 local.get 0 i32.div_u");
    not_proved("i32.const 10 local.get 0 i32.div_s");
    all_proved("i32.const 10 local.get 0 i32.const 1 i32.or i32.rem_s");
}

#[test]
fn a_memory_declared_larger_gives_more_room() {
    let f = "(func (result i32) i32.const 131000 i32.load)";
    let found = verdicts(&format!("(module (memory 3) {f})"));
    assert_eq!(found, [(Fault::OutOfBounds, Verdict::Proved)]);
    let found = verdicts(&format!("(module (memory 1) {f})"));
    assert_eq!(found, [(Fault::OutOfBounds, Verdict::Possible)]);
    let found = verdicts(&format!("(module (import \"a\" \"m\" (memory 3)) {f})"));
    assert_eq!(found, [(Fault::OutOfBounds, Verdict::Proved)]);
}

#[test]
fn control_flow_it_does_not_model_is_skipped_and_says_so() {
    for body in [
        "(tag) (func (result i32) block try_table (catch_all 0) throw 0 end end i32.const 0)",
        "(func (result i32) i32.const 0 return_call 0)",
        "(func (param funcref) (result i32) block local.get 0 br_on_null 0 drop end i32.const 0)",
    ] {
        let lowered = lower(&wat::parse_str(format!("(module {body})")).unwrap()).unwrap();
        assert_eq!(lowered.skipped.len(), 1, "{body}");
        assert!(lowered.program.functions.is_empty());
    }
}

#[test]
fn block_parameters_and_multiple_results_flow_through() {
    let module = "(module (memory 1) (type $t (func (param i32) (result i32 i32)))
      (func (result i32)
        i32.const 8
        block (type $t)
          i32.const 16
        end
        i32.load
        drop
        i32.load))";
    let found = verdicts(module);
    assert_eq!(
        found,
        [
            (Fault::OutOfBounds, Verdict::Proved),
            (Fault::OutOfBounds, Verdict::Proved)
        ]
    );
    // An `if` with no else has to pass its parameters through.
    let module = "(module (memory 1) (type $t (func (param i32) (result i32)))
      (func (param i32) (result i32)
        i32.const 8
        local.get 0
        if (type $t)
          drop i32.const 12
        end
        i32.load))";
    assert_eq!(verdicts(module), [(Fault::OutOfBounds, Verdict::Proved)]);
}

#[test]
fn garbage_is_an_error_and_never_a_panic() {
    assert!(lower(b"\0asm\x01\0\0\0\x09").is_err());
    let good = wat::parse_str(module(COPY)).unwrap();
    let mut state = 0x1234_5678_9ABC_DEF1u64;
    for _ in 0..3_000 {
        let mut bytes = good.clone();
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let at = (state >> 8) as usize % bytes.len();
        bytes[at] = state as u8;
        if let Ok(lowered) = lower(&bytes) {
            for f in &lowered.program.functions {
                analyze(f);
            }
        }
    }
}
