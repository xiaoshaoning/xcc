/* The program the xxhash fixture is compiled against: the three hashes over one string, printed as
 * hex, which is what a `clang` build of the same files prints.
 *
 * The library's own `-DXXH_VECTOR=0` selects its portable scalar path: the vector paths are
 * `__m128i` and `__m256i`, and this compiler shadows `emmintrin.h` with an empty bridge header on
 * purpose, so the SIMD paths are not something it can spell.
 */
#include <stdio.h>
#include <string.h>

#include "xxhash.h"

int main(void) {
  const char *text = "the quick brown fox jumps over the lazy dog";
  size_t length = strlen(text);
  printf("%016llx %016llx %08x\n", (unsigned long long)XXH64(text, length, 0),
         (unsigned long long)XXH3_64bits(text, length), XXH32(text, length, 0));
  return 0;
}
