# Changelog

Versions follow this rule: a bug or security fix increments the last number, and new functionality or new
platform support increments the middle one.

## 1.1.1

- Fixed the release workflow, which stopped before publishing the Windows builds of `wasm-superopt` and `absint`, because it used a checksum tool Windows runners do not have.

## 1.1.0

Adds two Rust tools, with prebuilt binaries for Linux, macOS and Windows. Each binary has a SHA-256 checksum
and a build provenance attestation.

- `wasm-superopt` simplifies WebAssembly modules. It lifts stack code into an SSA graph, removes Mixed
  Boolean-Arithmetic and opaque predicates using rules that are proofs, removes dead code and shuffling,
  and lowers the result back. The module's behavior does not change, and it never gets larger.
- `absint` is an abstract interpretation analyzer. It proves the absence of out of bounds accesses, null
  dereferences, division by zero and uninitialized reads, proves the absence of data races through locksets,
  proves WebAssembly memory accesses in bounds, and proves that x86-64 functions cannot overwrite their return
  addresses through their own stack pointers.

Also fixes the main CI workflow, which GitHub had been rejecting as invalid because it used `hashFiles` in
a job-level `if`, and adds a repository guard against supply chain tampering.

## 1.0.0

SMT devirtualization and LLVM IR recovery suite.
