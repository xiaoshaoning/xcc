/* Six files for the measuring stick: both compilers build every one of them.
 *
 * Freestanding on purpose - no `#include` - because Borland C++ 2.0's headers come with the
 * product and are not in this repository. That is also why the corpus is small and plain: it is
 * here to be compiled twice, not to be interesting.
 */
int add(int a, int b) { return a + b; }
int sub(int a, int b) { return a - b; }
int mul(int a, int b) { return a * b; }
int divi(int a, int b) { return b == 0 ? 0 : a / b; }
int mod(int a, int b) { return b == 0 ? 0 : a % b; }

int mixed(int a, int b) {
    int r = a;
    r = r + b;
    r = r - (b * 2);
    r = r * 3;
    r = r / 4;
    r = r % 100;
    return r;
}

int shifts(int a, int b) {
    return (a << 3) + (a >> 1) + (b << 2) - (b >> 3);
}

int bits(int a, int b) {
    return (a & b) | (a ^ b) | (~a & 0xF0);
}

int compare(int a, int b) {
    int score = 0;
    if (a < b) score = score + 1;
    if (a <= b) score = score + 2;
    if (a == b) score = score + 4;
    if (a != b) score = score + 8;
    if (a > b) score = score + 16;
    if (a >= b) score = score + 32;
    return score;
}
