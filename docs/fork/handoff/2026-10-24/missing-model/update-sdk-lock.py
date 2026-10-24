"""Merge the pinned consumer's new lock records without resolving unrelated packages."""
from pathlib import Path
import re
import tomllib

root = Path('/home/fg/projects/org/platform/sigma-crates')
path = root / 'Cargo.lock'
text = path.read_text()
packages = tomllib.loads(text)['package']
source = 'git+https://github.com/our-forks/async-openai.git?rev=95b52ebdedf42143083cf3d6f0e0be7c84e9c808#95b52ebdedf42143083cf3d6f0e0be7c84e9c808'

def dep(name, line=None):
    choices = [p for p in packages if p['name'] == name and (line is None or p['version'].startswith(line + '.'))]
    if len(choices) != 1:
        raise RuntimeError(f'Ambiguous dependency: {name}, {line}')
    package = choices[0]
    if sum(p['name'] == name for p in packages) > 1:
        return name + ' ' + package['version']
    return name

def record(name, version, dependencies, package_source=source, checksum=None):
    block = f'[[package]]\nname = "{name}"\nversion = "{version}"\nsource = "{package_source}"\n'
    if checksum:
        block += f'checksum = "{checksum}"\n'
    block += 'dependencies = [\n' + ''.join(f' "{d}",\n' for d in dependencies) + ']\n'
    return block

sdk_deps = ['async-openai-macros', 'backoff'] + [dep(name, line) for name, line in [
    ('base64', '0.22'), ('bytes', None), ('derive_builder', None), ('eventsource-stream', None),
    ('futures', None), ('getrandom', '0.3'), ('rand', '0.9'), ('reqwest', '0.12'),
    ('reqwest-eventsource', None), ('secrecy', '0.10'), ('serde', None), ('serde_json', None),
    ('serde_urlencoded', None), ('thiserror', '2.0'), ('tokio', None), ('tokio-stream', None),
    ('tokio-util', '0.7'), ('tracing', None), ('url', None),
]]
new = [record('async-openai', '0.33.1', sdk_deps),
       record('async-openai-macros', '0.1.1', [dep('proc-macro2'), dep('quote'), dep('syn', '2.0')]),
       record('backoff', '0.4.0', [dep('futures-core'), dep('getrandom', '0.2'), dep('instant'),
              dep('pin-project-lite'), dep('rand', '0.8'), dep('tokio')],
              'registry+https://github.com/rust-lang/crates.io-index',
              'b62ddb9cb1ec0a098ad4bbf9344d0713fa193ae1a80af55febcff2627b6a00c1')]
blocks = text.split('[[package]]\n')
for index in range(1, len(blocks)):
    block = blocks[index]
    if block.startswith('name = "sigma-gateway"\n'):
        block = block.replace('dependencies = [\n', 'dependencies = [\n "async-openai",\n', 1)
    if block.startswith('name = "secrecy"\nversion = "0.10.') and ' "serde",\n' not in block:
        block = block.replace('dependencies = [\n', 'dependencies = [\n "serde",\n', 1)
    blocks[index] = block
text = '[[package]]\n'.join(blocks)
for block in new:
    name = tomllib.loads(block)['package'][0]['name']
    if not any(p['name'] == name for p in packages):
        text += '\n' + block
path.write_text(text)
