"""Check generated icon sizes and exact PNG resources embedded in a Windows EXE."""
from pathlib import Path
import ctypes
import hashlib
import json
import struct
import sys

root = Path(__file__).resolve().parent.parent
svg = (root / 'src/assets/app-icon.svg').read_text(encoding='utf-8')
assert '<image' not in svg and 'data:image' not in svg, 'Master logo must contain vectors only'
ico = (root / 'src-tauri/icons/icon.ico').read_bytes()
reserved, kind, count = struct.unpack_from('<HHH', ico)
assert (reserved, kind) == (0, 1)
frames = {}
for index in range(count):
    width, height, _, _, planes, bits, length, offset = struct.unpack_from('<BBBBHHII', ico, 6 + index * 16)
    width, height = width or 256, height or 256
    png = ico[offset:offset + length]
    assert png.startswith(b'\x89PNG\r\n\x1a\n') and struct.unpack_from('>II', png, 16) == (width, height)
    assert planes == 1 and bits == 32 and len(png) == length
    frames[(width, height)] = png
assert sorted(width for width, _ in frames) == [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]

if len(sys.argv) > 1:
    assert sys.platform == 'win32', 'Native resource check requires Windows'
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel.LoadLibraryExW.argtypes = [ctypes.c_wchar_p, ctypes.c_void_p, ctypes.c_uint32]
    kernel.LoadLibraryExW.restype = ctypes.c_void_p
    kernel.FindResourceW.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p]
    kernel.FindResourceW.restype = ctypes.c_void_p
    kernel.SizeofResource.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    kernel.SizeofResource.restype = ctypes.c_uint32
    kernel.LoadResource.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    kernel.LoadResource.restype = ctypes.c_void_p
    kernel.LockResource.argtypes = [ctypes.c_void_p]
    kernel.LockResource.restype = ctypes.c_void_p
    kernel.FreeLibrary.argtypes = [ctypes.c_void_p]
    module = kernel.LoadLibraryExW(str(Path(sys.argv[1]).resolve()), None, 0x22)
    assert module, ctypes.WinError(ctypes.get_last_error())

    def resource(name, resource_type):
        entry = kernel.FindResourceW(module, name, resource_type)
        assert entry, ctypes.WinError(ctypes.get_last_error())
        size = kernel.SizeofResource(module, entry)
        data = kernel.LockResource(kernel.LoadResource(module, entry))
        assert data and size
        return ctypes.string_at(data, size)

    groups = []
    callback_type = ctypes.WINFUNCTYPE(ctypes.c_bool, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_ssize_t)

    @callback_type
    def collect(_, __, name, ___):
        groups.append(resource(name, 14))
        return True

    kernel.EnumResourceNamesW.argtypes = [ctypes.c_void_p, ctypes.c_void_p, callback_type, ctypes.c_ssize_t]
    try:
        assert kernel.EnumResourceNamesW(module, 14, collect, 0), ctypes.WinError(ctypes.get_last_error())
        matched = False
        for group in groups:
            group_frames = {}
            for index in range(struct.unpack_from('<H', group, 4)[0]):
                width, height, _, _, _, _, size, icon_id = struct.unpack_from('<BBBBHHIH', group, 6 + index * 14)
                png = resource(icon_id, 3)
                assert len(png) == size
                group_frames[(width or 256, height or 256)] = png
            matched |= group_frames == frames
        assert matched, 'EXE icon resources differ from the canonical generated ICO'
    finally:
        kernel.FreeLibrary(module)

print(json.dumps({'passed': True, 'vector_source': True, 'sizes': [w for w, _ in frames],
                  'exe_verified': len(sys.argv) > 1,
                  'ico_sha256': hashlib.sha256(ico).hexdigest()}, indent=2))
