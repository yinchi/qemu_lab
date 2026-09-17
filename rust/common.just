# Shared recipes for every r01_ to r05_ crate under rust/. Each crate's own
# justfile imports this file and sets BIN to its own package/binary name --
# everything else lives here once instead of being duplicated per crate.
#
# `cd {{justfile_directory()}}` cds to the importing justfile's directory,
# otherwise commands will run from *this* file's directory.
#
# r06_ forward have differing requirements and do not use these shared recipes.

# List available recipes.
list:
    cd {{justfile_directory()}} && just --list

# Build the binary and copy it out of the target/ directory as an ELF file
build:
    cd {{justfile_directory()}} && cargo build --target aarch64-unknown-none-softfloat --release
    cd {{justfile_directory()}} && cp target/aarch64-unknown-none-softfloat/release/{{BIN}} {{BIN}}.elf

# Run the built binary in QEMU with the serial output connected to the terminal (stdio)
run-stdio: build
    cd {{justfile_directory()}} && qemu-system-aarch64 -M virt -cpu max -nodefaults -serial mon:stdio -kernel {{BIN}}.elf

# Run the built binary in QEMU with the serial output connected to a virtual console (vc)
run: build
    cd {{justfile_directory()}} && qemu-system-aarch64 -M virt -cpu max -nodefaults -serial vc -kernel {{BIN}}.elf

# ELF file inspection on the built binary (copied out of target/ by `just build`)
elf:
    bash elf_test.sh {{justfile_directory()}}/{{BIN}}.elf
