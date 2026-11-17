/* A switch with fall-through, a default in the middle, and negative cases: what a 1991 back end
 * had to lower with compare chains and jump tables, and what our cir lowers too. */
int classify(int kind) {
    int result = 0;
    switch (kind) {
    case 0:
        result = 10;
        break;
    case 1:
        result = 20;
    case 2:
        result = result + 30;
        break;
    default:
        result = -1;
        break;
    case -1:
        result = 99;
        break;
    }
    return result;
}

int sparse(int value) {
    switch (value) {
    case 1: return 1;
    case 2: return 4;
    case 3: return 9;
    case 5: return 25;
    case 7: return 49;
    case 11: return 121;
    case 13: return 169;
    case 17: return 289;
    default: return 0;
    }
}

int dense(int value) {
    switch (value) {
    case 0: return 100;
    case 1: return 101;
    case 2: return 102;
    case 3: return 103;
    case 4: return 104;
    case 5: return 105;
    case 6: return 106;
    case 7: return 107;
    default: return 0;
    }
}
