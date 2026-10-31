#!/usr/bin/env python3
"""Verify the immutable Cargo family producer and archive."""
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import tarfile
import tomllib
import urllib.request


FORK_SOURCE = 'b4e19fb9ae93d9a5cc6156412b751ac32d6fdcec'
BINARIES = {'cargo-localbuild', 'cargo-quick', 'cargo-perf', 'cargo-size',
            'cargo-release', 'cargo-remote-bld', 'sig'}
REQUIRED = {'source.tar', 'sidecar.tar', 'SOURCE.txt', 'help.txt', 'SHA256SUMS',
            'resolution/metadata.json', 'resolution/vendor-config.toml',
            'resolution/manifest-projection.diff', 'resolution/Cargo.lock',
            'resolution/Cargo.toml', 'resolution/rust-toolchain.toml'} | {f'bin/{name}' for name in BINARIES}


def fail(message):
    raise SystemExit(f'Cargo family: {message}')


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def main():
    def env(name, pattern=None):
        value = os.environ.get('CARGO_LOCALBUILD_FAMILY_' + name, '')
        if not value or (pattern and not re.fullmatch(pattern, value)):
            fail(f'invalid {name}')
        return value

    base = env('BASE_URL').rstrip('/')
    if not base.startswith('https://'):
        fail('API base URL must use HTTPS')
    project = env('PROJECT_ID', r'[1-9][0-9]*')
    if project != '28':
        fail('producer project must be sigma-crates project 28')
    pipeline = env('PIPELINE_ID', r'[1-9][0-9]*')
    job = env('JOB_ID', r'[1-9][0-9]*')
    commit = env('COMMIT', r'[0-9a-f]{40}')
    archive_sha = env('ARCHIVE_SHA256', r'[0-9a-f]{64}')
    fork_sha = env('CARGO_FORK_SHA256', r'[0-9a-f]{64}')
    seal_sha = env('SEAL_FILE_SHA256', r'[0-9a-f]{64}')
    job_url = env('JOB_URL')
    pipeline_url = env('PIPELINE_URL')
    if (f'/org/platform/sigma-crates/-/jobs/{job}' not in job_url
            or f'/org/platform/sigma-crates/-/pipelines/{pipeline}' not in pipeline_url):
        fail('producer URLs do not identify sigma-crates')
    token = Path('/run/secrets/gitlab_job_token').read_text()
    if not re.fullmatch(r'[A-Za-z0-9._-]+', token):
        fail('invalid job token format')

    class HTTPSRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, req, fp, code, msg, headers, newurl):
            if not newurl.startswith('https://'):
                fail('artifact redirect must use HTTPS')
            return super().redirect_request(req, fp, code, msg, headers, newurl)

    opener = urllib.request.build_opener(HTTPSRedirect())

    def fetch(url, destination=None):
        request = urllib.request.Request(url, headers={'JOB-TOKEN': token})
        with opener.open(request) as response:
            if destination is None:
                return response.read()
            with destination.open('wb') as output:
                shutil.copyfileobj(response, output)

    root = f'{base}/projects/{project}'
    job_data = json.loads(fetch(f'{root}/jobs/{job}'))
    if job_data.get('status') != 'success':
        fail(f'producer job {job} is not successful (status: {job_data.get("status")})')
    if (job_data.get('commit', {}).get('id') != commit
            or job_data.get('pipeline', {}).get('id') != int(pipeline)
            or job_data.get('web_url') != job_url):
        fail('job provenance differs from pinned identity')
    pipeline_data = json.loads(fetch(f'{root}/pipelines/{pipeline}'))
    if (pipeline_data.get('status'), pipeline_data.get('sha'), pipeline_data.get('web_url')) != ('success', commit, pipeline_url):
        fail('pipeline provenance differs from pinned identity')
    artifacts = f'{root}/jobs/{job}/artifacts/ci-artifacts/cargo-localbuild'
    archive_path = Path('/tmp/family-archive.tar')
    seal_path = Path('/tmp/family-complete.toml')
    fetch(f'{artifacts}/family-archive.tar', archive_path)
    fetch(f'{artifacts}/family-complete.toml', seal_path)
    if digest(archive_path) != archive_sha or digest(seal_path) != seal_sha:
        fail('archive or seal digest differs from pin')
    seal = tomllib.loads(seal_path.read_text())
    if (seal.get('schema') != 1 or seal.get('family') != 'cargo-localbuild'
            or seal.get('pipeline_id') != int(pipeline) or seal.get('job_id') != int(job)
            or seal.get('source_sha') != commit or seal.get('archive_file') != 'family-archive.tar'
            or seal.get('archive_sha256') != archive_sha
            or seal.get('archive_size') != archive_path.stat().st_size
            or seal.get('cargo_fork_source') != FORK_SOURCE
            or seal.get('cargo_fork_sha256') != fork_sha):
        fail('seal does not match pinned producer identity')
    files = seal.get('files')
    if not isinstance(files, list):
        fail('seal has no file inventory')
    entries = {}
    for entry in files:
        if not isinstance(entry, dict):
            fail('invalid seal inventory entry')
        name = entry.get('path')
        if not isinstance(name, str):
            fail('invalid seal path')
        path = PurePosixPath(name)
        if (not name or path.is_absolute() or any(part in ('', '.', '..') for part in name.split('/'))
                or '\\' in name or '\t' in name or name.startswith(' ') or name in entries):
            fail('unsafe or duplicate seal path')
        mode = entry.get('mode')
        if (entry.get('package_file') != 'family-archive.tar'
                or not isinstance(entry.get('sha256'), str)
                or not re.fullmatch(r'[0-9a-f]{64}', entry['sha256'])
                or type(entry.get('size')) is not int or entry['size'] < 0
                or type(mode) is not int
                or (mode != 0o755 if name.startswith('bin/') else mode not in (0o600, 0o644, 0o755))):
            fail(f'invalid inventory metadata: {name}')
        entries[name] = entry
    names = set(entries)
    if (not REQUIRED <= names or {n for n in names if n.startswith('bin/')} != {f'bin/{n}' for n in BINARIES}
            or not any(n.startswith('resolution/vendor/') for n in names)
            or any(n not in REQUIRED and not n.startswith('resolution/') for n in names)):
        fail('seal does not contain the complete producer inventory')
    with tarfile.open(archive_path, 'r:') as archive:
        members = archive.getmembers()
        if len(members) != len(entries) or {m.name for m in members} != names:
            fail('archive member list differs from seal')
        for member in members:
            entry = entries[member.name]
            if not member.isfile() or member.mode != entry['mode'] or member.size != entry['size']:
                fail(f'unsafe or mismatched archive member: {member.name}')
            content = archive.extractfile(member)
            if content is None:
                fail(f'unreadable archive member: {member.name}')
            h = hashlib.sha256()
            for block in iter(lambda: content.read(1024 * 1024), b''):
                h.update(block)
            if h.hexdigest() != entry['sha256']:
                fail(f'archive member digest differs from seal: {member.name}')


if __name__ == '__main__':
    main()
