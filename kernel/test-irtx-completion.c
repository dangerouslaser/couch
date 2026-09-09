/* SPDX-License-Identifier: GPL-2.0 */
/* Build/run on Ollie against the actual kernel helper, not a copied model:
 * cc -std=c99 -Wall -Wextra -Werror -I "$KTREE/drivers/misc/mediatek/irtx/mt6580" 
 *    kernel/test-irtx-completion.c -o /tmp/couch-irtx-completion-test
 */
#include <assert.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
typedef uint32_t u32;
typedef int64_t s64;
#include "couch_irtx_complete.h"

int main(void)
{
    bool zero = true;
    /* Hardware may finish during setup, but padded samples still get a full
     * conservative duration after configuration before success is reported. */
    for (s64 us = 0; us < 281; ++us)
        assert(!couch_irtx_complete(1, &zero, us, 281));
    assert(couch_irtx_complete(1, &zero, 281, 281));

    /* A prior frame's retained count must never complete the next frame,
     * even after the entire timeout budget has elapsed. */
    zero = false;
    for (s64 us = 0; us < 60000; us += 500)
        assert(!couch_irtx_complete(1, &zero, us, 281));
    assert(!zero);
    assert(!couch_irtx_complete(0, &zero, 0, 281));
    assert(zero);
    assert(!couch_irtx_complete(1, &zero, 280, 281));
    assert(couch_irtx_complete(1, &zero, 281, 281));

    /* A full protocol frame must not inherit the tiny probe's guard. */
    assert(!couch_irtx_complete(1, &zero, 281, 68000));
    assert(!couch_irtx_complete(1, &zero, 67999, 68000));
    assert(couch_irtx_complete(1, &zero, 68000, 68000));
    assert(!couch_irtx_complete(0, &zero, 1000000, 68000));
    assert(!couch_irtx_complete(2, &zero, 1000000, 68000));
    puts("IR completion guards passed: duration, stale count, reset, full frame, invalid count");
    return 0;
}
