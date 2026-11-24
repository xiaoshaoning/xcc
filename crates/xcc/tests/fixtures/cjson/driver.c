/* What a caller does with cJSON: parse, read a value by name, and print the tree back.
 *
 * The JSON is in the source, so this driver reads no files and the test is hermetic. Its output is
 * compared against what a clang build of these same files printed.
 */
#include <stdio.h>
#include <stdlib.h>
#include "cJSON.h"

static const char json_text[] =
    "{\"name\":\"xcc\",\"version\":0.1,"
    "\"features\":[\"front end\",\"own back end\",\"linker\"],"
    "\"nested\":{\"deep\":{\"deeper\":true}},\"nothing\":null}";

int main(void) {
    cJSON *root = cJSON_Parse(json_text);
    if (!root) {
        printf("not JSON\n");
        return 1;
    }
    printf("members=%d\n", cJSON_GetArraySize(root));
    const cJSON *version = cJSON_GetObjectItem(root, "version");
    printf("version=%d %s\n", version ? version->valueint : -1,
           cJSON_IsNumber(version) ? "a number" : "not a number");
    const cJSON *features = cJSON_GetObjectItem(root, "features");
    const cJSON *first = cJSON_GetArrayItem(features, 0);
    printf("first feature=%s\n", cJSON_IsString(first) ? first->valuestring : "(none)");
    const cJSON *deeper = cJSON_GetObjectItem(cJSON_GetObjectItem(cJSON_GetObjectItem(root, "nested"), "deep"), "deeper");
    printf("deeper=%s\n", cJSON_IsTrue(deeper) ? "true" : "not true");
    char *printed = cJSON_PrintUnformatted(root);
    printf("round trip: %s\n", printed ? printed : "(none)");
    free(printed);
    cJSON_Delete(root);
    return 0;
}
