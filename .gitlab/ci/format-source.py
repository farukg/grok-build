"""Generate adoption-safe rustfmt outputs for changed, owned source only."""

import hashlib
import io
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tomllib
import urllib.request

from format_evidence import manifest


ROOT = Path('/build')
OUTPUT = ROOT / 'dist/format'
TOKIO_SHA256 = '8fc7f01b389ac15039e4dc9531aa973a135d7a4135281b12d7c1bc79fd57fffe'
MAX_ARCHIVE_BYTES = 16 * 1024 * 1024


def git(*args):
    return subprocess.check_output(['git', *args], cwd=ROOT)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def snapshot(path):
    relative = path.relative_to(ROOT).as_posix()
    data = path.read_bytes()
    destination = OUTPUT / 'inputs' / relative
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(data)
    return {'path': relative, 'sha256': digest(data)}


def ancestors(path):
    while path.is_relative_to(ROOT):
        yield path
        if path == ROOT:
            break
        path = path.parent


def formatter_owner(path):
    manifest = None
    config = None
    for directory in ancestors(path.parent):
        candidate = directory / 'Cargo.toml'
        if manifest is None and candidate.is_file():
            document = tomllib.loads(candidate.read_text())
            if 'package' in document:
                manifest = candidate
                package = document['package']
        if config is None:
            for name in ('.rustfmt.toml', 'rustfmt.toml'):
                candidate = directory / name
                if candidate.is_file():
                    config = candidate
                    break
    if manifest is None or config is None:
        raise ValueError(f'No package/formatter owner for {path.relative_to(ROOT)}')
    edition = package.get('edition', '2015')
    if edition == {'workspace': True}:
        workspace_path = package.get('workspace')
        if workspace_path is not None:
            workspace = (manifest.parent / workspace_path / 'Cargo.toml').resolve()
            if not workspace.is_relative_to(ROOT):
                raise ValueError('Workspace edition owner is outside source checkout')
        else:
            workspace = next(
                (directory / 'Cargo.toml' for directory in ancestors(manifest.parent)
                 if (directory / 'Cargo.toml').is_file()
                 and 'workspace' in tomllib.loads((directory / 'Cargo.toml').read_text())),
                None,
            )
        if workspace is None:
            raise ValueError(f'No inherited edition owner for {manifest}')
        edition = tomllib.loads(workspace.read_text())['workspace']['package']['edition']
        edition_owner = snapshot(workspace)
    else:
        edition_owner = snapshot(manifest)
    if edition not in ('2015', '2018', '2021', '2024'):
        raise ValueError(f'Unsupported package edition: {edition}')
    return edition, config, snapshot(manifest), edition_owner


def pristine_tokio():
    package = tomllib.loads((ROOT / 'third_party/tokio/Cargo.toml').read_text())['package']
    if package['name'] != 'tokio' or package['version'] != '1.52.3':
        raise ValueError('Update the verified Tokio provenance before formatting another version')
    cache = Path('/cache/grok-build/format')
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / f'tokio-1.52.3-{TOKIO_SHA256}.crate'
    if archive.is_file():
        data = archive.read_bytes()
    else:
        with urllib.request.urlopen(
            'https://static.crates.io/crates/tokio/tokio-1.52.3.crate', timeout=60,
        ) as response:
            data = response.read(MAX_ARCHIVE_BYTES + 1)
    if len(data) > MAX_ARCHIVE_BYTES or digest(data) != TOKIO_SHA256:
        raise ValueError('Tokio pristine archive checksum/size mismatch')
    archive.write_bytes(data)
    files = {}
    with tarfile.open(fileobj=io.BytesIO(data), mode='r:gz') as source:
        total = 0
        for member in source:
            if not member.isfile() or not member.name.endswith('.rs'):
                continue
            if not member.name.startswith('tokio-1.52.3/'):
                raise ValueError('Unexpected Tokio archive layout')
            total += member.size
            if total > 64 * 1024 * 1024:
                raise ValueError('Tokio source exceeds formatting provenance bound')
            with source.extractfile(member) as stream:
                files[member.name.removeprefix('tokio-1.52.3/')] = stream.read()
    return files


def generate(report, toolchain, pipeline_source, before, explicit_base, commit):
    if git('rev-parse', 'HEAD').decode().strip() != commit:
        raise ValueError('Source SHA does not match checkout')
    subprocess.run(['git', 'diff', '--quiet', 'HEAD', '--'], cwd=ROOT, check=True)
    base = before if pipeline_source == 'push' else explicit_base
    if not re.fullmatch('[0-9a-f]{40}', base) or base == '0' * 40:
        raise ValueError('A real push CI_COMMIT_BEFORE_SHA or explicit full FORMAT_BASE_SHA is required')
    subprocess.run(['git', 'cat-file', '-e', f'{base}^{{commit}}'], cwd=ROOT, check=True)
    subprocess.run(['git', 'merge-base', '--is-ancestor', base, commit], cwd=ROOT, check=True)
    report.update(base_sha=base, pipeline_source=pipeline_source, source_sha=commit)
    report['toolchain'] = snapshot(ROOT / 'rust-toolchain.toml')
    report['rustfmt_version'] = subprocess.check_output(
        ['rustup', 'run', toolchain, 'rustfmt', '--version'], text=True,
    ).strip()
    report['vendor_policy'] = {
        'tokio_version': '1.52.3', 'pristine_archive_sha256': TOKIO_SHA256,
        'selection': 'changed Rust files minus byte-identical pristine Tokio sources',
    }
    changed = git('diff', '--no-ext-diff', '--name-only', '-z', '--no-renames',
                  '--diff-filter=ACMT', base, commit, '--', '*.rs')
    paths = [Path(name.decode()) for name in changed.split(b'\0') if name]
    if len(paths) > 2000:
        raise ValueError('More than 2000 changed Rust sources; narrow the legitimate base')
    pristine = pristine_tokio() if any(path.is_relative_to('third_party/tokio') for path in paths) else {}
    for relative in paths:
        path = ROOT / relative
        if path.is_symlink() or not path.is_file() or path.resolve() != path:
            raise ValueError(f'Only regular checkout sources may be formatted: {relative}')
        original = path.read_bytes()
        if relative.is_relative_to('third_party/tokio'):
            upstream = relative.relative_to('third_party/tokio').as_posix()
            if pristine.get(upstream) == original:
                report['pristine_tokio_excluded'].append(relative.as_posix())
                continue
        elif relative.is_relative_to('third_party'):
            baseline = subprocess.run(
                ['git', 'cat-file', '-e', f'{base}:{relative.as_posix()}'],
                cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            )
            if baseline.returncode != 0:
                raise ValueError(f'New vendor source requires explicit pristine provenance: {relative}')
        if len(original) > 4 * 1024 * 1024:
            raise ValueError(f'Source exceeds per-file formatter bound: {relative}')
        edition, config, manifest, edition_owner = formatter_owner(path)
        command = ['rustup', 'run', toolchain, 'rustfmt', '--edition', edition,
                   '--config-path', str(config.relative_to(ROOT)),
                   '--config', 'skip_children=true', '--emit', 'files', relative.as_posix()]
        entry = {
            'path': relative.as_posix(), 'preimage_sha256': digest(original),
            'edition': edition, 'manifest': manifest, 'edition_owner': edition_owner,
            'config': snapshot(config), 'command': command,
        }
        report['files'].append(entry)
        print(f'Formatting owned source: {relative}', flush=True)
        try:
            result = subprocess.run(command, cwd=ROOT, timeout=120)
            formatted = path.read_bytes()
        finally:
            path.write_bytes(original)
        entry['formatter_exit'] = result.returncode
        if result.returncode != 0:
            entry['outcome'] = 'formatter-error'
            continue
        if formatted == original:
            entry['outcome'] = 'consistent'
            continue
        destination = OUTPUT / 'formatted' / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(formatted)
        entry.update(outcome='diff', formatted_sha256=digest(formatted),
                     output=destination.relative_to(OUTPUT).as_posix())
    subprocess.run(['git', 'diff', '--quiet', 'HEAD', '--'], cwd=ROOT, check=True)
    return 1 if any(entry['outcome'] != 'consistent' for entry in report['files']) else 0


def main():
    OUTPUT.mkdir(parents=True, exist_ok=True)
    report = {'schema': 1, 'files': [], 'pristine_tokio_excluded': []}
    report['provenance'] = (OUTPUT / 'provenance.txt').read_text()
    status = 1
    try:
        status = generate(report, *sys.argv[1:])
        report['outcome'] = 'consistent' if status == 0 else 'format-diff-or-error'
    except Exception as error:
        report.update(outcome='gate-error', error=str(error))
        print(f'Format gate failed: {error}', file=sys.stderr)
    finally:
        (OUTPUT / 'manifest.toml').write_text(manifest(report), encoding='utf-8')
    print(f"Format gate: {report['outcome']}; evidence: dist/format/manifest.toml")
    return status


if __name__ == '__main__':
    sys.exit(main())
