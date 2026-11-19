"""本次 Lua 语法样本的实验复现配方；不是 TEST-001 build/capture 接口。

用法：python3 pack.py BINARY_DIR NEW_DIR
BINARY_DIR 提供按 #52 命名的四个 ELF；输出未压缩 newc 和逐成员清单。
"""
import json
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / 'scripts'))
import test001_audit as common


def pack(binary_root, output):
    data, lock = common.load_inputs(common.INPUTS)
    init = Path(__file__).with_name('init.sh').read_bytes()
    binaries = {}
    with tempfile.TemporaryDirectory(prefix='lua-sample-') as tmp:
        for name, meta in lock['executables'].items():
            filename = name.replace('/', '-')
            private = Path(tmp) / filename
            common.copy_image_snapshot(binary_root / filename, private, meta['size'])
            raw = private.read_bytes()
            common.check_bytes(raw, meta, name)
            binaries[name] = raw
    output.mkdir(exist_ok=False)
    for libc in ('musl', 'glibc'):
        entries = [('bin', 0o40755, b'', 0, 0), ('dev', 0o40755, b'', 0, 0),
                   (libc, 0o40755, b'', 0, 0), ('dev/console', 0o20600, b'', 5, 1),
                   ('dev/null', 0o20666, b'', 1, 3),
                   ('bin/busybox', 0o120777, ('../' + libc + '/busybox').encode(), 0, 0),
                   ('bin/sh', 0o120777, b'busybox', 0, 0), ('init', 0o100755, init, 0, 0)]
        entries.extend((f'{libc}/{name}', 0o100755, binaries[f'{libc}/{name}'], 0, 0)
                       for name in ('busybox', 'lua'))
        for name in lock['files']:
            if name.startswith(libc + '/') and (name.endswith('.lua') or
                    name.endswith('/test.sh') or name.endswith('/lua_testcode.sh')):
                entries.append((name, 0o100755, data[name], 0, 0))
        blob, records = bytearray(), []
        for ino, (name, mode, raw, major, minor) in enumerate(entries + [('TRAILER!!!', 0, b'', 0, 0)], 1):
            encoded = name.encode() + b'\0'
            fields = (ino, mode, 0, 0, 1, 0, len(raw), 0, 0, major, minor, len(encoded), 0)
            blob.extend(b'070701' + b''.join(f'{v:08x}'.encode() for v in fields) + encoded)
            blob.extend(b'\0' * (-len(blob) % 4))
            blob.extend(raw)
            blob.extend(b'\0' * (-len(blob) % 4))
            if name != 'TRAILER!!!':
                records.append({'path': name, 'mode': oct(mode), 'size': len(raw),
                                'sha256': common.digest(raw), 'rdev': [major, minor]})
        (output / f'{libc}.cpio').write_bytes(blob)
        (output / f'{libc}-members.json').write_text(json.dumps(records, indent=2) + '\n')


if __name__ == '__main__':
    pack(Path(sys.argv[1]), Path(sys.argv[2]))
