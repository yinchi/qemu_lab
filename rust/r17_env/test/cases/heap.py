"""Last updated: Stage 16.

The user heap: `brk` (the program break) and `alloc` on top of it.

Before Stage 16 a program had no heap: it used the stack, `.bss`, or a fixed array. Now the kernel gives each
program a break -- it starts at the end of the image -- that `brk` moves, and `userlib`'s `heap` feature turns
that into a `#[global_allocator]` (`linked_list_allocator`'s `Heap`, grown from the break in chunks).

`probe brk` drives the syscall directly: growing by a page and a byte, zeroed writable memory, the kernel's own
pointer check against what is mapped, shrinking (the pages above go, the rest of the page is zeroed), and what
is refused. `heapuse` is a program that allocates the way one would (a big `Vec`, many small boxes, a string, a
`Vec` growing by doubling, an allocation that cannot fit, reuse after a free). And what one program's heap was is
gone for the next.
"""

BASE = 0x4400_0000

BRK = (
    "start: page-aligned true\n"
    "brk(0) again: same true\n"
    "grow +3 pages +1 byte: granted true\n"
    "fresh memory: zero true, writable\n"
    "kernel write at the last mapped bytes: true\n"
    "kernel write across the end: -14\n"
    "kernel write just past it: -14\n"
    "shrink to +100: granted true\n"
    "shrink: below the break kept true, above it zeroed true\n"
    "shrink: the page above is unmapped: -14\n"
    "regrow: granted true\n"
    "regrow: zero again true\n"
    "below the start: unchanged true\n"
    "into the stack's guard: unchanged true\n"
    "the whole address space: unchanged true\n"
    "up to the guard exactly: granted true\n"
    "back down: granted true\n"
)

HEAPUSE = (
    "big: 1048570078\n"
    "small: 299980000\n"
    "string: 58890\n"
    "grow: 500000 124999750000\n"
    "oom: refused\n"
    "reuse: yes\n"
    "heap: grew on demand\n"
)


def faults(out):
    return "Segmentation fault" in out


def run(ctx):
    s, check = ctx.s, ctx.check
    for name in ["probe.exe", "heapuse.exe"]:
        s.run(f"chmod +x tests/{name}")

    check("brk: grow, shrink, zeroing, the kernel's pointer check, and what is refused",
          s.run("tests/probe.exe brk"), "tests/probe.exe brk\n" + BRK)
    check("...and the same again: a program starts with its own fresh break", s.run("tests/probe.exe brk"),
          "tests/probe.exe brk\n" + BRK)

    check("a program that allocates: Vec, Box, String, growth, an allocation that cannot fit, reuse",
          s.run("tests/heapuse.exe"),
          "tests/heapuse.exe\n" + HEAPUSE.format(big=1048570078, small=299980000, string=58890, grow=124999750000))
    check("...and again (its heap was given back and starts fresh)", s.run("tests/heapuse.exe"),
          "tests/heapuse.exe\n" + HEAPUSE.format(big=1048570078, small=299980000, string=58890, grow=124999750000))

    # --- the heap does not outlive the program ---
    s.run("tests/heapuse.exe")
    out = s.run(f"tests/probe.exe poke {BASE + 0x40_0000}")
    check("what a program's heap was is unmapped for the next program", faults(out), True)

    check("date and stat (built on the user heap) still work",
          s.run("date -u -d @1000000000"), "date -u -d @1000000000\nSun Sep  9 01:46:40 UTC 2001\n")
    check("shell alive", s.run("echo ok"), "echo ok\nok\n")
