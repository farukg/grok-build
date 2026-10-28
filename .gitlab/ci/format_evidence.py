"""Encode the versioned formatter adoption manifest as TOML."""


def string(value: str) -> str:
    escaped = []
    for character in value:
        code = ord(character)
        if character in ('"', '\\'):
            escaped.append('\\' + character)
        elif code < 0x20 or code == 0x7f:
            escaped.append(f'\\u{code:04x}')
        elif 0xd800 <= code <= 0xdfff:
            raise ValueError('Invalid Unicode in formatter evidence')
        else:
            escaped.append(character)
    return '"' + ''.join(escaped) + '"'


def strings(values: list[str]) -> str:
    return '[' + ', '.join(string(value) for value in values) + ']'


def snapshot(value: dict[str, str]) -> str:
    return '{ path = ' + string(value['path']) + ', sha256 = ' + string(value['sha256']) + ' }'


def manifest(report: dict) -> str:
    lines = [f"schema = {report['schema']}\n"]
    for key in ('provenance', 'base_sha', 'pipeline_source', 'source_sha',
                'rustfmt_version', 'outcome', 'error'):
        if key in report:
            lines.append(f'{key} = {string(report[key])}\n')
    lines.append('pristine_tokio_excluded = ' + strings(report['pristine_tokio_excluded']) + '\n')
    if 'toolchain' in report:
        lines.append('toolchain = ' + snapshot(report['toolchain']) + '\n')
    if not report['files']:
        lines.append('files = []\n')
    if 'vendor_policy' in report:
        lines.append('\n[vendor_policy]\n')
        for key in ('tokio_version', 'pristine_archive_sha256', 'selection'):
            lines.append(f'{key} = {string(report["vendor_policy"][key])}\n')
    for entry in report['files']:
        lines.append('\n[[files]]\n')
        for key in ('path', 'preimage_sha256', 'edition', 'outcome', 'formatted_sha256', 'output'):
            if key in entry:
                lines.append(f'{key} = {string(entry[key])}\n')
        for key in ('manifest', 'edition_owner', 'config'):
            lines.append(f'{key} = {snapshot(entry[key])}\n')
        lines.append('command = ' + strings(entry['command']) + '\n')
        if 'formatter_exit' in entry:
            lines.append(f'formatter_exit = {entry["formatter_exit"]}\n')
    return ''.join(lines)
