"""The QEMU test harness: boots the kernel headless and drives it like a user would -- typing on the
virtio keyboard through the QEMU monitor's `sendkey` -- so a test case can check what each command
printed (from the serial log, which mirrors the console; see `fd.rs`'s `uart_write`), what is on the
display (`Session.screendump`), and, after QEMU has exited, what ended up on the disk.

Test cases live in `cases/`; `run_tests.py` runs them. Any `Kernel Panic!` or `Unexpected exception`
on the serial log fails the run at once, wherever it shows up (see `Session.wait_until`).
"""

import os
import re
import socket
import subprocess
import time

# Seconds between keys. QEMU's own monitor round trip for `sendkey` is sub-millisecond regardless of
# this value -- what actually gates it is the guest's own interrupt-handling-and-redraw pipeline, which
# drains the keyboard queue at roughly 35-40ms per key no matter how fast keys are sent (measured by
# sending a burst with no delay at all and timing how long the guest took to catch up). Below that rate,
# keys just pile up in the queue instead of the guest going any faster; below about 10ms on this
# machine, they pile up faster than the guest can drain them and the 16-slot test-build queue
# (`keyboard/queue.rs`'s `CAPACITY` under `testhooks`) overflows, which can drop the Enter that finishes
# a long typed line and hang the session waiting for a prompt that never comes. This value trades away
# some of that margin for speed (down from an original, untested 50ms) but keeps real headroom above the
# observed failure point, since going lower than the guest's own ~35-40ms/key ceiling buys nothing --
# it's not the bottleneck once you're below it.
KEY_DELAY = 0.025
HOLD_MS = 20  # how long QEMU holds each key down
TIMEOUT = 20  # seconds to wait for a prompt/text before giving up

# Characters that need a name other than themselves for the monitor's `sendkey`.
KEY_NAMES = {
    " ": "spc", ".": "dot", "-": "minus", "/": "slash", "=": "equal", ",": "comma",
    "+": "shift-equal", "_": "shift-minus", "\n": "ret",
    # Shell syntax: redirections, pipes, quoting, comments.
    ">": "shift-dot", "<": "shift-comma", "|": "shift-backslash", '"': "shift-apostrophe",
    "'": "apostrophe", "\\": "backslash", "#": "shift-3", "~": "shift-grave_accent",
    ";": "semicolon", "&": "shift-7", "$": "shift-4", "*": "shift-8", "?": "shift-slash",
    "!": "shift-1", "@": "shift-2", "%": "shift-5", "^": "shift-6", ":": "shift-semicolon",
    "(": "shift-9", ")": "shift-0", "[": "bracket_left", "]": "bracket_right",
    "{": "shift-bracket_left", "}": "shift-bracket_right", "`": "grave_accent",
}

# Non-text keys, for `Session.keys`: QEMU's own names for them (`s.keys([UP, UP, ENTER])`).
UP, DOWN, LEFT, RIGHT = "up", "down", "left", "right"
HOME, END, DELETE, BACKSPACE, ENTER = "home", "end", "delete", "backspace", "ret"
CTRL_A, CTRL_D, CTRL_E, CTRL_K, CTRL_U = "ctrl-a", "ctrl-d", "ctrl-e", "ctrl-k", "ctrl-u"

# The kernel built with the `testhooks` feature (what `just test` runs) prints one of these lines when a
# program ends; `Session.log` strips them so transcripts read the same as from a normal build.
TESTHOOK_LINE = re.compile(r"\[testhooks\] console_flushes=(\d+)\r?\n")

# What ends the serial log when the kernel has died -- never legitimate output of any test.
FATAL_MARKERS = ("Kernel Panic!", "Unexpected exception")


class Session:
    def __init__(self, elf, img, workdir, extra_imgs=()):
        """`extra_imgs` are `(path, before)` pairs: further disks attached with the system image, each before it
        on the QEMU command line (`before` true) or after it. (QEMU gives the devices virtio-mmio slots in the
        opposite order to the command line, which is why a test wants to choose.)"""

        self.serial_path = os.path.join(workdir, "serial.log")
        """Path to the serial log file."""

        self.sock_path = os.path.join(workdir, "mon.sock")
        """Path to the QEMU monitor socket."""

        open(self.serial_path, "w").close()
        disks = [(path, True) for path, before in extra_imgs if before] + [(img, True)]
        disks += [(path, False) for path, before in extra_imgs if not before]
        drives = []
        for n, (path, _) in enumerate(disks):
            drives += ["-drive", f"if=none,file={path},id=hd{n},format=raw", "-device", f"virtio-blk-device,drive=hd{n}"]
        self.qemu = subprocess.Popen(
            [
                "qemu-system-aarch64", "-M", "virt", "-cpu", "max", "-nodefaults",
                "-display", "none",
                "-serial", f"file:{self.serial_path}",
                "-monitor", f"unix:{self.sock_path},server,nowait",
                *drives,
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

    def log_bytes(self):
        """Returns the serial log exactly as received (testhooks lines included)."""
        with open(self.serial_path, "rb") as f:
            return f.read()

    def log(self):
        """Returns the current contents of the serial log, `\\r\\n` as `\\n`, testhooks lines removed."""
        text = self.log_bytes().decode(errors="replace").replace("\r\n", "\n")
        return TESTHOOK_LINE.sub("", text)

    def flush_counts(self):
        """The console flush count each program that has ended reported, in order (see `syscall/fd.rs`)."""
        text = self.log_bytes().decode(errors="replace")
        return [int(n) for n in TESTHOOK_LINE.findall(text)]

    def check_alive(self):
        """Raises if the kernel has panicked or QEMU has exited -- checked on every wait, so a
        crash anywhere in a session fails it at the point it happened, not at the next timeout."""
        log = self.log()
        for marker in FATAL_MARKERS:
            if marker in log:
                raise RuntimeError(f"kernel died ({marker!r}):\n{log[log.index(marker):]}")

    def screendump(self):
        """Captures the display through the monitor's `screendump`. Returns `(width, height,
        pixels)`, `pixels` being `height` rows of `width` `(r, g, b)` tuples."""
        path = os.path.join(os.path.dirname(self.serial_path), "screen.ppm")
        self.sock.sendall(f"screendump {path}\n".encode())
        deadline = time.time() + TIMEOUT
        while not (os.path.exists(path) and os.path.getsize(path) > 0):
            if time.time() > deadline:
                raise TimeoutError("screendump produced no file")
            time.sleep(0.05)
        time.sleep(0.1)
        self.sock.recv(4096)
        with open(path, "rb") as f:
            magic, dims, _maxval = f.readline(), f.readline(), f.readline()
            assert magic.strip() == b"P6", f"unexpected screendump format {magic!r}"
            width, height = map(int, dims.split())
            data = f.read()
        os.remove(path)
        rows = [[tuple(data[(y * width + x) * 3:(y * width + x) * 3 + 3]) for x in range(width)]
                for y in range(height)]
        return width, height, rows

    def screendump_settled(self, stable_for=0.2):
        """Like `screendump`, but waits until the display stops changing for at least `stable_for`
        seconds before returning a capture. A bare `screendump` captures whatever is in the framebuffer
        at that instant, and nothing guarantees the guest has finished reacting to the caller's last
        `type`/`keys`/`run` by then -- those only wait for QEMU to acknowledge the last keystroke (or,
        for `run`, for the *serial* transcript to show the next prompt), not for the guest's own redraw
        and virtio-gpu's own, separately-scheduled flush to the host to have actually happened. Two
        captures matching by coincidence -- both catching the same mid-draw or pre-flush frame,
        especially under the contention several concurrent QEMU sessions create -- would be a false
        "settled" signal; requiring the match to hold for a whole time window, not just one comparison,
        rules that out."""
        deadline = time.time() + TIMEOUT
        previous = self.screendump()
        stable_since = time.time()
        while time.time() < deadline:
            current = self.screendump()
            if current != previous:
                previous = current
                stable_since = time.time()
            elif time.time() - stable_since >= stable_for:
                return current
        raise TimeoutError("display did not settle")

    def run_raw(self, command):
        """Like `run`, but returns the bytes the command sent to the serial port, exactly -- for output
        that is not text. Everything between the echoed command line's end and the next prompt."""
        start = len(self.log_bytes())
        self.run(command)
        raw = TESTHOOK_LINE.sub("", self.log_bytes()[start:].decode("latin-1")).encode("latin-1")
        echoed = raw.index(b"\r\n") + 2  # the command line the shell echoed
        assert raw.endswith(b"> "), raw[-20:]
        return raw[echoed:-2]

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
            self.check_alive()
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

    def run_status(self, command):
        """Runs `command`, then `echo $?`: returns `(transcript, status)`, the transcript as `run` gives it and the
        status the shell reported, as an int. What to check where the exit status matters, now that the shell
        no longer prints one itself."""
        transcript = self.run(command)
        return transcript, int(self.run("echo $?").split("\n")[1])

    def status(self, command):
        """`run_status`'s status alone."""
        return self.run_status(command)[1]

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


ENVIRONMENT_FILE = "etc/environment"
DEFAULT_ENVIRONMENT = "HOME=/\n"  # what a test kernel boots with unless its module says otherwise


def set_environment(img_path, workdir, text):
    """Puts `text` in the image's `/etc/environment` (the initial environment, read once at boot), or removes
    the file if `text` is None. Only ever done to a group's private copy of the image, before it boots."""
    if text is None:
        subprocess.run(["mdel", "-i", img_path, f"::{ENVIRONMENT_FILE}"], check=True)
        return
    local = os.path.join(workdir, "environment")
    with open(local, "w") as f:
        f.write(text)
    subprocess.run(["mcopy", "-i", img_path, "-o", local, f"::{ENVIRONMENT_FILE}"], check=True)


def home_of(environment):
    """The `HOME` an environment file (text) names, `/` if it names none."""
    home = "/"
    for line in (environment or "").splitlines():
        if line.startswith("HOME="):
            home = line[len("HOME="):] or "/"
    return home


def set_profile(img_path, workdir, environment, content):
    """Puts `content` (text or bytes) in `$HOME/.profile` on the image, where `$HOME` is what the environment file
    `environment` says, or -- with `content` None -- removes the file, so the image's own default profile (which sits at
    `/root/.profile`, and which no test group's `HOME` reaches) is never involved. Private copy, before boot."""
    home = home_of(environment).strip("/")
    target = f"::{home + '/' if home else ''}.profile"
    if content is None:
        subprocess.run(["mdel", "-i", img_path, target], stderr=subprocess.DEVNULL)  # nothing to remove is fine
        return
    local = os.path.join(workdir, "profile")
    with open(local, "wb") as f:
        f.write(content if isinstance(content, bytes) else content.encode())
    subprocess.run(["mcopy", "-i", img_path, "-o", local, target], check=True)


def relabel(img_path, label=None, volume_id=None):
    """Gives the FAT volume in `img_path` a label and/or a volume ID (8 hex digits, as `XXXXXXXX`); `mlabel`
    writes both the boot sector and the root directory's label entry. Only on a private copy, before boot."""
    if volume_id is not None:
        subprocess.run(["mlabel", "-i", img_path, "-N", volume_id, "::"], check=True)
    if label is not None:
        subprocess.run(["mlabel", "-i", img_path, f"::{label}"], check=True)


def make_extra_disk(path, spec):
    """Creates an extra disk image from a description: a dict with `kind` `"fat"` (the default; `label`,
    `volume_id`, `size_kib`), `"blank"` (all zeros) or `"noise"` (deterministic random bytes, not a filesystem)."""
    kind = spec.get("kind", "fat")
    size_kib = spec.get("size_kib", 1024)
    if kind == "fat":
        cmd = ["mkfs.fat", "-C", "-n", spec.get("label", "NOLABEL"), "-i", spec.get("volume_id", "00000001")]
        subprocess.run(cmd + [path, str(size_kib)], check=True, capture_output=True)
    elif kind == "blank":
        with open(path, "wb") as f:
            f.truncate(size_kib * 1024)
    elif kind == "noise":
        import random
        rng = random.Random(spec.get("seed", 1))
        with open(path, "wb") as f:
            f.write(rng.randbytes(size_kib * 1024))
    else:
        raise ValueError(f"unknown extra disk kind {kind!r}")


def file_hash(path):
    """SHA-256 of a file, for checking that an image was left untouched."""
    import hashlib
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def lsblk_table(output):
    """Parses what `s.run("lsblk")` returned (the echoed command line, then the table) into one dict per device,
    keyed by the header's words. Columns are cut where the header's words start, so an empty cell reads as `""`."""
    lines = output.split("\n")[1:]
    lines = [line for line in lines if line != ""]
    header = lines[0]
    names = header.split()
    starts = [header.index(name) for name in names]
    rows = []
    for line in lines[1:]:
        ends = starts[1:] + [len(line) + 1]
        rows.append({name: line[a:b].strip() for name, a, b in zip(names, starts, ends)})
    return rows


CELL_W, CELL_H = 8, 16  # a console cell on the display


def text_bands(dump):
    """The rows of character cells of a `screendump` that are not blank, as `(cell_row, band)` where
    `band` is that cell row's pixels -- for comparing what two commands drew, glyph for glyph."""
    _w, height, rows = dump
    bands = []
    for r in range(height // CELL_H):
        band = rows[r * CELL_H:(r + 1) * CELL_H]
        if any(px != band[0][0] for line in band for px in line):
            bands.append((r, band))
    return bands


class Context:
    """What a case module gets: the live `session`, a `check(name, got, want)` that records a
    PASS/FAIL, and the host-side paths it may need."""

    def __init__(self, session, check, disk_dir, img, workdir, extra_imgs=(), extra_hashes=()):
        self.s = session
        self.check = check
        self.tests_dir = os.path.join(disk_dir, "tests")
        """Host copy of the fixtures on the image's `/tests/` -- the ground truth to compare against."""
        self.bin_dir = os.path.join(disk_dir, "bin")
        """Host copy of the image's `/bin/` -- every tier's staged `*`, the ground truth for what
        `ls bin` should list. Unlike `tests_dir`'s fixtures, this directory is *meant* to grow as
        programs are added, so a test checking its full contents should derive the expected list from
        here rather than hardcoding one."""
        self.img = img
        """The image QEMU is running against (a private copy; readable from the host once QEMU has exited)."""
        self.workdir = workdir
        self.extra_imgs = list(extra_imgs)
        """The paths of the group's extra disks (module attribute `EXTRA_DISKS`), in that order."""
        self.extra_hashes = list(extra_hashes)
        """Each extra disk's `file_hash` from before boot -- unequal afterwards means the guest wrote to it."""

    def fixture(self, name):
        """The text of a fixture under `disk/tests/`."""
        with open(os.path.join(self.tests_dir, name)) as f:
            return f.read()

    def fixture_path(self, name):
        return os.path.join(self.tests_dir, name)
