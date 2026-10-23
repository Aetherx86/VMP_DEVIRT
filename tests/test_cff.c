#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>

/**
 * Test Case 2: Control Flow Flattening (CFF) with Opaque Predicates
 * 
 * Target Ground Truth Logic:
 *   Euclidean Greatest Common Divisor (GCD) algorithm:
 *   gcd(a, b) {
 *     while (b != 0) {
 *       int t = b;
 *       b = a % b;
 *       a = t;
 *     }
 *     return a;
 *   }
 * 
 * Obfuscation:
 *   Chenxi Wang / OLLVM / Tigress style state machine flattening with
 *   an affine state-variable dispatcher and opaque predicates.
 */

uint32_t canonical_gcd(uint32_t a, uint32_t b) {
    while (b != 0) {
        uint32_t t = b;
        b = a % b;
        a = t;
    }
    return a;
}

uint32_t flattened_gcd(uint32_t a, uint32_t b) {
    uint32_t state = 0x1A2B;
    uint32_t t = 0;
    
    // Opaque invariant predicate: for all k, (k * (k + 1)) % 2 == 0
    uint32_t opaque_var = a;
    
    while (state != 0x9999) {
        switch (state) {
            case 0x1A2B: // Header: check loop condition
                // Opaque predicate injection
                if ((opaque_var * (opaque_var + 1)) % 2 == 0) {
                    if (b != 0) {
                        state = 0x3C4D; // Enter loop body
                    } else {
                        state = 0x5E6F; // Exit loop
                    }
                } else {
                    // Dead path that is never taken
                    state = 0xDEAD;
                }
                break;
                
            case 0x3C4D: // Body 1: t = b
                t = b;
                state = 0x7A8B;
                break;
                
            case 0x7A8B: // Body 2: b = a % b; a = t
                b = a % b;
                a = t;
                state = 0x1A2B; // Loop back to header
                break;
                
            case 0x5E6F: // Exit block
                state = 0x9999;
                break;
                
            case 0xDEAD: // Bogus unreachable trap block
                a = a ^ 0xCAFEBABE;
                state = 0x9999;
                break;
                
            default:
                state = 0x9999;
                break;
        }
    }
    return a;
}

int main(int argc, char* argv[]) {
    int mismatches = 0;
    for (uint32_t a = 1; a <= 100; a++) {
        for (uint32_t b = 1; b <= 100; b++) {
            uint32_t c = canonical_gcd(a, b);
            uint32_t f = flattened_gcd(a, b);
            if (c != f) {
                mismatches++;
            }
        }
    }
    if (mismatches == 0) {
        printf("[SUCCESS] Control Flow Flattening test passed 10,000 cases with 100%% semantic parity!\n");
        return 0;
    } else {
        printf("[FAILURE] Found %d mismatches in flattened GCD!\n", mismatches);
        return 1;
    }
}
