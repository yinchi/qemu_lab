#!/usr/bin/env python3
"""Records the golden transcript for `cases/step04_line_discipline.py` from an older stage's kernel.

Usage: mkgolden.py <kernel.elf> <disk.img> <out.json>
e.g.   mkgolden.py ../../r11_busybox/r11_busybox.elf ../../r11_busybox/disk.img golden/step04_r11.json

Run once, on r11 (`just build disk` there first); the result is checked in, so `just test` does not depend
on the r11 directory.
"""

import json
import os
import subprocess
import sys
import tempfile

from cases.step04_line_discipline import golden_session
from harness import Session


def main():
    elf, img, out = (os.path.abspath(a) for a in sys.argv[1:4])
    workdir = tempfile.mkdtemp(prefix="golden-")
    copy = os.path.join(workdir, "disk.img")
    subprocess.run(["cp", "--sparse=always", img, copy], check=True)
    s = Session(elf, copy, workdir)
    try:
        transcripts = golden_session(s)
    finally:
        s.close()
    with open(out, "w") as f:
        json.dump(transcripts, f, indent=1, ensure_ascii=False)
        f.write("\n")
    print(f"wrote {len(transcripts)} transcripts to {out}")


if __name__ == "__main__":
    main()
