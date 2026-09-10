#!/usr/bin/env python3
"""Terminal front end for Couch's existing installer policy (Linux first).

No hardware-write implementation lives here. Simulation must be explicitly
selected; an unavailable physical adapter never falls back to simulation.
"""
import argparse
from contextlib import redirect_stdout
from dataclasses import dataclass
from pathlib import Path
import sys

import couch_install as core

LOGO = r"""
                         _
   ___ ___  _   _  ___ | |__
  / __/ _ \| | | |/ __|| '_ \
 | (_| (_) | |_| | (__ | | | | _
  \___\___/ \__,_|\___||_| |_|(_)
"""


@dataclass(frozen=True)
class Plan:
    target: str
    release: str
    backups: tuple[str, ...]
    writes: tuple[str, ...]
    backup_dir: Path
    simulation: bool


class CoreAdapter:
    """Small boundary to the reviewed policy engine; never duplicates its checks."""
    def __init__(self, args):
        self.args = args
        self.prepared = None

    def plan(self):
        if not self.args.simulation:
            raise core.InstallError("Physical installation is not approved yet. USB observation is available; no remote will be flashed.")
        if not all((self.args.manifest, self.args.device_dir, self.args.identity, self.args.backup_dir)):
            raise core.InstallError("Simulation requires --manifest, --device-dir, --identity and --backup-dir.")
        release, bundle = core.load_release(self.args.manifest)
        device = core.FileDevice(self.args.device_dir)
        core.preflight(release, device)
        identity = core.identity_record(self.args.identity)
        self.prepared = (release, bundle, device, identity)
        return Plan(device.description['storage_id'], str(self.args.manifest),
                    tuple(sorted(core.IDENTITY_PARTITIONS | release['images'].keys())),
                    tuple(name for name in core.WRITE_ORDER if name in release['images']),
                    self.args.backup_dir, True)

    def apply(self, plan, confirmation, progress):
        if not plan.simulation or not self.args.simulation or self.prepared is None:
            raise core.InstallError("No reviewed simulation plan is available; physical writes remain disabled.")
        if confirmation != plan.target:
            raise core.InstallError("Target confirmation does not match")
        release, bundle, device, identity = self.prepared
        with redirect_stdout(progress):
            core.install_simulated(release, bundle, device, plan.backup_dir,
                                   identity, confirmation, self.args.resume)

    def observe(self, progress):
        with redirect_stdout(progress):
            core.watch_usb(self.args.usb_timeout)


class Terminal:
    def __init__(self, reader, writer):
        self.reader, self.writer = reader, writer

    def line(self, text=""):
        self.writer.write(text + '\n')
        self.writer.flush()

    def ask(self, prompt):
        self.writer.write(prompt)
        self.writer.flush()
        answer = self.reader.readline()
        if not answer:
            raise EOFError
        return answer.strip()

    def run(self, adapter, simulation=False):
        self.line(LOGO)
        self.line("Couch installer · Linux")
        self.line("SIMULATION — regular files only" if simulation else "Physical installation is not approved yet.")
        while True:
            self.line()
            self.line("  1  Review installation plan" if simulation else "  1  Check installation availability")
            self.line("  2  Observe USB connection (descriptors only)")
            self.line("  q  Quit")
            try:
                choice = self.ask("Choose [1/2/q]: ").lower()
                if choice in ('q', 'quit'):
                    return 0
                if choice == '2':
                    self.line("Connect the remote by USB and reboot it. Ctrl-C stops observation.")
                    adapter.observe(self.writer)
                    continue
                if choice != '1':
                    self.line("Choose 1, 2 or q.")
                    continue
                plan = adapter.plan()
                self.line("\nReview this plan")
                self.line("Mode: " + ("SIMULATION — no physical device" if plan.simulation else "PHYSICAL DEVICE"))
                self.line("Target: " + plan.target)
                self.line("Release: " + plan.release)
                self.line("Back up: " + ', '.join(plan.backups))
                self.line("Write and verify: " + ' -> '.join(plan.writes))
                self.line("Originals and journal: " + str(plan.backup_dir))
                self.line("Other partitions are preserved. No automatic reboot.")
                typed = self.ask("Type the exact target ID to continue, or Enter to cancel: ")
                if typed != plan.target:
                    self.line("Cancelled. Nothing was written.")
                    continue
                self.line("\nRunning the reviewed plan. Progress below comes from the policy engine.")
                adapter.apply(plan, typed, self.writer)
                self.line("Simulation completed. Keep the originals and journal." if plan.simulation else "Installation completed. Keep the originals and journal.")
            except KeyboardInterrupt:
                self.line("\nInterrupted. Retain any originals and journal; do not delete recovery information.")
                return 130
            except EOFError:
                self.line("\nInput closed; exiting without further actions.")
                return 0
            except (core.InstallError, OSError, ValueError, KeyError, TypeError) as error:
                self.line("Stopped: " + str(error))


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument('--simulation', action='store_true', help='Explicitly use regular-file simulated device')
    result.add_argument('--manifest', type=Path)
    result.add_argument('--device-dir', type=Path)
    result.add_argument('--identity', type=Path)
    result.add_argument('--backup-dir', type=Path)
    result.add_argument('--resume', action='store_true')
    result.add_argument('--usb-timeout', type=float, default=15)
    return result


def main(argv=None):
    args = parser().parse_args(argv)
    if not sys.platform.startswith('linux'):
        print("The installer terminal is currently supported on Linux only.", file=sys.stderr)
        return 2
    reader = sys.stdin
    opened = None
    try:
        if not reader.isatty():
            opened = open('/dev/tty', 'r', encoding='utf-8')
            reader = opened
        if not sys.stdout.isatty():
            raise OSError("Interactive output needs a terminal")
        return Terminal(reader, sys.stdout).run(CoreAdapter(args), args.simulation)
    except OSError:
        print("An interactive terminal is required. Run from a terminal; for automation use couch_install.py plan/simulate.", file=sys.stderr)
        return 2
    finally:
        if opened is not None:
            opened.close()


if __name__ == '__main__':
    sys.exit(main())
