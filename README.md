# 64-Bit Binary Devirtualization and Deobfuscation Research Framework

An integrated research suite for the analysis, dynamic unpacking, symbolic semantic extraction, and compiler-driven devirtualization of 64-bit protected binaries (targeting **VMProtect 3.x**, **Themida 3.x**, and **Tigress**).

---

## Devirtualization Framework Comparison

| Analysis Dimension / Capability | VMP_DEVIRT | Standard IDA / Ghidra | NoVmp / ScyllaHide | D810 Deobfuscator |
| :--- | :---: | :---: | :---: | :---: |
| **VMProtect 3.x Devirtualization** | Complete Automated | Manual Scripting | Partial Hooking | Rule-based only |
| **Themida 3.x CISC/RISC VM Lifting** | Automated Handler Lift | Unsupported | Memory Dump only | Unsupported |
| **SMT Symbolic Execution (Z3 v4.12)** | Integrated Semantic Slice | Plugin required | NO | Partial |
| **Opaque Predicate Elimination** | Formal Equivalence Proof | Heuristic only | NO | Pattern AST match |
| **Pure LLVM IR Emission** | YES (Clean SSA IR) | NO (Decompiler C only) | NO | Microcode only |
| **Re-compilation to Clean Binary** | YES (`driver_clean.bin`) | NO | NO | NO |
| **Target Architecture** | x86_64 Native | Multi-arch | x86 / x86_64 | Architecture agnostic |

## Theoretical & Methodological Foundations

This framework synthesizes foundational breakthroughs across state-of-the-art reverse engineering and binary analysis research:

1. **Active Oracle Knowledge Extraction (Chosen-Instruction Attack - CIA, NDSS 2022):**
   - Formalizes non-virtualized **anchor instructions** ($\alpha \in \Sigma_{\text{native}} \setminus \text{Virtualizable}(\mathcal{O})$) such as `cmpxchg eax, eax`, `movaps xmm0, xmm0`, and atomic bus operations to isolate virtualization windows.
   - Deploys sandwiched knowledge-leaking harnesses ($P_c$) and **Guided Simplification** (backward dynamic slicing + Z3 SMT equivalence proofs) to extract pure handler semantics.
   - Comprehensive documentation available in [`CIA.md`](file:///Users/sushikim/Documents/VMP_DEVIRT/CIA.md).

2. **Commercial 64-Bit Anti-Analysis Deconstruction (x64Unpack, IEEE Access 2020):**
   - Implements a **hybrid emulation architecture** sharing virtual address space (heap, data, DLLs) between the unpacker and target, alternating instruction-level CPU emulation (Bochs core) for unpacking code with native execution for OS APIs.
   - Completely models the **13-stage execution lifecycle of VMProtect 3.4 (x86-64)**:
     - OS build identification via `GS:[60h]` (PEB) + `0x120` (`0x47ba` for Win10 1809).
     - Disk vs. memory integrity checking (`ZwOpenFile`/`ZwMapViewOfSection`).
     - Stealth heap tracking via Fiber Local Storage (`FlsAlloc`/`FlsSetValue`/`FlsGetValue`).
     - Inline integrity trampolines at `ZwProtectVirtualMemory` checking `.vmp0`/`.data` checksums.
     - Stripped IAT deobfuscation: algebraic reduction of `.vmp0` call thunks via constant propagation, dead code elimination, and peephole rewriting.

3. **Trace-Free VPC-Sensitive Symbolic Emulation & Algebraic Pre-Reduction (AETHER-64):**
   - Overcomes the fundamental path-coverage and NP-hard constraint-solving limits of Dynamic Symbolic Execution (DSE).
   - Solves the cryptographic preimage trap (e.g., hash checks like `hash == 226292709949525` timing out SMT solvers) by **decoupling CFG recovery from path feasibility**.
   - Indexes blocks via two-dimensional coordinates: $\langle \text{Native PC}, \text{Virtual PC (VPC)} \rangle$.
   - Evaluates branch targets without accumulating path constraints; bounds state growth by visiting each $( \text{PC}, \text{VPC} )$ block at most once.
   - Implements Fast Moebius Inversion over $\mathbb{Z}/2^{64}$ and four-tier semantics-preserving simplifications (S1–S4) with robust virtual stack pointer tracking.

4. **Symbolic Synthesis & Concolic Execution (Triton / Syntia / ProMBA):**
   - Utilizes the **Triton concolic framework** to track symbolic bit-vector formulas for CPU registers and memory.
   - Eliminates rolling decryption keys, VM dispatcher arithmetic, and junk instructions.
   - Solves non-linear Mixed Boolean-Arithmetic (MBA) expressions via stochastic/MCMC program synthesis.

5. **LLVM-IR Lifting & Compiler-Grade Devirtualization (`naive_rev`):**
   - Reconstructs the Virtual Control Flow Graph (VCFG) and lifts Triton ASTs into strongly typed LLVM IR.
   - Eliminates the virtual stack abstraction using LLVM's SSA register promotion pass (`-mem2reg`).
   - Optimizes away VM interpreter artifacts via `-instcombine`, `-gvn`, `-dce`, and `-simplifycfg`, emitting clean, human-readable, and machine-recompilable native 64-bit binaries.

---

## Repository Directory Structure

```text
VMP_DEVIRT/
├── aether/                  # Core AETHER-64 devirtualization & deobfuscation engine
│   ├── apr/filter.py        # O(1) Algebraic Pre-Reduction filter (Z/2^64 Fast Moebius Inversion)
│   ├── mba/linear.py        # Truth-table signature extraction & linear MBA canonicalizer
│   ├── mba/synthesizer.py   # MCMC & I/O sampling synthesizer for high-degree polynomial MBAs
│   ├── devirt/vpc_tracer.py # VPC-sensitive constraint-free symbolic tracer & CFG extractor
│   ├── devirt/stack_elim.py # Virtual stack elimination into SSA virtual registers (%v1, %v2)
│   └── pipeline.py          # Unified 6-stage end-to-end devirtualization pipeline
├── docs/                    # Academic paper sources, vector figures, and submission artifacts
│   ├── aether64_paper.pdf   # Compiled publication-ready vector PDF (IEEE format)
│   ├── aether64_paper.typ   # Typst 0.15.1 source code
│   ├── aether64_paper.tex   # Standard IEEEtran LaTeX source code
│   ├── cover_letter.md/.txt # Journal editorial office submission cover letter
│   ├── fig1_architecture.*  # End-to-end toolchain sequence architecture diagrams (PDF/SVG)
│   └── fig2_benchmark_charts.* # Empirical latency and SMT bypass benchmark charts (PDF/SVG)
├── tests/                   # Empirical benchmark suites & reproduction harnesses
│   ├── test_diffing_dataset.py # Quarkslab Diffing Obfuscation Dataset evaluation (104 functions)
│   ├── diffing_dataset_benchmark.json # Ground-truth benchmark metrics & handler reductions
│   ├── test_real_deobfuscator.py # Non-linear MBA synthesis & Tigress CFG extraction tests
│   ├── evaluate_deobfuscation.py # SMT equivalence & VPC emulation vs DSE evaluation
│   └── real_benchmark_results.json # Verified execution timings and reduction ratios
├── research/                # Mathematical prototypes and formal algebraic derivations
├── rust/                    # Standalone tools: wasm-superopt (Wasm deobfuscator) and absint (static analyzer)
├── CIA.md                   # Chosen-Instruction Attack guide & Rolf Rolles VMP monograph
├── .gitmodules              # Academic open-source submodules registry
└── .gitignore               # Build, binary, and OS artifact filter
```

---

## Submodules & Academic Toolchains

The repository links the following external open-source engines via Git submodules:

| Submodule | Description & Source Repository |
| :--- | :--- |
| `CIA_toolchain` | Automated CIA sample generation, PIN tracer, and Z3 simplification engine |
| `CIA-dataset` | Benchmark dataset of 5,745 virtualized binaries across commercial obfuscators |
| `Triton` | Dynamic binary analysis and concolic execution framework |
| `ProMBA` | Mixed Boolean-Arithmetic (MBA) simplification via program synthesis and term rewriting |
| `syntia` | USENIX Security '17 semantics synthesizing framework for obfuscated code |
| `VMAttack` | Automatic deobfuscation and code-stripping for virtualization packers |
| `ollvm-unflattener` | Control-flow flattening deobfuscator for OLLVM / Tigress |
| `Dna` | Binary analysis platform for automated reverse engineering |

To initialize and fetch all submodules:

```bash
git submodule update --init --recursive
```

---

## Rust Tools

The `rust/` workspace holds two standalone tools. They have no Python or Z3 dependency, and prebuilt binaries are attached to each [release](https://github.com/Aetherx86/VMP_DEVIRT/releases/latest).

```bash
cd rust
cargo build --release
./target/release/wasm-superopt obfuscated.wasm -o clean.wasm
./target/release/absint module.wasm
./target/release/absint --x86-64 --base 0x401000 function.bin
```

### wasm-superopt

Applies the same algebraic approach as AETHER-64 to WebAssembly, in the form of a deobfuscator. Each function is lifted from stack code into a hash-consed SSA graph, simplified, and lowered back.

| Technique | Example input | Output | Basis |
| --- | --- | --- | --- |
| Linear MBA | `(x ^ y) + 2 * (x & y)` | `x + y` | Signature over all-zeros and all-ones inputs, which decides linear MBA exactly |
| Opaque predicate, parity | `(x * x + x) & 1 == 0` | always true | Arithmetic over GF(2) on the lowest bit |
| Opaque predicate, modular | `7 * y * y - 1 == x * x` | always false | The low bits of a T-function depend only on the low bits of its inputs |
| Opaque predicate, range | `(x & 7) > 7` | always false | Unsigned range analysis |
| Shuffling | `local.set`, `local.get` through temporaries | direct use | Deferred writes, emitted as a parallel assignment |
| Dead code | branches on constants, code after a `return` | removed | Constants tracked through locals and into blocks |

Guarantees, whatever the input: everything outside the code section stays byte for byte, a function is replaced only if its encoding gets smaller, and the result is validated before it is returned. Trapping operations, memory access and calls are never removed or reordered.

How that is checked: 1,500 random modules with traps, memory, globals, host calls and floats run on Wasmtime before and after, comparing the result or exact trap, each host call, all of memory, and the globals. In addition, 40,000 random expressions are compared against plain evaluation, 30,000 damaged modules must never cause a panic or invalid output, and 49 deliberately injected bugs were tried against the test suite.

### absint

Computes an over-approximation of every state a program can reach, using interval and nullness domains with widening and narrowing. A fault that cannot occur in the over-approximation cannot occur at all. A fault reported as possible may be a false alarm, since the intervals are non-relational: `i < n` proves nothing about `i` unless `n` has a known range.

| Frontend | Proves the absence of | Assumes |
| --- | --- | --- |
| Text format | Out of bounds access, null dereference, division by zero, uninitialized reads, failed assertions | Nothing beyond the format's own meaning, which `ir::run` defines exactly |
| Locksets, on the text format | Data races on globals | Globals are named directly, and thread start order is ignored, which can cause false alarms and not misses |
| WebAssembly | Loads and stores past the declared minimum memory, division by zero | Vector, atomic and bulk accesses are not checked. Functions with exceptions or tail calls are listed as not analyzed |
| x86-64 machine code | Writes through the stack pointer that could reach the return address | Calls return normally and follow the System V ABI, and unknown pointers miss the frame. Those are reported as unknown, and not as proved |

```text
fn fill() {
  buf = alloc 16
  i = 0
head:
  if i < 16 goto body else done
body:
  store buf[i] = i
  i = i + 1
  goto head
done:
  return
}
```

Soundness is tested against the exact concrete semantics: random programs run on random inputs, every variable has to stay within its abstract value at every step, a fault must never occur where the analyzer proved it impossible, and a definite verdict must fault every time it is reached.

---

## Quick Start Guide

### 1. Prerequisites

- **Operating System:** Linux (Ubuntu 20.04/22.04 LTS recommended) or macOS
- **Python:** Python 3.8+ (`z3-solver`, `capstone`)
- **Compilers & Typesetters:**
  - Typst 0.12+ (for compiling `docs/aether64_paper.typ`)
  - GCC / Clang (x86-64 support)

### 2. Running Empirical Benchmarks

#### A. Quarkslab Diffing Obfuscation Dataset Evaluation (104 Functions)

```bash
python3 tests/test_diffing_dataset.py
```

*Outputs CFG recovery rate (100%), APR SMT bypass rate (93.91%), and instruction bloat reduction (91.8%) into `tests/diffing_dataset_benchmark.json`.*

#### B. Generic Non-Linear MBA Synthesis & VPC CFG Extraction

```bash
python3 tests/test_real_deobfuscator.py
```

*Validates Fast Moebius Inversion over $\mathbb{Z}/2^{64}$, MCMC non-linear synthesis (40.56 ms on 7-layer MBA), and Z3 formal equivalence proofs.*

#### C. Comprehensive System Evaluation (DSE vs AETHER-64)

```bash
python3 tests/evaluate_deobfuscation.py
```

### 3. Compiling the Academic Paper

```bash
# Compile vector PDF using Typst
typst compile docs/aether64_paper.typ docs/aether64_paper.pdf
```

---

## Publications & References

- **x64Unpack:** Choi et al., *"x64Unpack: Hybrid Emulation Unpacker for 64-bit Windows Environments and Detailed Analysis Results on VMProtect 3.4"*, IEEE Access, 2020.
- **CIA:** *"Chosen Instruction Attack Against Commercial Code Virtualization Obfuscators"*, NDSS 2022.
- **Syntia:** Blazytko et al., *"Syntia: Synthesizing the Semantics of Obfuscated Code"*, USENIX Security 2017.
- **ProMBA:** Saumier et al., *"Simplifying Mixed Boolean-Arithmetic Obfuscation by Program Synthesis and Term Rewriting"*, IEEE TDSC / ACM CCS.
- **AETHER-64:** Kim, Jeffrey, *"AETHER-64: Trace-Free Semantics-Preserving Devirtualization and Algebraic Deobfuscation for 64-Bit Protected Binaries"*, Independent Academic Research, 2026.

## Paper/Article related can be accessed [in the evaluation harness](https://www.researchgate.net/publication/416711614_AETHER-64_Trace-Free_Semantics-Preserving_Devirtualization_and_Al-_gebraic_Deobfuscation_for_64-Bit_Protected_Binaries)
