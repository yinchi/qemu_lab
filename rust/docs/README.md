# Documentation

How the pieces of `r16_brk` (the latest stage) fit together. Each file stands alone; the links between them
are where one leans on another.

| Document | Covers |
|---|---|
| [`build_sequence.md`](build_sequence.md) | How the kernel is built: `build.rs`, the assembly files, the linker script, the resulting ELF |
| [`memory_regions.md`](memory_regions.md) | The QEMU `virt` memory map (boot ROM, GIC, UART, VirtIO) and the kernel image in RAM: sections, stack, guard, heap |
| [`mmu.md`](mmu.md) | The page tables, memory attributes, turning translation on, and the user memory window |
| [`launching_programs.md`](launching_programs.md) | How a program is found, loaded, entered at EL0 and given its arguments |
| [`syscalls.md`](syscalls.md) | The system-call interface: convention, every call, pointer checks, errors |
| [`filesystem.md`](filesystem.md) | The FAT volume, open files, paths, directory records, attributes |
| [`virtio.md`](virtio.md) | The VirtIO drivers and HAL, and the console's text rendering (font, cells, cursor) |
| [`console.md`](console.md) | Keyboard input: tokens, the queue, the line discipline, prompt vs canonical mode, the token table |
| [`shell.md`](shell.md) | The shell: grammar, redirection, pipelines, builtins, scripts, the frame stack |
| [`shell.ebnf`](shell.ebnf) | The shell's grammar |
| [`progs.md`](progs.md) | The user programs: what each supports, and deliberately doesn't |
| [`tests.md`](tests.md) | The host and QEMU test suites |
