import re
from types import SimpleNamespace as NS
import unittest
from couch_install import InstallError
from couch_serial import CouchSerial, Unavailable


class SerialTests(unittest.TestCase):
    def fixture(self, cid='1'*32, *, reboot_short=False, query_echo=False):
        serial = CouchSerial.__new__(CouchSerial)
        serial.attempted = False
        writes, pending = [], bytearray()
        def write(data, **kwargs):
            writes.append(data)
            if data == b'\nsync; reboot\n':
                return 1 if reboot_short else len(data)
            marker = re.search(rb'COUCH_[0-9a-f]{32}', data).group()
            pending.extend(data if query_echo else b'\n' + marker + b':' + cid.encode() + b':END\r\n')
            return len(data)
        def read(size, **kwargs):
            if not pending:
                raise RuntimeError('fixture has no more output')
            value = bytes(pending[:size]); del pending[:size]; return value
        serial.outgoing = NS(write=write)
        serial.incoming = NS(read=read)
        serial.usb = NS(core=NS(USBTimeoutError=TimeoutError))
        return serial, writes

    def test_matching_nonce_cid_allows_one_fixed_reboot(self):
        serial, writes = self.fixture()
        serial.restart('1'*32)
        self.assertEqual(writes[-1], b'\nsync; reboot\n')
        with self.assertRaises(InstallError): serial.restart('1'*32)
        self.assertEqual(len(writes), 2)

    def test_wrong_identity_never_reboots_or_falls_back(self):
        serial, writes = self.fixture(cid='2'*32)
        with self.assertRaises(InstallError): serial.restart('1'*32)
        self.assertEqual(len(writes), 1)
        self.assertFalse(serial.attempted)

    def test_echoed_query_is_not_identity_evidence(self):
        serial, writes = self.fixture(query_echo=True)
        with self.assertRaises(Unavailable): serial.restart('1'*32)
        self.assertEqual(len(writes), 1)

    def test_ambiguous_reboot_consumes_attempt(self):
        serial, writes = self.fixture(reboot_short=True)
        with self.assertRaises(InstallError): serial.restart('1'*32)
        self.assertTrue(serial.attempted)
        with self.assertRaises(InstallError): serial.restart('1'*32)
        self.assertEqual(len(writes), 2)

    def test_wrong_physical_port_is_never_claimed(self):
        claims = []
        usb = NS(core=NS(find=lambda **kw:[NS(bus=2,port_numbers=(3,))]),
                 util=NS(claim_interface=lambda *args:claims.append(args)))
        with self.assertRaises(Unavailable):
            CouchSerial(usb, object(), NS(bus=2, ports=(4,)))
        self.assertEqual(claims, [])

if __name__ == '__main__': unittest.main()
