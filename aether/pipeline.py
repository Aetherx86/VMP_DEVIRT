"""
aether.pipeline: Unified AETHER-64 Deobfuscation Pipeline.
Integrates VPC-Sensitive Symbolic Emulation, the Algebraic Pre-Reduction (APR) Filter,
Conjunction-Basis Moebius Inversion, MCMC Non-Linear Synthesis, and LLVM SSA Lifting.
"""

import time
import z3
from aether.mba.linear import canonicalize_linear_mba
from aether.mba.synthesizer import NonLinearMBASynthesizer
from aether.devirt.vpc_tracer import VPCTracer
from aether.devirt.stack_elim import VirtualStackEliminator
from aether.apr.filter import AlgebraicPreReductionFilter


class AetherPipeline:
    def __init__(self):
        self.tracer = VPCTracer()
        self.stack_eliminator = VirtualStackEliminator()
        self.apr_filter = AlgebraicPreReductionFilter()
        self.synthesizer = NonLinearMBASynthesizer(bits=64)

    def deobfuscate_linear_expression(self, expr):
        """Simplifies arbitrary linear MBA expressions using conjunction basis Moebius transform."""
        return canonicalize_linear_mba(expr, bits=64)

    def deobfuscate_nonlinear_expression(self, expr, bits=64):
        """Synthesizes minimal native expressions for arbitrary polynomial MBAs."""
        synth = NonLinearMBASynthesizer(bits=bits)
        return synth.synthesize(expr)

    def devirtualize_tigress_function(self, c_source, func_name):
        """
        End-to-End Devirtualization of a real Tigress virtual machine function:
        Recovers CFG, eliminates virtual stack, and outputs clean SSA statements.
        """
        t0 = time.time()
        handlers, bytecode = self.tracer.parse_tigress_function_ast(c_source, func_name)
        nodes, edges = self.tracer.recover_virtual_cfg(handlers, bytecode)
        ssa_ir = self.stack_eliminator.lift_handlers_to_ssa(bytecode, handlers)
        elapsed = (time.time() - t0) * 1000
        
        return {
            "function_name": func_name,
            "handlers_discovered": len(handlers),
            "bytecode_size": len(bytecode),
            "recovered_cfg_nodes": len(nodes),
            "recovered_cfg_edges": len(edges),
            "ssa_statements_count": len(ssa_ir),
            "ssa_statements": [str(s) for s in ssa_ir[:10]],
            "latency_ms": elapsed
        }
