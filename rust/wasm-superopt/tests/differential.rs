//! Random modules have to behave the same before and after optimization, on a real engine.
//!
//! Each module runs in a fresh instance on the same inputs, and everything observable is compared: the
//! result or the exact trap, every call to the host in order, all of memory, and the globals. That also
//! covers a run that traps partway, where what had already happened still has to match.

mod common;

use common::Rng;
use wasm_superopt::optimize;
use wasmtime::{Caller, Engine, Instance, Linker, Module, Store, Trap};

/// Generates valid functions in the text format, by keeping track of the types as it goes.
struct Generator {
    rng: Rng,
    code: String,
    /// Whether each enclosing label is a block, which is safe to branch to. A loop is left out,
    /// because a branch to it could run forever.
    labels: Vec<bool>,
    loops: u32,
    /// How many earlier helper functions this one can call, which rules out recursion.
    helpers: u32,
    statements_left: u32,
}

const I32_LOCALS: [u32; 4] = [0, 1, 3, 4];
const I64_LOCALS: [u32; 2] = [2, 5];
const F64_LOCAL: u32 = 6;
const NEVER_WRITTEN_LOCAL: u32 = 7;
const FIRST_COUNTER: u32 = 8;

impl Generator {
    fn emit(&mut self, text: &str) {
        self.code.push(' ');
        self.code.push_str(text);
    }

    fn i32_const(&mut self) -> String {
        let value = match self.rng.below(8) {
            0 => 0,
            1 => 1,
            2 => -1,
            3 => i32::MIN,
            4 => i32::MAX,
            5 => 2,
            _ => self.rng.next() as i32 >> self.rng.below(32),
        };
        format!("i32.const {value}")
    }

    fn i64_const(&mut self) -> String {
        let value = match self.rng.below(6) {
            0 => 0,
            1 => 1,
            2 => -1,
            3 => i64::MIN,
            _ => self.rng.next() as i64 >> self.rng.below(64),
        };
        format!("i64.const {value}")
    }

    /// An address that is usually in bounds, and sometimes not, so that some runs trap.
    fn address(&mut self, depth: u32) {
        self.i32_expr(depth);
        if !self.rng.chance(24) {
            self.emit("i32.const 2047 i32.and");
        }
    }

    fn i32_expr(&mut self, depth: u32) {
        if depth == 0 || self.rng.chance(8) {
            match self.rng.below(6) {
                0 | 1 => {
                    let text = self.i32_const();
                    self.emit(&text);
                }
                2 => {
                    let local = NEVER_WRITTEN_LOCAL;
                    self.emit(&format!("local.get {local}"));
                }
                3 => self.emit("global.get $g"),
                _ => {
                    let local = *self.rng.pick(&I32_LOCALS);
                    self.emit(&format!("local.get {local}"));
                }
            }
            return;
        }
        let d = depth - 1;
        match self.rng.below(30) {
            0..=7 => {
                self.i32_expr(d);
                self.i32_expr(d);
                let op = *self.rng.pick(&[
                    "add", "sub", "mul", "and", "or", "xor", "shl", "shr_s", "shr_u", "rotl",
                    "rotr",
                ]);
                self.emit(&format!("i32.{op}"));
            }
            8 => {
                self.i32_expr(d);
                let op =
                    *self
                        .rng
                        .pick(&["clz", "ctz", "popcnt", "extend8_s", "extend16_s", "eqz"]);
                self.emit(&format!("i32.{op}"));
            }
            9 | 10 => {
                self.i32_expr(d);
                self.i32_expr(d);
                let op = *self.rng.pick(&[
                    "eq", "ne", "lt_s", "lt_u", "gt_s", "gt_u", "le_s", "le_u", "ge_s", "ge_u",
                ]);
                self.emit(&format!("i32.{op}"));
            }
            11 => {
                self.i64_expr(d);
                self.i64_expr(d);
                let op = *self.rng.pick(&[
                    "eq", "ne", "lt_s", "lt_u", "gt_s", "gt_u", "le_s", "le_u", "ge_s", "ge_u",
                ]);
                self.emit(&format!("i64.{op}"));
            }
            12 => {
                self.i64_expr(d);
                let op = *self.rng.pick(&["i32.wrap_i64", "i64.eqz"]);
                self.emit(op);
            }
            13 => {
                self.i32_expr(d);
                self.i32_expr(d);
                self.i32_expr(d);
                self.emit("select");
            }
            14 => {
                self.i32_expr(d);
                let local = *self.rng.pick(&[3, 4]);
                self.emit(&format!("local.tee {local}"));
            }
            15 | 16 => {
                self.address(d);
                let op = *self
                    .rng
                    .pick(&["load", "load8_s", "load8_u", "load16_s", "load16_u"]);
                self.emit(&format!("i32.{op}"));
            }
            17 => {
                self.i32_expr(d);
                self.emit("call $observe");
            }
            18 => {
                // Division, with a divisor that may be zero or -1, so it may trap.
                self.i32_expr(d);
                if self.rng.chance(2) {
                    self.i32_expr(d);
                } else {
                    let text = self.i32_const();
                    self.emit(&text);
                }
                let op = *self.rng.pick(&["div_s", "div_u", "rem_s", "rem_u"]);
                self.emit(&format!("i32.{op}"));
            }
            19 => {
                self.i32_expr(d);
                self.emit("if (result i32)");
                self.i32_expr(d);
                self.emit("else");
                self.i32_expr(d);
                self.emit("end");
            }
            20 => {
                // A value that is either carried out by a branch or replaced.
                self.emit("block (result i32)");
                self.i32_expr(d);
                self.i32_expr(d);
                self.emit("br_if 0 drop");
                self.i32_expr(d);
                self.emit("end");
            }
            21 => {
                self.f64_expr(d);
                let op = *self.rng.pick(&[
                    "i32.trunc_sat_f64_s",
                    "i32.trunc_sat_f64_u",
                    "f64.ceil i32.trunc_sat_f64_s",
                ]);
                self.emit(op);
            }
            22 => {
                self.f64_expr(d);
                self.emit("i32.trunc_f64_s");
            }
            23 if self.helpers > 0 => {
                let helper = self.rng.below(u64::from(self.helpers));
                self.i32_expr(d);
                self.i32_expr(d);
                self.i64_expr(d);
                self.emit(&format!("call $h{helper}"));
            }
            // What an obfuscator would write in place of an addition.
            24 => {
                let (x, y) = (*self.rng.pick(&I32_LOCALS), *self.rng.pick(&I32_LOCALS));
                self.emit(&format!(
                    "local.get {x} local.get {y} i32.xor  local.get {x} local.get {y} i32.and i32.const 2 i32.mul  i32.add"
                ));
            }
            25 => {
                let (x, y) = (*self.rng.pick(&I32_LOCALS), *self.rng.pick(&I32_LOCALS));
                self.emit(&format!("local.get {x} local.get {y} i32.or  local.get {x} local.get {y} i32.and  i32.add"));
            }
            // A predicate that is always true, with a trap down the path never taken.
            26 => {
                let x = *self.rng.pick(&I32_LOCALS);
                self.emit(&format!(
                    "local.get {x} local.get {x} i32.mul local.get {x} i32.add i32.const 1 i32.and i32.eqz if (result i32)"
                ));
                self.i32_expr(d);
                self.emit("else unreachable end");
            }
            // One that is never true.
            27 => {
                let (x, y) = (*self.rng.pick(&I32_LOCALS), *self.rng.pick(&I32_LOCALS));
                self.emit(&format!(
                    "i32.const 7 local.get {y} local.get {y} i32.mul i32.mul i32.const 1 i32.sub local.get {x} local.get {x} i32.mul i32.eq if (result i32) unreachable else"
                ));
                self.i32_expr(d);
                self.emit("end");
            }
            // One that looks like the others, but really does depend on the input.
            28 => {
                let x = *self.rng.pick(&I32_LOCALS);
                self.emit(&format!("local.get {x} local.get {x} i32.mul i32.const 1 i32.and i32.eqz if (result i32)"));
                self.i32_expr(d);
                self.emit("else");
                self.i32_expr(d);
                self.emit("end");
            }
            _ => self.i32_expr(0),
        }
    }

    fn i64_expr(&mut self, depth: u32) {
        if depth == 0 || self.rng.chance(6) {
            if self.rng.chance(2) {
                let text = self.i64_const();
                self.emit(&text);
            } else if self.rng.chance(4) {
                self.emit("global.get $w");
            } else {
                let local = *self.rng.pick(&I64_LOCALS);
                self.emit(&format!("local.get {local}"));
            }
            return;
        }
        let d = depth - 1;
        match self.rng.below(10) {
            0..=3 => {
                self.i64_expr(d);
                self.i64_expr(d);
                let op = *self.rng.pick(&[
                    "add", "sub", "mul", "and", "or", "xor", "shl", "shr_s", "shr_u", "rotl",
                    "rotr",
                ]);
                self.emit(&format!("i64.{op}"));
            }
            4 => {
                self.i32_expr(d);
                let op = *self.rng.pick(&["i64.extend_i32_s", "i64.extend_i32_u"]);
                self.emit(op);
            }
            5 => {
                self.i64_expr(d);
                let op = *self.rng.pick(&[
                    "clz",
                    "ctz",
                    "popcnt",
                    "extend8_s",
                    "extend16_s",
                    "extend32_s",
                ]);
                self.emit(&format!("i64.{op}"));
            }
            6 => {
                self.i64_expr(d);
                self.i64_expr(d);
                let op = *self.rng.pick(&["div_s", "div_u", "rem_s", "rem_u"]);
                self.emit(&format!("i64.{op}"));
            }
            7 => {
                self.address(d);
                let op = *self
                    .rng
                    .pick(&["load", "load8_s", "load16_u", "load32_s", "load32_u"]);
                self.emit(&format!("i64.{op}"));
            }
            8 => {
                self.i64_expr(d);
                self.i64_expr(d);
                self.i32_expr(d);
                self.emit("select");
            }
            _ => {
                self.i64_expr(d);
                self.emit("local.tee 5");
            }
        }
    }

    fn f64_expr(&mut self, depth: u32) {
        if depth == 0 || self.rng.chance(3) {
            if self.rng.chance(2) {
                self.emit(&format!("local.get {F64_LOCAL}"));
            } else {
                let value = self.rng.below(2000) as f64 / 8.0 - 100.0;
                self.emit(&format!("f64.const {value:?}"));
            }
            return;
        }
        let d = depth - 1;
        match self.rng.below(4) {
            0 => {
                self.f64_expr(d);
                self.f64_expr(d);
                let op = *self.rng.pick(&["add", "sub", "mul", "min", "max"]);
                self.emit(&format!("f64.{op}"));
            }
            1 => {
                self.i32_expr(d);
                self.emit("f64.convert_i32_s");
            }
            // A float under an integer, which the translator has to spill with the right type.
            2 => {
                self.f64_expr(d);
                self.f64_expr(d);
                self.i32_expr(d);
                self.emit("select");
            }
            _ => {
                self.f64_expr(d);
                self.emit(&format!("local.tee {F64_LOCAL}"));
            }
        }
    }

    fn statement(&mut self, depth: u32) {
        if self.statements_left == 0 {
            return;
        }
        self.statements_left -= 1;
        let d = depth.saturating_sub(1);
        match self.rng.below(22) {
            0..=3 => {
                self.i32_expr(3);
                let local = *self.rng.pick(&I32_LOCALS);
                self.emit(&format!("local.set {local}"));
            }
            4 => {
                self.i64_expr(3);
                let local = *self.rng.pick(&I64_LOCALS);
                self.emit(&format!("local.set {local}"));
            }
            5 => {
                self.f64_expr(2);
                self.emit(&format!("local.set {F64_LOCAL}"));
            }
            6 => {
                self.i32_expr(3);
                self.emit("global.set $g");
            }
            7 => {
                self.i64_expr(2);
                self.emit("global.set $w");
            }
            8 | 9 => {
                self.address(2);
                self.i32_expr(3);
                let op = *self.rng.pick(&["store", "store8", "store16"]);
                self.emit(&format!("i32.{op}"));
            }
            10 => {
                self.address(2);
                self.i64_expr(2);
                let op = *self.rng.pick(&["store", "store8", "store16", "store32"]);
                self.emit(&format!("i64.{op}"));
            }
            11 => {
                self.i32_expr(3);
                self.emit("call $log");
            }
            12 => {
                self.i32_expr(3);
                self.emit("drop");
            }
            13 | 14 if depth > 0 => {
                self.i32_expr(2);
                self.emit("if");
                self.labels.push(true);
                self.statements(d);
                if self.rng.chance(2) {
                    self.emit("else");
                    self.statements(d);
                }
                self.labels.pop();
                self.emit("end");
            }
            15 | 16 if depth > 0 => {
                self.emit("block");
                self.labels.push(true);
                self.statements(d);
                self.labels.pop();
                self.emit("end");
            }
            17 if depth > 0 && self.loops < 3 => {
                // A loop counted by a local that nothing else writes, so that it always ends.
                let counter = FIRST_COUNTER + self.loops;
                let times = 1 + self.rng.below(4);
                self.emit(&format!("i32.const {times} local.set {counter} loop"));
                self.loops += 1;
                self.labels.push(false);
                self.statements(d);
                self.labels.pop();
                self.loops -= 1;
                self.emit(&format!(
                    "local.get {counter} i32.const 1 i32.sub local.tee {counter} br_if 0 end"
                ));
            }
            18 => {
                if let Some(target) = self.block_label() {
                    self.i32_expr(2);
                    self.emit(&format!("br_if {target}"));
                }
            }
            19 => {
                // A table, which has all its targets at statement level, so all with no values.
                if let (Some(a), Some(b)) = (self.block_label(), self.block_label()) {
                    self.i32_expr(2);
                    self.emit(&format!("br_table {a} {b} {a}"));
                }
            }
            20 => {
                if self.rng.chance(3) {
                    self.i32_expr(2);
                    self.emit("return");
                } else if self.rng.chance(4) {
                    self.emit("unreachable");
                } else if let Some(target) = self.block_label() {
                    self.emit(&format!("br {target}"));
                }
            }
            _ => self.emit("nop"),
        }
    }

    fn block_label(&mut self) -> Option<usize> {
        let blocks: Vec<usize> = self
            .labels
            .iter()
            .rev()
            .enumerate()
            .filter(|(_, is_block)| **is_block)
            .map(|(depth, _)| depth)
            .collect();
        if blocks.is_empty() {
            None
        } else {
            Some(*self.rng.pick(&blocks))
        }
    }

    fn statements(&mut self, depth: u32) {
        for _ in 0..1 + self.rng.below(4) {
            self.statement(depth);
        }
    }

    fn function(&mut self, name: &str, helpers: u32) -> String {
        self.code.clear();
        self.helpers = helpers;
        self.loops = 0;
        self.labels.clear();
        self.statements_left = 30;
        self.statements(3);
        self.i32_expr(4);
        format!(
            "(func {name} (param i32 i32 i64) (result i32) (local i32 i32 i64 f64 i32 i32 i32 i32) {})",
            self.code
        )
    }

    fn module(&mut self) -> String {
        let mut text = String::from(
            r#"(module
              (import "env" "log" (func $log (param i32)))
              (memory (export "memory") 1)
              (data (i32.const 16) "the quick brown fox")
              (global $g (export "g") (mut i32) (i32.const 7))
              (global $w (export "w") (mut i64) (i64.const -3))
              (func $observe (param i32) (result i32) local.get 0 call $log local.get 0)
            "#,
        );
        let helpers = self.rng.below(3) as u32;
        for i in 0..helpers {
            let f = self.function(&format!("$h{i}"), i);
            text.push_str(&f);
        }
        let f = self.function("$main (export \"main\")", helpers);
        text.push_str(&f);
        text.push(')');
        text
    }
}

#[derive(Debug, PartialEq)]
struct Observed {
    result: Result<i32, Trap>,
    log: Vec<i32>,
    memory_hash: u64,
    g: i32,
    w: i64,
}

fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xCBF2_9CE4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01B3)
    })
}

fn run(engine: &Engine, module: &Module, inputs: (i32, i32, i64)) -> Observed {
    let mut linker: Linker<Vec<i32>> = Linker::new(engine);
    linker
        .func_wrap(
            "env",
            "log",
            |mut caller: Caller<'_, Vec<i32>>, value: i32| caller.data_mut().push(value),
        )
        .unwrap();
    let mut store = Store::new(engine, Vec::new());
    let instance: Instance = linker.instantiate(&mut store, module).unwrap();
    let main = instance
        .get_typed_func::<(i32, i32, i64), i32>(&mut store, "main")
        .unwrap();
    let result = main.call(&mut store, inputs).map_err(|e| {
        *e.downcast_ref::<Trap>()
            .unwrap_or_else(|| panic!("not a trap: {e:?}"))
    });
    let memory = instance.get_memory(&mut store, "memory").unwrap();
    let memory_hash = fingerprint(memory.data(&store));
    let g = instance
        .get_global(&mut store, "g")
        .unwrap()
        .get(&mut store)
        .unwrap_i32();
    let w = instance
        .get_global(&mut store, "w")
        .unwrap()
        .get(&mut store)
        .unwrap_i64();
    Observed {
        result,
        log: store.into_data(),
        memory_hash,
        g,
        w,
    }
}

fn differential(seed: u64, modules: u32) -> (u32, u32, u32) {
    let engine = Engine::default();
    let mut generator = Generator {
        rng: Rng(seed),
        code: String::new(),
        labels: Vec::new(),
        loops: 0,
        helpers: 0,
        statements_left: 0,
    };
    let (mut smaller, mut trapped, mut finished) = (0, 0, 0);
    for index in 0..modules {
        let text = generator.module();
        let input = wat::parse_str(&text)
            .unwrap_or_else(|e| panic!("the generator made invalid text: {e}\n{text}"));
        let out = optimize(&input)
            .unwrap_or_else(|e| panic!("seed {seed:#x} module {index}: {e}\n{text}"));
        assert!(out.wasm.len() <= input.len(), "module {index} got larger");
        if out.wasm.len() < input.len() {
            smaller += 1;
        }
        let before = Module::new(&engine, &input).unwrap();
        let after = Module::new(&engine, &out.wasm).unwrap();
        for _ in 0..6 {
            let inputs = (
                *generator
                    .rng
                    .pick(&[0, 1, -1, 2, i32::MIN, generator.rng.0 as i32]),
                generator.rng.next() as i32 >> generator.rng.below(32),
                generator.rng.next() as i64 >> generator.rng.below(64),
            );
            let a = run(&engine, &before, inputs);
            let b = run(&engine, &after, inputs);
            assert_eq!(
                a, b,
                "seed {seed:#x} module {index} differs on {inputs:?}\n{text}"
            );
            if a.result.is_ok() {
                finished += 1;
            } else {
                trapped += 1;
            }
        }
    }
    (smaller, trapped, finished)
}

#[test]
fn optimized_modules_behave_identically() {
    let modules = 1_500;
    let (smaller, trapped, finished) = differential(0x5DEE_CE66_D1CE_4E5B, modules);
    println!(
        "{smaller} of {modules} got smaller, with {finished} runs finishing and {trapped} trapping"
    );
    // If these fail, the test has stopped testing what it claims to.
    assert!(smaller > modules / 2, "only {smaller} modules got smaller");
    assert!(trapped > 100, "only {trapped} runs trapped");
    assert!(finished > 100, "only {finished} runs finished");
}

#[test]
fn a_second_run_finds_nothing_more() {
    let mut generator = Generator {
        rng: Rng(0xABCD_EF01_2345_6789),
        code: String::new(),
        labels: Vec::new(),
        loops: 0,
        helpers: 0,
        statements_left: 0,
    };
    for _ in 0..200 {
        let text = generator.module();
        let input = wat::parse_str(&text).unwrap();
        let once = optimize(&input).unwrap();
        let twice = optimize(&once.wasm).unwrap();
        // It stops at the pass limit if there is still progress to make, so it can be smaller again,
        // but it must not be different in any way other than that.
        assert!(twice.wasm.len() <= once.wasm.len());
        if once.report.passes < 8 {
            assert_eq!(
                twice.wasm, once.wasm,
                "it stopped early, so another run should change nothing\n{text}"
            );
        }
    }
}
