"""Last updated: Stage 12, Step 8.

Redirection: `>`/`>>` (stdout, defaulting to fd 1), `<` (stdin), `2>`/`2>>` (stderr) and `>&`/`2>&1`
(duplicating one descriptor onto another) all bind through the frame's
`stdio` triple (`with_stdio`, `exec/frame_stack.rs`): a builtin's own state change (`cd`) still
sticks under a redirect, redirections apply strictly left to right (order matters for `>&`), and any
error -- a failed redirect open, or the command itself -- is reported through whichever redirects on
the same line already succeeded, exactly like a program's own stderr.

Runs from `/tests`, where the fixtures and `probe.exe` live; returns to `/` at the end. Leaves several
small files behind (`f`, `g`, `e`, `o`, `both.txt`, `only_out.txt`, `shared.txt`, `lone.txt`, `newappend.txt`) -- `verify_disk`
checks a couple of them straight off the image, once QEMU has exited.
"""

def run(ctx):
    s, check = ctx.s, ctx.check
    s.run("chmod +x tests/probe.exe")  # self-sufficient, same reason as cwd.py's own copy of this line

    check("start from /tests", s.run("cd /tests"), "cd /tests\n")

    # --- T8.1: basic output redirection ---
    # Listing /bin (not the working directory) so the redirect's own output file, created in
    # /tests, can never show up as one of the entries being compared.
    listing_body = s.run("ls /bin").split("\n", 1)[1]
    check("ls /bin > listing.txt prints nothing itself",
          s.run("ls /bin > listing.txt"), "ls /bin > listing.txt\n")
    check("cat listing.txt matches ls /bin", s.run("cat listing.txt"), "cat listing.txt\n" + listing_body)

    # --- T8.2: > truncates, >> appends, >> on a missing file creates it ---
    check("echo x > f", s.run("echo x > f"), "echo x > f\n")
    check("f holds x", s.run("cat f"), "cat f\nx\n")
    check("echo y > f truncates", s.run("echo y > f"), "echo y > f\n")
    check("f now holds only y", s.run("cat f"), "cat f\ny\n")
    check("echo z >> f appends", s.run("echo z >> f"), "echo z >> f\n")
    check("f holds y then z", s.run("cat f"), "cat f\ny\nz\n")
    check(">> on a missing file creates it", s.run("echo w >> newappend.txt"), "echo w >> newappend.txt\n")
    check("newappend.txt holds w", s.run("cat newappend.txt"), "cat newappend.txt\nw\n")

    # --- T8.3: input redirection, and both directions together ---
    check("cat < f reads it", s.run("cat < f"), "cat < f\ny\nz\n")
    check("cat < f > g copies", s.run("cat < f > g"), "cat < f > g\n")
    check("g matches f", s.run("cat g"), "cat g\ny\nz\n")

    # --- T8.4: error cases; a failed redirect does not run the command ---
    check("< of a missing file errors, command not run",
          s.run("cat < nosuchfile"), "cat < nosuchfile\nnosuchfile: No such file or directory\n")
    check("> to a directory errors", s.run("echo x > /bin"), "echo x > /bin\n/bin: Is a directory\n")
    check("chmod -w f", s.run("chmod -w f"), "chmod -w f\n")
    check("> to a read-only file errors", s.run("echo x > f"), "echo x > f\nf: Permission denied\n")
    check("the read-only file is untouched", s.run("cat f"), "cat f\ny\nz\n")
    check("restore write permission on f", s.run("chmod +w f"), "chmod +w f\n")

    # --- T8.5: a command's own nonzero status still reports under a redirect ---
    check("false > f still reports exit 1", s.run("false > f"), "false > f\nexit 1\n")
    check("f was still truncated (the redirect took effect regardless)", s.run("cat f"), "cat f\n")

    # --- stdout redirect doesn't hide stderr, and vice versa ---
    check("a redirected stdout doesn't hide the command's own stderr",
          s.run("cat nosuchfile > o"),
          "cat nosuchfile > o\ncat: nosuchfile: No such file or directory\nexit 1\n")
    check("o is empty (nothing was ever written to stdout)", s.run("cat o"), "cat o\n")
    check("a redirected stderr leaves the console silent",
          s.run("cat nosuchfile 2> e"), "cat nosuchfile 2> e\n")
    check("e holds the error text and the exit status, both redirected the same way",
          s.run("cat e"), "cat e\ncat: nosuchfile: No such file or directory\nexit 1\n")

    # --- T8.5a: redirection applies to builtins too ---
    check("cd > f creates/truncates an empty file and still changes directory",
          (s.run("cd docs > f"), s.run("pwd")), ("cd docs > f\n", "pwd\n/tests/docs\n"))
    check("f is empty", s.run("cat ../f"), "cat ../f\n")
    check("cd back to /tests", s.run("cd .."), "cd ..\n")
    check("cd nosuch 2> e captures the error; console stays silent",
          s.run("cd nosuch 2> e"), "cd nosuch 2> e\n")
    check("e holds cd's own error", s.run("cat e"), "cat e\ncd: nosuch: No such file or directory\n")
    check("the working directory is unchanged", s.run("pwd"), "pwd\n/tests\n")
    check("a failed redirect open on a builtin reports and does not run it",
          s.run("cd bin > nosuchdir/f"), "cd bin > nosuchdir/f\nnosuchdir/f: No such file or directory\n")
    check("...and the working directory really is unchanged", s.run("pwd"), "pwd\n/tests\n")

    # --- T8.5b: stderr redirection, `>&`, and that redirect order matters ---
    # A bare name is only ever found in /bin (see cwd.py's "a bare name is not looked up in the
    # working directory"), so `probe.exe` here -- run from /tests -- has to be `./probe.exe`.
    # `probe interleave` writes "OUT" (fd 1, no newline), then "ERR" (fd 2, no newline), then a
    # newline (fd 1) -- so whichever stream(s) reach the console show up concatenated on one line.
    check("plain: both streams reach the console, interleaved in write order",
          s.run("./probe.exe interleave"), "./probe.exe interleave\nOUTERR\n")
    check("> f alone: only stdout moves, ERR still on the console",
          s.run("./probe.exe interleave > only_out.txt"), "./probe.exe interleave > only_out.txt\nERR\n")
    check("only_out.txt holds just OUT", s.run("cat only_out.txt"), "cat only_out.txt\nOUT\n")
    check("> f 2>&1: both move to f, sharing its position",
          s.run("./probe.exe interleave > both.txt 2>&1"), "./probe.exe interleave > both.txt 2>&1\n")
    check("both.txt holds OUT then ERR then the newline, in write order",
          s.run("cat both.txt"), "cat both.txt\nOUTERR\n")
    check("2>&1 > f: order reversed -- stderr dups the *old* stdout (the console), only stdout moves",
          s.run("./probe.exe interleave 2>&1 > only_out.txt"),
          "./probe.exe interleave 2>&1 > only_out.txt\nERR\n")
    check("only_out.txt again holds just OUT (truncated fresh)",
          s.run("cat only_out.txt"), "cat only_out.txt\nOUT\n")

    # A file shared by two fds (`2>&1`) stays open until the last reference goes: closing fd 1 must
    # not destroy what fd 2 (and the shell's own binding) still hold.
    check("program closes stdout under > f 2>&1: stderr still reaches f",
          s.run("./probe.exe close-out > shared.txt 2>&1"), "./probe.exe close-out > shared.txt 2>&1\n")
    check("shared.txt holds what was written to fd 2 after fd 1 closed (and was committed)",
          s.run("cat shared.txt"), "cat shared.txt\nclose(1)=0 write(1)=-9\n")
    check("program closes stdout under > f alone: the file survives, empty, and the shell is fine",
          s.run("./probe.exe close-out > lone.txt"), "./probe.exe close-out > lone.txt\nclose(1)=0 write(1)=-9\n")
    check("lone.txt exists and is empty", s.run("wc -c lone.txt"), "wc -c lone.txt\n0 lone.txt\n")

    check("back to /", s.run("cd .."), "cd ..\n")


def verify_disk(ctx):
    """Host-side confirmation (T8.1) that what a redirect wrote is really what `mtools` sees on the
    image, not just what the kernel's own `cat` reports back."""
    import os

    from harness import mcopy_out

    check = ctx.check
    dest = os.path.join(ctx.workdir, "both.txt")
    mcopy_out(ctx.img, "tests/both.txt", dest)
    with open(dest, "rb") as f:
        check("mtools sees both.txt's real bytes on disk", f.read(), b"OUTERR\n")

    shared_dest = os.path.join(ctx.workdir, "shared.txt")
    mcopy_out(ctx.img, "tests/shared.txt", shared_dest)
    with open(shared_dest, "rb") as f:
        check("mtools sees shared.txt's real bytes on disk (committed by the shell's own close)",
              f.read(), b"close(1)=0 write(1)=-9\n")

    listing_dest = os.path.join(ctx.workdir, "listing.txt")
    mcopy_out(ctx.img, "tests/listing.txt", listing_dest)
    with open(listing_dest) as f:
        check("mtools sees listing.txt is non-empty", len(f.read()) > 0, True)
