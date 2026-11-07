from triton import *
from triton.ast import AST_NODE
import json
import os
from llvmlite import ir
from collections import defaultdict
import argparse
import subprocess
import re

class AnalysisContext:
    def __init__(self):
        self.control_flow = defaultdict(list)  # Basic block -> successors
        self.memory_accesses = defaultdict(list)  # Address -> access patterns
        self.register_usage = defaultdict(list)  # Register -> usage patterns
        self.function_boundaries = set()  # Function entry points
        self.basic_blocks = set()  # Basic block start addresses
        self.data_flow = defaultdict(dict)  # Instruction -> data dependencies
        self.value_sets = defaultdict(set)  # Variable -> possible values
        self.pointer_aliases = defaultdict(set)  # Pointer -> aliases
        self.cross_references = defaultdict(set)  # Address -> references

class VMAnalyzer:
    def __init__(self, binary_path):
        self.binary_path = binary_path
        self.vm_start_patterns = [
            r'pushfq',  # Common VM entry
            r'pushad',  # Common VM entry
            r'pusha',   # Common VM entry
            r'pushfd',  # Common VM entry
            r'push\s+rbp',  # Function prologue
            r'mov\s+rbp,\s+rsp',  # Function prologue
            r'sub\s+rsp,\s+[0-9A-F]+h',  # Stack allocation
        ]
        self.vm_end_patterns = [
            r'popfq',   # Common VM exit
            r'popad',   # Common VM exit
            r'popa',    # Common VM exit
            r'popfd',   # Common VM exit
            r'leave',   # Function epilogue
            r'ret',     # Return
            r'retn',    # Return
        ]
        self.analysis_ctx = AnalysisContext()
        self.mappings = self.load_instruction_mappings()
        self.ctx = TritonContext()
        self.ctx.setArchitecture(ARCH.X86_64)

    def detect_vm_boundaries(self):
        """Automatically detect VM boundaries using objdump"""
        cmd = ['objdump', '-d', self.binary_path]
        result = subprocess.run(cmd, capture_output=True, text=True)
        
        vm_sections = []
        current_section = None
        vm_start = None
        
        for line in result.stdout.split('\n'):
            # Look for section headers
            if 'section' in line and 'text' in line:
                if current_section:
                    if vm_start:
                        vm_sections.append((vm_start, current_section['end']))
                current_section = {'start': None, 'end': None}
                vm_start = None
                continue
                
            # Parse instruction lines
            if ':' in line and '\t' in line:
                addr = int(line.split(':')[0].strip(), 16)
                if not current_section['start']:
                    current_section['start'] = addr
                current_section['end'] = addr
                
                # Check for VM start patterns
                for pattern in self.vm_start_patterns:
                    if re.search(pattern, line, re.IGNORECASE):
                        vm_start = addr
                        break
                        
                # Check for VM end patterns
                if vm_start:
                    for pattern in self.vm_end_patterns:
                        if re.search(pattern, line, re.IGNORECASE):
                            vm_sections.append((vm_start, addr))
                            vm_start = None
                            break
        
        return vm_sections

    def analyze_binary(self):
        """Analyze the binary file"""
        # Detect VM boundaries
        vm_sections = self.detect_vm_boundaries()
        print(f"Detected {len(vm_sections)} VM sections")
        
        # Process each VM section
        for start_addr, end_addr in vm_sections:
            print(f"Analyzing VM section: {hex(start_addr)} - {hex(end_addr)}")
            self.process_vm_section(start_addr, end_addr)
        
        # Generate reports
        self.generate_reports()

    def process_vm_section(self, start_addr, end_addr):
        """Process a VM section"""
        # Set up symbolic execution
        self.ctx.setConcreteRegisterValue(self.ctx.registers.rip, start_addr)
        
        # Process instructions until end of section
        while self.ctx.getConcreteRegisterValue(self.ctx.registers.rip) <= end_addr:
            inst = Instruction()
            inst.setAddress(self.ctx.getConcreteRegisterValue(self.ctx.registers.rip))
            
            # Get instruction bytes
            code = self.ctx.getConcreteMemoryAreaValue(
                inst.getAddress(),
                inst.getSize()
            )
            inst.setOpcode(code)
            
            # Process instruction
            self.process_instruction(inst)
            
            # Move to next instruction
            self.ctx.processing(inst)

    def generate_reports(self):
        """Generate analysis reports"""
        # Generate LLVM IR
        module = self.generate_llvm_ir()
        with open("devirt_func.ll", "w") as f:
            f.write(str(module))
        
        # Generate analysis report
        report = self.generate_analysis_report()
        with open("analysis_report.json", "w") as f:
            json.dump(report, f, indent=2)

def load_instruction_mappings():
    """Load VM instruction mappings from CIA-toolchain"""
    mappings = {}
    # Load VMP3 mappings
    with open("CIA-toolchain/contrast/ins_records/out_vmp3_formal.json", "r") as f:
        vmp_mappings = json.load(f)
        for entry in vmp_mappings:
            mappings[entry["name"]] = entry["ins"]
    
    # Load additional mappings from CIA-dataset
    cia_mappings_path = "CIA-toolchain/auto_gen/testset"
    if os.path.exists(cia_mappings_path):
        for filename in os.listdir(cia_mappings_path):
            if filename.endswith(".txt"):
                with open(os.path.join(cia_mappings_path, filename), "r") as f:
                    for line in f:
                        if line.strip() and not line.startswith("#"):
                            # Parse instruction and add to mappings
                            parts = line.strip().split()
                            if len(parts) >= 2:
                                mnemonic = parts[0]
                                native_ins = parts[1:]
                                mappings[mnemonic] = native_ins
    return mappings

def parse_trace_line(line):
    parts = line.split()
    rip = int(parts[1], 16)
    opcode_str = parts[3]
    opcode = bytes.fromhex(opcode_str.replace(" ", ""))
    regs = {}
    mem_read = None
    mem_write = None

    for part in parts[4:]:
        if '=' in part:
            reg, val = part.split('=')
            regs[reg] = int(val, 16)
        elif 'MEMORY_READ' in part:
            addr, val = part.split('=')[1].split('=')
            mem_read = (int(addr, 16), int(val, 16))
        elif 'MEMORY_WRITE' in part:
            addr, val = part.split('=')[1].split('=')
            mem_write = (int(addr, 16), int(val, 16))
    return rip, opcode, regs, mem_read, mem_write

def validate_with_test_cases(ctx, inst, mappings):
    """Validate symbolic execution results against CIA test cases"""
    mnemonic = inst.getDisassembly().split()[0]
    
    # Check if we have test cases for this instruction
    test_cases_path = "CIA-toolchain/auto_gen/testset"
    if os.path.exists(test_cases_path):
        for filename in os.listdir(test_cases_path):
            if filename.endswith(".txt"):
                with open(os.path.join(test_cases_path, filename), "r") as f:
                    for line in f:
                        if line.strip() and not line.startswith("#"):
                            parts = line.strip().split()
                            if parts[0] == mnemonic:
                                # Found matching test case
                                expected_result = parts[-1]  # Last part is expected result
                                # Compare with symbolic execution result
                                actual_result = ctx.getSymbolicRegister(ctx.registers.rax).getAst()
                                if str(actual_result) != expected_result:
                                    print(f"Warning: Test case mismatch for {mnemonic}")
                                    print(f"Expected: {expected_result}")
                                    print(f"Actual: {actual_result}")

def analyze_control_flow(ctx, inst, analysis_ctx):
    """Analyze control flow patterns"""
    addr = inst.getAddress()
    disassembly = inst.getDisassembly()
    
    # Identify basic blocks
    if addr not in analysis_ctx.basic_blocks:
        analysis_ctx.basic_blocks.add(addr)
    
    # Track control flow
    if inst.isBranch():
        target = inst.getOperands()[0].getValue()
        analysis_ctx.control_flow[addr].append(target)
        if inst.isCall():
            analysis_ctx.function_boundaries.add(target)

def analyze_memory_access(ctx, inst, analysis_ctx):
    """Analyze memory access patterns"""
    for mem in ctx.getSymbolicMemory():
        addr = mem.getAddress()
        size = mem.getSize()
        access_type = "read" if mem.isRead() else "write"
        analysis_ctx.memory_accesses[addr].append({
            'type': access_type,
            'size': size,
            'value': ctx.getConcreteMemoryValue(addr)
        })

def analyze_register_usage(ctx, inst, analysis_ctx):
    """Analyze register usage patterns"""
    for reg in ctx.getSymbolicRegisters():
        reg_name = reg.getName()
        value = ctx.getConcreteRegisterValue(reg)
        analysis_ctx.register_usage[reg_name].append({
            'value': value,
            'symbolic': ctx.getSymbolicRegister(reg).getAst()
        })

def analyze_data_flow(ctx, inst, analysis_ctx):
    """Analyze data dependencies"""
    addr = inst.getAddress()
    ast = ctx.getAstContext()
    
    for reg in ctx.getSymbolicRegisters():
        reg_name = reg.getName()
        reg_ast = ctx.getSymbolicRegister(reg).getAst()
        
        # Track data dependencies
        if reg_ast.getType() != ast.BV:
            analysis_ctx.data_flow[addr][reg_name] = {
                'dependencies': reg_ast.getChildren(),
                'value_set': set()
            }
            
            # Update value sets
            if reg_ast.getType() == ast.CONCAT:
                analysis_ctx.value_sets[reg_name].add(reg_ast.getInteger())
            elif reg_ast.getType() == ast.EXTRACT:
                analysis_ctx.value_sets[reg_name].add(reg_ast.getInteger())

def analyze_pointers(ctx, inst, analysis_ctx):
    """Analyze pointer relationships"""
    for mem in ctx.getSymbolicMemory():
        addr = mem.getAddress()
        if ctx.isMemorySymbolized(addr):
            # Track pointer aliases
            for other_addr in ctx.getSymbolicMemory():
                if addr != other_addr and ctx.isMemorySymbolized(other_addr):
                    if ctx.getConcreteMemoryValue(addr) == ctx.getConcreteMemoryValue(other_addr):
                        analysis_ctx.pointer_aliases[addr].add(other_addr)

def analyze_cross_references(ctx, inst, analysis_ctx):
    """Analyze cross-references"""
    addr = inst.getAddress()
    for mem in ctx.getSymbolicMemory():
        mem_addr = mem.getAddress()
        if ctx.isMemorySymbolized(mem_addr):
            value = ctx.getConcreteMemoryValue(mem_addr)
            if value in analysis_ctx.basic_blocks:
                analysis_ctx.cross_references[value].add(addr)

def process_instruction(ctx, inst, mappings, analysis_ctx):
    """Process a single instruction with comprehensive analysis"""
    # Get instruction mnemonic
    mnemonic = inst.getDisassembly().split()[0]
    
    # Check if this is a VM instruction
    vm_ins = None
    for name, native_ins in mappings.items():
        if mnemonic in native_ins:
            vm_ins = name
            break
            
    if vm_ins:
        # This is a VM instruction, process it symbolically
        ctx.processing(inst)
        
        # Perform comprehensive analysis
        analyze_control_flow(ctx, inst, analysis_ctx)
        analyze_memory_access(ctx, inst, analysis_ctx)
        analyze_register_usage(ctx, inst, analysis_ctx)
        analyze_data_flow(ctx, inst, analysis_ctx)
        analyze_pointers(ctx, inst, analysis_ctx)
        analyze_cross_references(ctx, inst, analysis_ctx)
        
        # Validate against test cases
        validate_with_test_cases(ctx, inst, mappings)
    else:
        # Regular instruction, just process it
        ctx.processing(inst)

def generate_llvm_ir(ctx, ast):
    """Generate LLVM IR from symbolic AST"""
    # Create LLVM module
    module = ir.Module(name="devirt_func")
    
    # Create function type
    func_type = ir.FunctionType(ir.IntType(64), [ir.IntType(64)])
    
    # Create function
    func = ir.Function(module, func_type, name="devirt_func")
    
    # Create basic block
    block = func.append_basic_block(name="entry")
    builder = ir.IRBuilder(block)
    
    # Convert Triton AST to LLVM IR
    # This is a simplified version - actual implementation would need
    # to handle all AST node types
    if ast.getType() == AST_NODE.BV:
        value = ast.getInteger()
        result = ir.Constant(ir.IntType(64), value)
    else:
        # Handle more complex expressions
        result = ir.Constant(ir.IntType(64), 0)  # Placeholder
        
    # Return result
    builder.ret(result)
    
    return module

def generate_analysis_report(analysis_ctx):
    """Generate comprehensive analysis report"""
    report = {
        'control_flow': dict(analysis_ctx.control_flow),
        'memory_accesses': dict(analysis_ctx.memory_accesses),
        'register_usage': dict(analysis_ctx.register_usage),
        'function_boundaries': list(analysis_ctx.function_boundaries),
        'basic_blocks': list(analysis_ctx.basic_blocks),
        'data_flow': dict(analysis_ctx.data_flow),
        'value_sets': {k: list(v) for k, v in analysis_ctx.value_sets.items()},
        'pointer_aliases': {k: list(v) for k, v in analysis_ctx.pointer_aliases.items()},
        'cross_references': {k: list(v) for k, v in analysis_ctx.cross_references.items()}
    }
    return report

def main():
    parser = argparse.ArgumentParser(description='Analyze VM-protected binary')
    parser.add_argument('binary', help='Path to the encrypted binary')
    parser.add_argument('--output-dir', default='.', help='Output directory for reports')
    args = parser.parse_args()
    
    # Create output directory if it doesn't exist
    os.makedirs(args.output_dir, exist_ok=True)
    
    # Initialize analyzer
    analyzer = VMAnalyzer(args.binary)
    
    # Analyze binary
    analyzer.analyze_binary()

if __name__ == "__main__":
    main()

"""
Usage:
    python triton_script/attack_vmp.py <binary_path> [--output-dir <dir>]

Arguments:
    binary_path    : Path to the encrypted binary file to analyze
    --output-dir   : (Optional) Directory to save analysis results (default: current directory)

Example:
    python triton_script/attack_vmp.py ./encrypted_binary
    python triton_script/attack_vmp.py ./encrypted_binary --output-dir ./analysis_results

Output:
    - devirt_func.ll     : Generated LLVM IR
    - analysis_report.json : Comprehensive analysis results including:
        * Control flow analysis
        * Memory access patterns
        * Register usage
        * Data dependencies
        * Pointer relationships
        * Cross-references
        * Function boundaries
        * Basic blocks
"""