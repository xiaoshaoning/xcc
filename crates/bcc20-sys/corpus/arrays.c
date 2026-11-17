/* Arrays and pointers: the two compilers use different models (16-bit far vs 64-bit flat), so
 * this file is about the *shape* of the code both emit: indexed loads and stores, and a walk. */
int sum_array(void) {
    int values[8];
    int i;
    int total = 0;
    for (i = 0; i < 8; i = i + 1) {
        values[i] = i * i;
    }
    for (i = 0; i < 8; i = i + 1) {
        total = total + values[i];
    }
    return total;
}

int walk(int *p, int n) {
    int total = 0;
    while (n > 0) {
        total = total + *p;
        p = p + 1;
        n = n - 1;
    }
    return total;
}

int write_through(int *p, int n) {
    int i;
    for (i = 0; i < n; i = i + 1) {
        *(p + i) = i;
    }
    return n;
}

int two_dim(void) {
    int grid[3][4];
    int i, j, total = 0;
    for (i = 0; i < 3; i = i + 1) {
        for (j = 0; j < 4; j = j + 1) {
            grid[i][j] = i * 4 + j;
        }
    }
    for (i = 0; i < 3; i = i + 1) {
        for (j = 0; j < 4; j = j + 1) {
            total = total + grid[i][j];
        }
    }
    return total;
}
