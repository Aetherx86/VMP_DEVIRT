//! What the tool is for: known obfuscations going in, and exactly the plain code coming out.

mod common;

use common::{bodies, local_counts};
use wasm_superopt::optimize;

/// Optimizes one function, and checks that it comes out as exactly the expected instructions.
fn check(signature: &str, body: &str, expected: &str) {
    let input = wat::parse_str(format!("(module (memory 1) (func {signature} {body}))")).unwrap();
    let want =
        wat::parse_str(format!("(module (memory 1) (func {signature} {expected}))")).unwrap();
    let out = optimize(&input).unwrap();
    assert_eq!(
        bodies(&out.wasm),
        bodies(&want),
        "\nfor: {body}\nwant: {expected}\n{:?}",
        out.report
    );
}

const XY: &str = "(param i32 i32) (result i32)";
const XYZ: &str = "(param i32 i32 i32) (result i32)";
const X: &str = "(param i32) (result i32)";

#[test]
fn addition_hidden_behind_mba() {
    let plain = "local.get 0 local.get 1 i32.add";
    check(
        XY,
        "local.get 0 local.get 1 i32.xor  local.get 0 local.get 1 i32.and i32.const 2 i32.mul  i32.add",
        plain,
    );
    check(
        XY,
        "local.get 0 local.get 1 i32.xor  local.get 0 local.get 1 i32.and i32.const 1 i32.shl  i32.add",
        plain,
    );
    check(
        XY,
        "local.get 0 local.get 1 i32.or  local.get 0 local.get 1 i32.and  i32.add",
        plain,
    );
    check(
        XY,
        "local.get 0 local.get 1 i32.or i32.const 2 i32.mul  local.get 0 local.get 1 i32.xor  i32.sub",
        plain,
    );
}

#[test]
fn other_operators_hidden_behind_mba() {
    check(
        XY,
        "local.get 0 local.get 1 i32.add  local.get 0 local.get 1 i32.and i32.const 2 i32.mul  i32.sub",
        "local.get 0 local.get 1 i32.xor",
    );
    check(
        XY,
        "local.get 0 local.get 1 i32.and  local.get 0 local.get 1 i32.xor  i32.or",
        "local.get 0 local.get 1 i32.or",
    );
    // x - y, written as ~(~x + y).
    check(
        XY,
        "local.get 0 i32.const -1 i32.xor  local.get 1  i32.add  i32.const -1 i32.xor",
        "local.get 0 local.get 1 i32.sub",
    );
    check(
        XY,
        "local.get 0 local.get 1 i32.xor  local.get 0 i32.const -1 i32.xor local.get 1 i32.and i32.const 2 i32.mul  i32.sub",
        "local.get 0 local.get 1 i32.sub",
    );
}

#[test]
fn mba_is_decided_exactly_at_both_widths_and_with_three_variables() {
    check(
        "(param i64 i64) (result i64)",
        "local.get 0 local.get 1 i64.xor  local.get 0 local.get 1 i64.and i64.const 2 i64.mul  i64.add",
        "local.get 0 local.get 1 i64.add",
    );
    // x + y + z - (x | y) - (x & y) is z.
    check(
        XYZ,
        "local.get 0 local.get 1 i32.add local.get 2 i32.add  local.get 0 local.get 1 i32.or i32.sub  local.get 0 local.get 1 i32.and i32.sub",
        "local.get 2",
    );
    // This one always comes to a constant, which is an opaque predicate in disguise.
    check(
        XY,
        "local.get 0 local.get 1 i32.or  local.get 0 local.get 1 i32.and i32.sub  local.get 0 local.get 1 i32.xor i32.sub  i32.const 9 i32.add",
        "i32.const 9",
    );
}

#[test]
fn mba_over_a_subexpression_treats_it_as_one_variable() {
    let shifted = "local.get 0 i32.const 3 i32.shr_u";
    check(
        XY,
        &format!(
            "{shifted} local.get 1 i32.xor  {shifted} local.get 1 i32.and i32.const 2 i32.mul  i32.add"
        ),
        &format!("{shifted} local.get 1 i32.add"),
    );
}

/// A two-armed `if` whose condition never changes must lose its other arm.
fn always(condition: &str, taken: bool) {
    let live = if taken {
        "i32.const 11"
    } else {
        "i32.const 22"
    };
    check(
        XY,
        &format!("{condition} if (result i32) i32.const 11 else i32.const 22 end"),
        live,
    );
}

#[test]
fn opaque_predicates_from_parity() {
    let x = "local.get 0";
    let product = format!("{x} {x} i32.const 1 i32.add i32.mul");
    let square_plus = format!("{x} {x} i32.mul {x} i32.add");
    always(&format!("{square_plus} i32.const 1 i32.and i32.eqz"), true);
    always(&format!("{square_plus} i32.const 1 i32.and"), false);
    always(
        &format!("{product} i32.const 1 i32.and i32.const 0 i32.eq"),
        true,
    );
    always(&format!("{product} i32.const 2 i32.rem_u i32.eqz"), true);
    always(&format!("{product} i32.const 2 i32.rem_s i32.eqz"), true);
    always(
        &format!("{product} i32.const 2 i32.rem_s i32.const 1 i32.eq"),
        false,
    );
    always(&format!("{square_plus} i32.const 1 i32.or i32.eqz"), false);
    // Always odd, so it never equals an even constant.
    always(
        &format!("{product} i32.const 1 i32.add i32.const 100 i32.eq"),
        false,
    );
}

#[test]
fn opaque_predicates_from_modular_arithmetic() {
    let (x, y) = ("local.get 0", "local.get 1");
    // 7y^2 - 1 is never a square, which is a classic predicate from the literature on this.
    always(
        &format!(
            "i32.const 7 {y} {y} i32.mul i32.mul i32.const 1 i32.sub  {x} {x} i32.mul  i32.eq"
        ),
        false,
    );
    always(
        &format!(
            "i32.const 7 {y} {y} i32.mul i32.mul i32.const 1 i32.sub  {x} {x} i32.mul  i32.ne"
        ),
        true,
    );
    // A square is 0, 1 or 4 modulo 8, so these are never reached.
    always(&format!("{x} {x} i32.mul i32.const 2 i32.eq"), false);
    always(
        &format!("{x} {x} i32.mul i32.const 3 i32.and i32.const 2 i32.eq"),
        false,
    );
    always(
        &format!("{x} {x} i32.mul i32.const 4 i32.rem_u i32.const 2 i32.eq"),
        false,
    );
    always(
        &format!("{x} {x} i32.mul i32.const 4 i32.rem_s i32.const 2 i32.eq"),
        false,
    );
    always(&format!("{x} i32.const 1 i32.or i32.eqz"), false);
    always(
        &format!("{x} i32.const 3 i32.shl i32.const 1 i32.or i32.const 8 i32.eq"),
        false,
    );
}

#[test]
fn opaque_predicates_from_ranges() {
    let x = "local.get 0";
    always(
        &format!("{x} i32.const 7 i32.and i32.const 7 i32.gt_u"),
        false,
    );
    always(
        &format!("{x} i32.const 7 i32.and i32.const 8 i32.lt_u"),
        true,
    );
    always(
        &format!("{x} i32.const 7 i32.and i32.const 0 i32.lt_s"),
        false,
    );
    always(
        &format!("{x} i32.const 28 i32.shr_u i32.const 16 i32.lt_u"),
        true,
    );
    always(&format!("{x} i32.popcnt i32.const 32 i32.gt_u"), false);
    always(
        &format!("{x} i32.const 100 i32.rem_u i32.const 100 i32.lt_u"),
        true,
    );
    always(
        &format!("{x} i32.const 255 i32.and i32.const 1 i32.add i32.eqz"),
        false,
    );
    always(&format!("{x} {x} i32.eq"), true);
    always(&format!("{x} i32.const 0 i32.lt_u"), false);
}

/// Conditions that really do depend on the input have to stay.
fn undecided(condition: &str) {
    let input = wat::parse_str(format!(
        "(module (func {XY} {condition} if (result i32) i32.const 11 else i32.const 22 end))"
    ))
    .unwrap();
    let out = optimize(&input).unwrap();
    let has_arm = |text: &str| bodies(&out.wasm)[0].iter().any(|op| op.contains(text));
    assert!(
        has_arm("value: 11") && has_arm("value: 22"),
        "this depends on the input, so both arms must stay: {condition}\n{:?}",
        bodies(&out.wasm)
    );
}

#[test]
fn real_conditions_are_never_mistaken_for_opaque_ones() {
    let (x, y) = ("local.get 0", "local.get 1");
    undecided(&format!("{x} {y} i32.eq"));
    undecided(&format!("{x} {x} i32.mul i32.const 1 i32.and i32.eqz"));
    undecided(&format!("{x} {x} i32.mul i32.const 4 i32.eq"));
    undecided(&format!("{x} i32.const 7 i32.and i32.const 6 i32.gt_u"));
    undecided(&format!(
        "{x} i32.const 1 i32.add i32.const 1 i32.and i32.eqz"
    ));
    undecided(&format!(
        "{x} {x} i32.const 1 i32.add i32.mul {y} i32.add i32.const 1 i32.and i32.eqz"
    ));
    undecided(&format!(
        "{x} {x} i32.const 1 i32.add i32.mul i32.const 3 i32.and i32.eqz"
    ));
    // Even by an even divisor, a remainder is only known to be zero if the divisor is exactly 2.
    undecided(&format!(
        "{x} {x} i32.const 1 i32.add i32.mul i32.const 6 i32.rem_u i32.eqz"
    ));
    undecided(&format!(
        "{x} {x} i32.const 1 i32.add i32.mul i32.const 6 i32.rem_s i32.eqz"
    ));
    // An odd divisor does not keep the parity of what it divides.
    undecided(&format!(
        "{x} {x} i32.const 1 i32.add i32.mul i32.const 3 i32.rem_u i32.const 1 i32.and i32.eqz"
    ));
    undecided(&format!(
        "{x} {x} i32.const 1 i32.add i32.mul i32.const 3 i32.rem_s i32.const 1 i32.and i32.eqz"
    ));
    // A remainder of an odd number is 1 or -1 when signed, so this one can go either way.
    undecided(&format!(
        "{x} {x} i32.const 1 i32.add i32.mul i32.const 1 i32.add i32.const 2 i32.rem_s i32.const 1 i32.eq"
    ));
    // A variable left shift is not a T-function, because the shift amount is not fixed.
    undecided(&format!(
        "{x} i32.const 1 i32.or {y} i32.shl i32.const 2 i32.eq"
    ));
}

#[test]
fn shuffling_through_locals_disappears() {
    // Moves both values around, swaps them, and only then uses them.
    check(
        "(param i32 i32) (result i32) (local i32 i32)",
        "local.get 0 local.set 2  local.get 1 local.set 3  local.get 3 local.get 2 i32.sub",
        "local.get 1 local.get 0 i32.sub",
    );
    let input = wat::parse_str(
        "(module (func (param i32 i32) (result i32) (local i32 i32)
            local.get 0 local.set 2 local.get 1 local.set 3 local.get 3 local.get 2 i32.sub))",
    )
    .unwrap();
    assert_eq!(
        local_counts(&optimize(&input).unwrap().wasm),
        [0],
        "the unused locals go too"
    );
}

#[test]
fn parallel_assignment_reads_the_old_values() {
    // Swaps two parameters that are read again after a block boundary, which forces real writes.
    let input = wat::parse_str(
        "(module (func (export \"f\") (param i32 i32) (result i32)
            local.get 1 local.get 0 local.set 1 local.set 0
            block end
            local.get 0 i32.const 100 i32.mul local.get 1 i32.add))",
    )
    .unwrap();
    let out = optimize(&input).unwrap();
    let ops = bodies(&out.wasm)[0].join(" ");
    assert!(
        !ops.contains("Block"),
        "an empty block has no reason to stay: {ops}"
    );
}

#[test]
fn constants_flow_through_locals_and_into_blocks() {
    check(
        "(result i32) (local i32)",
        "i32.const 5 local.set 0  block  local.get 0 i32.const 5 i32.eq br_if 0  unreachable  end  i32.const 8",
        "i32.const 8",
    );
    check(
        "(result i32) (local i32)",
        "local.get 0 i32.const 3 i32.add",
        "i32.const 3",
    );
    check(
        "(param i32) (result i32) (local i32)",
        "i32.const 4 local.set 1  local.get 0 if  i32.const 4 local.set 1 end  local.get 1",
        "i32.const 4 local.set 1  local.get 0 if  i32.const 4 local.set 1 end  local.get 1",
    );
}

#[test]
fn knowledge_about_locals_stops_where_it_stops_being_true() {
    // After the `if`, the local may hold either value.
    check(
        "(param i32) (result i32) (local i32)",
        "i32.const 4 local.set 1  local.get 0 if  i32.const 9 local.set 1 end  local.get 1 i32.const 4 i32.eq",
        "i32.const 4 local.set 1  local.get 0 if  i32.const 9 local.set 1 end  local.get 1 i32.const 4 i32.eq",
    );
    // The loop's second time around sees what the end of the first time around wrote.
    let looping = "i32.const 1 local.set 1  loop  local.get 1 i32.const 1 i32.eq  if  i32.const 2 local.set 1  br 1  end  end  local.get 1";
    let looping = looping
        .replace("local.set 1", "local.set 0")
        .replace("local.get 1", "local.get 0");
    check("(result i32) (local i32)", &looping, &looping);
    // A later write replaces what was known, even when the new value is not a constant.
    check(
        "(param i32) (result i32) (local i32)",
        "i32.const 1 local.set 1  local.get 0 local.set 1  block  local.get 1 i32.const 1 i32.eq br_if 0  i32.const 9 return  end  i32.const 8",
        "local.get 0 local.set 1  block  local.get 1 i32.const 1 i32.eq br_if 0  i32.const 9 return  end  i32.const 8",
    );
    // The same when the constant really was written first, so that it had become known.
    check(
        "(param i32) (result i32) (local i32)",
        "i32.const 1 local.set 1  block  local.get 0 local.set 1  block  local.get 1 i32.const 1 i32.eq br_if 0  i32.const 9 return  end  end  i32.const 8",
        "local.get 0 local.set 1  block  local.get 1 i32.const 1 i32.eq br_if 0  i32.const 9 return  end  i32.const 8",
    );
    // A parameter is not zero just because nothing writes it.
    check(
        "(param i32) (result i32)",
        "local.get 0 i32.const 3 i32.add",
        "local.get 0 i32.const 3 i32.add",
    );
    // A `local.tee` counts as a write, even with no `local.set` anywhere.
    check(
        "(param i32) (result i32) (local i32)",
        "block  local.get 0 local.tee 1 br_if 0  end  local.get 1",
        "block  local.get 0 local.tee 1 br_if 0  end  local.get 1",
    );
    // The `else` starts from what was true before the `if`, and not from what the other arm did.
    check(
        "(param i32) (result i32) (local i32)",
        "i32.const 1 local.set 1
         local.get 0 if (result i32) i32.const 2 local.set 1 local.get 1 else local.get 1 end",
        "local.get 0 if (result i32) i32.const 2 else i32.const 1 end",
    );
}

#[test]
fn dead_branches_and_dead_code_are_removed() {
    check(
        X,
        "i32.const 0 if  unreachable  end  local.get 0",
        "local.get 0",
    );
    check(
        X,
        "i32.const 1 if  local.get 0 i32.eqz drop  else  unreachable  end  local.get 0",
        "local.get 0",
    );
    check(
        X,
        "block  i32.const 0 br_if 0  end  local.get 0",
        "local.get 0",
    );
    check(
        X,
        "block  i32.const 1 br_if 0  unreachable  end  local.get 0",
        "local.get 0",
    );
    let table = |index: i32| {
        format!(
            "block  block  i32.const {index} br_table 0 1 1  unreachable  end  i32.const 5 return  end  local.get 0"
        )
    };
    check(X, &table(0), "i32.const 5 return");
    check(X, &table(1), "local.get 0");
    check(X, &table(9), "local.get 0");
    // A branch that really does jump over something has to stay.
    check(
        X,
        "block  local.get 0 br_if 0  i32.const 1 return  end  i32.const 2",
        "block  local.get 0 br_if 0  i32.const 1 return  end  i32.const 2",
    );
    check(
        X,
        "local.get 0 return  i32.const 1 i32.const 2 i32.add",
        "local.get 0 return",
    );
}

#[test]
fn a_branch_only_goes_away_if_it_really_does_nothing() {
    // It also throws away a value, and falling off the end of the block could not do that.
    let keep = |signature: &str, body: &str| check(signature, body, body);
    keep("", "block  i32.const 1  br 0  end");
    check(
        "(param i32)",
        "block  local.get 0  i32.const 1  br_if 0  drop  end",
        "block  local.get 0  br 0  end",
    );
    // In a loop, the same branch goes back to the start, and this one never finishes.
    keep("", "loop  br 0  end");
    // With exactly the right values on the stack, falling through is the same.
    check(
        "(param i32) (result i32)",
        "block (result i32)  local.get 0  br 0  end",
        "local.get 0",
    );
    check(
        "(param i32) (result i32)",
        "block (result i32)  local.get 0  i32.const 1  br_if 0  end",
        "local.get 0",
    );
    check(
        "(param i32) (result i32)",
        "local.get 0 if (result i32)  i32.const 1  br 0  else  i32.const 2  end",
        "local.get 0 if (result i32)  i32.const 1  else  i32.const 2  end",
    );
}

#[test]
fn a_branch_table_index_is_not_a_condition() {
    // `x != 0` as an index is 0 or 1, and x itself could be anything, so it must not be simplified to x.
    check(
        X,
        "block block  local.get 0 i32.const 0 i32.ne  br_table 0 1  end  i32.const 1 return  end  i32.const 2",
        "block block  local.get 0 i32.const 0 i32.ne  br_table 0 1  end  i32.const 1 return  end  i32.const 2",
    );
}

#[test]
fn branches_keep_their_targets_when_blocks_around_them_go_away() {
    // The middle block is the one a folded `if` left behind, and the branch crosses it.
    check(
        X,
        "block  i32.const 1 if  local.get 0 br_if 1  end  i32.const 5 return  end  local.get 0",
        "block  local.get 0 br_if 0  i32.const 5 return  end  local.get 0",
    );
    // A branch to the innermost frame crosses nothing, even with removed blocks further out.
    check(
        X,
        "block  block  local.get 0 if  local.get 0 br_if 0  i32.const 7 drop  end  end  end  local.get 0",
        "local.get 0 if  local.get 0 br_if 0  end  local.get 0",
    );
    // The same for a table.
    check(
        X,
        "block  block  i32.const 1 if  local.get 0 br_table 1 2  end  i32.const 5 return  end  i32.const 6 return  end  local.get 0",
        "block  block  local.get 0 br_table 0 1  end  i32.const 6 return  end  local.get 0",
    );
}

#[test]
fn conditions_lose_what_makes_no_difference() {
    let keep = "if (result i32) i32.const 1 else i32.const 2 end";
    check(
        XY,
        &format!("local.get 0 i32.const 0 i32.ne {keep}"),
        &format!("local.get 0 {keep}"),
    );
    check(
        XY,
        &format!("local.get 0 i32.eqz i32.eqz {keep}"),
        &format!("local.get 0 {keep}"),
    );
    // As a value and not a condition, x != 0 is not the same as x.
    check(
        XY,
        "local.get 0 i32.const 0 i32.ne",
        "local.get 0 i32.const 0 i32.ne",
    );
    check(
        XY,
        "local.get 0 i32.eqz i32.eqz",
        "local.get 0 i32.eqz i32.eqz",
    );
}

#[test]
fn nothing_that_can_trap_or_has_an_effect_is_removed_or_moved() {
    let keep = |signature: &str, body: &str| check(signature, body, body);
    // The results are unused, but each of these can trap.
    keep("(param i32)", "local.get 0 i32.const 0 i32.div_u drop");
    keep("(param i32 i32)", "local.get 0 local.get 1 i32.div_s drop");
    keep("(param i32)", "local.get 0 i32.const -1 i32.div_s drop");
    keep("(param i32)", "local.get 0 i32.const 0 i32.rem_u drop");
    keep("(param i32)", "local.get 0 i32.load drop");
    keep(
        "(param i32)",
        "local.get 0 i32.const 1 i32.store  local.get 0 i32.const 2 i32.store",
    );
    // A load before a store has to stay before it.
    keep(
        "(param i32 i32) (result i32)",
        "local.get 0 i32.load  local.get 1 i32.const 9 i32.store  local.get 0 i32.load  i32.add",
    );
}

#[test]
fn division_by_a_safe_constant_is_simplified_like_anything_else() {
    check(X, "local.get 0 i32.const 1 i32.div_s", "local.get 0");
    // On their own these are a tie in size, which keeps the original, so each comes with something to remove.
    check(
        X,
        "local.get 0 i32.const 8 i32.div_u i32.const 0 i32.add",
        "local.get 0 i32.const 3 i32.shr_u",
    );
    check(
        X,
        "local.get 0 i32.const 8 i32.rem_u i32.const 0 i32.add",
        "local.get 0 i32.const 7 i32.and",
    );
    check(X, "local.get 0 i32.const 1 i32.rem_s", "i32.const 0");
    check(X, "local.get 0 i32.const -1 i32.rem_s", "i32.const 0");
    check(
        X,
        "local.get 0 i32.const 3 i32.div_s drop local.get 0",
        "local.get 0",
    );
    check(X, "i32.const -7 i32.const 2 i32.rem_s", "i32.const -1");
    check(X, "i32.const -7 i32.const 2 i32.div_s", "i32.const -3");
}

#[test]
fn modules_with_nothing_to_do_come_back_byte_for_byte() {
    for text in [
        "(module)",
        "(module (func))",
        "(module (func (param i32) (result i32) local.get 0))",
        "(module (func (export \"a\") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add))",
    ] {
        let input = wat::parse_str(text).unwrap();
        assert_eq!(optimize(&input).unwrap().wasm, input, "{text}");
    }
}

#[test]
fn everything_outside_the_code_section_is_kept_exactly() {
    let input = wat::parse_str(
        r#"(module $named
            (import "env" "log" (func $log (param i32)))
            (memory (export "memory") 1 2)
            (table 2 funcref)
            (global $g (mut i32) (i32.const 7))
            (elem (i32.const 0) $f $log)
            (data (i32.const 16) "hello")
            (start $s)
            (func $s)
            (func $f (export "f") (param $p i32) (param $q i32) (result i32) (local $unused i32)
                local.get $p local.get $q i32.xor  local.get $p local.get $q i32.and i32.const 2 i32.mul  i32.add))"#,
    )
    .unwrap();
    let sections = |wasm: &[u8]| {
        let mut out = Vec::new();
        for payload in wasmparser::Parser::new(0).parse_all(wasm) {
            let payload = payload.unwrap();
            if let (Some((id, range)), false) = (
                payload.as_section(),
                matches!(payload, wasmparser::Payload::CodeSectionStart { .. }),
            ) {
                out.push((id, wasm[range.start as usize..range.end as usize].to_vec()));
            }
        }
        out
    };
    let out = optimize(&input).unwrap();
    assert_eq!(
        sections(&out.wasm),
        sections(&input),
        "the same sections, in the same order, with the same bytes"
    );
    assert!(out.wasm.len() < input.len());
    // The names in the name section refer to this local, so it has to stay even though it is unused.
    assert_eq!(local_counts(&out.wasm), [0, 1]);
}

#[test]
fn a_custom_section_after_the_code_stays_after_the_code() {
    let mut input =
        wat::parse_str("(module (func (param i32) (result i32) local.get 0 i32.const 0 i32.add))")
            .unwrap();
    // A custom section with a one-byte name and no contents, which belongs after the code.
    input.extend_from_slice(&[0, 2, 1, b'z']);
    let out = optimize(&input).unwrap();
    assert_eq!(&out.wasm[out.wasm.len() - 4..], &[0, 2, 1, b'z']);
    assert_eq!(bodies(&out.wasm)[0].len(), 2);
}

#[test]
fn functions_it_does_not_model_are_left_exactly_as_they_were() {
    for body in [
        // Exception handling, where a write before a call that throws is visible to the handler.
        "(tag) (func (result i32) (local i32)
            block  try_table (catch_all 0)  i32.const 1 local.set 0  throw 0  end  end  i32.const 0 i32.const 0 i32.add)",
        // A tail call, which ends the function like a return does.
        "(func (result i32) i32.const 0 i32.const 0 i32.add return_call 0)",
        // A branch on a reference, which is a branch that the graph knows nothing about.
        "(func (param funcref) (result i32) block local.get 0 br_on_null 0 drop end i32.const 0 i32.const 0 i32.add)",
        // Vector instructions.
        "(func (result i32) i32.const 0 i32.const 0 i32.add v128.const i32x4 0 0 0 0 i32x4.extract_lane 0 i32.add)",
    ] {
        let input = wat::parse_str(format!("(module {body})")).unwrap();
        assert_eq!(optimize(&input).unwrap().wasm, input, "{body}");
    }
}

#[test]
fn floats_and_other_types_pass_through_temporaries_correctly() {
    // The operand under the `i32` here is a float, and taking the `i32` out must not disturb it.
    let input = wat::parse_str(
        "(module
          (func (param i32 f64) (result f64) (local f64)
            local.get 1 f64.const 2 f64.mul  local.get 0 i32.const 0 i32.add  f64.convert_i32_s  f64.add local.set 2
            local.get 2  local.get 2  local.get 0 i32.const 1 i32.or  select))",
    )
    .unwrap();
    optimize(&input).expect("the result has to validate");
}

#[test]
fn the_report_counts_what_happened() {
    let input = wat::parse_str(
        "(module (func (param i32 i32) (result i32)
            local.get 0 local.get 1 i32.xor  local.get 0 local.get 1 i32.and i32.const 2 i32.mul  i32.add
            local.get 0 local.get 0 i32.mul local.get 0 i32.add i32.const 1 i32.and i32.eqz
            if (result i32) i32.const 1 i32.const 2 i32.add else i32.const 3 end
            i32.add))",
    )
    .unwrap();
    let out = optimize(&input).unwrap();
    let r = out.report;
    assert_eq!(r.functions, 1);
    assert_eq!(r.functions_improved, 1);
    assert!(r.mba_simplified >= 1, "{r:?}");
    assert!(r.opaque_predicates >= 1, "{r:?}");
    assert!(r.constants_folded >= 1, "{r:?}");
    assert_eq!(r.branches_removed, 1, "{r:?}");
    assert!(r.instructions_after < r.instructions_before, "{r:?}");
    assert_eq!(r.bytes_before, input.len() as u64);
    assert_eq!(r.bytes_after, out.wasm.len() as u64);
    assert!(r.bytes_after < r.bytes_before);
    assert!(r.passes >= 2, "it runs until a pass finds nothing: {r:?}");
}
