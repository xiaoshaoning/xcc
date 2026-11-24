#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#define LZ4_STATIC_LINKING_ONLY
#define LZ4_HC_STATIC_LINKING_ONLY
#include "lz4hc.h"
#include "lz4frame.h"
#include "lz4file.h"

static unsigned long long h = 1469598103934665603ULL;
static void mix(const void *p, size_t n) {
  const unsigned char *b = (const unsigned char *)p; size_t i;
  for (i = 0; i < n; i++) { h ^= b[i]; h *= 1099511628211ULL; }
}
static void say(const char *what, long value) { printf("%-42s %ld\n", what, value); }

#define N 300000
static char src[N], dst[N + N/128 + 1024], back[N];

static void fill(void) {
  int i; unsigned s = 12345;
  for (i = 0; i < N; i++) {
    s = s * 1103515245u + 12345u;
    src[i] = (char)((s >> 16) % 40 + ((i % 700) < 20 ? (int)(s >> 8) % 90 : 'A'));
  }
}

int main(void) {
  int i;
  fill();

  /* 1. one-shot block, all accelerations */
  for (i = 1; i <= 12; i += 3) {
    int c = LZ4_compress_fast(src, dst, N, sizeof dst, i);
    int d = LZ4_decompress_safe(dst, back, c, N);
    say("compress_fast/decompress_safe", (d == N && !memcmp(src, back, N)) ? c : -1);
    mix(dst, (size_t)c);
  }

  /* 2. destSize: bounded output, compression stops when it runs out */
  {
    int cap = 4000;
    int c = LZ4_compress_destSize(src, dst, &cap, N);
    int produced = LZ4_decompress_safe(dst, back, c, N);
    say("compress_destSize", (produced > 0 && !memcmp(src, back, (size_t)produced)) ? c : -1);
    mix(dst, (size_t)c);
  }

  /* 3. user-supplied state: alignment is the caller's problem, ours must match */
  {
    static long long state[1 << 16];
    int c = LZ4_compress_fast_extState(state, src, dst, N, sizeof dst, 3);
    int d = LZ4_decompress_safe(dst, back, c, N);
    say("compress_fast_extState", (d == N && !memcmp(src, back, N)) ? c : -1);
    mix(dst, (size_t)c);
  }

  /* 4. partial decompression */
  {
    int c = LZ4_compress_default(src, dst, N, sizeof dst);
    int d = LZ4_decompress_safe_partial(dst, back, c, 1000, N);
    say("decompress_safe_partial(1000)", (d >= 1000 && !memcmp(src, back, 1000)) ? d : -1);
  }

  /* 5. high compression */
  {
    int c = LZ4_compress_HC(src, dst, N, sizeof dst, 12);
    int d = LZ4_decompress_safe(dst, back, c, N);
    say("compress_HC(12)", (d == N && !memcmp(src, back, N)) ? c : -1);
    mix(dst, (size_t)c);
    static int hcState[1 << 18];
    c = LZ4_compress_HC_extStateHC(hcState, src, dst, N, sizeof dst, 9);
    d = LZ4_decompress_safe(dst, back, c, N);
    say("compress_HC_extStateHC(9)", (d == N && !memcmp(src, back, N)) ? c : -1);
    mix(dst, (size_t)c);
  }

  /* 6. streaming block with a saved dictionary, decompressed with the same dict */
  {
    LZ4_stream_t *s = LZ4_createStream();
    LZ4_streamDecode_t *sd = LZ4_createStreamDecode();
    size_t const step = 40000;
    char dict[64 * 1024];
    size_t dictSize = 0, off;
    int total = 0;
    for (off = 0; off + step <= N; off += step) {
      int c = LZ4_compress_fast_continue(s, src + off, dst, (int)step, sizeof dst, 1);
      if (c <= 0) { total = -1; break; }
      mix(dst, (size_t)c);
      if (LZ4_decompress_safe_continue(sd, dst, back + off, c, (int)step) != (int)step) { total = -2; break; }
      total += c;
      dictSize = (size_t)LZ4_saveDict(s, dict, sizeof dict);
    }
    say("fast_continue + safe_continue + saveDict",
        (total > 0 && !memcmp(src, back, N - (N % step))) ? total + (int)dictSize : -1);
    LZ4_freeStream(s); LZ4_freeStreamDecode(sd);
  }

  /* 7. compression with a dictionary loaded from bytes, decoded using the same one */
  {
    LZ4_stream_t *s = LZ4_createStream();
    int c;
    LZ4_loadDict(s, src, 65536);
    c = LZ4_compress_fast_continue(s, src + 65536, dst, 100000, sizeof dst, 1);
    back[100000] = 0;
    say("loadDict + fast_continue",
        (c > 0 && LZ4_decompress_safe_usingDict(dst, back, c, 100000, src, 65536) == 100000
         && !memcmp(src + 65536, back, 100000)) ? c : -1);
    mix(dst, (size_t)c);
    LZ4_freeStream(s);
  }

  /* 8. HC streaming across buffers */
  {
    LZ4_streamHC_t *s = LZ4_createStreamHC();
    size_t off; int total = 0;
    for (off = 0; off + 30000 <= N; off += 30000) {
      int c = LZ4_compress_HC_continue(s, src + off, dst, 30000, sizeof dst);
      int d = LZ4_decompress_safe_usingDict(dst, back + off, c, 30000,
                                           off ? src : 0, off ? 65536 : 0);
      if (c <= 0 || d != 30000) { total = -1; break; }
      total += c; mix(dst, (size_t)c);
    }
    say("compress_HC_continue", total);
    LZ4_freeStreamHC(s);
  }

  /* 9. frame API, written in odd chunks and read back in odd chunks */
  {
    LZ4F_preferences_t prefs = { 0 };
    prefs.frameInfo.blockSizeID = LZ4F_max256KB;
    prefs.frameInfo.contentChecksumFlag = LZ4F_contentChecksumEnabled;
    prefs.frameInfo.blockChecksumFlag = LZ4F_blockChecksumEnabled;
    prefs.compressionLevel = 7;
    LZ4F_compressionContext_t cx;
    LZ4F_createCompressionContext(&cx, LZ4F_VERSION);
    size_t bound = LZ4F_compressBound(N, &prefs), got = 0, off = 0;
    char *out = malloc(bound + 1000);
    got += LZ4F_compressBegin(cx, out, bound, &prefs);
    while (off < N) {
      size_t chunk = (off % 7000) + 1;
      if (off + chunk > N) chunk = N - off;
      got += LZ4F_compressUpdate(cx, out + got, bound - got, src + off, chunk, NULL);
      off += chunk;
    }
    got += LZ4F_compressEnd(cx, out + got, bound - got, NULL);
    say("frame compress in odd chunks", got > bound + 1000 ? -1 : (long)got);
    mix(out, got);
    LZ4F_freeCompressionContext(cx);

    LZ4F_dctx *dx;
    LZ4F_createDecompressionContext(&dx, LZ4F_VERSION);
    size_t inOff = 0, outTotal = 0;
    while (inOff < got) {
      size_t inSize = (inOff % 3000) + 1, outSize;
      if (inOff + inSize > got) inSize = got - inOff;
      outSize = N - outTotal < 5000 ? N - outTotal : 5000;
      size_t r = LZ4F_decompress(dx, back + outTotal, &outSize, out + inOff, &inSize, NULL);
      if (LZ4F_isError(r)) { outTotal = 0; break; }
      inOff += inSize; outTotal += outSize;
      if (r == 0) break;
    }
    say("frame decompress in odd chunks",
        (outTotal == N && !memcmp(src, back, N)) ? (long)outTotal : -1);
    LZ4F_freeDecompressionContext(dx);
    free(out);
  }

  /* 10. frame with a prepared dictionary context */
  {
    LZ4F_preferences_t prefs = { 0 };
    LZ4F_CDict *cd = LZ4F_createCDict(src, 65536);
    LZ4F_cctx *fcx;
    LZ4F_createCompressionContext(&fcx, LZ4F_VERSION);
    size_t got, outSize;
    prefs.frameInfo.contentChecksumFlag = LZ4F_contentChecksumEnabled;
    got = LZ4F_compressFrame_usingCDict(fcx, dst, sizeof dst, src + 65536, 100000, cd, &prefs);
    say("compressFrame_usingCDict", LZ4F_isError(got) ? -1 : (long)got);
    mix(dst, got);
    {
      LZ4F_dctx *fdx;
      size_t inSize = got;
      size_t r;
      LZ4F_createDecompressionContext(&fdx, LZ4F_VERSION);
      outSize = 100000;
      r = LZ4F_decompress(fdx, back, &outSize, dst, &inSize, NULL);
      say("decompress after usingCDict", (LZ4F_isError(r) || outSize != 100000) ? -1
                                : (!memcmp(src + 65536, back, 100000) ? (long)outSize : -2));
      LZ4F_freeDecompressionContext(fdx);
    }
    LZ4F_freeCompressionContext(fcx);
    LZ4F_freeCDict(cd);
  }


  /* 11. the stdio wrapper: a frame written to a real FILE and read back through one */
  {
    LZ4F_preferences_t prefs = { 0 };
    LZ4_writeFile_t *wf = 0;
    LZ4_readFile_t *rf = 0;
    FILE *f = tmpfile();
    size_t wrote = 0, got = 0;
    static char back2[8000];
    prefs.frameInfo.contentChecksumFlag = LZ4F_contentChecksumEnabled;
    if (f == 0) {
      say("stdio wrapper round trip", -1);
    } else {
      if (!LZ4F_isError(LZ4F_writeOpen(&wf, f, &prefs))) {
        wrote = LZ4F_write(wf, src, 7000);
        (void)LZ4F_writeClose(wf);
        rewind(f);
      }
      if (!LZ4F_isError(LZ4F_readOpen(&rf, f))) {
        got = LZ4F_read(rf, back2, sizeof back2);
        (void)LZ4F_readClose(rf);
      }
      fclose(f);
      say("stdio wrapper round trip",
          (wrote == 7000 && got == 7000 && !memcmp(src, back2, 7000)) ? (long)got : -1);
      mix(back2, got);
    }
  }

  printf("digest %016llx\n", h);
  return 0;
}
