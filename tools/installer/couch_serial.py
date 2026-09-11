"""One fixed CID query and at most one reboot on an explicitly selected Couch CDC port."""
import re
import secrets
import time
from couch_install import require


class Unavailable(Exception):
    """No reboot was attempted; a manual restart is still possible."""


class CouchSerial:
    def __init__(self, usb, backend, selected):
        self.usb, self.device = usb, None
        self.claimed, self.detached = [], []
        self.attempted = False
        try:
            devices = [d for d in usb.core.find(find_all=True, idVendor=0x0e8d,
                       idProduct=0x201c, backend=backend)
                       if d.bus == selected.bus and tuple(d.port_numbers or ()) == selected.ports]
            if len(devices) != 1:
                raise Unavailable('Couch serial device unavailable on selected port')
            self.device = devices[0]
            interfaces = list(self.device.get_active_configuration())
            data = [i for i in interfaces if i.bInterfaceClass == 10 and i.bNumEndpoints == 2]
            control = [i for i in interfaces if i.bInterfaceClass == 2 and i.bInterfaceSubClass == 2]
            if len(data) != 1 or len(control) != 1:
                raise Unavailable('Couch serial interface unavailable')
            endpoints = list(data[0])
            incoming = [e for e in endpoints if e.bmAttributes & 3 == 2 and e.bEndpointAddress & 0x80]
            outgoing = [e for e in endpoints if e.bmAttributes & 3 == 2 and not e.bEndpointAddress & 0x80]
            if len(incoming) != 1 or len(outgoing) != 1:
                raise Unavailable('Couch serial endpoints unavailable')
            for interface in (control[0], data[0]):
                number = interface.bInterfaceNumber
                try:
                    if self.device.is_kernel_driver_active(number):
                        self.device.detach_kernel_driver(number)
                        self.detached.append(number)
                except NotImplementedError:
                    pass
                usb.util.claim_interface(self.device, number)
                self.claimed.append(number)
            self.incoming, self.outgoing = incoming[0], outgoing[0]
            self.device.ctrl_transfer(0x21, 0x22, 3, control[0].bInterfaceNumber, None, timeout=1000)
        except Exception as error:
            self.close()
            raise Unavailable('Couch serial interface cannot be opened') from error

    def restart(self, expected_cid):
        require(re.fullmatch('[0-9a-f]{32}', expected_cid) is not None, 'Invalid retained CID')
        require(not self.attempted, 'Couch reboot already attempted')
        marker = ('COUCH_' + secrets.token_hex(16)).encode()
        command = b'\nread -r c < /sys/block/mmcblk0/device/cid; printf "\\n' + marker + b':%s:END\\n" "$c"\n'
        try:
            if self.outgoing.write(command, timeout=1000) != len(command):
                raise Unavailable('CID query write incomplete')
            output = bytearray()
            deadline = time.monotonic() + 5
            matches = []
            while time.monotonic() < deadline and len(output) <= 16384:
                try:
                    part = bytes(self.incoming.read(512, timeout=250))
                except self.usb.core.USBTimeoutError:
                    continue
                output.extend(part)
                if len(output) > 16384:
                    raise Unavailable('Couch CID response exceeds bound')
                matches = re.findall(rb'(?:^|\n)' + marker + rb':([0-9a-fA-F]{32}):END\r?(?:\n|$)', output)
                if matches:
                    break
        except Unavailable:
            raise
        except Exception as error:
            raise Unavailable('Couch CID query unavailable') from error
        if not matches:
            raise Unavailable('Couch CID query did not answer')
        require(len(matches) == 1 and matches[0].decode().lower() == expected_cid,
                'Connected Couch CID differs from retained enrollment; no reboot attempted')
        # Consume before the one write. An exception or short write is ambiguous:
        # propagate it as a hard failure, never offer an automatic second attempt.
        self.attempted = True
        reboot = b'\nsync; reboot\n'
        require(self.outgoing.write(reboot, timeout=1000) == len(reboot),
                'Couch reboot delivery ambiguous; do not retry automatically')

    def close(self):
        if self.device is None:
            return
        for number in reversed(self.claimed):
            try:
                self.usb.util.release_interface(self.device, number)
            except Exception:
                pass
        if not self.attempted:
            for number in self.detached:
                try:
                    self.device.attach_kernel_driver(number)
                except Exception:
                    pass
        try:
            self.usb.util.dispose_resources(self.device)
        except Exception:
            pass
        self.device = None
