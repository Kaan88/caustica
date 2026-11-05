"""Minimal reader for the EXRs the native exe writes: single- or multi-part (the layer EXRs),
scanline or single-level tiled, NONE, RLE, ZIPS or ZIP compression, half / float / uint
channels.

    read(path)          -> (h, w, 3) float32 RGB
    read_channels(path) -> {name: (h, w) float32}, every channel (multi-part: "<part>.<channel>")
"""
import struct
import zlib
import numpy as np


def _unpredict(t):
    """Undo EXR's byte-delta predictor and the split of even / odd bytes."""
    t = np.frombuffer(t, np.uint8).astype(np.int64)
    t[1:] -= 128
    t = (np.cumsum(t) & 0xff).astype(np.uint8)
    half = (len(t) + 1) // 2
    res = np.empty(len(t), np.uint8); res[0::2] = t[:half]; res[1::2] = t[half:]
    return res.tobytes()


def _rle(data, size):
    out = bytearray(); i = 0
    while i < len(data):
        n = struct.unpack_from('b', data, i)[0]; i += 1
        if n < 0:
            out += data[i:i - n]; i -= n
        else:
            out += bytes([data[i]]) * (n + 1); i += 1
    res = _unpredict(bytes(out))
    assert len(res) == size
    return res


def _decode(data, size, comp):
    if len(data) >= size:
        return data
    if comp == 1:
        return _rle(data, size)
    res = _unpredict(zlib.decompress(data))
    assert len(res) == size
    return res


_TYPES = {0: ('<u4', 4), 1: ('<f2', 2), 2: ('<f4', 4)}


def _header(d, p):
    attrs = {}
    while d[p] != 0:
        e = d.index(b'\0', p); name = d[p:e].decode(); p = e + 1
        e = d.index(b'\0', p); p = e + 1
        n = struct.unpack_from('<i', d, p)[0]; p += 4
        attrs[name] = d[p:p + n]; p += n
    return attrs, p + 1


def _part(d, attrs, offsets, prefix, out):
    """Decode one part's chunks (`prefix` bytes before each: the part number in multi-part files)."""
    chans, c, q = [], attrs['channels'], 0
    while c[q] != 0:
        e = c.index(b'\0', q)
        chans.append((c[q:e].decode(), struct.unpack_from('<i', c, e + 1)[0]))
        q = e + 1 + 16
    x0, y0, x1, y1 = struct.unpack('<4i', attrs['dataWindow'])
    w, h = x1 - x0 + 1, y1 - y0 + 1
    comp = attrs['compression'][0]
    assert comp in (0, 1, 2, 3), f'compression {comp} unsupported'
    name_of = lambda ch: ch
    if 'name' in attrs:
        part = attrs['name'].rstrip(b'\0').decode()
        name_of = lambda ch: f'{part}.{ch}'
    for name, _ in chans:
        out[name_of(name)] = np.zeros((h, w), np.float32)

    def block(data, rows, cols, y, x):
        # A block holds each row's channels in turn (channels sorted by name, as listed).
        o = 0
        for r in range(rows):
            for name, t in chans:
                dt, size = _TYPES[t]
                out[name_of(name)][y + r, x:x + cols] = np.frombuffer(data, dt, cols, o)
                o += cols * size

    row_bytes = lambda cols: cols * sum(_TYPES[t][1] for _, t in chans)
    if 'tiles' in attrs:
        tw, th = struct.unpack_from('<II', attrs['tiles'])
        for off in offsets:
            off += prefix
            tx, ty, lx, ly, n = struct.unpack_from('<5i', d, off)
            assert lx == 0 and ly == 0
            cw, ch = min(tw, w - tx * tw), min(th, h - ty * th)
            data = _decode(d[off + 20: off + 20 + n], ch * row_bytes(cw), comp)
            block(data, ch, cw, ty * th, tx * tw)
    else:
        lines = {3: 16}.get(comp, 1)
        for off in offsets:
            off += prefix
            y, n = struct.unpack_from('<ii', d, off)
            rows = min(lines, y1 + 1 - y)
            data = _decode(d[off + 8: off + 8 + n], rows * row_bytes(w), comp)
            block(data, rows, w, y - y0, 0)


def _chunks(attrs):
    if 'chunkCount' in attrs:
        return struct.unpack('<i', attrs['chunkCount'])[0]
    x0, y0, x1, y1 = struct.unpack('<4i', attrs['dataWindow'])
    w, h = x1 - x0 + 1, y1 - y0 + 1
    if 'tiles' in attrs:
        tw, th = struct.unpack_from('<II', attrs['tiles'])
        return -(-w // tw) * -(-h // th)
    return -(-h // {3: 16}.get(attrs['compression'][0], 1))


def read_channels(path):
    """Every channel by name; in a multi-part file (the layer EXRs) named "<part>.<channel>"."""
    d = open(path, 'rb').read()
    assert struct.unpack_from('<I', d, 0)[0] == 20000630
    multi = struct.unpack_from('<I', d, 4)[0] & 0x1000
    p, headers = 8, []
    while True:
        attrs, p = _header(d, p)
        headers.append(attrs)
        if not multi or d[p] == 0:
            break
    if multi:
        p += 1
    out = {}
    for attrs in headers:
        n = _chunks(attrs)
        offsets = struct.unpack_from(f'<{n}Q', d, p); p += 8 * n
        _part(d, attrs, offsets, 4 if multi else 0, out)
    return out


def read(path):
    ch = read_channels(path)
    return np.stack([ch[c] for c in 'RGB'], axis=-1)
