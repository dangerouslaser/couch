#!/usr/bin/env python3
"""Collect verifiable public source inputs; never inspect a device or copy binary payloads.

Collection is resumable. A manifest is only marked complete when every required
source byte is present and verified. This is provenance tooling, not a license grant.
"""
import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tarfile
import tomllib
from urllib.parse import quote, urlsplit
from urllib.request import Request, urlopen

MAX_DOWNLOAD = 1024 * 1024 * 1024
SOURCE_TOP = {'assets', 'clients', 'daemon', 'gui', 'initramfs', 'kernel', 'model',
              'recovery', 'src', 'stage2', 'tools', 'ui', 'web', 'docs', '.github'}
ROOT_FILES = {'COPYING', 'README.md', 'AGENTS.md', '.gitignore'}
EXCLUDE_PARTS = {'target', 'dist', 'build', '.git', 'scratchpad', 'node_modules', '__pycache__'}
FORBIDDEN_SUFFIXES = {'.img', '.apk', '.so', '.a', '.o', '.pem', '.key', '.elf', '.bin'}
MANIFESTS = ('model/Cargo.toml', 'clients/Cargo.toml', 'daemon/Cargo.toml',
             'ui/Cargo.toml', 'web/Cargo.toml', 'tools/installer/tui/Cargo.toml',
             'tools/installer/host/Cargo.toml', 'tools/installer/linux_stage/probe/Cargo.toml',
             'tools/installer/linux_stage/storage/Cargo.toml')


def sha(path, algorithm='sha256'):
    h = hashlib.new(algorithm)
    with Path(path).open('rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def checked_path(name):
    p = PurePosixPath(name)
    if not name or p.is_absolute() or any(v in ('', '.', '..') for v in name.split('/')) or '\\' in name or any(ord(c) < 32 for c in name):
        raise ValueError('Unsafe source path')
    return p


def write(path, data):
    path = Path(path)
    if path.is_symlink():
        raise ValueError('Symlink output refused')
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        if path.read_bytes() != data:
            raise ValueError(f'Existing source differs: {path.name}')
        return
    temporary = path.with_name(path.name + '.partial')
    with temporary.open('xb') as stream:
        stream.write(data)
    temporary.replace(path)


def report(path, value):
    # Reports are replaceable; source bytes are immutable within a collection.
    data = (json.dumps(value, indent=2, sort_keys=True) + '\n').encode()
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + '.report-new')
    temporary.write_bytes(data)
    temporary.replace(path)


def git(repo, *args):
    return subprocess.check_output(['git', '-C', str(repo), *args], stderr=subprocess.PIPE)


def commit_id(repo, revision):
    value = git(repo, 'rev-parse', '--verify', revision + '^{commit}').decode().strip()
    if not re.fullmatch('[0-9a-f]{40}', value):
        raise ValueError('A full Git commit is required')
    return value


def source_allowed(name):
    path = checked_path(name)
    if any(part in EXCLUDE_PARTS for part in path.parts):
        return False
    if name not in ROOT_FILES and path.parts[0] not in SOURCE_TOP:
        return False
    if path.suffix.lower() in FORBIDDEN_SUFFIXES:
        raise ValueError(f'Binary/private input in source tree: {name}')
    if path.name in ('.env', 'id_rsa', 'id_ed25519'):
        raise ValueError('Private input in source tree')
    return True


def project(repo, revision, output):
    commit = commit_id(repo, revision)
    entries = git(repo, 'ls-tree', '-rz', '--full-tree', commit).split(b'\0')
    files, excluded = {}, []
    for entry in filter(None, entries):
        header, raw_name = entry.split(b'\t', 1)
        mode, kind, oid = header.decode().split()
        name = raw_name.decode('utf-8')
        if not source_allowed(name):
            excluded.append(name)
            continue
        if kind != 'blob' or mode not in ('100644', '100755'):
            raise ValueError('Source symlinks/submodules require explicit review')
        data = git(repo, 'cat-file', 'blob', oid)
        if data.startswith((b'\x7fELF', b'MZ')):
            raise ValueError(f'Executable binary in public source: {name}')
        path = output / 'couch' / name
        write(path, data)
        path.chmod(0o755 if mode == '100755' else 0o644)
        files[name] = hashlib.sha256(data).hexdigest()
    manifest = {'schema': 1, 'kind': 'couch-project-source', 'complete': True,
                'commit': commit, 'source_date_epoch': int(git(repo, 'show', '-s', '--format=%ct', commit)), 'files': files, 'excluded_generated_or_unrelated': excluded}
    report(output / 'project.json', manifest)
    return manifest


def tree_hashes(root):
    values = {}
    for path in sorted(root.rglob('*')):
        if path.is_symlink():
            raise ValueError('Source archive cannot contain symlinks')
        if path.is_file():
            values[path.relative_to(root).as_posix()] = sha(path)
        elif not path.is_dir():
            raise ValueError('Special file in sources')
    return values


def cargo_sources(output, offline=False):
    root = output / 'couch'
    absent = [name for name in MANIFESTS if not (root / name).is_file()]
    if absent:
        raise ValueError('Missing release Cargo manifests: ' + ', '.join(absent))
    command = ['cargo', 'vendor', '--locked', '--versioned-dirs', '--manifest-path', str(root / MANIFESTS[0])]
    if offline:
        command.append('--offline')
    for name in MANIFESTS[1:]:
        command.extend(['--sync', str(root / name)])
    command.append(str(output / 'cargo-vendor'))
    config = subprocess.check_output(command, cwd=root)
    # Make Cargo's generated source replacement relocatable in this archive.
    config = config.decode().replace(str(output / 'cargo-vendor'), 'cargo-vendor')
    write(output / 'cargo-config/vendor.toml', config.encode())
    packages = []
    for directory in sorted((output / 'cargo-vendor').iterdir()):
        if not directory.is_dir() or directory.is_symlink():
            raise ValueError('Unexpected vendor directory')
        metadata = tomllib.loads((directory / 'Cargo.toml').read_text())['package']
        notices = [p.relative_to(directory).as_posix() for p in directory.rglob('*')
                   if p.is_file() and any(word in p.name.lower() for word in ('license', 'licence', 'copying', 'notice', 'copyright'))]
        packages.append({'name': metadata['name'], 'version': metadata['version'],
                         'license': metadata.get('license'), 'license_file': metadata.get('license-file'),
                         'directory': directory.name, 'notice_files': sorted(notices)})
    # Verify each registry package in every lock is actually included. Git sources
    # remain checked by cargo vendor; they retain their Cargo checksum manifests.
    present = {(p['name'], p['version']) for p in packages}
    for name in MANIFESTS:
        lock = root / str(PurePosixPath(name).parent / 'Cargo.lock')
        if not lock.exists():
            # Linux-stage crates may share their nearest workspace lock.
            parents = list(lock.parents)
            lock = next((p / 'Cargo.lock' for p in parents if (p / 'Cargo.lock').is_file()), lock)
        parsed = tomllib.loads(lock.read_text())
        for p in parsed['package']:
            if p.get('source') and (p['name'], p['version']) not in present:
                raise ValueError('A locked Cargo source is absent')
    for name in MANIFESTS:
        subprocess.run(['cargo', 'metadata', '--format-version=1', '--locked', '--offline',
                        '--all-features', '--config', str(output / 'cargo-config/vendor.toml'),
                        '--manifest-path', str(root / name)], cwd=root, stdout=subprocess.DEVNULL, check=True)
    result = {'schema': 1, 'kind': 'couch-cargo-sources', 'complete': True,
              'manifests': list(MANIFESTS), 'packages': packages,
              'files': tree_hashes(output / 'cargo-vendor'), 'config_sha256': sha(output / 'cargo-config/vendor.toml')}
    report(output / 'cargo.json', result)
    return result


def apk_info(path):
    # APKs contain concatenated gzip/tar members. Never extract their payloads.
    with gzip.open(path, 'rb') as compressed, tarfile.open(fileobj=compressed, mode='r|', ignore_zeros=True) as archive:
        for entry in archive:
            if entry.name in ('.PKGINFO', './.PKGINFO'):
                if not entry.isfile() or entry.size > 65536:
                    raise ValueError('Invalid APK metadata')
                text = archive.extractfile(entry).read().decode()
                result = {}
                for line in text.splitlines():
                    if ' = ' in line:
                        key, value = line.split(' = ', 1)
                        if key in ('pkgname', 'pkgver', 'origin', 'commit', 'license', 'arch'):
                            if key in result:
                                raise ValueError('Duplicate package metadata')
                            result[key] = value
                return result
    raise ValueError('APK metadata absent')


def checksums(recipe):
    result = {}
    for checksum, name in re.findall(r'([0-9a-f]{128})[ \t]+([^\s"\']+)', recipe):
        checked_path(name)
        if name in result and result[name] != checksum:
            raise ValueError('Ambiguous upstream source checksum')
        result[name] = checksum
    if not result:
        raise ValueError('Recipe lacks supported SHA512 source checksums')
    return result


def download(url, target, expected, offline=False):
    if target.exists():
        if sha(target, 'sha512') != expected:
            raise ValueError('Cached source checksum differs')
        return
    if offline:
        raise ValueError('Source unavailable in offline cache')
    parsed = urlsplit(url)
    if parsed.scheme != 'https' or parsed.username or parsed.password or parsed.fragment:
        raise ValueError('Source downloads require public HTTPS URLs')
    temporary = target.with_name(target.name + '.download')
    target.parent.mkdir(parents=True, exist_ok=True)
    try:
        with urlopen(Request(url, headers={'User-Agent': 'Couch-corresponding-source/1'}), timeout=45) as response, temporary.open('xb') as stream:
            if urlsplit(response.url).scheme != 'https':
                raise ValueError('Source redirected away from HTTPS')
            count = 0
            while block := response.read(1024 * 1024):
                count += len(block)
                if count > MAX_DOWNLOAD:
                    raise ValueError('Source exceeds download bound')
                stream.write(block)
        if sha(temporary, 'sha512') != expected:
            raise ValueError('Downloaded upstream checksum differs')
        temporary.replace(target)
    finally:
        temporary.unlink(missing_ok=True)


def unpack_recipe(data):
    """Flatten only safe, bounded in-recipe aliases; retain original Git tar too."""
    files, aliases = {}, {}
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        for entry in archive:
            if entry.isdir():
                continue
            checked_path(entry.name)
            if entry.issym() or entry.islnk():
                checked_path(entry.linkname)
                target = str(PurePosixPath(entry.name).parent / entry.linkname) if entry.issym() else entry.linkname
                aliases[entry.name] = target
            elif entry.isfile() and entry.size <= 32 * 1024 * 1024:
                files[entry.name] = archive.extractfile(entry).read()
            else:
                raise ValueError('Unsafe recipe archive entry')
    for name, target in aliases.items():
        seen = {name}
        while target in aliases:
            if target in seen or len(seen) >= 16:
                raise ValueError('Cyclic recipe alias')
            seen.add(target)
            target = aliases[target]
        if target not in files:
            raise ValueError('Missing recipe alias target')
        files[name] = files[target]
    if '.couch-original-recipe.tar' in files:
        raise ValueError('Reserved recipe receipt name')
    files['.couch-original-recipe.tar'] = data
    return files


def recipe_files(aports, commit, origin):
    if not re.fullmatch('[0-9a-f]{40}', commit) or not re.fullmatch('[a-z0-9][a-z0-9+_.-]*', origin):
        raise ValueError('Invalid recipe identity')
    for repository in ('main', 'community', 'testing'):
        directory = f'{repository}/{origin}'
        try:
            data = git(aports, 'archive', f'{commit}:{directory}')
        except subprocess.CalledProcessError:
            continue
        files = unpack_recipe(data)
        if 'APKBUILD' not in files:
            raise ValueError('Recipe missing APKBUILD')
        return directory, files
    raise ValueError('Pinned recipe not found in aports cache')


def alpine_sources(closure_dir, metadata_file, aports, cache, output, offline=False, overrides=None):
    import package_closure
    closure = package_closure.verify(closure_dir)
    rows = json.loads(metadata_file.read_text())
    packages, origins, errors = [], {}, []
    declared = {(r['pkgname'], r['pkgver']): r for r in rows}
    if len(declared) != len(rows):
        raise ValueError('Duplicate package metadata rows')
    for name in sorted(closure['files']):
        if not name.startswith('packages/'):
            continue
        info = apk_info(closure_dir / name)
        if declared.get((info['pkgname'], info['pkgver'])) is None:
            raise ValueError('Closure package absent from source metadata')
        row = declared[(info['pkgname'], info['pkgver'])]
        if any(row.get(key) != info.get(key) for key in ('pkgname', 'pkgver', 'origin', 'commit', 'license', 'arch')):
            raise ValueError('Source metadata differs from verified APK')
        packages.append({**info, 'apk_sha256': closure['files'][name]})
        origins[(info['origin'], info['commit'])] = None
    if len(packages) != len(rows):
        raise ValueError('Source metadata contains packages outside the closure')
    for origin, commit in sorted(origins):
        key = origin + '-' + commit
        try:
            directory, files = recipe_files(aports, commit, origin)
            source_checksums = checksums(files['APKBUILD'].decode())
            for name, data in files.items():
                write(output / 'alpine' / key / 'recipe' / name, data)
            sources = []
            for name, expected in source_checksums.items():
                target = cache / expected[:16] / name
                url = (overrides or {}).get(key + '/' + name, 'https://distfiles.alpinelinux.org/distfiles/v3.21/' + quote(name))
                if name in files:
                    if hashlib.sha512(files[name]).hexdigest() != expected:
                        raise ValueError('Recipe local-file checksum differs')
                    write(target, files[name])
                    url = f'https://gitlab.alpinelinux.org/alpine/aports/-/blob/{commit}/{directory}/{name}'
                else:
                    download(url, target, expected, offline)
                destination = output / 'alpine' / key / 'sources' / name
                write(destination, target.read_bytes())
                sources.append({'file': name, 'sha512': expected, 'sha256': sha(target), 'url': url})
            origins[(origin, commit)] = {'origin': origin, 'commit': commit, 'recipe_path': directory, 'sources': sources}
            print(f'Collected Alpine source: {origin}', flush=True)
        except (ValueError, OSError, subprocess.SubprocessError) as error:
            errors.append({'origin': origin, 'commit': commit, 'error': str(error)})
    result = {'schema': 1, 'kind': 'couch-alpine-sources', 'complete': not errors,
              'closure_sha256': sha(closure_dir / 'closure.json'), 'packages': packages,
              'origins': [v for v in origins.values() if v], 'errors': errors,
              'files': tree_hashes(output / 'alpine')}
    report(output / 'alpine.json', result)
    if errors:
        raise ValueError(f'{len(errors)} Alpine source origins remain incomplete; see alpine.json')
    return result


def external_sources(directory, receipt_file, output):
    """Import audited kernel/BusyBox inputs; receipts must name actual source bytes."""
    receipt = json.loads(receipt_file.read_text())
    name = receipt.get('component')
    if receipt.get('schema') != 1 or receipt.get('kind') != 'couch-external-source' or name not in ('kernel', 'busybox', 'rust-stdlib'):
        raise ValueError('Unsupported external source receipt')
    files = receipt.get('files', {})
    required = [receipt.get('source_archive'), receipt.get('configuration'), receipt.get('build_recipe'), receipt.get('toolchain_receipt')]
    if not files or any(not p or p not in files for p in required):
        raise ValueError('External receipt needs source, configuration, build recipe and toolchain bytes')
    if not re.fullmatch('[0-9a-f]{64}', receipt.get('binary_sha256', '')):
        raise ValueError('External source must identify its corresponding binary')
    for filename, expected in files.items():
        checked_path(filename)
        path = directory / filename
        if path.is_symlink() or not path.is_file() or sha(path) != expected:
            raise ValueError('External source receipt checksum differs')
        if path.suffix in FORBIDDEN_SUFFIXES or path.read_bytes()[:4] == b'\x7fELF':
            raise ValueError('Binary artifact supplied instead of corresponding source')
        write(output / 'external' / name / filename, path.read_bytes())
    result = {**receipt, 'complete': True}
    report(output / (name + '.json'), result)
    return result


def assemble(output, archive_path):
    included, components = {}, {}
    for name, directory in [('project', 'couch'), ('cargo', 'cargo-vendor'), ('alpine', 'alpine'),
                            ('kernel', 'external/kernel'), ('busybox', 'external/busybox'), ('rust-stdlib', 'external/rust-stdlib')]:
        receipt = output / (name + '.json')
        if not receipt.is_file():
            raise ValueError('Missing source component: ' + name)
        value = json.loads(receipt.read_text())
        if value.get('complete') is not True or value.get('errors'):
            raise ValueError('Incomplete source component: ' + name)
        if tree_hashes(output / directory) != value['files']:
            raise ValueError('Source files changed after collection: ' + name)
        components[name] = value
        included[name + '.json'] = sha(receipt)
        for path, checksum in value['files'].items():
            checked_path(path)
            included[directory + '/' + path] = checksum
    config = output / 'cargo-config/vendor.toml'
    if sha(config) != components['cargo']['config_sha256']:
        raise ValueError('Cargo source replacement configuration changed')
    included['cargo-config/vendor.toml'] = sha(config)
    notices = ['# Couch source and third-party notices', '',
               'Couch uses GPL-3.0-or-later (see couch/COPYING). This release selects the GPL route for Slint.',
               'Complete vendored sources retain upstream license/copyright/notice files. Alpine upstream archives and recipes retain their notices.',
               'Declared license expressions below are metadata, not a replacement for those texts.', '', '## Rust dependencies', '']
    for p in components['cargo']['packages']:
        notices.append(f"- {p['name']} {p['version']}: {p.get('license') or 'see license-file'}; source cargo-vendor/{p['directory']}; notices: {', '.join(p['notice_files']) or 'see source headers and package metadata'}")
    notices.extend(['', '## Alpine runtime packages', ''])
    for p in components['alpine']['packages']:
        notices.append(f"- {p['pkgname']} {p['pkgver']}: {p['license']}; origin {p['origin']} at {p['commit']} (alpine/{p['origin']}-{p['commit']}/).")
    notices.extend(['', '## Building', '',
                    'Use the matching Couch source recipes in couch/tools and couch/kernel. Each external component includes its configuration, build recipe and toolchain receipt.',
                    'To use vendored Rust dependencies, run Cargo from couch/ and pass --config ../cargo-config/vendor.toml --locked --offline with the chosen workspace manifest. Keep the archive directory layout intact.',
                    'This source archive contains no Android/vendor payload, calibration, user configuration or binary image. Vendor extraction remains an owner-local step.', ''])
    write(output / 'NOTICES.md', ('\n'.join(notices)).encode())
    included['NOTICES.md'] = sha(output / 'NOTICES.md')
    manifest = {'schema': 1, 'kind': 'couch-corresponding-source-archive', 'complete': True,
                'project_commit': components['project']['commit'], 'files': included,
                'scope': 'Couch/runtime/installer, locked Cargo dependencies, Alpine closure, kernel, BusyBox, Rust standard library; excludes owner-local Android vendor inputs'}
    report(output / 'SOURCE-MANIFEST.json', manifest)
    names = sorted([*included, 'SOURCE-MANIFEST.json'])
    if archive_path.exists():
        raise ValueError('Refusing to overwrite a source archive')
    archive_path.parent.mkdir(parents=True, exist_ok=True)
    temporary = archive_path.with_name(archive_path.name + '.partial')
    epoch = components['project']['source_date_epoch']
    try:
        with temporary.open('xb') as stream, gzip.GzipFile(fileobj=stream, mode='wb', filename='', mtime=0) as compressed, tarfile.open(fileobj=compressed, mode='w|') as archive:
            for name in names:
                path = output / name
                entry = tarfile.TarInfo('couch-source/' + name)
                entry.size = path.stat().st_size
                entry.mode = 0o755 if path.stat().st_mode & 0o111 else 0o644
                entry.mtime = epoch
                with path.open('rb') as source:
                    archive.addfile(entry, source)
        temporary.replace(archive_path)
    finally:
        temporary.unlink(missing_ok=True)
    result = {'archive': archive_path.name, 'sha256': sha(archive_path), 'project_commit': manifest['project_commit'], 'complete': True}
    report(archive_path.with_name(archive_path.name + '.json'), result)
    return result


def verify_archive(path):
    """Verify every published tar member without extracting anything."""
    hashes, manifest = {}, None
    with gzip.open(path, 'rb') as compressed, tarfile.open(fileobj=compressed, mode='r|') as archive:
        for count, entry in enumerate(archive):
            if count >= 200000 or not entry.isfile() or entry.size > MAX_DOWNLOAD:
                raise ValueError('Unexpected source archive member')
            if not entry.name.startswith('couch-source/'):
                raise ValueError('Unexpected source archive prefix')
            name = entry.name[len('couch-source/'):]
            checked_path(name)
            if name in hashes or (name == 'SOURCE-MANIFEST.json' and manifest is not None):
                raise ValueError('Duplicate source archive member')
            stream = archive.extractfile(entry)
            if name == 'SOURCE-MANIFEST.json':
                if entry.size > 16 * 1024 * 1024:
                    raise ValueError('Oversized source manifest')
                manifest = json.loads(stream.read())
            else:
                digest = hashlib.sha256()
                for block in iter(lambda: stream.read(1024 * 1024), b''):
                    digest.update(block)
                hashes[name] = digest.hexdigest()
    if not manifest or manifest.get('kind') != 'couch-corresponding-source-archive' or manifest.get('schema') != 1 or manifest.get('complete') is not True or hashes != manifest.get('files'):
        raise ValueError('Source archive differs from its complete manifest')
    return {'archive': Path(path).name, 'sha256': sha(path), 'project_commit': manifest['project_commit'], 'verified_files': len(hashes)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    p = sub.add_parser('project'); p.add_argument('--repo', type=Path, required=True); p.add_argument('--commit', required=True); p.add_argument('--output', type=Path, required=True)
    p = sub.add_parser('cargo'); p.add_argument('--output', type=Path, required=True); p.add_argument('--offline', action='store_true')
    p = sub.add_parser('alpine'); p.add_argument('--closure', type=Path, required=True); p.add_argument('--metadata', type=Path, required=True); p.add_argument('--aports', type=Path, required=True); p.add_argument('--cache', type=Path, required=True); p.add_argument('--output', type=Path, required=True); p.add_argument('--offline', action='store_true'); p.add_argument('--source-overrides', type=Path)
    p = sub.add_parser('external'); p.add_argument('--directory', type=Path, required=True); p.add_argument('--receipt', type=Path, required=True); p.add_argument('--output', type=Path, required=True)
    p = sub.add_parser('assemble'); p.add_argument('--output', type=Path, required=True); p.add_argument('--archive', type=Path, required=True)
    p = sub.add_parser('verify-archive'); p.add_argument('--archive', type=Path, required=True)
    args = parser.parse_args()
    if args.command == 'project': project(args.repo, args.commit, args.output)
    elif args.command == 'cargo': cargo_sources(args.output, args.offline)
    elif args.command == 'external': external_sources(args.directory, args.receipt, args.output)
    elif args.command == 'verify-archive': print(json.dumps(verify_archive(args.archive)))
    elif args.command == 'assemble': print(json.dumps(assemble(args.output, args.archive)))
    elif args.command == 'alpine': alpine_sources(args.closure, args.metadata, args.aports, args.cache, args.output, args.offline, json.loads(args.source_overrides.read_text()) if args.source_overrides else None)


if __name__ == '__main__':
    main()
