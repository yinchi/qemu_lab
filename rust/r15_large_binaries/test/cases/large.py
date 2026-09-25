"""Last updated: Stage 15.

Arbitrarily large binaries. Before Stage 15 a program had a fixed 2 MiB window (an image of at most 960 KiB);
now the window is a 32 MiB *ceiling* and a program is given exactly the pages its segments and stack need.
`bigimage` (a test program) has about 11 MiB of memory -- a 2 MiB `.data` table, 1 MiB of read-only data
and 8 MiB of `.bss` -- and checks every byte range it owns.

Also checked: what one program was given is unmapped again before the next runs (a small program is not left
holding a big one's memory, and a `.bss` is zero every time), and what is still refused -- a segment whose
memory would not fit under the ceiling, and a file too big to read into the kernel's heap.
"""

BASE = 0x4400_0000
FAULT = "exit 139\n"

DATA_SUM = 512 * 1024 * 0xA5A5_A5A5
RODATA_SUM = 7 * 1024 * 1024
BIGIMAGE = f"data: {DATA_SUM}\nrodata: {RODATA_SUM}\nbss: 2048 pages\n"


def faults(out):
    return "Segmentation fault" in out and out.endswith(FAULT)


def run(ctx):
    s, check = ctx.s, ctx.check
    for name in ["bigimage.exe", "probe.exe", "elf-hugebss.exe", "elf-toolargefile.exe"]:
        s.run(f"chmod +x tests/{name}")

    # --- an image bigger than the old window runs, and every part of it is right ---
    check("an ~11 MiB image runs: .data copied in, .rodata readable, .bss zero and writable",
          s.run("tests/bigimage.exe"), "tests/bigimage.exe\n" + BIGIMAGE)
    check("...and a second run sees a fresh .bss (the first run wrote every page of it)",
          s.run("tests/bigimage.exe"), "tests/bigimage.exe\n" + BIGIMAGE)

    # --- none of it outlives the program: the next program runs with only what it needs ---
    s.run("tests/bigimage.exe")
    def poke(addr):
        return s.run(f"tests/probe.exe poke {addr}")
    s.run("tests/bigimage.exe")
    check("the big program's .data is unmapped for the next program", faults(poke(BASE + 0x20_0000)), True)
    s.run("tests/bigimage.exe")
    check("...and its .bss, deep in what it was given", faults(poke(BASE + 0x80_0000)), True)
    s.run("tests/bigimage.exe")
    check("...and the last page it was given", faults(poke(BASE + 0xAF_F000)), True)
    check("a small program still works, and reads its own code", "read" in poke(BASE), True)

    # --- what is still refused ---
    check("hello after all that", s.run("hello"), "hello\nhello from userspace\n")
    check("a segment whose memory cannot fit under the ceiling is refused",
          s.run("tests/elf-hugebss.exe"), "tests/elf-hugebss.exe\ntests/elf-hugebss.exe: cannot execute: Exec format error\n")
    check("a file too big for the kernel's heap is refused, not a kernel panic",
          s.run("tests/elf-toolargefile.exe"),
          "tests/elf-toolargefile.exe\ntests/elf-toolargefile.exe: cannot execute: Exec format error\n")
    check("shell alive", s.run("echo ok"), "echo ok\nok\n")
