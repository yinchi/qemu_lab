"""Last updated: Stage 15 (the local time zone).

The real-time clock: the `clock_gettime` syscall (through `probe clock`) and the `date` program.

QEMU's PL031 starts from the host's clock and keeps counting, so `date +%s` must agree with the host's
own `time.time()` to within a few seconds, and advance while a busy-wait runs. Calendar and format
correctness (leap years, weekdays, every conversion) is in `timefmt`'s host tests; the QEMU tests here
use `date -d @N`, which prints a chosen instant, for the exact-output checks, and only compare the live
clock loosely.
"""

import datetime
import re
import time
from zoneinfo import ZoneInfo

TORONTO = ZoneInfo("America/Toronto")

HELP = (
    "date --help\n"
    "usage: date [-u] [-d @SECONDS] [-I[FMT] | -R | +FORMAT]\n"
    "  -u  print UTC instead of the local time (America/Toronto)\n"
    "  -d @N  show the time N seconds after 1970-01-01 00:00:00 UTC instead of now\n"
    "  -I[FMT]  ISO 8601: FMT is date (the default), hours, minutes or seconds\n"
    "  -R  RFC 5322 format\n"
    "  +FORMAT  strftime-style format (%Y %m %d %H %M %S %s %Z %z %a %A %b %B %e %j %F %T ... %%)\n"
)


def run(ctx):
    s, check = ctx.s, ctx.check

    # --- the syscall, through the test program ---
    s.run("chmod +x tests/probe.exe")
    check("clock_gettime: the clock, the other clock ids, bad pointers", s.run("tests/probe.exe clock"),
          "tests/probe.exe clock\n"
          "realtime: plausible=true nsec=0\n"
          "clock 1: -22\nclock 7: -22\nclock max: -22\n"
          "null pointer: -14\nwrapping pointer: -14\nread-only pointer: -14\n")

    # --- the live clock agrees with the host's, and moves ---
    def date_s():
        out = s.run("date +%s")
        return int(out.split("\n")[1])

    before = time.time()
    now = date_s()
    after = time.time()
    check("date +%s is the host's time (to 5 s)", before - 5 <= now <= after + 5, True)

    s.run("chmod +x tests/spin.exe")
    first = date_s()
    s.run("tests/spin.exe 2")
    second = date_s()
    check("date +%s advances across a 2 s busy-wait", 2 <= second - first <= 8, True)

    # --- the default layout, on the live clock: local time, in a real zone (EST or EDT) ---
    out = s.run("date").split("\n")[1]
    check("date's default layout",
          re.fullmatch(r"[A-Z][a-z]{2} [A-Z][a-z]{2} [ 0-9]\d \d\d:\d\d:\d\d E[SD]T \d{4}", out) is not None, True)
    now = datetime.datetime.now(TORONTO)
    check("...in the host's current year (America/Toronto)", out.endswith(f" {now.tzname()} {now.year}") or out.endswith(f" {now.year}"), True)
    out_utc = s.run("date -u").split("\n")[1]
    check("date -u prints UTC",
          re.fullmatch(r"[A-Z][a-z]{2} [A-Z][a-z]{2} [ 0-9]\d \d\d:\d\d:\d\d UTC \d{4}", out_utc) is not None, True)

    # --- exact output, for chosen instants (Toronto is UTC-5 in winter, UTC-4 in summer) ---
    for cmd, want in [
        ("date -d @0", "Wed Dec 31 19:00:00 EST 1969"),
        ("date -d @1000000000", "Sat Sep  8 21:46:40 EDT 2001"),
        ("date -u -d @1000000000", "Sun Sep  9 01:46:40 UTC 2001"),
        ("date -d @1000000000 -u", "Sun Sep  9 01:46:40 UTC 2001"),
        ("date --date=@951782400 +%F", "2000-02-28"),
        ("date -d @-1", "Wed Dec 31 18:59:59 EST 1969"),
        ("date -d @86400 +%Y-%j", "1970-001"),
        ("date -d @1234567890 '+%s %A %B'", "1234567890 Friday February"),
        ("date -d @1000000000 '+%-d/%_m/%b %j %A'", "8/ 9/Sep 251 Saturday"),
        ("date -d @1000000000 '+%Z %z'", "EDT -0400"),
        ("date -d @0 '+%Z %z'", "EST -0500"),
        ("date -u -d @0 '+%Z %z'", "UTC +0000"),
        ("date -d @0 '+100%%'", "100%"),
        ("date -d @1000000000 -I", "2001-09-08"),
        ("date -d @1000000000 -Iseconds", "2001-09-08T21:46:40-04:00"),
        ("date -d @1000000000 --iso-8601=minutes", "2001-09-08T21:46-04:00"),
        ("date -d @1000000000 -Ihours", "2001-09-08T21-04:00"),
        ("date -u -d @1000000000 -Iseconds", "2001-09-09T01:46:40+00:00"),
        ("date -d @1000000000 -R", "Sat, 08 Sep 2001 21:46:40 -0400"),
        ("date -d @1000000000 --rfc-email", "Sat, 08 Sep 2001 21:46:40 -0400"),
        ("date +%Y-%m-%d -d @86399", "1970-01-01"),  # options may follow the format
        # Daylight saving: the clocks jump forward at 02:00 EST on 2024-03-10 and back at 02:00 EDT on 2024-11-03.
        ("date -d @1710053999", "Sun Mar 10 01:59:59 EST 2024"),
        ("date -d @1710054000", "Sun Mar 10 03:00:00 EDT 2024"),
        ("date -d @1730613599", "Sun Nov  3 01:59:59 EDT 2024"),
        ("date -d @1730613600", "Sun Nov  3 01:00:00 EST 2024"),
    ]:
        check(cmd, s.run(cmd), f"{cmd}\n{want}\n")

    # --- refusals ---
    def refused(cmd, message, try_line=True):
        tail = "Try 'date --help' for more information.\n" if try_line else ""
        check(f"{cmd!r} is refused", s.run(cmd), f"{cmd}\ndate: {message}\n{tail}exit 1\n")

    refused("date foo", "invalid date 'foo'", try_line=False)
    refused("date -d tomorrow", "invalid date 'tomorrow'", try_line=False)
    refused("date -d @x", "invalid date '@x'", try_line=False)
    refused("date +%s extra", "invalid date 'extra'", try_line=False)
    # A conversion `chrono` does not know is an error, not printed as written (GNU prints it as written).
    check("date +%Q (an unknown conversion)", s.run("date '+%Q'"),
          "date '+%Q'\ndate: invalid format '+%Q'\nexit 1\n")
    refused("date -I -R", "multiple output formats specified", try_line=False)
    refused("date -Ix", "invalid argument 'x' for '--iso-8601'")
    refused("date -x", "invalid option -- 'x'")
    refused("date --now", "unrecognized option '--now'")
    refused("date -d", "option requires an argument -- 'd'")
    check("date --help", s.run("date --help"), HELP)

    check("shell alive", s.run("echo ok"), "echo ok\nok\n")
