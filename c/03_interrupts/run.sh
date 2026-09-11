#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"
make

exec qemu-system-aarch64 \
    -M virt \
    -nodefaults \
    -cpu max \
    -serial vc \
    -kernel start.elf
