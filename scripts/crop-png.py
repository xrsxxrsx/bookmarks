"""Crop a PNG using only the standard library.

Written because the screenshot has a strip of native WebView2 chrome-to-be painted at the
bottom-left corner, and PIL is not available here. Handles the subset this repository
produces: 8-bit truecolour, no interlacing.
"""

import struct
import sys
import zlib


def read_png(path):
    data = open(path, "rb").read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise SystemExit("not a PNG")
    pos = 8
    idat = b""
    while pos < len(data):
        length = struct.unpack_from(">I", data, pos)[0]
        ctype = data[pos + 4 : pos + 8]
        body = data[pos + 8 : pos + 8 + length]
        if ctype == b"IHDR":
            width, height, depth, colour, comp, filt, interlace = struct.unpack_from(
                ">IIBBBBB", body, 0
            )
            if depth != 8 or colour not in (2, 6) or interlace != 0:
                raise SystemExit(f"unsupported PNG: depth={depth} colour={colour} interlace={interlace}")
            channels = 3 if colour == 2 else 4
        elif ctype == b"IDAT":
            idat += body
        elif ctype == b"IEND":
            break
        pos += 12 + length
    raw = zlib.decompress(idat)
    return width, height, channels, raw


def unfilter(width, height, channels, raw):
    stride = width * channels
    out = bytearray(height * stride)
    prev = bytearray(stride)
    pos = 0
    for y in range(height):
        ftype = raw[pos]
        pos += 1
        line = bytearray(raw[pos : pos + stride])
        pos += stride
        if ftype == 1:
            for i in range(channels, stride):
                line[i] = (line[i] + line[i - channels]) & 0xFF
        elif ftype == 2:
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 0xFF
        elif ftype == 3:
            for i in range(stride):
                left = line[i - channels] if i >= channels else 0
                line[i] = (line[i] + ((left + prev[i]) >> 1)) & 0xFF
        elif ftype == 4:
            for i in range(stride):
                a = line[i - channels] if i >= channels else 0
                b = prev[i]
                c = prev[i - channels] if i >= channels else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pr = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[i] = (line[i] + pr) & 0xFF
        elif ftype != 0:
            raise SystemExit(f"bad filter type {ftype} on row {y}")
        out[y * stride : (y + 1) * stride] = line
        prev = line
    return out


def write_png(path, width, height, channels, pixels):
    colour = 2 if channels == 3 else 6
    stride = width * channels
    raw = bytearray()
    for y in range(height):
        raw.append(0)  # filter type 0
        raw += pixels[y * stride : (y + 1) * stride]

    def chunk(ctype, body):
        return (
            struct.pack(">I", len(body))
            + ctype
            + body
            + struct.pack(">I", zlib.crc32(ctype + body) & 0xFFFFFFFF)
        )

    out = b"\x89PNG\r\n\x1a\n"
    out += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, colour, 0, 0, 0))
    out += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    out += chunk(b"IEND", b"")
    open(path, "wb").write(out)


def main():
    src, dst = sys.argv[1], sys.argv[2]
    # Pixels to keep, from each edge: left top right bottom.
    left, top, right, bottom = (int(v) for v in sys.argv[3:7])

    width, height, channels, raw = read_png(src)
    pixels = unfilter(width, height, channels, raw)

    new_w = width - left - right
    new_h = height - top - bottom
    stride = width * channels
    new_stride = new_w * channels
    cropped = bytearray(new_h * new_stride)
    for y in range(new_h):
        src_off = (y + top) * stride + left * channels
        cropped[y * new_stride : (y + 1) * new_stride] = pixels[src_off : src_off + new_stride]

    write_png(dst, new_w, new_h, channels, cropped)
    print(f"{src} {width}x{height} -> {dst} {new_w}x{new_h}")


main()
