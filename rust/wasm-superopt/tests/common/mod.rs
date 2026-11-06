//! Helpers shared by the test binaries, not all of which use all of them.
#![allow(dead_code)]

use wasmparser::{Parser, Payload};

/// A small deterministic generator, so that a failure can be reproduced from its seed.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        (self.next() >> 16) % n
    }

    pub fn chance(&mut self, one_in: u64) -> bool {
        self.below(one_in) == 0
    }

    pub fn pick<'a, T>(&mut self, list: &'a [T]) -> &'a T {
        &list[self.below(list.len() as u64) as usize]
    }
}

/// The operators of every function body in a module, in a form that can be compared.
pub fn bodies(wasm: &[u8]) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for payload in Parser::new(0).parse_all(wasm) {
        if let Payload::CodeSectionEntry(body) = payload.unwrap() {
            let mut reader = body.get_operators_reader().unwrap();
            let mut ops = Vec::new();
            while !reader.eof() {
                ops.push(format!("{:?}", reader.read().unwrap()));
            }
            out.push(ops);
        }
    }
    out
}

/// The non-parameter locals of every function, as declared.
pub fn local_counts(wasm: &[u8]) -> Vec<u32> {
    let mut out = Vec::new();
    for payload in Parser::new(0).parse_all(wasm) {
        if let Payload::CodeSectionEntry(body) = payload.unwrap() {
            let mut reader = body.get_locals_reader().unwrap();
            let mut total = 0;
            for _ in 0..reader.get_count() {
                total += reader.read().unwrap().0;
            }
            out.push(total);
        }
    }
    out
}
