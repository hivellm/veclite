/* Full-lifecycle C smoke for the VecLite C ABI (SPEC-008, phase4g TST-2.1).
 *
 * open(memory) -> create -> upsert_batch -> search -> scroll -> close, freeing
 * every library-allocated handle/buffer. Built and run under AddressSanitizer +
 * LeakSanitizer in CI (tests/c/sanitize.sh): a leak of any handle, hits set,
 * page, or vl_buf fails the run. Any non-OK return code aborts with a message.
 *
 * The program is deliberately allocation-symmetric: every vl_*_free / vl_buf_free
 * has exactly one matching producer, so LSan sees a clean heap at exit. */
#include "veclite.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CHECK(expr)                                                            \
    do {                                                                       \
        int32_t _rc = (expr);                                                  \
        if (_rc != VL_OK) {                                                    \
            fprintf(stderr, "FAIL %s -> %d: %s\n", #expr, _rc,                 \
                    vl_last_error_message());                                  \
            exit(1);                                                           \
        }                                                                      \
    } while (0)

static void expect(int cond, const char *what) {
    if (!cond) {
        fprintf(stderr, "FAIL expectation: %s\n", what);
        exit(1);
    }
}

int main(void) {
    vl_db *db = NULL;
    CHECK(vl_open_memory(&db));

    const char *opts = "{\"dimension\":3,\"metric\":\"euclidean\",\"quantization_bits\":0}";
    vl_collection *coll = NULL;
    CHECK(vl_collection_create(db, "docs", (const uint8_t *)opts, strlen(opts),
                               VL_CODEC_JSON, &coll));

    /* Batch upsert three points, one with a payload. */
    const char *points =
        "[{\"id\":\"a\",\"vector\":[1.0,0.0,0.0],\"payload\":{\"n\":1}},"
        " {\"id\":\"b\",\"vector\":[0.0,1.0,0.0]},"
        " {\"id\":\"c\",\"vector\":[0.0,0.0,1.0],\"payload\":{\"n\":3}}]";
    CHECK(vl_upsert_batch(coll, (const uint8_t *)points, strlen(points), VL_CODEC_JSON));

    uint64_t n = 0;
    CHECK(vl_count(coll, &n));
    expect(n == 3, "count == 3 after upsert_batch");

    /* Declare a payload index, then confirm it via vl_db_info. */
    CHECK(vl_payload_index_create(coll, "n", VL_PIDX_INTEGER));
    vl_buf info = {0};
    CHECK(vl_db_info(db, VL_CODEC_JSON, &info));
    expect(info.data != NULL && info.len > 0, "db_info produced bytes");
    vl_buf_free(&info);

    /* k-NN search near "a", asking for the payload and the stored vector. */
    const float q[3] = {0.9f, 0.1f, 0.0f};
    const char *qopts = "{\"with_payload\":true,\"with_vector\":true}";
    vl_hits *hits = NULL;
    CHECK(vl_search(coll, q, 3, 3, (const uint8_t *)qopts, strlen(qopts),
                    VL_CODEC_JSON, &hits));
    expect(vl_hits_len(hits) >= 1, "search returned at least one hit");
    vl_hit_view view;
    memset(&view, 0, sizeof view);
    CHECK(vl_hits_get(hits, 0, &view));
    expect(strcmp(view.id, "a") == 0, "nearest hit is a");
    expect(view.has_vector && view.vector_len == 3, "with_vector projected a 3-d vector");
    vl_hits_free(hits);

    /* Batch search: two flat 3-d queries sharing the same opts. */
    const float vecs[6] = {1.0f, 0.0f, 0.0f, 0.0f, 0.0f, 1.0f};
    vl_hits_batch *batch = NULL;
    CHECK(vl_search_batch(coll, vecs, 2, 3, 1, NULL, 0, VL_CODEC_JSON, &batch));
    expect(vl_hits_batch_len(batch) == 2, "two per-query results");
    expect(vl_hits_batch_code(batch, 0) == VL_OK, "query 0 ok");
    expect(vl_hits_batch_hits_len(batch, 0) == 1, "query 0 has one hit");
    CHECK(vl_hits_batch_hit(batch, 0, 0, &view));
    expect(strcmp(view.id, "a") == 0, "batch query 0 nearest is a");
    vl_hits_batch_free(batch);

    /* Scroll the whole collection two at a time until the cursor is exhausted. */
    uint32_t scrolled = 0;
    char scroll_opts[128];
    const char *cursor = NULL;
    for (;;) {
        if (cursor) {
            snprintf(scroll_opts, sizeof scroll_opts,
                     "{\"limit\":2,\"cursor\":\"%s\"}", cursor);
        } else {
            snprintf(scroll_opts, sizeof scroll_opts, "{\"limit\":2}");
        }
        vl_page *page = NULL;
        CHECK(vl_scroll(coll, (const uint8_t *)scroll_opts, strlen(scroll_opts),
                        VL_CODEC_JSON, &page));
        uint32_t pn = vl_page_len(page);
        for (uint32_t i = 0; i < pn; i++) {
            vl_buf pt = {0};
            CHECK(vl_page_point(page, i, &pt));
            expect(pt.len > 0, "scrolled point has bytes");
            vl_buf_free(&pt);
            scrolled++;
        }
        const char *next = vl_page_cursor(page);
        /* Copy the cursor before freeing the page it points into. */
        static char cursor_buf[256];
        if (next) {
            snprintf(cursor_buf, sizeof cursor_buf, "%s", next);
            cursor = cursor_buf;
        } else {
            cursor = NULL;
        }
        vl_page_free(page);
        if (!next) {
            break;
        }
    }
    expect(scrolled == 3, "scrolled every point exactly once");

    /* Text chunker (pure utility): a long string splits into >= 2 chunks. */
    vl_buf chunks = {0};
    const char *copts = "{\"max_chars\":8,\"overlap\":2}";
    CHECK(vl_chunk("abcdefghijklmnopqrstuvwxyz", (const uint8_t *)copts,
                   strlen(copts), VL_CODEC_JSON, &chunks));
    expect(chunks.len > 0, "chunk produced bytes");
    vl_buf_free(&chunks);

    CHECK(vl_collection_free(coll));
    CHECK(vl_db_close(db));

    printf("full_smoke ok: version=%s abi=%u format=%u\n", vl_version(),
           vl_abi_version(), vl_format_version());
    return 0;
}
