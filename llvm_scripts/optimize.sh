#!/bin/bash
# run_optimize.sh

# Optimize the LLVM IR with -S (human-readable) and -O3 optimizations.
opt -S -O3 devirt.ll -o devirt_opt.ll

# Display the optimized LLVM IR.
cat devirt_opt.ll

#: <<'END'
# llc -c devirt_func.bc -o devirt_func.s
# gcc -c devirt_func.s -o devirt_func.o
# gcc devirt_func.o -o devirt_func
#END
