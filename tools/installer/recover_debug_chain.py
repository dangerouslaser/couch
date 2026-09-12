"""Admit restoring the original boot after verified debug-stage transitions.

A failed bootstrap session leaves its temporary installer image in the boot
slot, and recover_native_bootstrap restores the saved original from there. A
chain of completed debug transitions replaces that slot with a debug image
whose hash each completion receipt attests. This module validates such a chain
with the same checks the transition host applies, then hands the retained
failed-session proof and the attested current boot to the existing restore.
It never discovers USB, writes, or reboots.
"""
import argparse
import json
from pathlib import Path

from couch_install import require
import recover_native_bootstrap as recovery
import wifi_debug_transition as debug


def admit(completed, bus, ports):
    """Return (retained failed-session proof, current boot sha256, receipt sha256)."""
    proof = debug._validated_completed_receipt(completed, bus, ports)
    retained = proof.parent.retained
    original = retained.record['originals']['boot']['sha256']
    require(proof.boot_sha256 not in (original, retained.temporary_boot_sha256),
            'Chain must end at a distinct debug boot')
    return retained, proof.boot_sha256, proof.receipt_sha256


def load_attachment(path):
    raw = recovery.read(Path(path), 65536)
    config = json.loads(raw)
    completed = config.get('completed_transition')
    require(isinstance(completed, dict) and config.get('transition') is None
            and config.get('expected_stage') == 'wifi-debug-v1',
            'Expected a saved debug reattach configuration')
    return completed, config['bus'], config['ports']


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--attach-config', type=Path, required=True,
                        help='debug-attach.json saved by the completed debug transition')
    args = parser.parse_args()
    completed, bus, ports = load_attachment(args.attach_config)
    retained, current, receipt = admit(completed, bus, ports)
    print(json.dumps({'admitted': True, 'device_access': False, 'source_session': str(retained.source),
                      'temporary_boot_sha256': retained.temporary_boot_sha256,
                      'current_boot_sha256': current, 'chain_receipt_sha256': receipt,
                      'original_boot_sha256': retained.record['originals']['boot']['sha256']}))
