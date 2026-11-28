#include <stdio.h>
#include <stdint.h>
#include <math.h>

typedef int32_t q31_t;
#define Q31_MAX   (0x7FFFFFFF)
#define Q31_MIN   (-2147483647 - 1)

q31_t q31_add(q31_t a, q31_t b) {
    int64_t sum = (int64_t)a + (int64_t)b;
    if (sum > Q31_MAX) return Q31_MAX;
    if (sum < Q31_MIN) return Q31_MIN;
    return (q31_t)sum;
}

q31_t q31_sub(q31_t a, q31_t b) {
    int64_t diff = (int64_t)a - (int64_t)b;
    if (diff > Q31_MAX) return Q31_MAX;
    if (diff < Q31_MIN) return Q31_MIN;
    return (q31_t)diff;
}

q31_t q31_mul(q31_t a, q31_t b) {
    int64_t product = (int64_t)a * (int64_t)b;
    product += (1 << 30);
    product >>= 31;
    if (product > Q31_MAX) return Q31_MAX;
    if (product < Q31_MIN) return Q31_MIN;
    return (q31_t)product;
}

float q31_to_float(q31_t q) {
    return (float)q / (float)(1LL << 31);
}

float q30_to_float(q31_t q) {
    // Q30: 2 sign bits, 30 fractional bits, range [-2, 2)
    return (float)q / (float)(1LL << 30);
}

q31_t q31_isqrt(q31_t x)
{
    // Handle zero and negative inputs
    if (x <= 0) {
        return Q31_MAX;  // Approximate infinity with maximum value
    }

    // We'll compute y = 1/√x in Q30 format
    // Input x is Q31, output y is Q30 (stored in q31_t)

    // Initial guess: y0 ≈ 1/√x
    // For x in (0, 1], 1/√x ≥ 1, so in Q30, y0 ≥ 0x40000000 (1.0)
    // Simple guess: y0 = 1.0 in Q30 = 0x40000000
    // Better: use magic number approximation (like fast inverse square root)

    // Convert x to Q30 for computation: x_q30 = x >> 1
    q31_t x_q30 = x >> 1;  // x in Q30 (loses 1 bit of precision)

    // Initial guess using magic number (similar to fast inverse square root)
    // For floating-point: i = 0x5f3759df - (i >> 1);
    // For Q30 fixed-point, we need different magic number
    q31_t y = 0x40000000;  // 1.0 in Q30 as initial guess

    printf("DEBUG: x=0x%08x (%.6f Q31), x_q30=0x%08x, y0=0x%08x (%.6f Q30)\n",
           x, q31_to_float(x), x_q30, y, q30_to_float(y));

    // Newton-Raphson iteration: y_{n+1} = y_n * (1.5 - 0.5 * x * y_n^2)
    // All in Q30 format
    for (int i = 0; i < 4; i++) {  // Increased iterations
        // Compute y^2 in Q60 (Q30 * Q30)
        int64_t y2_64 = (int64_t)y * (int64_t)y;  // Q60

        // Compute x * y^2: x_q30 (Q30) * y2_64 (Q60) = Q90
        // We need 0.5 * x * y^2 in Q30
        // Scale: (x_q30 * y2_64) / 2^60 gives Q30
        int64_t x_y2_64 = (int64_t)x_q30 * y2_64;  // Q90
        // Divide by 2^60 to get Q30, then by 2 for 0.5 factor
        // So shift right 61
        int64_t half_x_y2_64 = x_y2_64 >> 61;  // Q90 >> 61 = Q29, but we want Q30?
        // Actually: x_q30 * y2_64 / 2^60 = value in Q30
        // Then 0.5 * that = /2 = shift right 1 more
        // Total shift: 61

        // Convert to Q30 (might need scaling)
        q31_t half_x_y2 = (q31_t)half_x_y2_64;

        printf("  Iter %d: y=0x%08x (%.6f Q30), y2_64=%lld (Q60), x_y2_64=%lld (Q90), half_x_y2=0x%08x\n",
               i, y, q30_to_float(y), y2_64, x_y2_64, half_x_y2);

        // Compute 1.5 - 0.5 * x * y^2
        // 1.5 in Q30 is 0x60000000
        q31_t one_point_five = 0x60000000;  // 1.5 in Q30
        q31_t term = q31_sub(one_point_five, half_x_y2);

        printf("  Iter %d: one_point_five=0x%08x, term=0x%08x (%.6f Q30)\n",
               i, one_point_five, term, q30_to_float(term));

        // Update: y = y * term
        // y (Q30) * term (Q30) = Q60, scale to Q30: >> 30
        int64_t y_new_64 = (int64_t)y * (int64_t)term;  // Q60
        y_new_64 >>= 30;  // Q60 >> 30 = Q30
        // Add rounding
        y_new_64 += (1 << 29);
        y_new_64 >>= 1;  // Equivalent to adding 0.5 before shift

        q31_t y_new = (q31_t)y_new_64;
        if (y_new > 0x7fffffff) y_new = 0x7fffffff;  // Clamp to Q30 positive range
        if (y_new < (q31_t)0x80000000) y_new = (q31_t)0x80000000;

        printf("  Iter %d: y_new=0x%08x (%.6f Q30)\n", i, y_new, q30_to_float(y_new));
        y = y_new;
    }

    // Result is in Q30 format
    return y;
}

int main() {
    printf("Testing isqrt(0.25):\n");
    q31_t quarter = 0x20000000; // 0.25 in Q31
    q31_t result = q31_isqrt(quarter);
    float expected = 1.0f / sqrtf(0.25f); // 2.0
    printf("Result: 0x%08x = %.6f Q31 = %.6f Q30 (expected: %.6f, error: %.6f)\n\n",
           result, q31_to_float(result), q30_to_float(result), expected, q30_to_float(result) - expected);

    printf("Testing isqrt(1.0):\n");
    result = q31_isqrt(Q31_MAX);
    expected = 1.0f / sqrtf(1.0f); // 1.0
    printf("Result: 0x%08x = %.6f Q31 = %.6f Q30 (expected: %.6f, error: %.6f)\n\n",
           result, q31_to_float(result), q30_to_float(result), expected, q30_to_float(result) - expected);

    printf("Testing isqrt(0.64):\n");
    q31_t point64 = (q31_t)(0.64f * (1LL << 31)); // 0.64 in Q31
    result = q31_isqrt(point64);
    expected = 1.0f / sqrtf(0.64f); // 1.25
    printf("Result: 0x%08x = %.6f Q31 = %.6f Q30 (expected: %.6f, error: %.6f)\n",
           result, q31_to_float(result), q30_to_float(result), expected, q30_to_float(result) - expected);

    return 0;
}