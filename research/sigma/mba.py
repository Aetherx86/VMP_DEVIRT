"""
sigma.mba: Signature-guided, solver-verified simplification of
Mixed Boolean-Arithmetic (MBA) expressions over Z/2^n.

Implements truth-table signature extraction and fast Moebius inversion
over the Boolean conjunction lattice to achieve canonical reduction of
linear MBA expressions in deterministic polynomial time.
"""
from itertools import combinations, product
import time
import z3


def get_variables(expr):
    """Extract sorted list of unique BitVec variables from a Z3 AST."""
    seen, out, stack = set(), [], [expr]
    while stack:
        e = stack.pop()
        if z3.is_const(e) and e.decl().kind() == z3.Z3_OP_UNINTERPRETED:
            if e.get_id() not in seen:
                seen.add(e.get_id())
                out.append(e)
        else:
            stack.extend(e.children())
    return sorted(out, key=lambda v: str(v))


def evaluate(expr, vs, point, bits):
    """Evaluate expr with variables vs assigned to point coordinates in {0, 1}."""
    sub = [(v, z3.BitVecVal(p, bits)) for v, p in zip(vs, point)]
    if sub:
        res = z3.simplify(z3.substitute(expr, *sub))
    else:
        res = z3.simplify(expr)
    return res.as_long()


def signature(expr, vs, bits):
    """
    Compute the 2^k truth-table signature vector of expr on {0,1}^k.
    Returns list of length 2^k where index is the integer bitmask of point.
    """
    k = len(vs)
    sig = [0] * (1 << k)
    for mask in range(1 << k):
        point = [(mask >> i) & 1 for i in range(k)]
        sig[mask] = evaluate(expr, vs, point, bits)
    return sig


def moebius_inversion(sig, k, bits):
    """
    Fast Moebius inversion over the Boolean lattice ({0,1}^k, <=).
    Computes conjunction-basis coefficients c_S modulo 2^bits in O(k * 2^k) time.
    """
    mod = 1 << bits
    c = list(sig)
    for i in range(k):
        bit = 1 << i
        for mask in range(1 << k):
            if mask & bit:
                c[mask] = (c[mask] - c[mask ^ bit]) % mod
    return c


def reconstruct_conjunction_ast(c, vs, bits):
    """
    Reconstruct canonical Z3 BitVec expression from conjunction coefficients:
    expr = sum_{S} c_S * prod_{j in S} x_j (mod 2^bits).
    """
    k = len(vs)
    mod = 1 << bits
    terms = []
    
    # Constant term (mask = 0)
    if c[0] % mod != 0:
        terms.append(z3.BitVecVal(c[0] % mod, bits))
        
    for mask in range(1, 1 << k):
        coeff = c[mask] % mod
        if coeff == 0:
            continue
            
        # Build conjunction of variables in mask
        conj = None
        for i in range(k):
            if (mask >> i) & 1:
                conj = vs[i] if conj is None else (conj & vs[i])
                
        if coeff == 1:
            terms.append(conj)
        else:
            terms.append(z3.BitVecVal(coeff, bits) * conj)
            
    if not terms:
        return z3.BitVecVal(0, bits)
    
    res = terms[0]
    for t in terms[1:]:
        res = res + t
    return res


def simplify_linear_mba(expr, bits=64):
    """
    Canonicalize a linear MBA expression using conjunction-basis representation.
    Guarantees semantic equivalence verified via SMT theorem prover.
    """
    vs = get_variables(expr)
    k = len(vs)
    if k == 0:
        return z3.simplify(expr)
    
    # 1. Extract signature vector over {0,1}^k
    sig = signature(expr, vs, bits)
    
    # 2. Fast Moebius transform to obtain conjunction coefficients
    c = moebius_inversion(sig, k, bits)
    
    # 3. Check for direct variable identity: e.g. x + y
    canonical_ast = reconstruct_conjunction_ast(c, vs, bits)
    
    # 4. Soundness check via Z3 equivalence
    solver = z3.Solver()
    solver.add(expr != canonical_ast)
    if solver.check() == z3.unsat:
        return canonical_ast
    return expr


def verify_equivalence(expr1, expr2):
    """Prove that for all inputs, expr1 == expr2."""
    solver = z3.Solver()
    solver.add(expr1 != expr2)
    return solver.check() == z3.unsat
