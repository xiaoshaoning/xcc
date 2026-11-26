/* cc52_ptrtoint.c - a pointer narrowed to an integer, which C99 6.3.2.3p6 makes
 * implementation-defined and the machine spells "the low bits".
 *
 * Found by `tre`, which hashes a pointer with `(int)p`: the conversion was refused
 * outright, so a regex library nine thousand lines long could not be compiled. The
 * corpus is where a language feature meets *both* back ends - level 2 through the own
 * one, levels 0 and 1 through Cranelift - and only one of them had seen this one.
 *
 * Exit 96, and the output is compared with clang byte for byte.
 */
int printf(const char *, ...);

int main(void) {
  char *p = (char *)0x7f3a5c10;

  /* The four widths, in the order that shows each truncation. `unsigned` first so the
   * left shift of a sign bit is not the thing being tested. */
  unsigned narrow = (unsigned)p;
  unsigned short half = (unsigned short)narrow;
  unsigned char byte = (unsigned char)narrow;
  unsigned long long whole = (unsigned long long)p;
  printf("%x %x %x %llx\n", narrow, half, byte, whole);

  /* The two that are *not* truncations: `_Bool` from a pointer asks whether it is null,
   * and a signed narrow type sign-extends the bits it kept. */
  int truth = (int)(p != 0);
  short signed_half = (short)narrow;
  printf("%d %d\n", truth, (int)signed_half);

  /* A null pointer is zero through every one of those, which is the case a truth test
   * and a truncation happen to agree about - so it says nothing about either. */
  printf("%d %d\n", (int)(unsigned)(char *)0, (int)((char *)0 != 0));

  /* ... and the round trip: an integer cast to a pointer and back is the same number. */
  char *back = (char *)(unsigned long long)narrow;
  printf("%d\n", back == p);

  return 96;
}
