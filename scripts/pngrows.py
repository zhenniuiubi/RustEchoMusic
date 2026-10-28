"""纯标准库 PNG 解码 + 按列区间统计每一行的内容范围（用于验证像素级对齐）。

用法: python scripts/pngrows.py <png> [x1:x2] [x1:x2] ...
针对每个 x 区间，输出「有内容」的行范围（与背景色差异 > 阈值）。
"""
import sys
import zlib
import struct


def load_png(path):
    data = open(path, 'rb').read()
    assert data[:8] == b'\x89PNG\r\n\x1a\n'
    pos = 8
    idat = b''
    w = h = bit = ctype = None
    while pos < len(data):
        ln = struct.unpack('>I', data[pos:pos + 4])[0]
        typ = data[pos + 4:pos + 8]
        chunk = data[pos + 8:pos + 8 + ln]
        pos += 12 + ln
        if typ == b'IHDR':
            w, h, bit, ctype, comp, filt, inter = struct.unpack('>IIBBBBB', chunk)
            assert bit == 8 and inter == 0, (bit, inter)
        elif typ == b'IDAT':
            idat += chunk
        elif typ == b'IEND':
            break
    raw = zlib.decompress(idat)
    nch = {0: 1, 2: 3, 4: 2, 6: 4}[ctype]
    stride = w * nch
    out = bytearray(w * h * nch)
    prev = bytearray(stride)
    p = 0
    for y in range(h):
        f = raw[p]
        p += 1
        line = bytearray(raw[p:p + stride])
        p += stride
        if f == 1:
            for i in range(nch, stride):
                line[i] = (line[i] + line[i - nch]) & 0xFF
        elif f == 2:
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 0xFF
        elif f == 3:
            for i in range(stride):
                a = line[i - nch] if i >= nch else 0
                line[i] = (line[i] + ((a + prev[i]) >> 1)) & 0xFF
        elif f == 4:
            for i in range(stride):
                a = line[i - nch] if i >= nch else 0
                b = prev[i]
                c = prev[i - nch] if i >= nch else 0
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                pr = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[i] = (line[i] + pr) & 0xFF
        out[y * stride:(y + 1) * stride] = line
        prev = line
    return w, h, nch, bytes(out)


def main():
    path = sys.argv[1]
    ranges = []
    for a in sys.argv[2:]:
        x1, x2 = a.split(':')
        ranges.append((int(x1), int(x2)))
    w, h, nch, px = load_png(path)
    print(f'{path}  {w}x{h}  ch={nch}')
    # 背景采样：取左上角 3x3 的中位数
    if not ranges:
        ranges = [(0, w)]
    for (x1, x2) in ranges:
        rows = []
        for y in range(h):
            best = 0
            for x in range(x1, min(x2, w)):
                i = (y * w + x) * nch
                r, g, b = px[i], px[i + 1], px[i + 2]
                a = px[i + 3] if nch == 4 else 255
                # 与「白色」和「近黑」都差得远算有内容
                d = min(abs(r - 255) + abs(g - 255) + abs(b - 255),
                        abs(r - 0) + abs(g - 0) + abs(b - 0))
                if d > best:
                    best = d
            if best > 40:
                rows.append(y)
        if rows:
            print(f'  x[{x1},{x2}) 内容行 {rows[0]}..{rows[-1]}  (高 {rows[-1]-rows[0]+1})')
        else:
            print(f'  x[{x1},{x2}) 无内容')


if __name__ == '__main__':
    main()
