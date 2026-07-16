/* 16-thread concurrency smoke for the VecLite C ABI (SPEC-008 FFI-001, phase4g
 * TST-2.2). The core is Send + Sync, so any thread may call any function on any
 * handle. Sixteen threads hammer ONE shared vl_db / vl_collection concurrently
 * — interleaved upserts, searches, counts, and scrolls — then the main thread
 * verifies every write landed. Built and run under ThreadSanitizer in CI
 * (tests/c/sanitize.sh): any data race inside the library fails the run.
 *
 * Uses C11 <threads.h> for portability (glibc, macOS, and MSVC/clang all ship
 * it). Thread t writes ids "t<thread>_<i>", so the final count is exact. */
#include "veclite.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <threads.h>

#define THREADS 16
#define PER_THREAD 64

static vl_collection *g_coll;

#define DIE(msg)                                                               \
    do {                                                                       \
        fprintf(stderr, "FAIL: %s: %s\n", msg, vl_last_error_message());       \
        exit(1);                                                               \
    } while (0)

static int worker(void *arg) {
    int tid = (int)(intptr_t)arg;
    for (int i = 0; i < PER_THREAD; i++) {
        char id[64];
        snprintf(id, sizeof id, "t%d_%d", tid, i);
        /* A distinct 4-d unit-ish vector per (tid,i). */
        float vec[4] = {(float)tid, (float)i, (float)(tid ^ i), 1.0f};
        if (vl_upsert(g_coll, id, vec, 4, NULL, 0, VL_CODEC_JSON) != VL_OK) {
            DIE("concurrent upsert");
        }

        /* Interleave reads that touch the shared index while writers run. */
        vl_hits *hits = NULL;
        if (vl_search(g_coll, vec, 4, 5, NULL, 0, VL_CODEC_JSON, &hits) != VL_OK) {
            DIE("concurrent search");
        }
        vl_hits_free(hits);

        uint64_t n = 0;
        if (vl_count(g_coll, &n) != VL_OK) {
            DIE("concurrent count");
        }

        if ((i & 7) == 0) {
            vl_page *page = NULL;
            const char *opts = "{\"limit\":8}";
            if (vl_scroll(g_coll, (const uint8_t *)opts, strlen(opts),
                          VL_CODEC_JSON, &page) != VL_OK) {
                DIE("concurrent scroll");
            }
            vl_page_free(page);
        }
    }
    return 0;
}

int main(void) {
    vl_db *db = NULL;
    if (vl_open_memory(&db) != VL_OK) {
        DIE("open_memory");
    }
    const char *opts = "{\"dimension\":4,\"metric\":\"cosine\",\"quantization_bits\":0}";
    if (vl_collection_create(db, "docs", (const uint8_t *)opts, strlen(opts),
                             VL_CODEC_JSON, &g_coll) != VL_OK) {
        DIE("collection_create");
    }

    thrd_t threads[THREADS];
    for (int t = 0; t < THREADS; t++) {
        if (thrd_create(&threads[t], worker, (void *)(intptr_t)t) != thrd_success) {
            DIE("thrd_create");
        }
    }
    for (int t = 0; t < THREADS; t++) {
        thrd_join(threads[t], NULL);
    }

    uint64_t n = 0;
    if (vl_count(g_coll, &n) != VL_OK) {
        DIE("final count");
    }
    if (n != (uint64_t)THREADS * PER_THREAD) {
        fprintf(stderr, "FAIL: count %llu != %d\n", (unsigned long long)n,
                THREADS * PER_THREAD);
        exit(1);
    }

    vl_collection_free(g_coll);
    vl_db_close(db);
    printf("concurrency ok: %d threads x %d writes = %llu live\n", THREADS,
           PER_THREAD, (unsigned long long)n);
    return 0;
}
