#!/usr/bin/env python3
"""Boots r11_busybox headless and drives it like a user would -- typing on the virtio keyboard
through the QEMU monitor's `sendkey` -- then checks what each command printed (from the serial
log, which mirrors the console; see `fd.rs`'s `uart_write`) and what ended up on the disk.

Usage: run_tests.py <kernel.elf> <disk.img>   (normally via `just test`)

The disk image is copied first, so the committed one is never modified.
"""

import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

KEY_DELAY = 0.05  # seconds between keys; QEMU queues them, this just avoids flooding
HOLD_MS = 20  # how long QEMU holds each key down
TIMEOUT = 20  # seconds to wait for a prompt/text before giving up

# Characters that need a name other than themselves for the monitor's `sendkey`.
KEY_NAMES = {
    " ": "spc", ".": "dot", "-": "minus", "/": "slash", "=": "equal", ",": "comma",
    "+": "shift-equal", "_": "shift-minus", "\n": "ret",
}


class Session:
    def __init__(self, elf, img, workdir):

        self.serial_path = os.path.join(workdir, "serial.log")
        """Path to the serial log file."""

        self.sock_path = os.path.join(workdir, "mon.sock")
        """Path to the QEMU monitor socket."""

        open(self.serial_path, "w").close()
        self.qemu = subprocess.Popen(
            [
                "qemu-system-aarch64", "-M", "virt", "-cpu", "max", "-nodefaults",
                "-display", "none",
                "-serial", f"file:{self.serial_path}",
                "-monitor", f"unix:{self.sock_path},server,nowait",
                "-drive", f"if=none,file={img},id=hd0,format=raw",
                "-device", "virtio-blk-device,drive=hd0",
                "-device", "virtio-gpu-device",
                "-device", "virtio-keyboard-device",
                "-kernel", elf,
            ],
            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
        )
        """The QEMU subprocess object."""

        self.sock = socket.socket(socket.AF_UNIX)
        """The QEMU monitor socket object."""

        deadline = time.time() + TIMEOUT
        while True:
            try:
                self.sock.connect(self.sock_path)
                break
            except (FileNotFoundError, ConnectionRefusedError):
                if time.time() > deadline or self.qemu.poll() is not None:
                    raise RuntimeError("QEMU did not start: " + self._stderr())
                time.sleep(0.1)

        time.sleep(0.3)
        self.sock.recv(4096)  # banner
        self.pos = 0
        self.wait_prompt()  # boot finished: the first prompt is on the serial log

    def _stderr(self):
        """Returns the current contents of the QEMU subprocess's standard error, if it has exited."""
        if self.qemu.poll() is None:
            return ""
        return self.qemu.stderr.read().decode(errors="replace")

    def log(self):
        """Returns the current contents of the serial log."""
        with open(self.serial_path, "rb") as f:
            return f.read().decode(errors="replace").replace("\r\n", "\n")

    def keys(self, names):
        """Sends a sequence of key names to the QEMU monitor, simulating key presses."""
        for name in names:
            self.sock.sendall(f"sendkey {name} {HOLD_MS}\n".encode())
            time.sleep(KEY_DELAY)
            self.sock.recv(4096)

    def type(self, text):
        """Types a string of text at the shell prompt, handling uppercase letters with Shift."""
        self.keys(KEY_NAMES.get(c) or (f"shift-{c.lower()}" if c.isupper() else c) for c in text)

    def wait_until(self, pred, what):
        """Waits until the predicate `pred` returns True for the new serial log content.
        Raises a TimeoutError if the condition is not met within the timeout period.
        
        Uses self.pos to track the position from which new log content should be considered.
        """
        deadline = time.time() + TIMEOUT
        while time.time() < deadline:
            new = self.log()[self.pos:]
            if pred(new):
                return new
            if self.qemu.poll() is not None:
                raise RuntimeError("QEMU exited: " + self._stderr())
            time.sleep(0.05)
        raise TimeoutError(f"timed out waiting for {what}; got {self.log()[self.pos:]!r}")

    def wait_prompt(self):
        """Waits for the shell's next prompt; returns everything printed since the last one,
        without the prompt itself."""
        new = self.wait_until(lambda t: t.endswith("> "), "prompt")
        self.pos = len(self.log())
        return new[: -len("> ")]

    def run(self, command):
        """Types `command` at the shell prompt; returns the transcript up to the next prompt --
        the echoed command line first, then whatever it printed."""
        self.type(command + "\n")
        return self.wait_prompt()

    def close(self):
        try:
            self.sock.sendall(b"quit\n")
        except OSError:
            pass
        try:
            self.qemu.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.qemu.kill()


def dir_attr(img_path, short_name):
    """The FAT attribute byte of the directory entry with 11-byte 8.3 name `short_name`."""
    data = open(img_path, "rb").read()
    i = data.find(short_name)
    assert i >= 0, f"no directory entry {short_name!r} on the disk"
    return data[i + 11]


def mcopy_out(img_path, name, dest):
    subprocess.run(["mcopy", "-i", img_path, f"::{name}", dest], check=True)


def main():
    elf, orig_img = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
    here = os.path.dirname(os.path.abspath(__file__))
    disk_dir = os.path.join(os.path.dirname(here), "disk")
    hello_txt = open(os.path.join(disk_dir, "hello.txt")).read()
    data_bin = os.path.join(disk_dir, "data.bin")

    workdir = tempfile.mkdtemp(prefix="r11-")
    img = os.path.join(workdir, "disk.img")
    shutil.copy(orig_img, img)

    failures = []

    def check(name, got, want):
        if got == want:
            print(f"PASS  {name}")
        else:
            print(f"FAIL  {name}\n  want: {want!r}\n  got:  {got!r}")
            failures.append(name)

    lines = hello_txt.splitlines(keepends=True)
    words = len(hello_txt.split())
    host_hexdump = subprocess.run(["hexdump", "-C", data_bin], capture_output=True, text=True, check=True).stdout

    s = Session(elf, img, workdir)
    try:
        # --- programs that already existed ---
        check("echo", s.run("echo hello world"), "echo hello world\nhello world\n")
        check("hello", s.run("hello"), "hello\nhello from userspace\n")
        check(
            "crash",
            s.run("crash"),
            "crash\nabout to crash\nSegmentation fault (address 0xffff800000000000, ESR_EL1 0x92000000)\nexit 139\n",
        )

        # --- launcher ---
        check("unknown program", s.run("nosuch"), "nosuch\nnosuch: not found\n")

        # --- cat / ls ---
        check("cat file", s.run("cat hello.txt"), "cat hello.txt\n" + hello_txt)
        check("cat subdirectory file", s.run("cat docs/example.txt"), "cat docs/example.txt\na file in a subdirectory\n")
        check("cat two files", s.run("cat docs/example.txt docs/example.txt"),
              "cat docs/example.txt docs/example.txt\n" + "a file in a subdirectory\n" * 2)
        check("cat missing", s.run("cat nosuch.txt"),
              "cat nosuch.txt\ncat: nosuch.txt: No such file or directory\nexit 1\n")
        check("cat directory", s.run("cat docs"), "cat docs\ncat: docs: Is a directory\nexit 1\n")
        check("ls", s.run("ls"), "ls\nbin\ndocs\nfonts\ndata.bin\nhello.txt\n")
        check("ls -F", s.run("ls -F"), "ls -F\nbin/\ndocs/\nfonts/\ndata.bin\nhello.txt\n")
        check("ls -F bin", s.run("ls -F bin"),
              "ls -F bin\n" + "".join(f"{n}.exe*\n" for n in
                  "cat chmod cp crash echo false head hello hexdump ls tail true wc".split()))
        check("ls file", s.run("ls hello.txt"), "ls hello.txt\nls: hello.txt: Not a directory\nexit 1\n")
        check("ls bad option", s.run("ls -x"), "ls -x\nls: unknown option: -x\nexit 1\n")

        # --- cp ---
        check("cp", s.run("cp hello.txt copy.txt"), "cp hello.txt copy.txt\n")
        check("cat copy", s.run("cat copy.txt"), "cat copy.txt\n" + hello_txt)
        check("cp binary", s.run("cp data.bin data2.bin"), "cp data.bin data2.bin\n")
        check("cp missing source", s.run("cp nosuch.txt x.txt"),
              "cp nosuch.txt x.txt\ncp: nosuch.txt: No such file or directory\nexit 1\n")
        check("cp directory source keeps dst", s.run("cp docs copy.txt"),
              "cp docs copy.txt\ncp: docs: Is a directory\nexit 1\n")
        check("cp overwrite shorter", s.run("cp docs/example.txt copy.txt"), "cp docs/example.txt copy.txt\n")
        check("cat overwritten", s.run("cat copy.txt"), "cat copy.txt\na file in a subdirectory\n")

        # --- head / tail / wc / hexdump ---
        check("head -n 3", s.run("head -n 3 hello.txt"), "head -n 3 hello.txt\n" + "".join(lines[:3]))
        check("head default", s.run("head hello.txt"), "head hello.txt\n" + "".join(lines[:10]))
        check("head -n 0", s.run("head -n 0 hello.txt"), "head -n 0 hello.txt\n")
        check("tail -n 2", s.run("tail -n 2 hello.txt"), "tail -n 2 hello.txt\n" + "".join(lines[-2:]))
        check("tail default", s.run("tail hello.txt"), "tail hello.txt\n" + "".join(lines[-10:]))
        check("tail -n 100", s.run("tail -n 100 hello.txt"), "tail -n 100 hello.txt\n" + hello_txt)
        check("head bad count", s.run("head -n x hello.txt"), "head -n x hello.txt\nusage: head [-n N] [file]\nexit 1\n")
        check("wc", s.run("wc hello.txt"), f"wc hello.txt\n{len(lines)} {words} {len(hello_txt)} hello.txt\n")
        check("wc -l", s.run("wc -l hello.txt"), f"wc -l hello.txt\n{len(lines)} hello.txt\n")
        check("wc -wc", s.run("wc -wc hello.txt"), f"wc -wc hello.txt\n{words} {len(hello_txt)} hello.txt\n")
        check("hexdump", s.run("hexdump data.bin"), "hexdump data.bin\n" + host_hexdump)
        check("hexdump of copy", s.run("hexdump data2.bin"), "hexdump data2.bin\n" + host_hexdump)

        # --- exit status ---
        check("true", s.run("true"), "true\n")
        check("false", s.run("false"), "false\nexit 1\n")

        # --- chmod ---
        check("chmod -x", s.run("chmod -x bin/hello.exe"), "chmod -x bin/hello.exe\n")
        check("run without exec bit", s.run("hello"), "hello\nhello: not executable\n")
        check("chmod +x", s.run("chmod +x bin/hello.exe"), "chmod +x bin/hello.exe\n")
        check("run with exec bit", s.run("hello"), "hello\nhello from userspace\n")
        check("chmod -w", s.run("chmod -w copy.txt"), "chmod -w copy.txt\n")
        check("cp onto read-only", s.run("cp hello.txt copy.txt"),
              "cp hello.txt copy.txt\ncp: copy.txt: Permission denied\nexit 1\n")
        check("chmod bad mode", s.run("chmod 755 copy.txt"), "chmod 755 copy.txt\nchmod: invalid mode: 755\nexit 1\n")
        check("chmod missing", s.run("chmod +x nosuch"),
              "chmod +x nosuch\nchmod: nosuch: No such file or directory\nexit 1\n")

        # --- stdin: a program reading typed lines (Backspace absorbed, Ctrl+D ends) ---
        s.type("cat\n")
        s.wait_until(lambda t: t.endswith("cat\n"), "cat to start")
        s.type("hellp")
        s.keys(["backspace"])
        s.type("o\n")
        s.wait_until(lambda t: t.endswith("hello\nhello\n"), "cat to echo the line back")
        s.type("second line\n")
        s.wait_until(lambda t: t.endswith("second line\nsecond line\n"), "cat to echo the second line")
        s.keys(["ctrl-d"])
        check("cat stdin", s.wait_prompt(), "cat\nhello\nhello\nsecond line\nsecond line\n")

        s.type("wc\n")
        s.wait_until(lambda t: t.endswith("wc\n"), "wc to start")
        s.type("one two\nthree\n")
        s.wait_until(lambda t: t.endswith("three\n"), "wc to read")
        s.keys(["ctrl-d"])
        check("wc stdin", s.wait_prompt(), "wc\none two\nthree\n2 3 14\n")

        s.type("tail -n 1\n")
        s.wait_until(lambda t: t.endswith("tail -n 1\n"), "tail to start")
        s.type("a\nb\nc\n")
        s.wait_until(lambda t: t.endswith("c\n"), "tail to read")
        s.keys(["ctrl-d"])
        check("tail stdin", s.wait_prompt(), "tail -n 1\na\nb\nc\nc\n")
    finally:
        s.close()

    # --- what ended up on the disk (QEMU is gone, so the image is quiescent) ---
    out = os.path.join(workdir, "out")
    os.makedirs(out)
    mcopy_out(img, "copy.txt", os.path.join(out, "copy.txt"))
    mcopy_out(img, "data2.bin", os.path.join(out, "data2.bin"))
    check("disk: copy.txt is the overwritten content",
          open(os.path.join(out, "copy.txt")).read(), "a file in a subdirectory\n")
    check("disk: data2.bin matches data.bin byte for byte",
          open(os.path.join(out, "data2.bin"), "rb").read(), open(data_bin, "rb").read())
    check("disk: copy.txt is read-only", dir_attr(img, b"COPY    TXT") & 0x01, 0x01)
    check("disk: hello.exe has the exec bit again", dir_attr(img, b"HELLO   EXE") & 0x40, 0x40)

    shutil.rmtree(workdir, ignore_errors=True)
    if failures:
        print(f"\n{len(failures)} failed: {', '.join(failures)}")
        return 1
    print("\nall passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
