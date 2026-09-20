#!/usr/bin/env python3
"""Generates the test fixtures that are derived from built programs, so no binary blobs are checked in:
malformed and oversized ELF files, made by corrupting `echo.exe`/`hello.exe`. Run by `just disk`, after
`disk/bin/` is staged.

Usage: mkfixtures.py <bin-dir> <tests-dir>

Every output is a `.exe` under `disk/tests/` (gitignored, like the test programs). The kernel must refuse
each malformed one with `cannot execute: Exec format error` rather than panic -- see
`cases/step01_launch.py` -- and run `bigpad.exe` (a valid program followed by 3 MiB of zeros, which the
loader ignores) as it would `hello`.
"""

import struct
import sys
from pathlib import Path

# ELF64 header / program header field offsets.
E_ENTRY, E_PHOFF, E_PHENTSIZE, E_PHNUM = 24, 32, 54, 56
P_TYPE, P_OFFSET, P_VADDR, P_FILESZ, P_MEMSZ = 0, 8, 16, 32, 40


def u16(b, off):
    return struct.unpack_from("<H", b, off)[0]


def u64(b, off):
    return struct.unpack_from("<Q", b, off)[0]


def patch(b, off, fmt, value):
    struct.pack_into(fmt, b, off, value)


def phdrs(b):
    """Offsets of every program header of `b`."""
    off, size, n = u64(b, E_PHOFF), u16(b, E_PHENTSIZE), u16(b, E_PHNUM)
    return [off + i * size for i in range(n)]


def first_load(b):
    return next(p for p in phdrs(b) if struct.unpack_from("<I", b, p + P_TYPE)[0] == 1)


def main():
    bin_dir, out = Path(sys.argv[1]), Path(sys.argv[2])
    echo = (bin_dir / "echo.exe").read_bytes()
    hello = (bin_dir / "hello.exe").read_bytes()

    def write(name, data):
        (out / name).write_bytes(bytes(data))

    write("bigpad.exe", hello + bytes(3 * 1024 * 1024))
    write("elf-trunc.exe", echo[:100])

    b = bytearray(echo)
    patch(b, first_load(b) + P_VADDR, "<Q", 0x5000_0000)  # segment far above the user window
    write("elf-badseg.exe", b)

    b = bytearray(echo)
    patch(b, E_PHOFF, "<Q", 1 << 40)  # program header table nowhere near the file
    write("elf-badphoff.exe", b)

    b = bytearray(echo)
    for p in phdrs(b):
        patch(b, p + P_TYPE, "<I", 0)  # PT_NULL: nothing to load
    write("elf-noload.exe", b)

    b = bytearray(echo)
    patch(b, E_ENTRY, "<Q", 0)  # entry point in no segment
    write("elf-badentry.exe", b)

    b = bytearray(echo)
    patch(b, first_load(b) + P_OFFSET, "<Q", len(echo) + 1000)  # file bytes past the end of the file
    write("elf-badoffset.exe", b)

    b = bytearray(echo)
    p = first_load(b)
    patch(b, p + P_MEMSZ, "<Q", u64(b, p + P_FILESZ) - 1)  # memsz < filesz
    write("elf-memlt.exe", b)


if __name__ == "__main__":
    main()
