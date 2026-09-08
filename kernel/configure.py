#!/usr/bin/env python3
"""Merge tracked Kconfig inputs, or reject requested settings Kconfig dropped."""
import re
import sys
from pathlib import Path


def settings(path):
    result = {}
    for line in Path(path).read_text().splitlines():
        match = re.fullmatch(r'(CONFIG_\w+)=(.*)', line)
        disabled = re.fullmatch(r'# (CONFIG_\w+) is not set', line)
        if match:
            result[match[1]] = match[2]
        elif disabled:
            result[disabled[1]] = 'n'
    return result


def main(args):
    if args[0] == '--check':
        requested, actual = map(settings, args[1:])
        errors = [f'{key}: wanted {value}, got {actual.get(key, "n")}'
                  for key, value in requested.items() if actual.get(key, 'n') != value]
        if errors:
            raise SystemExit('\n'.join(errors))
    else:
        base, fragment, output = args
        merged = settings(base)
        merged.update(settings(fragment))
        Path(output).write_text(''.join(
            f'# {key} is not set\n' if value == 'n' else f'{key}={value}\n'
            for key, value in merged.items()))


if __name__ == '__main__':
    main(sys.argv[1:])
