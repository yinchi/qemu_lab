"""Last updated: Stage 20, Step 1a.

The numeric keypad. NumLock starts **on**, as on a PC: the keypad types its digits and dot, and its operators
(`+ - * /`, which ignore NumLock as they do on a PC). With NumLock off the digits and dot are navigation
keys -- 4 Left, 6 Right, 7 Home, 1 End, dot Delete, 8 Up, 2 Down -- and the operators still type. The keypad's
Enter is Enter either way. A NumLock press toggles it exactly once.

The keypad is resolved in the token layer (`Token::char`, `abi::keys::effective_code`), so the shell's line
editor and a program's `read(0)` get it alike; the pure logic (the tables, the token fields) is in the host
tests, and this checks it end to end on the virtio keyboard.

Key repeat is not tested here: the monitor's `sendkey` presses and releases once, and the host's auto-repeat,
which the kernel now passes through (as a `repeat` token), only exists in a live window. The rules for what a
repeat may do (queued only into an empty queue, ignored for Ctrl+D) are in the host tests.
"""


def run(ctx):
    s, check = ctx.s, ctx.check

    # --- NumLock on (the boot state): digits, dot and operators type ---
    s.type("echo '")
    s.keys(["kp_1", "kp_2", "kp_3", "kp_decimal", "kp_add", "kp_subtract", "kp_multiply", "kp_divide"])
    s.type("'")
    s.keys(["kp_enter"])
    check("NumLock on: the keypad types digits, dot and operators; its Enter is Enter", s.wait_prompt(),
          "echo '123.+-*/'\n123.+-*/\n")

    # --- NumLock off: navigation, and the operators still type ---
    s.keys(["num_lock"])
    s.type("echo ac")
    s.keys(["kp_4"])  # Left
    s.type("b")
    s.keys(["kp_enter"])
    check("NumLock off: keypad 4 is Left", s.wait_prompt(), "echo abc\nabc\n")

    s.type("xecho abc")
    s.keys(["kp_7", "kp_decimal", "kp_1"])  # Home, Delete (removes the x), End
    s.type(" d")
    s.keys(["kp_enter"])
    check("NumLock off: keypad 7 is Home, dot is Delete, 1 is End", s.wait_prompt(), "echo abc d\nabc d\n")

    s.type("echo a")
    s.keys(["kp_9", "kp_3", "kp_0", "kp_5"])  # PgUp, PgDn, Insert, and 5, which navigates nowhere: nothing typed
    s.type("b")
    s.keys(["kp_add"])
    s.keys(["kp_enter"])
    check("NumLock off: the other keypad digits type nothing, the operators still do", s.wait_prompt(),
          "echo ab+\nab+\n")

    s.type("echo one\n")
    s.wait_prompt()
    s.type("echo two\n")
    s.wait_prompt()
    s.keys(["kp_8", "kp_enter"])  # Up recalls the last line
    check("NumLock off: keypad 8 is Up (history)", s.wait_prompt(), "echo two\ntwo\n")
    # Consecutive duplicates collapse, so history ends ... one, two. Up, Up reaches `echo one`; Down steps
    # forward to `echo two`, so a Down that did nothing would run `echo one` instead.
    s.keys(["kp_8", "kp_8", "kp_2", "kp_enter"])
    check("NumLock off: keypad 2 is Down (history)", s.wait_prompt(), "echo two\ntwo\n")

    # --- NumLock back on: one press toggled it once, and the digits type again ---
    s.keys(["num_lock"])
    s.type("echo ")
    s.keys(["kp_7", "kp_enter"])
    check("NumLock toggles back on with one press", s.wait_prompt(), "echo 7\n7\n")

    # --- a program's read(0) gets the keypad too ---
    s.type("cat\n")
    s.keys(["kp_4", "kp_2", "kp_enter"])
    s.keys(["ctrl-d"])
    got = s.wait_prompt()
    check("a program's read(0): the keypad types, its Enter ends the line", "42\n" in got, True)
    check("...and the shell is alive after", s.run("echo alive"), "echo alive\nalive\n")
