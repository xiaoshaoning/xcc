/* What a caller does with miniz: deflate a buffer, inflate it back, then write and read a zip.
 *
 * Everything is in memory, so the test is hermetic and needs no `-DSYS_STAT`. Its output is
 * compared against what a clang build of these same files printed - and the compressed stream is
 * one real zlib can read (measured when the fixture was added).
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "miniz.h"

static const char body[] = "the zip writer and reader, built by xcc";

int main(void) {
    size_t body_len = sizeof body - 1;

    /* The two halves of the compression library, against each other. */
    size_t packed_len = 0;
    void *packed = tdefl_compress_mem_to_heap(body, body_len, &packed_len, TDEFL_WRITE_ZLIB_HEADER);
    if (!packed) {
        printf("deflate failed\n");
        return 1;
    }
    size_t back_len = 0;
    void *back = tinfl_decompress_mem_to_heap(packed, packed_len, &back_len, TINFL_FLAG_PARSE_ZLIB_HEADER);
    int deflate_same = back && back_len == body_len && memcmp(back, body, body_len) == 0;
    printf("deflate/inflate: %s\n", deflate_same ? "identical" : "DIFFERENT");
    mz_free(back);
    mz_free(packed);

    /* And the archive: write one in memory, read the file back out of it. */
    mz_zip_archive writer;
    memset(&writer, 0, sizeof writer);
    if (!mz_zip_writer_init_heap(&writer, 0, 0)) {
        printf("writer init failed\n");
        return 1;
    }
    if (!mz_zip_writer_add_mem(&writer, "hello.txt", body, body_len, MZ_DEFAULT_COMPRESSION)) {
        printf("add failed\n");
        return 1;
    }
    void *archive = NULL;
    size_t archive_size = 0;
    int finalized = mz_zip_writer_finalize_heap_archive(&writer, &archive, &archive_size);
    mz_zip_writer_end(&writer);
    if (!finalized || !archive) {
        printf("finalize failed\n");
        return 1;
    }

    mz_zip_archive reader;
    memset(&reader, 0, sizeof reader);
    if (!mz_zip_reader_init_mem(&reader, archive, archive_size, 0)) {
        printf("reader init failed\n");
        return 1;
    }
    size_t got = 0;
    void *text = mz_zip_reader_extract_file_to_heap(&reader, "hello.txt", &got, 0);
    int zip_same = text && got == body_len && memcmp(text, body, body_len) == 0;
    printf("zip: %u file(s), extract %s\n", mz_zip_reader_get_num_files(&reader),
           zip_same ? "identical" : "DIFFERENT");
    const char *name = mz_zip_reader_get_filename(&reader, 0, NULL, 0) ? "named" : "unnamed";
    printf("zip entry %s\n", name);

    mz_free(text);
    mz_zip_reader_end(&reader);
    mz_free(archive);
    return (deflate_same && zip_same) ? 0 : 1;
}
