/* Loops and branches: one of the six the measuring stick compiles both ways. */
int sum_to(int n) {
    int i;
    int total = 0;
    for (i = 1; i <= n; i = i + 1) {
        total = total + i;
    }
    return total;
}

int sum_while(int n) {
    int total = 0;
    while (n > 0) {
        total = total + n;
        n = n - 1;
    }
    return total;
}

int sum_do(int n) {
    int total = 0;
    do {
        total = total + n;
        n = n - 1;
    } while (n > 0);
    return total;
}

int count_bits(int value) {
    int bits = 0;
    while (value != 0) {
        bits = bits + (value & 1);
        value = value >> 1;
    }
    return bits;
}

int collatz(int n) {
    int steps = 0;
    while (n != 1 && steps < 1000) {
        if (n % 2 == 0) {
            n = n / 2;
        } else {
            n = 3 * n + 1;
        }
        steps = steps + 1;
    }
    return steps;
}
