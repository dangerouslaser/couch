#!/bin/sh
# Maintainer-reviewed pins only. Never resolve "latest" or trust user-provided URLs.
set -eu
APPROVED_VERSION=''
APPROVED_SHA256=''

if [ -z "$APPROVED_VERSION" ] || [ -z "$APPROVED_SHA256" ]; then
    echo 'Couch has no approved public installer release yet. Nothing was downloaded or installed.' >&2
    exit 2
fi
case "$(uname -s):$(uname -m)" in
    Linux:x86_64) platform=linux-x86_64 ;;
    *) echo 'This installer release supports Linux x86_64 only.' >&2; exit 2 ;;
esac
command -v python3 >/dev/null 2>&1 || { echo 'Python 3.10 or newer is required.' >&2; exit 2; }
command -v curl >/dev/null 2>&1 || { echo 'curl is required.' >&2; exit 2; }
python3 -c 'import sys; sys.exit(0 if sys.version_info >= (3,10) else 2)' || {
    echo 'Python 3.10 or newer is required.' >&2; exit 2;
}
# curl | sh uses stdin for the script; interaction always reads the terminal.
if ! { exec 3</dev/tty; } 2>/dev/null; then
    echo 'Run this command in an interactive terminal.' >&2
    exit 2
fi
umask 077
work=$(mktemp -d "${TMPDIR:-/tmp}/couch-bootstrap.XXXXXXXX")
trap 'rm -rf "$work"' EXIT
trap 'exit 130' INT
trap 'exit 143' HUP TERM
url="https://github.com/dangerouslaser/couch/releases/download/$APPROVED_VERSION/couch-installer-$APPROVED_VERSION-$platform.tar.gz"
curl --fail --location --proto '=https' --proto-redir '=https' --tlsv1.2 \
    --connect-timeout 15 --max-time 300 --max-filesize 67108864 \
    --output "$work/installer.tar.gz" "$url"
python3 - "$work/installer.tar.gz" "$APPROVED_SHA256" "$work/payload" <<'COUCH_VERIFY_ARCHIVE'
import hashlib
from pathlib import Path, PurePosixPath
import re
import sys
import tarfile

archive, expected, destination = Path(sys.argv[1]), sys.argv[2], Path(sys.argv[3])
if not re.fullmatch(r'[0-9a-f]{64}', expected):
    raise SystemExit('Invalid approved checksum pin')
if archive.stat().st_size > 64 * 1024 * 1024:
    raise SystemExit('Installer download exceeds the approved size limit')
if hashlib.sha256(archive.read_bytes()).hexdigest() != expected:
    raise SystemExit('Installer checksum mismatch; nothing will be executed')
# Even authenticated packages cannot extract links, devices or paths outside the
# private destination. Validate all entries before creating any output files.
with tarfile.open(archive, 'r:gz') as bundle:
    members, seen, unpacked = [], set(), 0
    for member in bundle:
        unpacked += member.size
        if len(members) >= 10000 or unpacked > 128 * 1024 * 1024:
            raise SystemExit('Installer unpacked size limit exceeded')
        path = PurePosixPath(member.name)
        if (path.is_absolute() or '..' in path.parts or not path.parts
                or path.parts[0] != 'couch-installer'
                or not (member.isfile() or member.isdir())
                or member.name in seen):
            raise SystemExit('Unsafe installer archive member')
        seen.add(member.name)
        members.append(member)
    destination.mkdir(mode=0o700)
    for member in members:
        output = destination.joinpath(*PurePosixPath(member.name).parts)
        if member.isdir():
            output.mkdir(mode=0o700, parents=True, exist_ok=True)
        else:
            output.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            with bundle.extractfile(member) as source, output.open('xb') as target:
                while block := source.read(1024 * 1024):
                    target.write(block)
            output.chmod(0o600)
entry = destination / 'couch-installer/tools/installer/couch_tui.py'
if not entry.is_file():
    raise SystemExit('Installer package is missing its terminal entry point')
COUCH_VERIFY_ARCHIVE
python3 "$work/payload/couch-installer/tools/installer/couch_tui.py" <&3
