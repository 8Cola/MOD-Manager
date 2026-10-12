"""Copy verified build outputs and create a source archive without local tools/data."""
from pathlib import Path
import hashlib
import json
import os
import shutil
import zipfile

root = Path(__file__).resolve().parent.parent
version = json.loads((root / 'package.json').read_text(encoding='utf-8'))['version']
release = root / 'release'
release.mkdir(exist_ok=True)
candidates = [root / '_local' / 'target' / 'release' / 'mod-manager.exe',
              root / 'src-tauri' / 'target' / 'release' / 'mod-manager.exe']
existing = [p for p in candidates if p.is_file()]
if not existing:
    raise SystemExit('Missing Release executable; build the project first')
artifacts = {
    max(existing, key=lambda p: p.stat().st_mtime): release / 'MOD Manager.exe',
    root / 'LICENSE': release / 'LICENSE',
    root / 'THIRD_PARTY_NOTICES.md': release / 'THIRD_PARTY_NOTICES.md',
}
for source, destination in artifacts.items():
    if not source.is_file() or source.stat().st_size == 0:
        raise SystemExit(f'Missing build artifact: {source}')
    try:
        shutil.copy2(source, destination)
    except PermissionError:
        # Windows keeps an executable locked while that exact copy is running.
        # Preserve the active program and stage the newly built executable beside it.
        staged = destination.with_name(destination.stem + '-new.exe')
        shutil.copy2(source, staged)
        artifacts[source] = staged

# Retire the known development alias only after its replacement exists.
legacy = release / 'MOD Manager-updated.exe'
if legacy.is_file():
    legacy.unlink()

archive = release / f'MOD Manager-{version}-source.zip'
exclude = {'_local', '.tools', '.git', '.cargo', 'node_modules', 'dist', 'output', 'release', '.playwright-cli'}
with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as zipped:
    for directory, children, filenames in os.walk(root):
        location = Path(directory)
        children[:] = sorted(child for child in children if child not in exclude and not (location == root/'src-tauri' and child in {'target','gen'}))
        for filename in sorted(filenames):
            source = location / filename
            relative = source.relative_to(root)
            if not source.is_symlink() and source.suffix not in {'.tsbuildinfo', '.exe'}:
                zipped.write(source, 'Mod-Manager/' + relative.as_posix())

outputs = [*artifacts.values(), archive]
lines = [hashlib.sha256(p.read_bytes()).hexdigest() + '  ' + p.name for p in outputs]
(release / 'SHA256SUMS.txt').write_text('\n'.join(lines) + '\n', encoding='utf-8')
for p in outputs:
    print(f'{p.name}: {p.stat().st_size:,} bytes')
