#include <stdio.h>
#include <string.h>

#include "yyjson.h"

int main(void) {
  const char *text = "{\"a\":[1,2,3],\"b\":\"x\",\"n\":-0.5}";
  yyjson_doc *doc = yyjson_read(text, strlen(text), 0);
  yyjson_val *root = yyjson_doc_get_root(doc);
  yyjson_val *a = yyjson_obj_get(root, "a");
  printf("%zu %.14g %s %s %.2f\n", yyjson_arr_size(a),
         yyjson_get_num(yyjson_arr_get(a, 1)),
         yyjson_get_str(yyjson_obj_get(root, "b")), yyjson_get_type_desc(a),
         yyjson_get_num(yyjson_obj_get(root, "n")));

  /* a writer round trip: the mut document is built and then serialised, which is the other half
     of the library (yyjson_mut_*) and a different set of allocations. */
  yyjson_mut_doc *mut = yyjson_mut_doc_new(NULL);
  yyjson_mut_val *obj = yyjson_mut_obj(mut);
  yyjson_mut_doc_set_root(mut, obj);
  yyjson_mut_obj_add_str(mut, obj, "who", "xcc");
  yyjson_mut_obj_add_int(mut, obj, "version", 5);
  char *written = yyjson_mut_write(mut, 0, NULL);
  printf("%s\n", written);
  free(written);
  yyjson_mut_doc_free(mut);
  yyjson_doc_free(doc);
  return 0;
}
