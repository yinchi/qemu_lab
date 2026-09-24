"""Last updated: Stage 12, Step 11c.

`poweroff [--reboot]` and `reboot`: both reach PSCI (`SYSTEM_OFF`/`SYSTEM_RESET`) through the
kernel's `reboot` syscall -- see `docs/progs.md`. Both are real: `reboot` actually resets the board
(checked below by the full boot banner reappearing, not just a fresh prompt, which a plain shell
restart could never produce on its own) and `poweroff` actually terminates the emulated machine.
`poweroff` is therefore run last, and -- like `reboot`'s own real effect -- bypasses
`Session.run`/`wait_prompt`, which treat QEMU exiting mid-wait as a failure.
"""

import time

from harness import TIMEOUT


def run(ctx):
    s, check = ctx.s, ctx.check

    # --- --help and the one error path: no real PSCI call yet, so these are ordinary `s.run`s ---
    check("poweroff --help", s.run("poweroff --help"),
          "poweroff --help\nusage: poweroff [--reboot]\n  --reboot  restart instead of powering off\n")
    check("reboot --help", s.run("reboot --help"), "reboot --help\nusage: reboot\n")
    check("reboot unknown option", s.run("reboot -x"), "reboot -x\nreboot: invalid option -- 'x'\nTry 'reboot --help' for more information.\nexit 1\n")

    # --- `reboot`: the whole boot banner reappears, proving the board actually reset ---
    s.type("reboot\n")
    out = s.wait_until(lambda t: t.endswith("> ") and "Keyboard found" in t, "reboot to reach a fresh prompt")
    check("reboot resets the board", "Keyboard found -- listening for key events via IRQ." in out, True)
    s.pos = len(s.log())

    # --- `poweroff`: QEMU itself exits, cleanly ---
    s.type("poweroff\n")
    deadline = time.time() + TIMEOUT
    while s.qemu.poll() is None and time.time() < deadline:
        time.sleep(0.05)
    check("poweroff exits QEMU", s.qemu.poll(), 0)
