"""
aether.devirt.stack_elim: Virtual Stack Pointer (VSP) Elimination & SSA Transformation.
Eliminates virtual stack push/pop overhead from recovered bytecode sequences,
converting virtual interpreter sequences into minimal SSA statements.
"""

class SSAStatement:
    def __init__(self, target_reg, op, operands):
        self.target_reg = target_reg # e.g. %1, %2
        self.op = op                 # 'load', 'store', 'call', '+', '-', 'ret'
        self.operands = operands     # list of operands

    def __str__(self):
        if self.op == 'ret':
            return f"return {self.operands[0]}"
        elif self.op == 'call':
            args = ", ".join(str(o) for o in self.operands[1:])
            return f"{self.target_reg} = {self.operands[0]}({args})"
        elif self.op in ('+', '-', '^', '&', '|', '*'):
            return f"{self.target_reg} = {self.operands[0]} {self.op} {self.operands[1]}"
        elif self.op == 'load':
            return f"{self.target_reg} = load *{self.operands[0]}"
        elif self.op == 'store':
            return f"store {self.operands[1]} -> *{self.operands[0]}"
        return f"{self.target_reg} = {self.op} {self.operands}"


class VirtualStackEliminator:
    def __init__(self):
        self.reg_counter = 0

    def new_reg(self):
        self.reg_counter += 1
        return f"%v{self.reg_counter}"

    def lift_handlers_to_ssa(self, bytecode, handlers_map):
        """
        Symbolically evaluates virtual stack operations:
        Replaces VSP pushes and pops with direct SSA virtual registers.
        """
        vsp_stack = []
        ssa_statements = []
        
        vpc = 0
        while vpc < len(bytecode):
            op_code = bytecode[vpc]
            vpc += 1
            
            # Find matching handler name
            h_name = "unknown"
            h_obj = None
            for h in handlers_map.values():
                if h.opcode_id == op_code:
                    h_name = h.name
                    h_obj = h
                    break
                    
            if "formal" in h_name:
                reg = self.new_reg()
                ssa_statements.append(SSAStatement(reg, "formal", [f"arg_{op_code}"]))
                vsp_stack.append(reg)
                if h_obj: vpc += h_obj.reads_pc
            elif "local" in h_name:
                reg = self.new_reg()
                ssa_statements.append(SSAStatement(reg, "local", [f"local_offset_{op_code}"]))
                vsp_stack.append(reg)
                if h_obj: vpc += h_obj.reads_pc
            elif "load" in h_name:
                addr = vsp_stack.pop() if vsp_stack else "%ptr"
                reg = self.new_reg()
                ssa_statements.append(SSAStatement(reg, "load", [addr]))
                vsp_stack.append(reg)
            elif "store" in h_name:
                val = vsp_stack.pop() if vsp_stack else "%val"
                dest = vsp_stack.pop() if vsp_stack else "%dest"
                ssa_statements.append(SSAStatement(None, "store", [dest, val]))
            elif "call" in h_name:
                reg = self.new_reg()
                args = [vsp_stack.pop() for _ in range(min(len(vsp_stack), 2))]
                ssa_statements.append(SSAStatement(reg, "call", ["sub_call"] + args))
                vsp_stack.append(reg)
                if h_obj: vpc += h_obj.reads_pc
            elif "return" in h_name:
                ret_val = vsp_stack.pop() if vsp_stack else "0"
                ssa_statements.append(SSAStatement(None, "ret", [ret_val]))
                break
                
        return ssa_statements
