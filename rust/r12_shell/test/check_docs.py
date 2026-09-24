#!/usr/bin/env python3
"""Checks that the docs mention everything they must (Stage 12, Step 13; `just check-docs`).

- every program in every tier under `user/progs*/src/bin/` has a row in `docs/progs.md`;
- every syscall number in `user/abi/src/syscall.rs` has a row in `docs/syscalls.md`;
- every test program in `test/progs/src/bin/` is described in `test/README.md`;
- no test program's name also exists under `user/` (test-only programs belong to the stage).

Exits nonzero, listing every problem, if any check fails.
"""

import re
import sys
from pathlib import Path

STAGE = Path(__file__).resolve().parent.parent  # rust/r12_shell
RUST = STAGE.parent
USER = RUST / "user"


def bin_names(directory):
    return sorted(p.stem for p in directory.glob("src/bin/*.rs"))


def main():
    problems = []

    progs_md = (RUST / "docs" / "progs.md").read_text()
    # A program's row starts `| `name` ...` or lists several: `| `hello`, `crash` | ...`.
    row_names = set()
    for line in progs_md.splitlines():
        if line.startswith("| `"):
            first_cell = line.split("|")[1]
            row_names.update(re.findall(r"`([^`]+)`", first_cell))

    user_programs = {}
    for tier in sorted(USER.glob("progs*")):
        for name in bin_names(tier):
            user_programs.setdefault(name, []).append(tier.name)
            if name not in row_names:
                problems.append(f"{tier.name}/{name}: no row in docs/progs.md")

    syscall_rs = (USER / "abi" / "src" / "syscall.rs").read_text()
    syscalls_md = (RUST / "docs" / "syscalls.md").read_text()
    for name, number in re.findall(r"pub const (SYS_\w+): usize = (\d+);", syscall_rs):
        if not re.search(rf"^\| {number} \|", syscalls_md, re.MULTILINE):
            problems.append(f"{name} ({number}): no row in docs/syscalls.md")

    test_readme = (STAGE / "test" / "README.md").read_text()
    for name in bin_names(STAGE / "test" / "progs"):
        if f"`{name}`" not in test_readme:
            problems.append(f"test/progs/{name}: not described in test/README.md")
        if name in user_programs:
            problems.append(f"test/progs/{name}: also exists under user/{user_programs[name][0]}/")

    if problems:
        print("docs check FAILED:")
        for p in problems:
            print(f"  {p}")
        return 1
    print(f"docs check ok ({len(user_programs)} programs, {len(re.findall('pub const SYS_', syscall_rs))} syscalls)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
