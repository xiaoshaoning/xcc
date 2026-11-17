/* Storage that outlives a call: a global, a static local, and a function that keeps a tally. */
int counter;
int seed = 7;
static int hidden = 11;

int bump(void) {
    counter = counter + 1;
    return counter;
}

int tally(void) {
    static int calls = 0;
    calls = calls + 1;
    return calls;
}

int next_value(void) {
    seed = seed * 1103515245 + 12345;
    return (seed >> 16) & 0x7FFF;
}

int read_hidden(void) {
    return hidden;
}

int accumulate(int value) {
    static int total = 100;
    total = total + value;
    return total;
}
