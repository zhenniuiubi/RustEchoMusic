"""纯标准库 PNG 缩放/裁剪（没有 PIL 时用）。

用法: python scripts/pngzoom.py <in.png> <out.png> <scale> [x1 y1 x2 y2]
"""
import sys
import zlib
import struct

from pngrows import load_png


def save_png(path, w, h, rgb_rows):
    raw = bytearray()
    for row in rgb_rows:
        raw.append(0)
        raw += row
    comp = zlib.compress(bytes(raw), 9)

    def chunk(typ, data):
        return (struct.pack('>I', len(data)) + typ + data
                + struct.pack('>I', zlib.crc32(typ + data) & 0xFFFFFFFF))

    hdr = struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)
    with open(path, 'wb') as f:
        f.write(b'\x89PNG\r\n\x1a\n')
        f.write(chunk(b'IHDR', hdr))
        f.write(chunk(b'IDAT', comp))
        f.write(chunk(b'IEND', b''))


def main():
    src, dst, scale = sys.argv[1], sys.argv[2], int(sys.argv[3])
    w, h, nch, px = load_png(src)
    x1, y1, x2, y2 = 0, 0, w, h
    if len(sys.argv) > 7:
        x1, y1, x2, y2 = (int(v) for v in sys.argv[4:8])
    cw, ch = x2 - x1, y2 - y1
    ow, oh = cw * scale, ch * scale
    rows = []
    for oy in range(oh):
        sy = y1 + oy // scale
        row = bytearray()
        for ox in range(ow):
            sx = x1 + ox // scale
            i = (sy * w + sx) * nch
            row += bytes((px[i], px[i + 1], px[i + 2]))
        rows.append(row)
    save_png(dst, ow, oh, rows)
    print(f'{dst} {ow}x{oh}')


if __name__ == '__main__':
    main()
