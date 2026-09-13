"""One-off validation tool: renders a raw VGA-style font dump (256 glyphs x
16 bytes each, 8x16 pixels, 1 bit per pixel, MSB = leftmost) as a viewable
1-bit BMP -- a vertical strip, one glyph wide, so each glyph's 16 rows are
literally consecutive bytes in the source file.

The raw file itself (rust/r06_vgadisp/font/spleen.raw) is what the guest
actually reads and slices at runtime; this script only exists to visually
confirm that file's glyphs look correct, by inserting the row padding a
real 1bpp BMP requires (which the tightly-packed raw file doesn't have) and
attaching a header -- see the conversation this was built from for why
that padding can't just be skipped.

Usage: python raw_font_to_bmp.py [first_char] [last_char]
Defaults to the printable ASCII range, 0x20..0x7E inclusive.
"""

import struct
import sys

RAW_FONT_PATH = "../rust/r06_vgadisp/font/spleen.raw"
OUT_PATH = "font_check.bmp"
GLYPH_WIDTH = 8
GLYPH_HEIGHT = 16
BYTES_PER_GLYPH = GLYPH_HEIGHT  # 1 byte per row at 8 pixels wide


def build_bmp(rows: bytes) -> bytes:
    """`rows` is one byte per pixel-row, 8 pixels wide, MSB = leftmost --
    exactly the raw font's own layout. Pads each row to BMP's mandatory
    4-byte boundary and prepends a real BMP header."""
    height = len(rows)
    padded_row_size = 4  # 8 pixels * 1 bit, rounded up to a 4-byte boundary
    pixel_data = b"".join(bytes([row, 0, 0, 0]) for row in rows)
    assert len(pixel_data) == height * padded_row_size

    palette = struct.pack("<4B", 0, 0, 0, 0) + struct.pack("<4B", 255, 255, 255, 0)
    info_header_size = 40
    palette_size = len(palette)
    pixel_data_offset = 14 + info_header_size + palette_size
    file_size = pixel_data_offset + len(pixel_data)

    file_header = struct.pack("<2sIHHI", b"BM", file_size, 0, 0, pixel_data_offset)
    info_header = struct.pack(
        "<IiiHHIIiiII",
        info_header_size,
        GLYPH_WIDTH,
        -height,  # negative = top-down, so file order matches visual order
        1,  # color planes
        1,  # bits per pixel
        0,  # BI_RGB, uncompressed
        len(pixel_data),
        2835,  # ~72 DPI, arbitrary but conventional
        2835,
        2,  # colors used (2-entry palette)
        0,  # all colors "important"
    )
    return file_header + info_header + palette + pixel_data


def main():
    first = int(sys.argv[1], 0) if len(sys.argv) > 1 else 0x20
    last = int(sys.argv[2], 0) if len(sys.argv) > 2 else 0x7E

    with open(RAW_FONT_PATH, "rb") as f:
        font = f.read()
    assert len(font) == 256 * BYTES_PER_GLYPH, f"unexpected font size {len(font)}"

    rows = font[first * BYTES_PER_GLYPH : (last + 1) * BYTES_PER_GLYPH]
    bmp = build_bmp(rows)

    with open(OUT_PATH, "wb") as f:
        f.write(bmp)
    print(f"Wrote {OUT_PATH}: glyphs {first:#04x}..{last:#04x}, {len(rows)} rows, {len(bmp)} bytes")


if __name__ == "__main__":
    main()
