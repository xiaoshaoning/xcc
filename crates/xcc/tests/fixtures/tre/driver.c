/* The program the tre fixture is compiled against: one POSIX extended regular expression matched
 * once, printed as offsets, which is what a `clang` build of the same files prints.
 *
 * Three groups over one string exercises the parse, the compile and the parallel matcher without
 * depending on anything this compiler does not have: the library's Windows branch reads
 * `basetsd.h`, so the fixture needs the SDK - unlike xxhash, which needs none.
 */
#include <stdio.h>

#include "regex.h"

int main(void) {
  regex_t expression;
  regmatch_t match[3];
  if (regcomp(&expression, "(a+)(b+)", REG_EXTENDED) != 0) {
    return 1;
  }
  if (regexec(&expression, "xxaaabbbzz", 3, match, 0) != 0) {
    regfree(&expression);
    return 2;
  }
  printf("%d %d %d %d %d %d\n", (int)match[0].rm_so, (int)match[0].rm_eo,
         (int)match[1].rm_so, (int)match[1].rm_eo, (int)match[2].rm_so, (int)match[2].rm_eo);
  regfree(&expression);
  return 0;
}
