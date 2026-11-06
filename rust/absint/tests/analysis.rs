//! What the analyzer proves about programs in the text format, and what it refuses to.

use absint::analysis::{Verdict, analyze};
use absint::ir::Fault;
use absint::text::parse;

/// The verdict for each fault that was not proved, worst first, as `(line, fault, verdict)`.
fn unproved(source: &str) -> Vec<(u32, Fault, Verdict)> {
    let program = parse(source).unwrap_or_else(|e| panic!("{e}\n{source}"));
    let mut out = Vec::new();
    for function in &program.functions {
        let result = analyze(function);
        out.extend(result.unproved().map(|c| (c.line, c.fault, c.verdict)));
    }
    out.sort_by_key(|(l, f, _)| (*l, *f));
    out
}

fn safe(body: &str) {
    let source = format!("fn f(n, m: 0..10) {{\n{body}\n}}");
    assert_eq!(unproved(&source), [], "\n{source}");
}

/// The first non-proved check has to be this fault, with this verdict.
fn flagged(body: &str, fault: Fault, verdict: Verdict) {
    let source = format!("fn f(n, m: 0..10) {{\n{body}\n}}");
    let found = unproved(&source);
    assert!(
        found.iter().any(|(_, f, v)| *f == fault && *v == verdict),
        "wanted {fault:?} {verdict:?}, got {found:?}\n{source}"
    );
}

const LOOP: &str = "
  buf = alloc 16
  i = 0
head:
  if i < 16 goto body else done
body:
  store buf[i] = i
  i = i + 1
  goto head
done:
  return";

#[test]
fn a_bounded_loop_is_proved_in_bounds() {
    safe(LOOP);
}

#[test]
fn an_off_by_one_loop_is_caught() {
    flagged(
        &LOOP.replace("i < 16", "i <= 16"),
        Fault::OutOfBounds,
        Verdict::Possible,
    );
    flagged(
        &LOOP.replace("alloc 16", "alloc 15"),
        Fault::OutOfBounds,
        Verdict::Possible,
    );
    // This one faults on the very first time around, and the analysis can tell.
    flagged(
        &LOOP.replace("i = 0", "i = -1"),
        Fault::OutOfBounds,
        Verdict::Definite,
    );
    flagged(
        &LOOP
            .replace("i = i + 1", "i = i + 2")
            .replace("i < 16", "i <= 15")
            .replace("alloc 16", "alloc 15"),
        Fault::OutOfBounds,
        Verdict::Possible,
    );
}

#[test]
fn large_loops_converge_by_widening_to_a_constant_in_the_program() {
    safe(&LOOP.replace("16", "100000"));
    // A loop counting down, through a bound that is only a comparison.
    safe(
        "
  buf = alloc 1000
  i = 999
head:
  if i >= 0 goto body else done
body:
  x = load buf[i]
  i = i - 1
  goto head
done:
  return",
    );
    // A nested loop.
    safe(
        "
  buf = alloc 64
  i = 0
outer:
  if i < 8 goto setup else done
setup:
  j = 0
inner:
  if j < 8 goto body else next
body:
  k = i * 8
  k = k + j
  store buf[k] = 1
  j = j + 1
  goto inner
next:
  i = i + 1
  goto outer
done:
  return",
    );
}

#[test]
fn an_unbounded_loop_terminates_with_no_false_proof() {
    flagged(
        "
  buf = alloc 16
  i = 0
head:
  store buf[i] = 0
  i = i + 1
  if i != m goto head else done
done:
  return",
        Fault::OutOfBounds,
        Verdict::Possible,
    );
}

#[test]
fn input_ranges_and_checks_on_inputs_prove_accesses() {
    safe("  buf = alloc 11\n  store buf[m] = 1");
    flagged(
        "  buf = alloc 10\n  store buf[m] = 1",
        Fault::OutOfBounds,
        Verdict::Possible,
    );
    flagged(
        "  buf = alloc 10\n  store buf[n] = 1",
        Fault::OutOfBounds,
        Verdict::Possible,
    );
    safe(
        "
  buf = alloc 10
  if n < 0 goto no else maybe
maybe:
  if n >= 10 goto no else yes
yes:
  store buf[n] = 1
no:
  return",
    );
    safe(
        "  buf = alloc 8\n  i = n & 7\n  assume n >= 0\n  store buf[i] = 1"
            .replace(
                "  i = n & 7\n  assume n >= 0",
                "  assume n >= 0\n  i = n & 7",
            )
            .as_str(),
    );
    safe("  buf = alloc 8\n  assume n >= 0\n  i = n % 8\n  x = load buf[i]");
}

#[test]
fn an_allocation_of_uncertain_size_counts_as_its_smallest() {
    // The allocation might hold only 5 cells, so index 7 is not safe.
    flagged(
        "  buf = alloc m\n  assume m >= 5\n  store buf[7] = 1"
            .replace(
                "  buf = alloc m\n  assume m >= 5",
                "  assume m >= 5\n  buf = alloc m",
            )
            .as_str(),
        Fault::OutOfBounds,
        Verdict::Possible,
    );
    safe("  assume m >= 8\n  buf = alloc m\n  store buf[7] = 1");
}

#[test]
fn definite_faults_are_called_definite() {
    flagged(
        "  buf = alloc 4\n  store buf[4] = 1",
        Fault::OutOfBounds,
        Verdict::Definite,
    );
    flagged(
        "  buf = alloc 4\n  store buf[-1] = 1",
        Fault::OutOfBounds,
        Verdict::Definite,
    );
    flagged(
        "  p = null\n  x = load p[0]",
        Fault::NullDereference,
        Verdict::Definite,
    );
    flagged("  x = 1 / 0", Fault::DivisionByZero, Verdict::Definite);
    flagged("  x = 1 % 0", Fault::DivisionByZero, Verdict::Definite);
    flagged(
        "  x = 3\n  assert x > 5",
        Fault::AssertionFailed,
        Verdict::Definite,
    );
    flagged(
        "  x = 3\n  y = load x[0]",
        Fault::NotAPointer,
        Verdict::Definite,
    );
    flagged(
        "  p = alloc 1\n  y = p + 1",
        Fault::NotAnInteger,
        Verdict::Definite,
    );
    flagged(
        "  p = alloc -1",
        Fault::NegativeAllocation,
        Verdict::Definite,
    );
}

#[test]
fn nullness_follows_the_checks_a_program_makes() {
    flagged(
        "  p = maybe_alloc 4\n  x = load p[0]",
        Fault::NullDereference,
        Verdict::Possible,
    );
    safe(
        "  p = maybe_alloc 4\n  if p == 0 goto out else use\nuse:\n  x = load p[0]\nout:\n  return",
    );
    safe(
        "  p = maybe_alloc 4\n  if p != 0 goto use else out\nuse:\n  x = load p[0]\nout:\n  return",
    );
    flagged(
        "  p = maybe_alloc 4\n  if p != 0 goto out else use\nuse:\n  x = load p[0]\nout:\n  return",
        Fault::NullDereference,
        Verdict::Definite,
    );
    // Having once worked, an access shows the pointer is not null.
    let source = "fn f() {\n  p = maybe_alloc 4\n  x = load p[0]\n  y = load p[1]\n}";
    assert_eq!(
        unproved(source),
        [(3, Fault::NullDereference, Verdict::Possible)]
    );
}

#[test]
fn uninitialized_variables_are_found_on_any_path() {
    flagged("  x = y + 1", Fault::Uninitialized, Verdict::Possible);
    flagged(
        "  if m > 5 goto a else b\na:\n  x = 1\nb:\n  y = x + 1",
        Fault::Uninitialized,
        Verdict::Possible,
    );
    safe("  if m > 5 goto a else b\na:\n  x = 1\n  goto c\nb:\n  x = 2\nc:\n  y = x + 1");
}

#[test]
fn divisors_are_checked_through_their_ranges() {
    flagged("  x = 10 / m", Fault::DivisionByZero, Verdict::Possible);
    safe("  d = m + 1\n  x = 10 / d");
    safe("  if m == 0 goto out else go\ngo:\n  x = 10 / m\nout:\n  return");
}

#[test]
fn assertions_are_proved_when_they_hold_and_narrow_when_they_might_not() {
    safe("  x = m * 2\n  assert x <= 20");
    flagged(
        "  x = m * 2\n  assert x < 20",
        Fault::AssertionFailed,
        Verdict::Possible,
    );
    // After a possibly failing assertion, later code gets to rely on it.
    let source = "fn f(m: 0..10) {\n  buf = alloc 5\n  assert m < 5\n  x = load buf[m]\n}";
    assert_eq!(
        unproved(source),
        [(3, Fault::AssertionFailed, Verdict::Possible)]
    );
}

#[test]
fn unreachable_code_is_proved_vacuously_and_marked() {
    let source =
        "fn f(m: 0..10) {\n  if m > 100 goto bad else ok\nbad:\n  x = 1 / 0\nok:\n  return\n}";
    let program = parse(source).unwrap();
    let result = analyze(&program.functions[0]);
    let check = result
        .checks
        .iter()
        .find(|c| c.fault == Fault::DivisionByZero)
        .unwrap();
    assert_eq!(check.verdict, Verdict::Proved);
    assert!(check.dead);
}

#[test]
fn a_branch_on_a_pointer_order_is_flagged() {
    flagged(
        "  p = alloc 1\n  if p < 3 goto a else a\na:\n  return",
        Fault::NotAnInteger,
        Verdict::Definite,
    );
}

#[test]
fn parse_errors_name_the_line() {
    for (source, line, fragment) in [
        ("fn f() {\n  x = \n}", 2, "cannot read"),
        ("fn f() {\n  goto nowhere\n}", 2, "no label"),
        ("fn f() {\nl:\nl:\n}", 3, "twice"),
        ("fn f() {\n  x = 1", 2, "no closing brace"),
        ("nonsense", 1, "expected a declaration"),
        ("fn f() {\n  write g = 1\n}", 2, "no global"),
        ("fn f() {\n  lock m\n}", 2, "no lock"),
        ("fn f(a: 5..1) {\n}", 1, "not a range"),
        (
            "fn f() {\n  if 1 ~ 2 goto a else b\n}",
            2,
            "not a comparison",
        ),
    ] {
        let e = parse(source).unwrap_err();
        assert_eq!(e.line, line, "{source}: {e}");
        assert!(e.to_string().contains(fragment), "{source}: {e}");
    }
}
