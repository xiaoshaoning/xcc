/* cc51_loops.c - the loop shapes that were lowered wrongly, and the folds that lied.
 *
 * Three bugs, each of which changed an answer rather than a message:
 *
 *   - A `for` loop whose update ends in a *join* - `p = c ? a : p->next` - lost its way back to
 *     the test. The step block was consulted instead of the block the cursor was actually in, so
 *     the loop ran once and then fell into a trap: a `ud2` inside `sqlite3.c`'s query planner with
 *     nothing to say which line it was.
 *   - `do { } while(1)` created a branch to an exit no path can take, which made the exit
 *     *reachable* to every reader of the graph - and the "control reaches the end of ..." warning
 *     fired on a function that cannot reach its end.
 *   - The front end's constant folder compared two operands *before* C99 6.3.1.8's usual
 *     arithmetic conversions, so `-1 > 0u` folded to false. A wrong constant used to be branched on
 *     rather than believed; once a constant condition decided the branch, it decided wrongly.
 *
 * Exit 96, and the output is compared with clang byte for byte.
 */
int printf(const char *, ...);

struct link {
  int value;
  struct link *next;
};

static int sum_with_ternary(struct link *p, int stop_at) {
  int total = 0;
  /* the trigger: an update expression that ends in the conditional's join */
  for (; p; p = (p->value == stop_at ? 0 : p->next)) {
    total += p->value;
  }
  return total;
}

static int count_do_while_forever(void) {
  int i = 0;
  do {
    i++;
    if (i == 5) {
      break;
    }
  } while (1);
  return i;
}

static int never_returns(void) {
  int i = 0;
  for (;;) {
    i++;
    if (i > 3) {
      return i;
    }
  }
}

int main(void) {
  struct link c = { 3, 0 };
  struct link b = { 2, &c };
  struct link a = { 1, &b };

  if (sum_with_ternary(&a, 3) != 6) {
    return 1;
  }
  /* the same loop stopping early: 1 + 2, then the ternary says stop */
  if (sum_with_ternary(&a, 2) != 3) {
    return 2;
  }
  if (count_do_while_forever() != 5) {
    return 3;
  }
  if (never_returns() != 4) {
    return 4;
  }

  /* C99 6.3.1.8: the conversions come first, so -1 is 4294967295 here */
  if (!(-1 > 0u)) {
    return 5;
  }
  if (-5 < 3u) {
    return 6;
  }
  if ((-1 / 2u) != 2147483647u) {
    return 7;
  }
  if ((-1 % 2u) != 1u) {
    return 8;
  }
  if ((1 ? 0 : 1)) {
    return 9;
  }
  if ((0 ? 1 : 0)) {
    return 10;
  }

  printf("loops ok\n");
  return 96;
}
