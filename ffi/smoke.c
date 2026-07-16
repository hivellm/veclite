/* VecLite C ABI smoke-link (SPEC-008, phase4e). Links the *shipped* library
 * (no Rust toolchain) and exercises the lifecycle: open an in-memory database,
 * report the version/ABI, and close it. Exit non-zero on any surprise.
 *
 * Built and run by ffi/smoke.sh against a release bundle. */

#include <stdio.h>

#include "veclite.h"

int main(void) {
    vl_db *db = NULL;

    int rc = vl_open_memory(&db);
    if (rc != VL_OK || db == NULL) {
        fprintf(stderr, "vl_open_memory failed: %d\n", rc);
        return 1;
    }

    rc = vl_db_close(db);
    if (rc != VL_OK) {
        fprintf(stderr, "vl_db_close failed: %d\n", rc);
        return 1;
    }

    /* Null-out-pointer guard must be rejected, not crash. */
    if (vl_open_memory(NULL) != VL_ERR_INVALID_ARGUMENT) {
        fprintf(stderr, "vl_open_memory(NULL) did not reject\n");
        return 1;
    }

    printf("veclite smoke ok: version=%s abi=%u format=%u\n",
           vl_version(), vl_abi_version(), vl_format_version());
    return 0;
}
