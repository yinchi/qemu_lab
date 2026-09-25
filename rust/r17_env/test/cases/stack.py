"""Last updated: Stage 12, Step 3.

The MMU is really on, and the user window is laid out as program image, then an unmapped gap and
64 KiB guard, then a 1 MiB stack (see the kernel's `platform/base_addresses.rs`).

`probe poke ADDR` / `poke-w ADDR` read or write one byte at an address: a page the program has no right
to faults, which the kernel reports as `Segmentation fault` and exit status 139.
"""

BASE = 0x4400_0000
WINDOW = 0x200_0000  # the ceiling; only what a program needs is mapped
STACK_BOTTOM = BASE + WINDOW - 0x10_0000
GUARD_BOTTOM = STACK_BOTTOM - 0x1_0000

MALFORMED = {
    "elf-inguard": "a segment in the guard",
    "elf-instack": "a segment in the stack",
    "elf-hugebss": "a segment whose memory does not fit under the ceiling",
    "elf-sharepage": "two segments sharing a page",
}


def faults(out):
    return "Segmentation fault" in out


def run(ctx):
    s, check = ctx.s, ctx.check
    # --- the MMU is on with its hardening: the boot log says what was enabled ---
    boot = s.log()
    check("boot: MMU on, with WXN and stack alignment checks", "MMU enabled." in boot and "MMU hardening: WXN, stack alignment checks" in boot, True)
    check("boot: PAN on (QEMU's -cpu max has it)", "checks, PAN." in boot, True)

    for name in ["probe", "overflow"] + list(MALFORMED):
        s.run(f"chmod +x tests/{name}")

    def probe(cmd, addr=None):
        return s.run(f"tests/probe {cmd}" + (f" {addr}" if addr is not None else ""))

    # --- what a program may touch ---
    check("stack: the top is usable", "wrote" in probe("poke-w", BASE + WINDOW - 8), True)
    check("stack: its lowest page is usable", "wrote" in probe("poke-w", STACK_BOTTOM), True)
    check("code is readable", "read" in probe("poke", BASE), True)

    # --- what it may not ---
    check("code is not writable", faults(probe("poke-w", BASE)), True)
    check("just below the stack (the guard) faults", faults(probe("poke", STACK_BOTTOM - 1)), True)
    check("...its lowest byte too", faults(probe("poke", GUARD_BOTTOM)), True)
    check("the gap after the program faults", faults(probe("poke", BASE + 0x8_0000)), True)
    check("just past the window faults", faults(probe("poke", BASE + WINDOW)), True)
    check("just below the window faults", faults(probe("poke", BASE - 0x1000)), True)
    check("kernel memory faults for EL0", faults(probe("poke", 0x4000_0000)), True)
    check("a device (the UART) faults for EL0", faults(probe("poke", 0x0900_0000)), True)

    # --- the stack is generous, and its bottom is a wall ---
    out = probe("stack", 256)
    check("256 KiB of stack use works", out.endswith("stack 256 KiB: 0\n") or "stack 256 KiB" in out, True)
    check("900 KiB of stack use works", "stack 900 KiB" in probe("stack", 900), True)
    check("1100 KiB runs off the stack and faults", faults(probe("stack", 1100)), True)

    # --- T3.1: unbounded recursion stops at the guard; nothing else is disturbed ---
    out = s.run("tests/overflow")
    check("overflow: faults at the guard", faults(out) and "overflowing" in out, True)
    check("overflow: fault address is just below the stack",
          f"address {STACK_BOTTOM - 0x1000 + 0x10:#x}" in out or "address 0x45eff" in out, True)
    check("hello afterwards is unharmed", s.run("hello"), "hello\nhello from userspace\n")
    check("the shell is unharmed", s.run("echo alive"), "echo alive\nalive\n")

    # --- the previous program's pages do not outlive it ---
    s.run("tail tests/hello.txt")  # tail's 512 KiB buffer maps far more than probe will
    check("pages of an earlier program are unmapped again",
          faults(probe("poke", BASE + 0x4_0000)), True)

    # --- a kernel that touches user memory checks first: bad pointers are EFAULT, never a kernel fault ---
    check("syscalls refuse pointers into unmapped or read-only memory", probe("user-ptrs"),
          "tests/probe user-ptrs\n"
          "write from the gap: -14\nwrite from the guard: -14\n"
          "write running off the top of the stack: -14\n"
          "read into read-only code: -14\nread into the guard: -14\n"
          "getdents into read-only code: -14\n"
          "open with the path in the guard: -14\nchmod with the path in the gap: -14\n"
          "write from code (the kernel only reads it: allowed): 0\n")

    # --- executables that don't fit the layout are refused, not loaded ---
    for name, what in MALFORMED.items():
        path = f"tests/{name}"
        check(f"refused: {what}", s.run(path), f"{path}\n{path}: cannot execute: Exec format error\n")
    check("shell still alive after those", s.run("echo alive"), "echo alive\nalive\n")
