#!/usr/bin/env bash
# Builds a FAT16 disk image from a source directory, generalizing r08_fs's
# inline `disk` recipe (r08_fs/justfile) so later stages -- starting with
# r09_userspace, the first stage whose disk image needs to hold a build
# *artifact* (a compiled user ELF) rather than only static files -- don't
# have to duplicate the same mkfs.fat/mtools sequence by hand. r08_fs's own
# justfile is intentionally left as-is (that stage is already complete);
# this script is only ever called by stages from r09_userspace onward.
#
# Usage: folder_to_img.sh <image> <size> <label> <serial> <source_dir>
#
# Walks <source_dir> recursively -- to any depth, not just one level --
# creating a matching directory (`mmd`) or copying a matching file
# (`mcopy`) for every entry, so `<source_dir>/bin/hello` lands at
# `::/bin/hello` in the image, `<source_dir>/fonts/spleen.raw` at
# `::/fonts/spleen.raw`, and so on -- no separate `mmd`/`mcopy` line needed
# per file the way r08_fs's inline recipe required. Directories are created
# in sorted order before any file copy runs: sorting the raw paths byte-wise
# (LC_ALL=C, so this doesn't depend on the host's locale) guarantees a
# parent directory always sorts before its children, since a parent's path
# is always a literal prefix of everything nested inside it -- `mmd` needs
# the parent to already exist in the image, at every depth, not just the
# first level.
#
# Byte-for-byte reproducible given the same inputs, same as r08_fs's own
# recipe: SOURCE_DATE_EPOCH pins mtools' file timestamps (mtools honors it;
# mkfs.fat's own `-n` volume-label stamping does not, which is why the
# volume label is set via a separate `mlabel` call instead) to a fixed
# default, overridable via the environment for a caller that wants a
# different epoch.

set -euo pipefail

if [ "$#" -ne 5 ]; then
    echo "usage: $0 <image> <size> <label> <serial> <source_dir>" >&2
    exit 1
fi

IMAGE="$1"
SIZE="$2"
LABEL="$3"
SERIAL="$4"
SOURCE_DIR="$5"

# 2026-01-01T00:00:00Z
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-1767225600}"
export SOURCE_DATE_EPOCH

rm -f "$IMAGE"
truncate -s "$SIZE" "$IMAGE"
mkfs.fat -F 16 -i "$SERIAL" "$IMAGE"
mlabel -i "$IMAGE" "::$LABEL"

# NUL-delimited (`-print0`/`read -d ''`) so filenames containing spaces
# don't break the loop.
while IFS= read -r -d '' dir; do
    rel="${dir#"$SOURCE_DIR"/}"
    mmd -i "$IMAGE" "::/$rel"
done < <(find "$SOURCE_DIR" -mindepth 1 -type d -print0 | LC_ALL=C sort -z)

while IFS= read -r -d '' file; do
    rel="${file#"$SOURCE_DIR"/}"
    mcopy -i "$IMAGE" "$file" "::/$rel"
done < <(find "$SOURCE_DIR" -mindepth 1 -type f -print0 | LC_ALL=C sort -z)
