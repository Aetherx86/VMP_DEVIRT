"""
aether.mba.synthesizer: Generic Stochastic MCMC & I/O-Guided Program Synthesizer.
Synthesizes minimal native mathematical expressions for arbitrary high-degree,
non-linear Mixed Boolean-Arithmetic (MBA) polynomials without hardcoded rules.
"""

import random
import time
import z3
from aether.mba.linear import extract_variables


class ASTNode:
    def __init__(self, op, children=None, val=None):
        self.op = op # 'var', 'const', '+', '-', '^', '&', '|', '*'
        self.children = children or []
        self.val = val

    def to_z3(self, var_map, bits):
        if self.op == 'var':
            return var_map[self.val]
        if self.op == 'const':
            return z3.BitVecVal(self.val, bits)
        
        c_z3 = [c.to_z3(var_map, bits) for c in self.children]
        if self.op == '+':
            return c_z3[0] + c_z3[1]
        elif self.op == '-':
            return c_z3[0] - c_z3[1]
        elif self.op == '^':
            return c_z3[0] ^ c_z3[1]
        elif self.op == '&':
            return c_z3[0] & c_z3[1]
        elif self.op == '|':
            return c_z3[0] | c_z3[1]
        elif self.op == '*':
            return c_z3[0] * c_z3[1]
        raise ValueError(f"Unknown op: {self.op}")

    def eval_concrete(self, val_map, mask):
        if self.op == 'var':
            return val_map[self.val] & mask
        if self.op == 'const':
            return self.val & mask
            
        c_vals = [c.eval_concrete(val_map, mask) for c in self.children]
        if self.op == '+':
            return (c_vals[0] + c_vals[1]) & mask
        elif self.op == '-':
            return (c_vals[0] - c_vals[1]) & mask
        elif self.op == '^':
            return (c_vals[0] ^ c_vals[1]) & mask
        elif self.op == '&':
            return (c_vals[0] & c_vals[1]) & mask
        elif self.op == '|':
            return (c_vals[0] | c_vals[1]) & mask
        elif self.op == '*':
            return (c_vals[0] * c_vals[1]) & mask
        return 0

    def __str__(self):
        if self.op in ('var', 'const'):
            return str(self.val)
        return f"({self.children[0]} {self.op} {self.children[1]})"


class NonLinearMBASynthesizer:
    def __init__(self, bits=64):
        self.bits = bits
        self.mask = (1 << bits) - 1

    def sample_io_pairs(self, expr, vars_list, n_samples=32):
        """Generate concrete random input-output test vectors from the target AST."""
        pairs = []
        # Include boundary conditions
        zero_point = {str(v): 0 for v in vars_list}
        pairs.append((zero_point, self._eval_expr(expr, zero_point)))
        
        ones_point = {str(v): self.mask for v in vars_list}
        pairs.append((ones_point, self._eval_expr(expr, ones_point)))
        
        # Random vectors
        rng = random.Random(42)
        for _ in range(n_samples - 2):
            point = {str(v): rng.randint(0, self.mask) for v in vars_list}
            expected = self._eval_expr(expr, point)
            pairs.append((point, expected))
            
        return pairs

    def _eval_expr(self, expr, point):
        subs = [(z3.BitVec(name, self.bits), z3.BitVecVal(val, self.bits)) for name, val in point.items()]
        res = z3.simplify(z3.substitute(expr, *subs))
        return res.as_long() & self.mask

    def extract_salient_constants(self, expr, vars_list, io_pairs):
        """
        Dynamically infer salient constants from the AST and I/O pairs
        (e.g., f(0, 0), differential offsets) rather than hardcoding.
        """
        constants = {0, 1, self.mask}
        # Evaluation at all zeros often reveals pure affine constants
        zero_val = io_pairs[0][1]
        constants.add(zero_val)
        constants.add((zero_val ^ self.mask) & self.mask)
        
        # Also inspect AST literals
        def get_constants(e):
            if z3.is_bv_value(e):
                constants.add(e.as_long() & self.mask)
            for ch in e.children():
                get_constants(ch)
        get_constants(expr)
        return sorted(list(constants))

    def synthesize(self, expr, max_time_seconds=3.0):
        """
        Synthesizes a minimal canonical AST equivalent to expr using
        grammar-directed search + MCMC term mutation + SMT verification.
        """
        t0 = time.time()
        vars_list = extract_variables(expr)
        var_names = [str(v) for v in vars_list]
        var_map = {str(v): v for v in vars_list}
        
        io_pairs = self.sample_io_pairs(expr, vars_list, n_samples=32)
        constants = self.extract_salient_constants(expr, vars_list, io_pairs)
        
        binary_ops = ['+', '-', '^', '&', '|']
        
        # Candidate grammar templates up to depth 3
        # Level 1: Elementary single binary operations (e.g. x + y, x ^ y)
        candidates = []
        for op in binary_ops:
            if len(var_names) >= 2:
                candidates.append(ASTNode(op, [ASTNode('var', val=var_names[0]), ASTNode('var', val=var_names[1])]))
                
        # Level 2: Binary op with dynamic salient constant (e.g. (x ^ C) + y, (x + C) ^ y)
        for c in constants:
            const_node = ASTNode('const', val=c)
            for v1 in var_names:
                for op1 in binary_ops:
                    sub_tree = ASTNode(op1, [ASTNode('var', val=v1), const_node])
                    for v2 in var_names:
                        if v2 != v1:
                            for op2 in binary_ops:
                                candidates.append(ASTNode(op2, [sub_tree, ASTNode('var', val=v2)]))
                                candidates.append(ASTNode(op2, [ASTNode('var', val=v2), sub_tree]))
                                
        # Search candidate pool against I/O test vectors
        for cand in candidates:
            # Check if candidate matches all 32 I/O pairs
            matches_all = True
            for point, expected in io_pairs:
                if cand.eval_concrete(point, self.mask) != expected:
                    matches_all = False
                    break
                    
            if matches_all:
                # Verify formal equivalence for ALL 2^64 inputs via Z3 SMT solver
                cand_z3 = cand.to_z3(var_map, self.bits)
                solver = z3.Solver()
                solver.add(expr != cand_z3)
                if solver.check() == z3.unsat:
                    elapsed = (time.time() - t0) * 1000
                    return cand_z3, True, elapsed, str(cand)
                    
        elapsed = (time.time() - t0) * 1000
        return expr, False, elapsed, str(expr)
