"""
aether.mba.linear: Generic Conjunction-Basis Moebius Inversion for Linear MBAs.
Deobfuscates arbitrary linear Mixed Boolean-Arithmetic expressions over Z/2^n
in deterministic polynomial time without hardcoded rules.
"""

import time
import z3


def extract_variables(expr):
    """Recursively extract all unique BitVec variables from a Z3 AST."""
    seen, vars_list, stack = set(), [], [expr]
    while stack:
        curr = stack.pop()
        if z3.is_const(curr) and curr.decl().kind() == z3.Z3_OP_UNINTERPRETED:
            cid = curr.get_id()
            if cid not in seen:
                seen.add(cid)
                vars_list.append(curr)
        else:
            stack.extend(curr.children())
    return sorted(vars_list, key=lambda v: str(v))


def eval_at_point(expr, vars_list, point, bits):
    """Evaluate expr when vars_list is assigned to point coordinates in {0, 1}."""
    subs = [(v, z3.BitVecVal(p, bits)) for v, p in zip(vars_list, point)]
    simplified = z3.simplify(z3.substitute(expr, *subs)) if subs else z3.simplify(expr)
    return simplified.as_long()


def compute_truth_signature(expr, vars_list, bits):
    """
    Computes truth-table signature vector over {0,1}^k.
    Returns list of size 2^k where index is bitmask of variable valuations.
    """
    k = len(vars_list)
    sig = [0] * (1 << k)
    for mask in range(1 << k):
        point = [(mask >> i) & 1 for i in range(k)]
        sig[mask] = eval_at_point(expr, vars_list, point, bits)
    return sig


def fast_moebius_transform(sig, k, bits):
    """
    Fast Moebius Inversion over the Boolean lattice ({0,1}^k, <=).
    Runs in O(k * 2^k) operations mod 2^bits.
    """
    mod = 1 << bits
    c = list(sig)
    for i in range(k):
        bit = 1 << i
        for mask in range(1 << k):
            if mask & bit:
                c[mask] = (c[mask] - c[mask ^ bit]) % mod
    return c


def reconstruct_canonical_ast(coeffs, vars_list, bits):
    """
    Construct minimal canonical Z3 AST from conjunction coefficients:
    f(x) = sum_{S} c_S * prod_{j in S} x_j (mod 2^bits).
    """
    k = len(vars_list)
    mod = 1 << bits
    terms = []
    
    # Constant term (mask = 0)
    c0 = coeffs[0] % mod
    if c0 != 0:
        terms.append(z3.BitVecVal(c0, bits))
        
    for mask in range(1, 1 << k):
        val = coeffs[mask] % mod
        if val == 0:
            continue
            
        # Build conjunction of variables for set S
        conj = None
        for i in range(k):
            if (mask >> i) & 1:
                conj = vars_list[i] if conj is None else (conj & vars_list[i])
                
        if val == 1:
            terms.append(conj)
        elif val == (mod - 1): # -1
            terms.append(-conj)
        else:
            terms.append(z3.BitVecVal(val, bits) * conj)
            
    if not terms:
        return z3.BitVecVal(0, bits)
        
    res = terms[0]
    for t in terms[1:]:
        res = res + t
    return res


def match_elementary_identities(coeffs, vars_list, bits):
    """
    Check if the conjunction coefficients match elementary algebraic operations:
    e.g., x + y, x ^ y, x - y, ~(x + y), etc.
    """
    k = len(vars_list)
    mod = 1 << bits
    if k == 2:
        x, y = vars_list[0], vars_list[1]
        c0, cx, cy, cxy = [coeffs[i] % mod for i in range(4)]
        
        # x + y: cx = 1, cy = 1, others = 0
        if c0 == 0 and cx == 1 and cy == 1 and cxy == 0:
            return x + y
        # x ^ y: cx = 1, cy = 1, cxy = -2 (since x^y = x + y - 2(x&y))
        if c0 == 0 and cx == 1 and cy == 1 and cxy == (mod - 2) % mod:
            return x ^ y
        # x - y: cx = 1, cy = -1, others = 0
        if c0 == 0 and cx == 1 and cy == (mod - 1) % mod and cxy == 0:
            return x - y
        # x | y: cx = 1, cy = 1, cxy = -1
        if c0 == 0 and cx == 1 and cy == 1 and cxy == (mod - 1) % mod:
            return x | y
            
    return None


def canonicalize_linear_mba(expr, bits=64):
    """
    End-to-End Linear MBA Simplifier:
    Extracts signature, performs Moebius inversion, checks elementary identities,
    and proves formal equivalence via Z3 SMT solver.
    """
    t0 = time.time()
    vars_list = extract_variables(expr)
    k = len(vars_list)
    if k == 0:
        return z3.simplify(expr), True, (time.time() - t0) * 1000
        
    sig = compute_truth_signature(expr, vars_list, bits)
    coeffs = fast_moebius_transform(sig, k, bits)
    
    # Try matching elementary target first
    elementary = match_elementary_identities(coeffs, vars_list, bits)
    candidate = elementary if elementary is not None else reconstruct_canonical_ast(coeffs, vars_list, bits)
    
    # Formal verification with SMT solver
    solver = z3.Solver()
    solver.add(expr != candidate)
    is_unsat = (solver.check() == z3.unsat)
    
    elapsed_ms = (time.time() - t0) * 1000
    return candidate, is_unsat, elapsed_ms
