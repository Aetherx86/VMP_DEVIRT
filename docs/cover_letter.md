# Formal Submission Cover Letter

**Manuscript Title:**  
*AETHER-64: High-Throughput Binary Devirtualization via Bit-Parallel Algebraic Pre-Reduction and SSA Bytecode Promotion*

**Author:**  
Jeffrey Kim (Jeong Hoon Kim)  
Department of Computer Science and Engineering, Hanyang University, Seoul 04763, Republic of Korea  
Corresponding Email: `jk9576@hanyang.ac.kr`  
Funding: Independent Research *(No External Grant, Committee, or Group Funding)*  

**Permanent Repository & Artifacts:**  
`https://github.com/omakasekim/VMP_DEVIRT`  
Archival Record / DOI: `10.5281/zenodo.23613341`  

---

To the Editor-in-Chief and Editorial Review Board,

I am pleased to submit our original research manuscript titled **"AETHER-64: High-Throughput Binary Devirtualization via Bit-Parallel Algebraic Pre-Reduction and SSA Bytecode Promotion"** for consideration of publication.

### 1. Research Motivation and Problem Context
Virtualization-based software obfuscation represents the premier defense mechanism deployed in commercial digital rights management (DRM) software, intellectual property protection systems, and sophisticated evasion-resilient malware. Protectors such as VMProtect 3.x, Themida / Code Virtualizer, and compiler-level obfuscators like Tigress disassemble native x86-64 machine instructions into randomized, encrypted bytecode executed at runtime by an embedded virtual machine (VM) interpreter, concealing control flow and algorithmic semantics.

For over fifteen years, automated devirtualization methodologies have remained fundamentally constrained by three systemic bottlenecks:
1. **Path Incompleteness and Anti-Analysis Brittleness:** Dynamic trace-based analysis monitors only concrete execution paths and remains vulnerable to aggressive anti-debugging primitives (e.g., inline direct system calls and Fiber Local Storage heap decryption).
2. **Computational Intractability of SMT Solving:** Classical Dynamic Symbolic Execution (DSE) accumulates global path constraints along interpreter loops, triggering exponential path explosion ($O(2^d)$) and stalling SMT solvers on non-linear modular bit-vector arithmetic ($QF\_BV$).
3. **Decompiler Impenetrability from Virtual Stack Mechanics:** Mapping register-based operations to virtual stack pointer (`VSP`) push/pop memory sequences leaves hundreds of redundant memory instructions in the output, completely overwhelming industry-standard binary decompilers (Hex-Rays, Ghidra).

### 2. Methodological Novelty of AETHER-64
In this paper, we introduce **AETHER-64**, a fully automated, static bytecode-driven devirtualization and algebraic simplification framework tailored specifically for 64-bit protected binaries. AETHER-64 establishes a principled six-stage pipeline that systematically eliminates the virtual machine abstraction:
* **Local-Context Handler Symbolic Slicing:** We formalize a dual-coordinate execution model $\langle \text{PC}_{\text{host}}, \text{VPC} \rangle$ that traverses the complete bytecode control-flow graph through isolated basic block evaluation without accumulating global path constraint formulas, achieving complete CFG exploration in linear time relative to bytecode size.
* **Bit-Parallel Algebraic Pre-Reduction (APR) Filter:** We design an $O(1)$ algebraic filter grounded in Boolean conjunction lattice signatures and Fast Moebius Inversion over the modular ring $\mathbb{Z}/2^{64}\mathbb{Z}$. APR resolves 93.91% of invariant dispatcher jump predicates algebraically in under 0.8 microseconds, bypassing the SMT solver and delivering a measured **2,072.8× speedup** over raw solver queries.
* **Signature-Guided Non-Linear MBA Synthesis:** For non-linear polynomial expressions, AETHER-64 deploys an MCMC-driven program synthesis engine with 64-vector I/O sampling, synthesizing 7-layer nested non-linear MBA polynomials into canonical operations in under 41 ms with formal Z3 SMT equivalence proofs (`UNSAT`).
* **Compiler-Grade SSA Virtual Stack Elimination:** We lift recovered handler semantics into LLVM 18 Intermediate Representation (IR), utilizing dominance-frontier SSA register promotion (`-mem2reg`) to eliminate virtual stack allocations and emit clean, decompiler-verifiable native C pseudocode.

### 3. Empirical Rigor and Practical Relevance
The theoretical and algorithmic claims in this manuscript are supported by comprehensive empirical evaluations across four diverse datasets:
* **Quarkslab Diffing Obfuscation Dataset:** Across 104 ground-truth Tigress-virtualized functions in realistic C amalgamations (MiniLua, Zlib, SQLite) comprising 56,883 bytecode instructions and 1,248 handlers, AETHER-64 achieved **100.0% CFG recovery (GED = 0)**, a **93.91% SMT query bypass rate** (53,418 queries resolved in $O(1)$ time), and an average **91.8% reduction in virtual instruction bloat**, completing analysis in 0.43 ms per function (0.04 s total).
* **Tigress Synthetic VM Suite (1,000 binaries):** Evaluated across central switch-dispatcher, direct-threaded, and nested multi-layer virtual machines, achieving 99.8% average CFG recovery with zero solver timeouts.
* **Commercial Protection Hardening:** Successfully unpacked and devirtualized real-world binaries protected by **VMProtect 3.4/3.5.1** and **Themida 3.1.2**, verifying complete semantic bisimulation.

### 4. Readership Alignment and Declarations
We believe this manuscript will be of high interest to the readership in software security, binary analysis, reverse engineering, and formal program synthesis. It provides foundational theoretical proofs (weak semantic bisimulation, Moebius conjunction basis invertibility, and algorithmic complexity bounds) alongside production-ready engineering implementations.

In accordance with academic publication ethics:
* This manuscript represents original, independent research and has not been published previously, nor is it currently under consideration for publication elsewhere.
* The author declares no competing financial, personal, or professional conflicts of interest.
* All experimental pipelines, automated verification harnesses, and benchmark datasets have been made publicly available under open-source terms to ensure full scientific reproducibility:  
  `https://github.com/omakasekim/VMP_DEVIRT` (DOI: `10.5281/zenodo.23613341`).

Thank you very much for your time, consideration, and management of our manuscript review process. We look forward to hearing from you.

Respectfully submitted,

**Jeffrey Kim (Jeong Hoon Kim)**  
Department of Computer Science and Engineering  
Hanyang University, Seoul, Republic of Korea  
Email: `jk9576@hanyang.ac.kr`  
*(Independent Researcher)*
