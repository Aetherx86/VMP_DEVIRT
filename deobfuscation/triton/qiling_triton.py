from qiling import Qiling
from qiling.const import QL_VERBOSE
from triton import TritonContext, ARCH, Instruction
from triton import *
import sys

# Enable or disable logging
LOGGING_ENABLED = False  # Set to False to disable logging

# Enable or disable periodic Triton context reset every 1000 instructions
RESET_TRITON_PERIODICALLY = False  # Set to False to disable periodic reset

# Triton initialization
triton_ctx = TritonContext()
triton_ctx.setArchitecture(ARCH.X86_64)

# Global variables
last_inst = None           # Stores the previous instruction (1-instruction delay)
instruction_count = 0      # Counts how many instructions were reached
seq_num = 1                # Unique sequence number for each instruction
mem_read_list = []         # Collects memory reads for the currently executing instruction
logfile = None             # File handle for logging

def is_memory_already_symbolic(ctx, addr, size):
    """
    Returns True if any byte in [addr, addr+size)
    is already symbolic in Triton.
    """
    for offset in range(size):
        sym_expr = ctx.getSymbolicMemory(addr + offset)
        if sym_expr is not None:
            # Check if the AST is actually symbolic
            # (e.g., depends on a symbolic variable or unknown input)
            if sym_expr.getAst().isSymbolized():
                return True
    return False

def sync_memory_read(ql, access, address, size, value):
    """
    Qiling memory-read hook.
    - If the memory area is not already symbolic, update Triton with the concrete bytes.
    - Otherwise, skip updating so we don't overwrite symbolic state with a new concrete value.
    """
    data = ql.mem.read(address, size)

    # # Only update Triton if it's not already symbolic.
    # if not is_memory_already_symbolic(triton_ctx, address, size):
    #     triton_ctx.setConcreteMemoryAreaValue(address, data)

    if LOGGING_ENABLED:
        mem_read_list.append((seq_num, address, data))

def sync_registers(ql, ctx):
    """
    Synchronize Qiling CPU registers into the Triton context.
    """
    regs = [
        ("rax", "rax"), ("rbx", "rbx"), ("rcx", "rcx"), ("rdx", "rdx"),
        ("rsi", "rsi"), ("rdi", "rdi"), ("rbp", "rbp"), ("rsp", "rsp"),
        ("r8",  "r8"),  ("r9",  "r9"),  ("r10", "r10"), ("r11", "r11"),
        ("r12", "r12"), ("r13", "r13"), ("r14", "r14"), ("r15", "r15"),
        ("rip", "rip")
    ]
    for ql_reg, triton_reg_name in regs:
        val = ql.arch.regs.read(ql_reg)
        triton_reg = getattr(ctx.registers, triton_reg_name)
        ctx.setConcreteRegisterValue(triton_reg, val)

def hook_code(ql, address, size):
    """
    This hook is triggered right before the NEXT instruction executes.
    We use this moment to process/log the PREVIOUS instruction (if any).
    Then we fetch the current instruction bytes and store them into last_inst.
    """
    global last_inst, triton_ctx, instruction_count, seq_num, mem_read_list

    # 1) Process the previously finished instruction, if available
    if last_inst is not None:
        # sync_registers(ql, triton_ctx)
        triton_ctx.disassembly(last_inst)
        triton_ctx.processing(last_inst)
        
        for se in last_inst.getSymbolicExpressions():
            se.setComment(str(last_inst))

        # Perform backward slicing if the specific opcode is found
        if bytes(last_inst.getOpcode()).hex() == "418b9432799ddff1":  # Example opcode
            print("perform backward slicing")
            disasm_str = last_inst.getDisassembly()
            print(f"Disassembly: {disasm_str}\n")
            
            # symbolic_regs = triton_ctx.getSymbolicRegisters()

            # if symbolic_regs:
            #     print("[+] Symbolic Registers:")
            #     for reg_id, expr in symbolic_regs.items():
            #         reg_name = triton_ctx.getRegister(reg_id).getName()
            #         print(f"    - {reg_name} (ID: {reg_id}) -> {expr}")
            # else:
            #     print("[!] No symbolic registers found.")
            
            # Get the symbolic expression of RDX
            rdxExpr = triton_ctx.getSymbolicRegisters().get(REG.X86_64.RSI)
            if rdxExpr:
                slicing = triton_ctx.sliceExpressions(rdxExpr)
                for k, v in sorted(slicing.items()):
                    print('[slicing]', v.getComment())
            else:
                print("[!] RDX is not symbolic!")
                
            exit(1)

        if LOGGING_ENABLED:
            log_instruction(last_inst, seq_num)
            mem_read_list.clear()

        last_inst = None
        seq_num += 1  # Increment sequence number

    # 2) Periodically reset Triton context (if enabled)
    instruction_count += 1
    if RESET_TRITON_PERIODICALLY and instruction_count % 1000 == 0:
        print("[*] Resetting Triton context at instruction count:", instruction_count)
        triton_ctx = TritonContext()
        triton_ctx.setArchitecture(ARCH.X86_64)
        sync_registers(ql, triton_ctx)

    # 3) Fetch the current instruction's bytes and store in last_inst
    opcode_bytes = ql.mem.read(address, size)
    inst = Instruction(address, bytes(opcode_bytes))
    last_inst = inst

def log_instruction(inst, seq_num):
    """
    Logs the instruction sequence number, address, opcode (hex), disassembly,
    registers, and any memory reads to the global logfile.
    """
    global logfile

    if not LOGGING_ENABLED:
        return  # Skip logging if disabled

    # Basic info
    addr = inst.getAddress()
    raw_opcode = bytes(inst.getOpcode()).hex()  # Get opcode in hex format
    disasm_str = inst.getDisassembly()

    logfile.write(f"Instruction Seq: {seq_num}\n")
    logfile.write(f"Instruction Address: 0x{addr:X}\n")
    logfile.write(f"Opcode (hex): {raw_opcode}\n")
    logfile.write(f"Disassembly: {disasm_str}\n")

    # Dump registers RAX..R15
    reg_names = ["rax","rbx","rcx","rdx","rsi","rdi","rbp","rsp","r8","r9","r10","r11","r12","r13","r14","r15"]
    for rname in reg_names:
        reg_obj = getattr(triton_ctx.registers, rname)
        reg_val = triton_ctx.getConcreteRegisterValue(reg_obj)
        logfile.write(f"{rname.upper()}: 0x{reg_val:016X}\n")

    # Dump memory reads captured for this instruction
    if mem_read_list:
        for (mem_seq, read_addr, read_val) in mem_read_list:
            if mem_seq == seq_num:  # Only log reads for this instruction
                logfile.write(f"Memory Read at 0x{read_addr:X}, value = {read_val.hex()}\n")

    logfile.write("\n")  # Blank line separator
    logfile.flush()

def emulate_and_inspect(binary_path, rootfs_path):
    global triton_ctx, last_inst, logfile, seq_num

    # Reinitialize Triton (optional)
    triton_ctx = TritonContext()
    triton_ctx.setArchitecture(ARCH.X86_64)

    # Open the output file only if logging is enabled
    if LOGGING_ENABLED:
        logfile = open("./output", "w")

    # Initialize Qiling
    ql = Qiling([binary_path], rootfs_path, verbose=QL_VERBOSE.DISABLED)

    # Sync registers before starting
    sync_registers(ql, triton_ctx)

    # Hook memory reads
    ql.hook_mem_read(sync_memory_read)

    # Hook code
    ql.hook_code(hook_code)

    # Start emulation
    ql.run()

    # Process the last instruction if not yet handled
    if last_inst is not None:
        sync_registers(ql, triton_ctx)
        triton_ctx.disassembly(last_inst)
        triton_ctx.processing(last_inst)
        
        if LOGGING_ENABLED:
            log_instruction(last_inst, seq_num)
            mem_read_list.clear()

        last_inst = None

    # Close logfile if logging was enabled
    if LOGGING_ENABLED:
        logfile.close()

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <binary>")
        sys.exit(1)

    binary_path = sys.argv[1]
    rootfs_path = "/root/packages/rootfs/x8664_linux"
    emulate_and_inspect(binary_path, rootfs_path)
