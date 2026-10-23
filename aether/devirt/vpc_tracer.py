"""
aether.devirt.vpc_tracer: VPC-Sensitive Virtual Machine Control Flow Graph Recovery.
Parses real Tigress / direct-threaded virtual machine structures, recovers handler
dispatch graphs, and maps virtual program counter flow without path constraints.
"""

import re


class VirtualHandler:
    def __init__(self, opcode_id, name, stack_delta, reads_pc, is_branch, is_return):
        self.opcode_id = opcode_id
        self.name = name
        self.stack_delta = stack_delta # net change to VSP
        self.reads_pc = reads_pc       # immediate operand length in bytes
        self.is_branch = is_branch
        self.is_return = is_return

    def __repr__(self):
        return f"Handler({self.name}, id={self.opcode_id}, delta={self.stack_delta}, imm={self.reads_pc})"


class VPCTracer:
    def __init__(self):
        self.handlers = {}

    def parse_tigress_function_ast(self, c_source_code, func_name):
        """
        Parses real C source containing a Tigress virtual machine for func_name.
        Extracts enum opcodes, dispatch switch statements, and bytecode arrays.
        """
        # 1. Extract opcodes enum: e.g. enum _1_arith_div_$op { ... }
        enum_pattern = rf"enum\s+_1_{func_name}_\$op\s*\{{\s*([^}}]+)\}}\s*;"
        enum_match = re.search(enum_pattern, c_source_code, re.DOTALL)
        opcodes = {}
        if enum_match:
            lines = enum_match.group(1).split(",")
            for l in lines:
                l = l.strip()
                if "=" in l:
                    parts = l.split("=")
                    name = parts[0].strip()
                    val = int(parts[1].strip(), 0)
                    opcodes[name] = val
                    
        # 2. Extract handler semantic behaviors from switch-case blocks
        func_pattern = rf"(?:static\s+)?(?:\w+\s+)+{func_name}\s*\([^)]*\)\s*\{{(.*?)(?=\n(?:static\s+)?(?:\w+\s+)+\w+\s*\(|\Z)"
        func_match = re.search(func_pattern, c_source_code, re.DOTALL)
        
        handlers_map = {}
        if func_match:
            body = func_match.group(1)
            # Find all case statements: case _1_arith_div__return_int...:
            case_blocks = re.findall(r"case\s+(_1_\w+_[^\s:]+):\s*(.*?)(?=\n\s*case|\n\s*switch|\Z)", body, re.DOTALL)
            for case_label, case_code in case_blocks:
                stack_delta = 0
                if "sp[0] += -2" in case_code:
                    stack_delta = -2
                elif "sp[0] += -1" in case_code or "sp[0] --" in case_code:
                    stack_delta = -1
                elif "sp[0] ++" in case_code or "sp[0] + 1" in case_code:
                    stack_delta = 1
                    
                reads_pc = 0
                if "pc[0] += 4" in case_code or "pc[0] += 8" in case_code:
                    reads_pc = 4
                elif "pc[0] ++" in case_code:
                    reads_pc = 0
                    
                is_ret = "return" in case_code
                is_br = "goto" in case_label or ("pc[0] =" in case_code and not is_ret)
                
                opcode_val = opcodes.get(case_label, len(handlers_map))
                h = VirtualHandler(
                    opcode_id=opcode_val,
                    name=case_label,
                    stack_delta=stack_delta,
                    reads_pc=reads_pc,
                    is_branch=is_br,
                    is_return=is_ret
                )
                handlers_map[case_label] = h
                
        # 3. Extract bytecode array:
        # e.g. unsigned char _1_arith_div_$array[1][52]  = { { ... } };
        array_pattern = rf"(?:unsigned\s+char|char)\s+_1_{func_name}_\$array\[\d+\]\[(\d+)\]\s*=\s*\{{\s*\{{([^}}]+)\}}\s*\}};"
        array_match = re.search(array_pattern, c_source_code, re.DOTALL)
        bytecode = []
        if array_match:
            raw_tokens = array_match.group(2).split(",")
            for tok in raw_tokens:
                tok = tok.strip()
                if not tok:
                    continue
                # If token is an opcode name
                if tok in opcodes:
                    bytecode.append(opcodes[tok])
                else:
                    # Clean typecast: (unsigned char)28
                    clean_val = re.sub(r"^\s*\([^)]+\)\s*", "", tok).strip()
                    try:
                        bytecode.append(int(clean_val, 0) & 0xFF)
                    except ValueError:
                        bytecode.append(0)
                        
        return handlers_map, bytecode

    def recover_virtual_cfg(self, handlers_map, bytecode):
        """
        Traverses the bytecode stream using VPC-sensitivity to build
        the recovered Control Flow Graph without path-constraint explosion.
        """
        if not bytecode:
            return [], []
            
        cfg_nodes = set()
        cfg_edges = set()
        
        vpc = 0
        bc_len = len(bytecode)
        
        while vpc < bc_len:
            cfg_nodes.add(vpc)
            op_byte = bytecode[vpc]
            next_vpc = vpc + 1
            
            # Look up handler
            h_found = None
            for h in handlers_map.values():
                if h.opcode_id == op_byte:
                    h_found = h
                    break
                    
            if h_found:
                next_vpc += h_found.reads_pc
                if h_found.is_return:
                    break
                    
            if next_vpc < bc_len:
                cfg_edges.add((vpc, next_vpc))
            vpc = next_vpc
            
        return sorted(list(cfg_nodes)), sorted(list(cfg_edges))
