"""Lists the descriptor bindings of SPIR-V modules (set, binding, kind, name), and the union
over all of them with conflicts flagged: what the Vulkan pipeline layout must hold.

    python tools/spirv_bindings.py DIR_OR_FILES...   (e.g. target/validation/debug/build/caustica-*/out)
"""
import glob
import os
import struct
import sys

OP_NAME, OP_DECORATE = 5, 71
OP_TYPE_IMAGE, OP_TYPE_SAMPLER, OP_TYPE_SAMPLED_IMAGE = 25, 26, 27
OP_TYPE_ARRAY, OP_TYPE_RUNTIME_ARRAY, OP_TYPE_STRUCT, OP_TYPE_POINTER = 28, 29, 30, 32
OP_VARIABLE, OP_TYPE_AS = 59, 5341
DEC_BINDING, DEC_SET = 33, 34
STORAGE = {0: 'UniformConstant', 2: 'Uniform', 12: 'StorageBuffer', 9: 'PushConstant'}


def bindings(path):
    words = struct.unpack('<%dI' % (os.path.getsize(path) // 4), open(path, 'rb').read())
    i, names, deco, types, variables = 5, {}, {}, {}, []
    while i < len(words):
        op, n = words[i] & 0xffff, words[i] >> 16
        a = words[i + 1:i + n]
        if op == OP_NAME:
            names[a[0]] = bytes(struct.pack('<%dI' % (n - 2), *a[1:])).split(b'\0')[0].decode()
        elif op == OP_DECORATE and a[1] in (DEC_BINDING, DEC_SET):
            deco.setdefault(a[0], {})[a[1]] = a[2]
        elif op in (OP_TYPE_IMAGE, OP_TYPE_SAMPLER, OP_TYPE_SAMPLED_IMAGE, OP_TYPE_AS, OP_TYPE_STRUCT):
            types[a[0]] = (op, a[1:])
        elif op in (OP_TYPE_ARRAY, OP_TYPE_RUNTIME_ARRAY, OP_TYPE_POINTER):
            types[a[0]] = (op, a[1:])
        elif op == OP_VARIABLE:
            variables.append((a[1], a[0], a[2]))
        i += n
    out = []
    for var, ptr, storage in variables:
        if var not in deco or DEC_BINDING not in deco[var]:
            continue
        t = types[ptr][1][1]
        array = ''
        while types[t][0] in (OP_TYPE_ARRAY, OP_TYPE_RUNTIME_ARRAY):
            array = '[]'
            t = types[t][1][0]
        op, rest = types[t]
        if op == OP_TYPE_IMAGE:
            kind = 'storage_image' if rest[5] == 2 else ('buffer_tex' if rest[0] == 5 else 'sampled_image')
            kind += '%dD' % (rest[0] + 1) if rest[0] < 3 else ''
        elif op == OP_TYPE_SAMPLER:
            kind = 'sampler'
        elif op == OP_TYPE_AS:
            kind = 'accel'
        elif op == OP_TYPE_STRUCT:
            kind = 'uniform' if STORAGE.get(storage) == 'Uniform' else 'storage_buffer'
        else:
            kind = 'op%d' % op
        out.append((deco[var].get(DEC_SET, 0), deco[var][DEC_BINDING], kind + array, names.get(var, '?')))
    return sorted(out)


def main():
    files = []
    for a in sys.argv[1:]:
        files += glob.glob(os.path.join(a, '*.spv')) if os.path.isdir(a) else glob.glob(a)
    union = {}
    for f in sorted(files):
        name = os.path.basename(f)[:-4]
        b = bindings(f)
        print(f'{name}: ' + ' '.join(f'{s}.{n}={k}' for s, n, k, _ in b))
        for s, n, k, v in b:
            union.setdefault((s, n), set()).add((k, v))
    print('\nunion:')
    for (s, n), ks in sorted(union.items()):
        kinds = sorted({k for k, _ in ks})
        flag = '  CONFLICT' if len(kinds) > 1 else ''
        print(f'  set {s} binding {n:3}: {", ".join(kinds)}  ({", ".join(sorted({v for _, v in ks}))}){flag}')


if __name__ == '__main__':
    main()
