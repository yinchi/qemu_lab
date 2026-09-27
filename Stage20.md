# Stage 20: a nano-style full-screen editor -- `r20_editor` (plan)

`ROADMAP.md` carries the summary of this stage; this file is the plan: the decisions, and the steps in the order they are built and committed. It is updated as each step lands (an "As built" note per step, as `Stage19.md` does).

| Step | What | Status |
|---|---|---|
| R | Roadmap: the Stage 20 section rewritten for a nano-style editor (it leaned on vi), Stage 20b split out, two later passages that named vim/vi reworded | done |
| 0 | Plain copy of `r19_mounts` as `r20_editor` | done |
| 1 | `TIOCGWINSZ` on the console `ioctl`, and a test program that prints the size | done |
| 1a | Keyboard layer: the held-key repeat check, `Token.num` and NumLock (on at boot), keypad characters, `abi::keys` with `effective_code`, the repeat bit with coalescing, lock flips gated on the down edge | done, except the live held-key check (see its notes) |
| 2 | `CONSOLE_READ_KEY`: one key event per call, blocking; a test program that echoes events | todo |
| 3 | `CONSOLE_DRAW`: draw a grid of cells (inverse, dim) and place the cursor; a test program that draws a frame | todo |
| 4 | The editor's pure modules, host-tested: buffer, soft-wrap layout (buffer position <-> screen cell), cursor and scroll math, display width, region and cut buffer, auto-indent, `.editrc` parser | todo |
| 5 | `edit`: open, view, scroll, quit; `.editrc` loaded (a new `progs_r20` tier) | todo |
| 6 | Editing, save and save-as, status bar, prompt widget, exit with unsaved changes, `^G` help screen | todo |
| 7 | Line cut, copy and paste; search; go to line | todo |
| 8 | Mark and region cut/copy, inverse-video selection, the line-number gutter, the auto-indent toggle | todo |
| 9 | Docs, roadmap "As built", regression sweep, the persistence demo | todo |

Stage 20b (`column`, `ls` in columns) is planned separately in `ROADMAP.md` and is not a step of this file.

## Decisions (settled)
- **Nano-shaped, not vi.** Modeless, GNU nano's keys, one program called `edit`. The parts a vi keymap would need (the buffer, the view, file I/O, drawing) are the same either way, but no vi keymap is planned.
- **Everything the editor needs from the kernel is a console `ioctl`** (on fd 0, 1 or 2 while they are the terminal -- fd 0 is the keyboard, fd 1 and 2 the console -- and `ENOTTY` otherwise; each request answers where it makes sense: `TIOCGWINSZ` on all three, `CONSOLE_READ_KEY` on stdin, `CONSOLE_DRAW` on stdout/stderr): `TIOCGWINSZ`, `CONSOLE_READ_KEY`, `CONSOLE_DRAW`. This replaces the raw-mode toggle on `read(0)` that the roadmap first planned: `read(0)` and `CONSOLE_READ_KEY` are two ways to drain the *one* token queue (through the line discipline into a line, or one token as a record), so there is no hidden mode, nothing to restore on exit or fault, and no `read()` whose shape depends on state. `PENDING` (unread bytes of a finished line) is untouched by `CONSOLE_READ_KEY` and cleared at launch; no echo; using both calls in one program is unsupported, not guarded.
- **The key record (8 bytes):** `code` u16 (raw evdev, never rewritten), `mods` u8 (SHIFT=1, CTRL=2, ALT=4, CAPS=8, NUM=16, REPEAT=32), a pad byte, `ch` u32 (the text the key produces, else 0; the kernel keeps the layout, so the editor needs no code-to-character table). It mirrors the kernel's `Token`. Commands match on `effective_code` plus modifiers, never on `ch` or CapsLock, so `^S` is `^S` with CapsLock on. Insert `ch` only when neither Ctrl nor Alt is set.
- **`effective_code(code, num) -> u16`** (a pure function in a new `abi::keys` module, which also holds the `KEY_*` evdev constants and the record): `KpEnter` is Enter; **the keypad's `/` and `-` are the main block's `KEY_SLASH` and `KEY_MINUS` whatever NumLock says** (as on a PC, where the operator keys ignore NumLock; the plan first said "with NumLock on"); with NumLock **off** `Kp7/8/9/4/6/1/2/3/0/Dot` are Home/Up/PgUp/Left/Right/End/Down/PgDn/Insert/Delete; with NumLock **on** `Kp0..Kp9`, `KpDot`, `KpSlash`, `KpMinus` become their main-block twins (`KEY_0..KEY_9`, `KEY_DOT`, `KEY_SLASH`, `KEY_MINUS`), so `Alt+Numpad6` is `Alt+6`. `KpAsterisk` and `KpPlus` have no unshifted twin and stay as they are (text through `ch` only). Shift is ignored for keypad keys. The kernel's line discipline uses it too.
- **NumLock starts on** (`LockState::new()`; it was off), as on a PC. It was tracked but never read, so the keypad produced nothing anywhere; `Token::char()` resolves the keypad's characters when NumLock is set.
- **Key repeat: the device's own.** `token_for` emits a token only when `KeyState::set` reports a change, which swallows a held key's repeats (its comment says the virtio-input/QEMU setup resends plain presses). A down event for an already-held key becomes a token with the REPEAT bit; a REPEAT token is dropped if the queue is not empty (no backlog that keeps the cursor moving after release; the 256-entry queue cannot fill with repeats); `LockState::apply` flips only on a down edge, or a resent NumLock/CapsLock press would toggle repeatedly. The editor accepts repeats for movement and text keys and ignores them for command keys. **First action of Step 1a** is to confirm what QEMU sends for a held key (a temporary UART log of `EV_KEY` code and value on a scratch build, GTK display). **Only if QEMU sends nothing** is a kernel timer (initial delay, rate, the Stage 3 timer, only while a program is reading keys) the fallback. Tests inject keys and are unaffected either way.
- **`CONSOLE_DRAW(cells, rows, cols, cursor_row, cursor_col)`; `Cell` = `ch` u32, `attr` u8, 3 pad bytes.** Attribute flags `ATTR_INVERSE` = 1 and `ATTR_DIM` = 2, no colour values in the ABI. The console's only colour constants are `FG` (0xFF55FF55) and `BG` (0xFF000000), `src/console/mod.rs`; INVERSE swaps them (as `line_discipline.rs` does for its cursor block), DIM adds one constant, `DIM_FG` (about half brightness, e.g. 0xFF2A7F2A), and INVERSE+DIM is black on `DIM_FG`. Cells hold printable characters and spaces only; the editor expands tabs itself (the console's own `TAB_WIDTH` is irrelevant). Rejected: a mini ANSI subset on `write`, which would bring back the escape vocabulary this project's display design avoids.
- **`.editrc`**: `$HOME/.editrc`, the `/etc/environment` format (`NAME=VALUE`, blanks and `#` comments ignored, malformed lines skipped with a note), three names: `TABSIZE=<1..16>` (4), `LINENOS=<0|1>` (0), `AUTOINDENT=<0|1>` (0). Read once at start-up; a missing file or unset `$HOME` is silent; a bad value gets one message-line note and that name's default. The editor has its own ~20-line parser (host-tested); the kernel's `shell/environment.rs` is in another crate, and only the format is shared.
- **Save in place:** truncate and write the whole buffer; a failed `write` is reported on the message line and the buffer stays modified. Accepted risk: a crash or a full disk mid-write leaves a truncated file. **LF-only, lossy:** carriage returns are dropped and invalid UTF-8 replaced on open, so a save can rewrite a CRLF or binary file (a one-line "N bytes replaced" warning on open is the cheap fix if it bites).
- **Long lines soft-wrap; there is no horizontal scrolling and no toggle** (one view code path). A line breaks at the last cell that fits (nano's and vim's default), not at blanks; wrapping at blanks (nano's `--atblanks`) can be added later inside the layout module without changing its interface. Tabs are stored as `\t` and drawn to the next `TABSIZE` stop, and a tab that crosses the row edge continues on the next row; a wide glyph takes two cells and, if only one cell is left in the row, goes to the next row (the console's own rule). A line that exactly fills the row gets one more, empty, row so the cursor can sit after its last character.
- **The layout module is the heart of the view** (pure, host-tested, Step 4): for one line and the current text width it yields the screen rows as character ranges, and maps buffer position (line, character) to (screen row within the line, cell column) and back. Consequences: **Up/Down move by screen row**, keeping a preferred *display* column, not a character index; **Home/End go to the start/end of the logical line**; **the view scrolls by whole logical lines**: its top is always the first row of a line, moved just far enough to keep the cursor's row visible (the line at the bottom edge may be cut off, drawn as far as it fits), and a far jump (go to line, search) centres the cursor's line by walking back from it, at most a screenful of rows. **The one exception is a line taller than the text area** (a 3000-character line is about 38 rows against 27 of text; a line that exactly fits is normal): while the cursor is in it the top is (line, screen row) and Up/Down scroll row by row through it, and when the cursor leaves it the top snaps back to a line start. PgUp/PgDn move the cursor a screenful of rows and the top re-anchors by the same rules. Layout is computed on demand for the visible lines and the cursor's line, never for the whole file per key, and `Ln x/y` counts logical lines so no total row count is needed. The text width is the screen width minus the gutter, so it is an input, not a constant: `Alt+N`, or the line count gaining a digit, simply changes it and the next frame re-lays-out. Continuation rows show a blank gutter; the mark's region and the cursor are decided per screen row.
- **Full-frame redraw per key, on purpose.** The screen is 640x480 (80x30 cells of 8x16), and the GPU driver's `flush()` takes no rectangle, so a flush costs the same whatever changed; single-cell updates would save only glyph drawing. A frame rebuilt from the model means the screen equals the model by construction (no stale cells, no half wide glyphs) and the ABI is one stateless call. If Step 3's timing shows drawing 2400 cells per key is too slow, the kernel may keep the previous frame and skip unchanged cells: an internal change, not an ABI one.
- **Tests never touch `home.img`**, the user's real home disk at the repository root (see "Tests").
- **`column` and `ls` in columns are Stage 20b**, not this stage. This stage only guarantees the two things they need: `TIOCGWINSZ`, and a display-width function written as a pure module with no editor types in it, so 20b can lift it into `userlib`.
- **Documentation stays stage-agnostic** (`<stage>` for what changes per stage; name the stage for a change attributable to one): new rows say "From Stage 20".

## The key set
Names follow GNU nano; Alt combinations use the ALT bit; no Ctrl+Alt combinations (QEMU's mouse-release chord).

**Core**

| Key | Action |
|---|---|
| printable, Enter, Tab | insert a character / split the line / insert `\t` |
| Backspace (`^H`), Delete (`^D`) | delete before / under the cursor; at a line's edge, join lines |
| arrows, `^B` `^F` `^P` `^N` | move by character / screen row |
| Home `^A`, End `^E` | start / end of the logical line |
| PgUp `^Y`, PgDn `^V` | a screenful up / down |
| Ctrl+Left, Ctrl+Right | word left / right |
| `Alt+\`, `Alt+/` | first / last line |
| `^S` | save (asks for a name only for a new buffer) |
| `^O` | write out: filename prompt |
| `^X` | exit; if modified, "Save modified buffer? (Y/N/Cancel)" |
| `^G` | help screen listing this table, drawn through `CONSOLE_DRAW`; any key returns |
| `^C` | show the position (line/total, column, character count) on the message line |
| Esc | cancel any prompt |

**Cut, search, goto**

| Key | Action |
|---|---|
| `^K` | cut the line into the cut buffer (consecutive cuts append, as nano's) |
| `Alt+6` | copy the line |
| `^U` | paste the cut buffer at the cursor |
| `^W` | search forward; the prompt is pre-filled with the last term; "Not found" on the message line |
| `Alt+W` | find next |
| `^_` (Ctrl+Shift+-), `Alt+G` | go to `line[,col]` prompt |

**Mark, gutter, auto-indent**

| Key | Action |
|---|---|
| `Alt+A` | set / clear the mark; the region between mark and cursor is drawn `ATTR_INVERSE` |
| `^K` / `Alt+6` / `^U` with a mark set | cut / copy / paste the *region* (multi-line, character-granular; the cut buffer is text with embedded newlines) |
| `Alt+N` | toggle the line-number gutter (digits of the line count + 1 wide, drawn `ATTR_DIM`; it narrows the text width the layout wraps to); starts as `LINENOS` says |
| `Alt+I` | toggle auto-indent: Enter copies the line's leading spaces and tabs to the new line (Enter on a whitespace-only line leaves it empty, as nano does); starts as `AUTOINDENT` says |

`Ctrl+Z` is unbound (Stage 22's kernel-level suspend is the only thing that will ever see it). The cut buffer and cursor model are settled in Step 4 (a region is two ordered (line, column) positions), so the mark's region cut is not a rewrite of the line cut.

## Screen layout
Rows 0 to R-3: text (and, when on, the gutter), soft-wrapped. Row R-2: an inverse-video status bar (`name [modified]   Ln 12/40, Col 5`). The last row (or two): the help footer (`^S Save  ^X Exit  ^K Cut  ^U Paste  ^W Find  ^G Help`), replaced by a message or a prompt while one is showing. R and C come from `TIOCGWINSZ`. The whole frame is rebuilt into a `Vec<Cell>` and submitted with one `CONSOLE_DRAW` per key event.

## Not doing
Undo and redo (`Alt+U`/`Alt+E`, an edit log of position and text), search-and-replace, backward and case-insensitive search, wrapping at blanks, inserting a file (`^R`), syntax highlighting, spell check (`^T`), justify (`^J`), multiple buffers, the mouse, macros, options beyond `edit FILE` (`+LINE` is cheap and may be added).

## Tests
- Host tests (`hosttests/`) for the pure modules: buffer, cursor and scroll, width, region and cut buffer, auto-indent, `.editrc`, and the keyboard layer (`effective_code`, repeat coalescing, lock gating).
- The QEMU suite types on the virtio keyboard: open, edit, save, exit with unsaved changes, cut and paste, search, mark, the gutter, the keypad with NumLock on and off. **The token queue holds 256 tokens and drops the newest when full; a `testhooks` build shrinks it to 16**, so a case typing a long string faster than the editor redraws can overflow it: type in short bursts (or wait for a frame between them), as the `token_queue` cases already do. The input flake that waits for Stage 26 applies: retry once.
- **Disk-image safety (hard rule).** The repository's `home.img` is the user's real persistent home folder (created once, replaced only by `just home-reset`). No test, fixture, helper or demo script may open, attach, copy over, `mcopy`/`mdel`/`mlabel`, or reset it. Tests build their own disks in the harness's workdir with `test/harness.py`'s `extra_imgs`, `make_extra_disk`, `relabel` and `mcopy_out` (a scratch FAT "home" image carrying the label and volume ID the kernel mounts at `/root`, and any `.editrc` a case needs). The persistence check (edit, save, power off, rebuild the system image, boot again) is automated the same way, against one scratch home image and a freshly built scratch system image. The harness asserts that no drive path resolves to `<repo root>/home.img`, so a mistake fails loudly. A step that really needs the real one is an explicit user action, never a script.

## Step 0 -- plain copy `rust/r20_editor`
As Stage 19's Step 0: copy the tracked files of `r19_mounts` (so no build artifacts or generated fixtures come along) as `r20_editor` and rename in `Cargo.toml`, `Cargo.lock`, `justfile` `BIN`, `hosttests/src/lib.rs`, `test/check_docs.py`, `test/run_tests.py` (its docstring and the `r20-` temp-dir prefix), `test/README.md`, `disk/tests/notes.txt` and `disk/fonts/NOTICE`. No new `user/` tier yet: the `progs_r19` tier is built as it is, and `progs_r20` arrives with the first program of this stage (Step 5). Nothing behavioral changes, so the whole existing suite must pass unchanged in the copy.

**As built (Step 0).** Copied the 179 tracked files of `r19_mounts` and renamed in the nine files above (the same nine places Stage 19's copy touched). No `user/` change. The ROADMAP rewrite and this file are in the same commit, as Stage 19's were. Regression run in `r20_editor` (`just test`): 316 host tests and 1955 QEMU-suite checks passed, none failed (the suite is slow, since it carries the cases of every stage before it).

## Step 1 -- `TIOCGWINSZ`
The console `ioctl` (the one `clear` uses, whose `ENOTTY` already means "not the console") gains Linux's request number and `struct winsize` (rows, columns, two pixel fields left 0), answered for fd 0, 1 and 2 while they are the terminal, `ENOTTY` when redirected. Constants and the struct go in `abi` (`abi::ioctl`), a wrapper in `userlib`, a row in `docs/syscalls.md`. Test program: prints the size, and the size through `> file` (`ENOTTY`). The shell's start-up may export `COLUMNS` and `LINES` as a convenience; nothing depends on them.

**As built (Step 1).** `abi::ioctl` gained `TIOCGWINSZ` (0x5413) and `WinSize` (`encode`/`decode`, Linux's four little-endian `u16`s, host-tested for layout and round trip); the module's comment now names this as the one request that keeps Linux's number. `userlib::winsize(fd) -> Result<WinSize, isize>` wraps it. In the kernel, `syscall/fd.rs`'s `ioctl` answers `TIOCGWINSZ` for **both** `FileDescriptor::Console` and `FileDescriptor::Keyboard` (fd 0 is the keyboard, a variant of its own, so answering only the console would have left stdin unable to say it is a terminal), through a new `window_size` that validates the output pointer as writable user memory (`EFAULT`); `CONSOLE_CLEAR` stays console-only, so `ioctl 0 1` is still `ENOTTY`. Test programs: `probe winsize FD` and `probe winsize-ptr FD ADDR` (a chosen output pointer). Fourteen new checks in `cases/console.py`: 30 rows x 80 columns (640x480 pixels of 8x16 cells) from fd 0, 1 and 2; `EBADF` for a closed fd; `ENOTTY` for redirected stdin and stdout, while stderr still answers when only stdout is redirected; and `EFAULT` for six bad output pointers (null, below the window, read-only code, straddling the window's top, the top itself, a wrapping address). `COLUMNS`/`LINES` were not added (nothing needs them yet). Regression run in `r20_editor` (`just test`): docs check, 316 host tests, 23 `abi` tests and 1969 QEMU-suite checks (the 1955 of Step 0 plus these fourteen) all passed. The `abi` and `userlib` changes are additive and shared by every stage's build; only this stage's suite was run.

## Step 1a -- the keyboard layer
Done before `CONSOLE_READ_KEY` so the record never changes. (1) The empirical repeat check (see Decisions). (2) `Token` gains `num` and `repeat`; `LockState::new()` starts NumLock on; `Token::char()` resolves the keypad's characters when NumLock is set. (3) `abi::keys`: the `KEY_*` constants the kernel's `tokens.rs` and the editor both use, the record, `effective_code`. (4) The line discipline matches on `effective_code`, so the keypad works in the shell. (5) REPEAT tokens, coalesced; lock flips gated. Pure logic, host-tested; QEMU cases type keypad keys with NumLock on and off (the guest's own toggle is the only truth: the virtio keyboard has no LEDs, so it can drift from the host's).

**As built (Step 1a).**
- **`abi::keys`** (new): the evdev `KEY_*` constants the kernel and the editor name (digits, the punctuation `effective_code` needs, the keypad, the navigation keys, the Ctrl-letter keys the shell's line editor uses; the editor's own letters are added when it needs them), the `MOD_*` bits, `KeyEvent` (the 8-byte record, `encode`/`decode`, helpers) and `effective_code`. `keyboard/tokens.rs` no longer has its own `KEY_*` copies: `line.rs` and `line_discipline.rs` import them from `abi::keys`.
- **Tokens** (`keyboard/tokens.rs`): `Token` gained `num` and `repeat`; `Token::effective_code()`; `Token::char()` resolves the keypad before the `KEY_NAMES` lookup (which calls `Kp7` a non-text key): `* - + /` always, digits and dot with NumLock on, Shift ignored; `emit` takes the `repeat` flag; a pure `admits(token, queued)` says a repeat may enter only an empty queue. `LockState::new()` starts NumLock on.
- **Repeat** (`keyboard/events.rs`, `queue.rs`): `token_for` asks `KeyState::is_held` *before* updating it; a press of a held key is a `repeat` token, and a lock key flips only on a fresh press (`key_changed`). A code past the held set's range is neither a press nor a repeat. `drain_keyboard` drops a repeat unless the queue is empty (`RingBuffer::len` is no longer test-only). The line discipline ignores a repeated Ctrl+D (holding it must not end one program's input and then the next one's); other repeats are accepted, so holding an arrow, Backspace or a letter works.
- **Line editing** uses `effective_code` (Up/Down history, Enter, the movement keys), so the keypad works at the prompt and in `read(0)`.
- **Tests:** 9 in `abi::keys`; 9 in `tokens.rs` (keypad characters and NumLock, `effective_code`, `emit`'s fields and modifiers, modifier keys never becoming tokens, `admits`); 3 in `line.rs` (keypad Enter, keypad navigation with NumLock off, keypad characters with it on). A new QEMU group `cases/keypad.py`, run as its own group: NumLock on types digits, dot and operators and the keypad's Enter is Enter; off, 4/7/1/dot are Left/Home/End/Delete, 8 and 2 are history Up and Down (a check that fails if Down does nothing), other digits type nothing, operators still type; one NumLock press toggles once; and `read(0)` in a program gets the keypad.
- **Not done here: the live held-key check.** The harness injects keys through the monitor's `sendkey`, which presses and releases once, and QEMU's host-side auto-repeat exists only in a real window, so a headless test cannot produce a repeat and none is claimed. The change assumes what `events.rs`'s old comment reported (a held key resends plain presses, or `value == 2`); either becomes a `repeat` token. **To confirm:** run `just run`, hold an arrow key or a letter at the prompt, and it should keep moving/typing. If nothing repeats, the fallback in Decisions (a kernel timer) is the next piece of work. The rules a repeat follows are host-tested; that they reach a consumer is not tested end to end.
- **Regression** (`just test` in `r20_editor`): docs check, 328 host tests (316 + 12), 32 `abi` tests (23 + 9) and 1979 QEMU-suite checks (1969 + the ten of `keypad`) all passed, no compiler warnings; after a formatting-only pass the keyboard-related groups (`keypad`, `line_editing`, `line_discipline`, `token_queue`, `console`) were re-run on a rebuilt test kernel and passed. New and touched files are `rustfmt`-clean as they were (`line_discipline.rs` and `userlib/src/io.rs` keep the formatting differences they already had).

## Step 2 -- `CONSOLE_READ_KEY`
An `ioctl` on the console fd filling one record; blocking follows `stdin.rs`'s pattern (drain the device into `keyboard/queue.rs`, pop a `Token`, oldest first), bypassing the line discipline. Test program: echoes each event's code, modifiers and character. QEMU test: type keys, check the events, then check that `read(0)` is still canonical afterwards.

## Step 3 -- `CONSOLE_DRAW`
The kernel loops `put_char_at` over the cells with the attribute's colours and flushes once; the cursor is drawn as an inverse cell at the given position. `DIM_FG` added next to `FG`/`BG`. Test program: draws a frame using both attributes and the cursor; a QEMU test reads the framebuffer back (`Session.screendump`). **Time a full 80x30 frame** (and, say, 30 in a row, the rate of a held key) and record it here; only if it is too slow is the kernel-side skip of unchanged cells worth adding.

## Step 4 -- the pure modules
No I/O, `no_std` + `alloc`, pulled into `hosttests` the way `environment.rs` is: buffer (insert, delete, split, join), the soft-wrap layout (rows of a line; position to cell and back, round-tripped for every character; wide glyph and tab at the row edge; the exactly-full row; a changing text width), cursor with a remembered display column, scroll by whole lines (a sub-row only inside a line taller than the text area), display width, the region and cut buffer, the auto-indent split, the `.editrc` parser. Written editor-independently where 20b will reuse it (width).

## Steps 5 to 8
5: the `edit` binary, a `progs_r20` tier, opening a file (a missing one is a new buffer), viewing and scrolling, quitting; `.editrc` loaded. 6: typing and deleting, save and save-as, the status bar, the prompt widget, the exit prompt, `^G`. 7: line cut, copy and paste, search, go to line. 8: the mark and the region operations, the gutter, the auto-indent toggle.

## Step 9 -- wrap-up
The docs under `rust/docs/` (`syscalls.md`, `console.md`, `tests.md`, `progs.md`) updated with the steps that changed what they describe, `test/check_docs.py` passing, the "As built" notes, and the demo: launch `edit` on a file on a scratch home disk (a copy of `disk-home-seed/` built into a scratch image), edit and save, power off, rebuild the system image, boot again against the same scratch home disk, confirm the edit.
