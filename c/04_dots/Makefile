# Compiler for AArch64 targets: install via:
#   sudo apt install gcc-aarch64-linux-gnu
CC = aarch64-linux-gnu-gcc

# -march=armv8-a: Target the ARMv8-A ISA (Instruction Set Architecture) regardless of host default
# -mabi=lp64: 64-bit pointers/longs (the standard AArch64 ABI, Application Binary Interface)
# -mgeneral-regs-only: Forbid floating-point/SIMD codegen; CPACR_EL1 is never configured to
#     enable it
# -mstrict-align: Don't assume unaligned accesses work before alignment checking is configured
# -ffreestanding: Do not link against the standard library
# -fno-stack-protector: Disable stack protection
# -fno-pic: Disable position-independent code generation
CFLAGS = -march=armv8-a -mabi=lp64 -mgeneral-regs-only -mstrict-align \
	-ffreestanding -fno-stack-protector -fno-pic -O2 -Wall

# -nostdlib: Do not link against the standard library
# -static: Create a statically linked executable
# -Wl,-T,link.ld: Pass the linker script link.ld to the linker
# -Wl,-z,max-page-size=0x1000: Use the minimum universal page size (4 KB) rather than ld's
#     64 KB AArch64 default, matching link.ld's own alignment and avoiding a large,
#     otherwise-unused gap reserved at the front of the file for headers
LDFLAGS = -nostdlib -static -Wl,-T,link.ld -Wl,-z,max-page-size=0x1000

start.elf: start.S vectors.S main.c link.ld
	$(CC) $(CFLAGS) $(LDFLAGS) -o $@ start.S vectors.S main.c

main.c: registers.h uart_io.h

clean:
	rm -f start.elf

.PHONY: clean
