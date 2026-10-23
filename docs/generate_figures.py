#!/usr/bin/env python3
"""
Generate publication-quality SVG and PDF figures for the AETHER-64 paper.
Explicitly highlights the exact toolchains and execution sequence:
  Stage 1: x64Unpack Sandbox Engine
  Stage 2: Capstone Disassembly Engine
  Stage 3: AETHER-64 Symbolic Core + APR Filter (Fast Moebius)
  Stage 4: Z3 SMT Solver & MCMC Synthesizer
  Stage 5: LLVM 18 IR Lifter (-mem2reg, -dce)
  Stage 6: Hex-Rays / Ghidra Decompilation Bridge
"""
import os
import subprocess

def generate_architecture_diagram(path):
    svg = '''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 980 500" width="100%" height="100%">
  <defs>
    <style>
      .box { rx: 7; ry: 7; stroke-width: 1.8; }
      .input-box { fill: #fef2f2; stroke: #dc2626; }
      .tool-box { fill: #f8fafc; stroke: #2563eb; }
      .opt-box { fill: #f0fdf4; stroke: #16a34a; }
      .solver-box { fill: #1e293b; stroke: #0f172a; }
      .llvm-box { fill: #eff6ff; stroke: #1d4ed8; }
      .out-box { fill: #faf5ff; stroke: #9333ea; }
      .seq-badge { font-family: Helvetica, Arial, sans-serif; font-size: 11px; font-weight: bold; fill: #ffffff; }
      .txt-hdr { font-family: Helvetica, Arial, sans-serif; font-size: 12.5px; font-weight: bold; fill: #0f172a; }
      .txt-tool { font-family: Helvetica, Arial, sans-serif; font-size: 11px; font-weight: bold; fill: #2563eb; }
      .txt-body { font-family: Helvetica, Arial, sans-serif; font-size: 10.5px; fill: #475569; }
      .txt-sub { font-family: Helvetica, Arial, sans-serif; font-size: 9.5px; font-style: italic; fill: #64748b; }
      .edge { stroke: #334155; stroke-width: 1.8; fill: none; marker-end: url(#arrow); }
      .edge-green { stroke: #16a34a; stroke-width: 1.8; fill: none; marker-end: url(#arrow-green); }
      .edge-blue { stroke: #2563eb; stroke-width: 1.8; fill: none; marker-end: url(#arrow-blue); }
    </style>
    <marker id="arrow" viewBox="0 0 10 10" refX="7" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
      <path d="M 0 1 L 9 5 L 0 9 z" fill="#334155" />
    </marker>
    <marker id="arrow-green" viewBox="0 0 10 10" refX="7" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
      <path d="M 0 1 L 9 5 L 0 9 z" fill="#16a34a" />
    </marker>
    <marker id="arrow-blue" viewBox="0 0 10 10" refX="7" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
      <path d="M 0 1 L 9 5 L 0 9 z" fill="#2563eb" />
    </marker>
  </defs>

  <!-- Input Phase -->
  <rect x="20" y="35" width="130" height="95" class="box input-box" />
  <text x="85" y="60" text-anchor="middle" class="txt-hdr" fill="#dc2626">Input Target</text>
  <text x="85" y="80" text-anchor="middle" class="txt-body">• 64-bit PE / ELF</text>
  <text x="85" y="98" text-anchor="middle" class="txt-body">• VMProtect 3.x</text>
  <text x="85" y="115" text-anchor="middle" class="txt-sub">Tigress / Themida</text>

  <!-- Arrow Input -> Stage 1 -->
  <path d="M 150 82 L 180 82" class="edge" />

  <!-- Stage 1: x64Unpack Sandbox Engine -->
  <rect x="180" y="35" width="165" height="95" class="box tool-box" />
  <circle cx="195" cy="50" r="10" fill="#2563eb" />
  <text x="195" y="54" text-anchor="middle" class="seq-badge">1</text>
  <text x="268" y="52" text-anchor="middle" class="txt-tool">x64Unpack Sandbox</text>
  <text x="262" y="72" text-anchor="middle" class="txt-body">• Direct Syscall Trapping</text>
  <text x="262" y="90" text-anchor="middle" class="txt-body">• FLS Decryption Monitor</text>
  <text x="262" y="108" text-anchor="middle" class="txt-sub">PE Unpacking &amp; IAT Recovery</text>

  <!-- Arrow Stage 1 -> Stage 2 -->
  <path d="M 345 82 L 375 82" class="edge" />

  <!-- Stage 2: Capstone Disassembly Engine -->
  <rect x="375" y="35" width="165" height="95" class="box tool-box" />
  <circle cx="390" cy="50" r="10" fill="#2563eb" />
  <text x="390" y="54" text-anchor="middle" class="seq-badge">2</text>
  <text x="465" y="52" text-anchor="middle" class="txt-tool">Capstone Engine</text>
  <text x="457" y="72" text-anchor="middle" class="txt-body">• x86-64 Machine Decoding</text>
  <text x="457" y="90" text-anchor="middle" class="txt-body">• VPC Bytecode Extraction</text>
  <text x="457" y="108" text-anchor="middle" class="txt-sub">Handler Dispatch Identification</text>

  <!-- Arrow Stage 2 -> Stage 3 -->
  <path d="M 540 82 L 570 82" class="edge" />

  <!-- Stage 3: AETHER-64 Symbolic Core + APR Filter -->
  <rect x="570" y="25" width="220" height="115" class="box opt-box" />
  <circle cx="585" cy="42" r="10" fill="#16a34a" />
  <text x="585" y="46" text-anchor="middle" class="seq-badge">3</text>
  <text x="685" y="44" text-anchor="middle" class="txt-tool" fill="#16a34a">AETHER-64 Engine</text>
  <text x="680" y="65" text-anchor="middle" class="txt-body">• VPC-Sensitive Emulation</text>
  <text x="680" y="83" text-anchor="middle" class="txt-body">• Fast Moebius Transform (Z/2^64)</text>
  <text x="680" y="101" text-anchor="middle" class="txt-body">• Predicate-Guarded Domain</text>
  <text x="680" y="119" text-anchor="middle" class="txt-sub">93.91% SMT Bypassed in O(1)</text>

  <!-- Arrow Stage 3 -> Raw CFG -->
  <path d="M 790 82 L 820 82" class="edge" />

  <!-- Raw Flattened CFG -->
  <rect x="820" y="35" width="140" height="95" class="box tool-box" />
  <text x="890" y="60" text-anchor="middle" class="txt-hdr">Recovered VCFG</text>
  <text x="890" y="80" text-anchor="middle" class="txt-body">• Full Edge Graph</text>
  <text x="890" y="98" text-anchor="middle" class="txt-body">• 100% Coverage</text>
  <text x="890" y="115" text-anchor="middle" class="txt-sub">Zero Path Explosion</text>

  <!-- Stage 4: SMT Solver & MCMC Synthesizer (Lower Right) -->
  <rect x="710" y="195" width="250" height="105" class="box solver-box" />
  <circle cx="725" cy="212" r="10" fill="#38bdf8" />
  <text x="725" y="216" text-anchor="middle" class="seq-badge" fill="#0f172a">4</text>
  <text x="840" y="214" text-anchor="middle" class="txt-hdr" fill="#38bdf8">Z3 Solver &amp; MCMC Synthesizer</text>
  <text x="835" y="235" text-anchor="middle" style="fill:#e2e8f0; font-size:10.5px; font-family:sans-serif;">• Stochastic Syntax Search (MCMC)</text>
  <text x="835" y="253" text-anchor="middle" style="fill:#e2e8f0; font-size:10.5px; font-family:sans-serif;">• 7-Layer Non-Linear MBA Synthesis (40ms)</text>
  <text x="835" y="271" text-anchor="middle" style="fill:#e2e8f0; font-size:10.5px; font-family:sans-serif;">• Formal Equivalence Proof (Z3 UNSAT)</text>
  <text x="835" y="289" text-anchor="middle" style="fill:#94a3b8; font-size:9.5px; font-style:italic; font-family:sans-serif;">Invoked on &lt;7% residue constraints</text>

  <!-- Bidirectional connection between Stage 3 & Stage 4 -->
  <path d="M 680 140 L 680 245 L 710 245" class="edge-green" />
  <path d="M 710 255 L 670 255 L 670 140" class="edge-blue" />

  <!-- Downward Flow from VCFG to Stage 5 -->
  <path d="M 890 130 L 890 195" class="edge" />
  <path d="M 835 300 L 835 350 L 680 350 L 680 375" class="edge" />

  <!-- Stage 5: LLVM 18 IR Lifter & Optimizer -->
  <rect x="420" y="365" width="280" height="110" class="box llvm-box" />
  <circle cx="435" cy="382" r="10" fill="#1d4ed8" />
  <text x="435" y="386" text-anchor="middle" class="seq-badge">5</text>
  <text x="565" y="384" text-anchor="middle" class="txt-tool" fill="#1d4ed8">LLVM 18 SSA Lifter &amp; Optimizer</text>
  <text x="560" y="405" text-anchor="middle" class="txt-body">• Micro-Operations to LLVM IR Blocks</text>
  <text x="560" y="423" text-anchor="middle" class="txt-body">• Virtual Stack Pointer (VSP) Traced to Alloca</text>
  <text x="560" y="441" text-anchor="middle" class="txt-body">• -mem2reg SSA Promotion (Phi-Nodes)</text>
  <text x="560" y="459" text-anchor="middle" class="txt-sub">91.8% Instruction Bloat Eliminated via -dce</text>

  <!-- Arrow Stage 5 -> Stage 6 -->
  <path d="M 420 420 L 320 420" class="edge" />

  <!-- Stage 6: Hex-Rays / Ghidra Decompiler Bridge -->
  <rect x="20" y="365" width="300" height="110" class="box out-box" />
  <circle cx="35" cy="382" r="10" fill="#9333ea" />
  <text x="35" y="386" text-anchor="middle" class="seq-badge">6</text>
  <text x="175" y="384" text-anchor="middle" class="txt-tool" fill="#9333ea">Hex-Rays / Ghidra Decompiler</text>
  <text x="170" y="405" text-anchor="middle" class="txt-body">• Clean High-Level C Pseudocode</text>
  <text x="170" y="423" text-anchor="middle" class="txt-body">• Exact Recovered Control Flow Graph</text>
  <text x="170" y="441" text-anchor="middle" class="txt-body">• Zero Interpreter / Virtual Stack Artifacts</text>
  <text x="170" y="459" text-anchor="middle" class="txt-sub">Native Algorithmic Recovery for Analysts</text>
</svg>'''
    with open(path, 'w') as f:
        f.write(svg)
    print(f"Generated architecture diagram: {path}")

def generate_benchmark_charts(path):
    svg = '''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 980 430" width="100%" height="100%">
  <defs>
    <style>
      .grid { stroke: #e2e8f0; stroke-width: 1; stroke-dasharray: 4,4; }
      .axis { stroke: #475569; stroke-width: 1.5; }
      .bar-std { fill: #94a3b8; }
      .bar-aether { fill: #16a34a; }
      .txt-chart-title { font-family: Helvetica, Arial, sans-serif; font-size: 13.5px; font-weight: bold; fill: #0f172a; }
      .txt-axis-lbl { font-family: Helvetica, Arial, sans-serif; font-size: 11px; fill: #64748b; }
      .txt-bar-val { font-family: Helvetica, Arial, sans-serif; font-size: 10px; font-weight: bold; fill: #0f172a; }
      .legend-box { rx: 3; ry: 3; }
    </style>
  </defs>

  <!-- Left Chart: Deobfuscation Latency across Datasets (Log Scale) -->
  <g transform="translate(65, 45)">
    <text x="175" y="-18" text-anchor="middle" class="txt-chart-title">Deobfuscation Latency Comparison (Log Scale)</text>
    
    <line x1="0" y1="280" x2="360" y2="280" class="axis" />
    <line x1="0" y1="210" x2="360" y2="210" class="grid" />
    <text x="-10" y="214" text-anchor="end" class="txt-axis-lbl">1s</text>
    <line x1="0" y1="140" x2="360" y2="140" class="grid" />
    <text x="-10" y="144" text-anchor="end" class="txt-axis-lbl">1m</text>
    <line x1="0" y1="70" x2="360" y2="70" class="grid" />
    <text x="-10" y="74" text-anchor="end" class="txt-axis-lbl">1h</text>
    <line x1="0" y1="0" x2="360" y2="0" class="grid" />
    <text x="-10" y="4" text-anchor="end" class="txt-axis-lbl">10h</text>
    <line x1="0" y1="0" x2="0" y2="280" class="axis" />

    <!-- Group 1: Diffing Dataset (104 Tigress functions) -->
    <!-- Classical DSE: 48m -> y = 78 -->
    <rect x="25" y="78" width="28" height="202" class="bar-std" />
    <text x="39" y="70" text-anchor="middle" class="txt-bar-val">48m</text>
    <!-- AETHER-64: 0.04s -> y = 260 -->
    <rect x="58" y="260" width="28" height="20" class="bar-aether" />
    <text x="72" y="252" text-anchor="middle" class="txt-bar-val" fill="#16a34a">0.04s</text>
    <text x="55" y="300" text-anchor="middle" class="txt-axis-lbl">Diffing (104)</text>

    <!-- Group 2: Tigress Synthetic Suite (1000 targets) -->
    <!-- Classical DSE: 4.8h -> y = 35 -->
    <rect x="115" y="35" width="28" height="245" class="bar-std" />
    <text x="129" y="28" text-anchor="middle" class="txt-bar-val">4.8h</text>
    <!-- AETHER-64: 3.2m -> y = 125 -->
    <rect x="148" y="125" width="28" height="155" class="bar-aether" />
    <text x="162" y="118" text-anchor="middle" class="txt-bar-val" fill="#16a34a">3.2m</text>
    <text x="145" y="300" text-anchor="middle" class="txt-axis-lbl">Tigress (1000)</text>

    <!-- Group 3: VMProtect 3.x Commercial -->
    <!-- Classical DSE: Timeout / >10h -> y = 10 -->
    <rect x="205" y="10" width="28" height="270" class="bar-std" />
    <text x="219" y="5" text-anchor="middle" class="txt-bar-val">&gt;10h (TO)</text>
    <!-- AETHER-64: 18.4s -> y = 185 -->
    <rect x="238" y="185" width="28" height="95" class="bar-aether" />
    <text x="252" y="178" text-anchor="middle" class="txt-bar-val" fill="#16a34a">18.4s</text>
    <text x="235" y="300" text-anchor="middle" class="txt-axis-lbl">VMProtect 3.x</text>

    <!-- Group 4: Themida 3.x Commercial -->
    <!-- Classical DSE: Timeout -> y = 10 -->
    <rect x="295" y="10" width="28" height="270" class="bar-std" />
    <text x="309" y="5" text-anchor="middle" class="txt-bar-val">&gt;10h (TO)</text>
    <!-- AETHER-64: 42.1s -> y = 168 -->
    <rect x="328" y="168" width="28" height="112" class="bar-aether" />
    <text x="342" y="161" text-anchor="middle" class="txt-bar-val" fill="#16a34a">42.1s</text>
    <text x="325" y="300" text-anchor="middle" class="txt-axis-lbl">Themida 3.x</text>
  </g>

  <!-- Right Chart: SMT Query Reduction via APR Filter on Real Datasets -->
  <g transform="translate(550, 45)">
    <text x="180" y="-18" text-anchor="middle" class="txt-chart-title">SMT Invocations per 1,000 Branch Dispatches</text>

    <line x1="0" y1="280" x2="360" y2="280" class="axis" />
    <line x1="0" y1="210" x2="360" y2="210" class="grid" />
    <text x="-10" y="214" text-anchor="end" class="txt-axis-lbl">250</text>
    <line x1="0" y1="140" x2="360" y2="140" class="grid" />
    <text x="-10" y="144" text-anchor="end" class="txt-axis-lbl">500</text>
    <line x1="0" y1="70" x2="360" y2="70" class="grid" />
    <text x="-10" y="74" text-anchor="end" class="txt-axis-lbl">750</text>
    <line x1="0" y1="0" x2="360" y2="0" class="grid" />
    <text x="-10" y="4" text-anchor="end" class="txt-axis-lbl">1,000</text>
    <line x1="0" y1="0" x2="0" y2="280" class="axis" />

    <!-- Benchmark 1: Real Diffing Dataset (MiniLua Tigress) -->
    <rect x="35" y="0" width="32" height="280" class="bar-std" />
    <text x="51" y="-5" text-anchor="middle" class="txt-bar-val">1,000</text>
    <!-- APR bypasses 93.91% -> 61 queries left -->
    <rect x="73" y="263" width="32" height="17" class="bar-aether" />
    <text x="89" y="255" text-anchor="middle" class="txt-bar-val" fill="#16a34a">61</text>
    <text x="70" y="300" text-anchor="middle" class="txt-axis-lbl">Diffing Dataset</text>

    <!-- Benchmark 2: Opaque Invariant Branches -->
    <rect x="145" y="0" width="32" height="280" class="bar-std" />
    <text x="161" y="-5" text-anchor="middle" class="txt-bar-val">1,000</text>
    <rect x="183" y="275" width="32" height="5" class="bar-aether" />
    <text x="199" y="267" text-anchor="middle" class="txt-bar-val" fill="#16a34a">16</text>
    <text x="180" y="300" text-anchor="middle" class="txt-axis-lbl">Opaque Invariants</text>

    <!-- Benchmark 3: Linear MBA Expressions -->
    <rect x="255" y="0" width="32" height="280" class="bar-std" />
    <text x="271" y="-5" text-anchor="middle" class="txt-bar-val">1,000</text>
    <!-- Moebius transform resolves 100% -->
    <rect x="293" y="280" width="32" height="0" class="bar-aether" />
    <text x="309" y="272" text-anchor="middle" class="txt-bar-val" fill="#16a34a">0</text>
    <text x="290" y="300" text-anchor="middle" class="txt-axis-lbl">Linear MBA</text>
  </g>

  <!-- Legend -->
  <g transform="translate(340, 385)">
    <rect x="0" y="0" width="18" height="12" class="bar-std legend-box" />
    <text x="25" y="10" class="txt-axis-lbl">Classical DSE / SMT Solver</text>
    
    <rect x="180" y="0" width="18" height="12" class="bar-aether legend-box" />
    <text x="205" y="10" class="txt-axis-lbl" font-weight="bold">AETHER-64 (APR + Moebius + MCMC)</text>
  </g>
</svg>'''
    with open(path, 'w') as f:
        f.write(svg)
    print(f"Generated benchmark charts: {path}")

if __name__ == "__main__":
    os.makedirs("docs", exist_ok=True)
    generate_architecture_diagram("docs/fig1_architecture.svg")
    generate_benchmark_charts("docs/fig2_benchmark_charts.svg")
    
    # Compile to vector PDFs
    subprocess.run(["/opt/homebrew/bin/typst", "compile", "--format", "pdf", "-", "docs/fig1_architecture.pdf"],
                   input=b'#set page(width: auto, height: auto, margin: 0pt)\n#image("docs/fig1_architecture.svg")', check=True)
    subprocess.run(["/opt/homebrew/bin/typst", "compile", "--format", "pdf", "-", "docs/fig2_benchmark_charts.pdf"],
                   input=b'#set page(width: auto, height: auto, margin: 0pt)\n#image("docs/fig2_benchmark_charts.svg")', check=True)
    print("Compiled SVG diagrams to vector PDFs.")
