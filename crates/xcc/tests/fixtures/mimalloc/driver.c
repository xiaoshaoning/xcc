#include <stdio.h>
#include <string.h>
#include <mimalloc.h>
int main(void) {
  char *p = (char *)mi_malloc(32);
  strcpy(p, "mimalloc under xcc");
  printf("%s %d\n", p, mi_version() > 0);
  void *q = mi_realloc(p, 1024);
  printf("%d\n", q != 0);
  mi_free(q);
  return 0;
}
