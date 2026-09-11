# QEMU is smart about loading and executing ELF files as kernel images,
# but a true bare-metal environment would start execution from a fixed address. This script
# checks the ELF file for section addresses and entry points.

BIN=${1:-r01_hello.elf}
echo "--- .text section address ---"
readelf -SlW "$BIN" | grep -C3 '\.text '

# Should both be 0x40000000 for ARMv8, if not, try editing `link.ld` to reorder the sections
echo "--- ELF entry point ---"
readelf -h "$BIN" | grep Entry
echo "--- _start symbol's actual address ---"
aarch64-linux-gnu-nm "$BIN" | grep ' _start$'
