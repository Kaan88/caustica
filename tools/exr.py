"""Minimal reader for the float RGB EXRs the native exe writes: scanline or
single-level tiled, NONE or RLE compression."""
import struct
import numpy as np


def _rle(data, size):
    out = bytearray(); i = 0
    while i < len(data):
        n = struct.unpack_from('b', data, i)[0]; i += 1
        if n < 0:
            out += data[i:i - n]; i -= n
        else:
            out += bytes([data[i]]) * (n + 1); i += 1
    t = np.frombuffer(bytes(out), np.uint8).astype(np.int64)
    t[1:] -= 128
    t = (np.cumsum(t) & 0xff).astype(np.uint8)   # undo the byte-delta predictor
    half = (len(t) + 1) // 2
    res = np.empty(len(t), np.uint8); res[0::2] = t[:half]; res[1::2] = t[half:]
    assert len(res) == size
    return res.tobytes()


def read(path):
    d = open(path, 'rb').read()
    assert struct.unpack_from('<I', d, 0)[0] == 20000630
    p, attrs = 8, {}
    while d[p] != 0:
        e = d.index(b'\0', p); name = d[p:e].decode(); p = e + 1
        e = d.index(b'\0', p); p = e + 1
        n = struct.unpack_from('<i', d, p)[0]; p += 4
        attrs[name] = d[p:p + n]; p += n
    p += 1
    chans, c, q = [], attrs['channels'], 0
    while c[q] != 0:
        e = c.index(b'\0', q); chans.append(c[q:e].decode()); q = e + 1 + 16
    x0, y0, x1, y1 = struct.unpack('<4i', attrs['dataWindow'])
    w, h, nc = x1 - x0 + 1, y1 - y0 + 1, len(chans)
    comp = attrs['compression'][0]
    assert comp in (0, 1), f'compression {comp} unsupported'
    img = np.zeros((h, nc, w), np.float32)
    if 'tiles' in attrs:
        tw, th = struct.unpack_from('<II', attrs['tiles'])
        count = -(-w // tw) * -(-h // th)
        for off in struct.unpack_from(f'<{count}Q', d, p):
            tx, ty, lx, ly, n = struct.unpack_from('<5i', d, off)
            assert lx == 0 and ly == 0
            cw, ch = min(tw, w - tx * tw), min(th, h - ty * th)
            size = cw * ch * nc * 4
            data = d[off + 20: off + 20 + n]
            if n < size: data = _rle(data, size)
            block = np.frombuffer(data, '<f4').reshape(ch, nc, cw)
            img[ty * th: ty * th + ch, :, tx * tw: tx * tw + cw] = block
    else:
        for off in struct.unpack_from(f'<{h}Q', d, p):
            y, n = struct.unpack_from('<ii', d, off)
            data = d[off + 8: off + 8 + n]
            if n < w * nc * 4: data = _rle(data, w * nc * 4)
            img[y - y0] = np.frombuffer(data, '<f4').reshape(nc, w)
    return img[:, [chans.index(ch) for ch in 'RGB'], :].transpose(0, 2, 1)
