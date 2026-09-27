"""Last updated: after Stage 12, Step 13.

The kernel stack has an unmapped guard below it (`link.ld`, `arch/mmu.rs`), so overflowing it is a
reported fault, not silent corruption of `.bss`. The test kernel (`testhooks`) has a builtin,
`__overflow_kernel_stack`, that recurses until the stack runs out.

The kernel panics by design here, so this module bypasses `Session.run`/`wait_prompt`/`wait_until`,
which treat a `Kernel Panic!` on the serial log as the whole run failing (see `harness.py`'s
`FATAL_MARKERS`) -- the same reason `power.py` talks to `Session` below that abstraction. It runs in
its own group (one QEMU instance) since nothing can run after the kernel has died.
"""

import time

from harness import TIMEOUT

OVERFLOW = "Kernel stack overflow: the stack ran into its guard"


def run(ctx):
    s, check = ctx.s, ctx.check

    s.type("__overflow_kernel_stack\n")

    deadline = time.time() + TIMEOUT
    while OVERFLOW not in s.log() and time.time() < deadline:
        time.sleep(0.05)
    log = s.log()

    check("the overflow is reported as a kernel stack overflow", OVERFLOW in log, True)
    # Without the dedicated exception stack, the fault handler would fault again on the overflowed
    # stack, and whatever it managed to print would be a generic (or no) unexpected-exception message.
    check("...not as a generic unexpected exception", "Unexpected exception" in log, False)
    check("...with the faulting address", "FAR_EL1: 0x" in log, True)

    # The kernel stops there: no prompt follows.
    time.sleep(0.5)
    tail = s.log()[s.log().index(OVERFLOW):]
    check("the kernel halts after reporting it (no further prompt)", "> " in tail.split("FAR_EL1", 1)[1], False)
