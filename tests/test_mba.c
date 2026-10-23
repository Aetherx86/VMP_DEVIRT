#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>

/**
 * Test Case 1: Mixed Boolean-Arithmetic (MBA) Obfuscated Computation
 * 
 * Target Ground Truth Logic:
 *   canonical_secret(x, y) = (int8_t)((x ^ 92) + y)
 * 
 * Obfuscation:
 *   Extracted from triton_script/vmp_binaries/samples-source/sample3.c.
 *   Synthesized with 7 layers of affine permutations, bitwise masking,
 *   and polynomial expansion over Z_{2^8}.
 */

int8_t canonical_secret(int8_t x, int8_t y) {
    return (int8_t)((x ^ 92) + y);
}

int8_t obfuscated_mba_secret(int8_t x, int8_t y) {
    int a = 229 * (int)x + 247;
    int b = 237 * a + 214 + ((38 * a + 85) & 254);
    int c = (b + ((-(2 * b) + 255) & 254)) * 3 + 77;
    int d = ((86 * c + 36) & 70) * 75 + 231 * c + 118;
    int e = ((58 * d + 175) & 244) + 99 * d + 46;
    int f = (e & 148);
    int g = (f - (e & 255) + f) * 103 + 13;
    int r = (237 * (45 * g + (174 * g | 34) * 229 + 194 - 247) & 255) + (int)y;
    return (int8_t)r;
}

int main(int argc, char* argv[]) {
    int mismatches = 0;
    for (int x = -128; x <= 127; x++) {
        for (int y = -128; y <= 127; y++) {
            int8_t c = canonical_secret((int8_t)x, (int8_t)y);
            int8_t o = obfuscated_mba_secret((int8_t)x, (int8_t)y);
            if (c != o) {
                mismatches++;
            }
        }
    }
    if (mismatches == 0) {
        printf("[SUCCESS] All 65,536 input combinations proved 100%% functionally identical!\n");
        return 0;
    } else {
        printf("[FAILURE] Found %d functional mismatches!\n", mismatches);
        return 1;
    }
}
