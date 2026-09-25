"""Last updated: Stage 17 (the zone comes from `$TZ`).

The real-time clock: the `clock_gettime` syscall (through `probe clock`) and the `date` program.

QEMU's PL031 starts from the host's clock and keeps counting, so `date +%s` must agree with the host's
own `time.time()` to within a few seconds, and advance while a busy-wait runs. Calendar and format
correctness (leap years, weekdays, every conversion) is in `timefmt`'s host tests; the QEMU tests here
use `date -d @N`, which prints a chosen instant, for the exact-output checks, and only compare the live
clock loosely.
"""

# The zone `date` and `stat` use is `$TZ`: this group's environment names Toronto, as the expectations below assume.
ENVIRONMENT = "HOME=/\nTZ=America/Toronto\n"

import datetime
import re
import time
from zoneinfo import ZoneInfo

TORONTO = ZoneInfo("America/Toronto")

HELP = (
    "date --help\n"
    "usage: date [-u] [-d @SECONDS] [-I[FMT] | -R | +FORMAT]\n"
    "  -u  print UTC instead of the local time ($TZ)\n"
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
        check(f"{cmd!r} is refused", s.run_status(cmd), (f"{cmd}\ndate: {message}\n{tail}", 1))

    refused("date foo", "invalid date 'foo'", try_line=False)
    refused("date -d tomorrow", "invalid date 'tomorrow'", try_line=False)
    refused("date -d @x", "invalid date '@x'", try_line=False)
    refused("date +%s extra", "invalid date 'extra'", try_line=False)
    # A conversion `chrono` does not know is an error, not printed as written (GNU prints it as written).
    check("date +%Q (an unknown conversion)", s.run_status("date '+%Q'"), ("date '+%Q'\ndate: invalid format '+%Q'\n", 1))
    refused("date -I -R", "multiple output formats specified", try_line=False)
    refused("date -Ix", "invalid argument 'x' for '--iso-8601'")
    refused("date -x", "invalid option -- 'x'")
    refused("date --now", "unrecognized option '--now'")
    refused("date -d", "option requires an argument -- 'd'")
    check("date --help", s.run("date --help"), HELP)

    # --- the zone is $TZ (Stage 17): an IANA name, UTC when it is unset, empty or unknown ---
    instant = 1719792000  # 2024-07-01 00:00:00 UTC: summer in every northern zone below
    fmt = "%a %b %e %H:%M:%S %Z %Y"

    def local(zone):
        return datetime.datetime.fromtimestamp(instant, tz=ZoneInfo(zone)).strftime(fmt)

    def date_in(command_prefix, zone):
        cmd = f"{command_prefix}date -d @{instant}"
        check(cmd, s.run(cmd), f"{cmd}\n{local(zone)}\n")

    date_in("", "America/Toronto")  # the environment file's
    date_in("TZ=America/Vancouver ", "America/Vancouver")  # for one command
    date_in("TZ=Europe/London ", "Europe/London")
    date_in("TZ=Asia/Tokyo ", "Asia/Tokyo")
    date_in("TZ=:Asia/Tokyo ", "Asia/Tokyo")  # POSIX's leading colon
    date_in("TZ=UTC ", "UTC")
    date_in("TZ=Australia/Adelaide ", "Australia/Adelaide")  # a half-hour offset
    check("the prefix went with the command", s.run("date -d @0 '+%Z'"), "date -d @0 '+%Z'\nEST\n")
    s.run("export TZ=Europe/London")
    date_in("", "Europe/London")
    date_in("TZ=Asia/Tokyo ", "Asia/Tokyo")  # a prefix overrides the exported one
    s.run("TZ=Asia/Tokyo")  # a plain assignment keeps the variable exported
    date_in("", "Asia/Tokyo")
    s.run("unset TZ")
    date_in("", "UTC")  # unset
    s.run("export TZ=")
    date_in("", "UTC")  # empty
    s.run("export TZ=Mars/Olympus")
    date_in("", "UTC")  # not in the database
    s.run("export TZ=america/toronto")
    date_in("", "UTC")  # names are case-sensitive
    s.run("export TZ=EST5EDT,M3.2.0,M11.1.0")
    date_in("", "UTC")  # POSIX rule strings are not understood
    s.run("export TZ=EST5EDT")
    date_in("", "EST5EDT")  # ...but the database's own EST5EDT is a name
    check("-u still wins over TZ", s.run("date -u -d @0 '+%Z'"), "date -u -d @0 '+%Z'\nUTC\n")
    s.run("unset TZ")

    # `stat` reads it too: the FAT epoch (1980-01-01 00:00:00 UTC) on the root directory.
    def stat_root(prefix, stamp):
        cmd = f"{prefix}stat /"
        check(cmd, s.run(cmd),
              f"{cmd}\n  File: /\n  Size: {0:<12} Type: directory\n Attrs: read-only=no  exec=no\n"
              f"Modify: {stamp}\nCreate: {stamp}\n")

    stat_root("", "1980-01-01 00:00:00 UTC")  # TZ is unset by now
    stat_root("TZ=America/Toronto ", "1979-12-31 19:00:00 EST")
    stat_root("TZ=UTC ", "1980-01-01 00:00:00 UTC")
    stat_root("TZ=Asia/Tokyo ", "1980-01-01 09:00:00 JST")
    stat_root("TZ=Mars/Olympus ", "1980-01-01 00:00:00 UTC")

    check("shell alive", s.run("echo ok"), "echo ok\nok\n")
