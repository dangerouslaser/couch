"""Experimental exact-device PyUSB binding; API only, hardware validation pending.

Instantiating imports dependencies; enumerate/claim/start are explicit operations.
The installer CLI deliberately does not invoke this backend yet.
"""
from contextlib import contextmanager
import importlib
import importlib.abc
import importlib.util
import logging
from pathlib import Path
import signal
import struct
import sys
import tempfile
import threading

from couch_install import InstallError, require
from mtk_session import Candidate, ReadPolicy, select_candidate, source_pin


class _Deadline(BaseException):
    # Upstream has broad Exception handlers. A deadline must escape those loops.
    pass


@contextmanager
def bounded_operation(seconds):
    require(threading.current_thread() is threading.main_thread() and hasattr(signal, "setitimer"),
            "Physical MTK operations require the Linux/macOS main thread")
    require(signal.getitimer(signal.ITIMER_REAL) == (0.0, 0.0), "Another operation owns the deadline timer")
    previous = signal.getsignal(signal.SIGALRM)
    def expired(signum, frame):
        raise _Deadline()
    signal.signal(signal.SIGALRM, expired)
    signal.setitimer(signal.ITIMER_REAL, seconds, 0.1)
    try:
        yield
    except _Deadline as error:
        raise InstallError("MTK operation deadline exceeded; session cannot reconnect") from error
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)


class PinnedImports(importlib.abc.MetaPathFinder, importlib.abc.Loader):
    """Compile verified package source directly, ignoring on-disk bytecode caches."""
    def __init__(self, checkout):
        self.checkout = Path(checkout).resolve()
        self.pinned = source_pin(self.checkout)
        require(not any(name == "mtkclient" or name.startswith("mtkclient.") for name in sys.modules),
                "Start the physical backend in a fresh process without imported mtkclient modules")
        self.loaded = []

    def find_spec(self, fullname, path=None, target=None):
        if fullname != "mtkclient" and not fullname.startswith("mtkclient."):
            return None
        base = self.checkout.joinpath(*fullname.split("."))
        filename = base / "__init__.py" if (base / "__init__.py").exists() else base.with_suffix(".py")
        require(filename in self.pinned, f"Module is not in the source pin: {fullname}")
        return importlib.util.spec_from_file_location(fullname, filename, loader=self,
                    submodule_search_locations=[str(base)] if filename.name == "__init__.py" else None)

    def create_module(self, spec):
        return None

    def exec_module(self, module):
        import hashlib
        path = Path(module.__spec__.origin)
        data = path.read_bytes()
        require(hashlib.sha256(data).hexdigest() == self.pinned[path], "Source changed during import")
        self.loaded.append(module.__name__)
        exec(compile(data, str(path), "exec"), module.__dict__)

    def close(self):
        if self in sys.meta_path:
            sys.meta_path.remove(self)
        for name in reversed(self.loaded):
            sys.modules.pop(name, None)


def descriptor(device):
    return Candidate(device.bus, device.address, tuple(device.port_numbers or ()), device.idVendor, device.idProduct)


class ExactUsbBackend:
    def __init__(self, checkout, *, usb=None, bindings=None):
        self.checkout = checkout
        if usb is None:
            import usb.core
            import usb.util
            usb = sys.modules["usb"]
        self.usb = usb
        self.bindings = bindings  # Unit tests supply constructors; production uses pinned imports.
        self.device = None
        self.interfaces = []
        self.detached = []
        self.mtk = None
        self.imports = None
        self.work = None
        self.started = False

    def enumerate(self):
        return [descriptor(dev) for dev in self.usb.core.find(find_all=True, idVendor=0x0e8d)]

    def claim(self, expected):
        with bounded_operation(10):
            self._claim(expected)

    def _claim(self, expected):
        require(self.device is None, "USB backend already claimed a device")
        devices = list(self.usb.core.find(find_all=True, idVendor=0x0e8d))
        select_candidate([descriptor(dev) for dev in devices], expected)
        dev = next(dev for dev in devices if descriptor(dev) == expected)
        self.device = dev  # Retain this exact descriptor; never rediscover on an error.
        dev.default_timeout = 1000
        config = dev.get_active_configuration()  # No reset or set_configuration fallback.
        self.configuration = config
        candidates = []
        for interface in config:
            if interface.bAlternateSetting != 0 or interface.bInterfaceClass != 0x0a:
                continue
            incoming = [ep for ep in interface if ep.bmAttributes & 3 == 2 and ep.bEndpointAddress & 0x80]
            outgoing = [ep for ep in interface if ep.bmAttributes & 3 == 2 and not ep.bEndpointAddress & 0x80]
            if len(incoming) == len(outgoing) == 1:
                candidates.append((interface, incoming[0], outgoing[0]))
        require(len(candidates) == 1, "Expected one CDC data interface with bulk IN/OUT")
        self.interface, self.ep_in, self.ep_out = candidates[0]
        for number in dict.fromkeys((0, self.interface.bInterfaceNumber)):
            try:
                active = dev.is_kernel_driver_active(number)
            except NotImplementedError:
                active = False
            if active:
                dev.detach_kernel_driver(number)
                self.detached.append(number)
            self.usb.util.claim_interface(dev, number)
            self.interfaces.append(number)

    def claimed_candidate(self):
        return descriptor(self.device) if self.device else None

    def _bindings(self):
        if self.bindings is not None:
            return self.bindings
        self.imports = PinnedImports(self.checkout)
        sys.meta_path.insert(0, self.imports)
        config = importlib.import_module("mtkclient.config.mtk_config").MtkConfig
        mtk = importlib.import_module("mtkclient.Library.mtk_class").Mtk
        return config, mtk

    def _forbidden(self, *args, **kwargs):
        raise InstallError("Reset, reconnect, security bypass and flash writes are disabled")

    def start_readonly(self, loader, policy):
        require(policy == ReadPolicy(), "Unsupported MTK startup policy")
        require(self.device is not None and not self.started, "Claim a fresh USB session before starting")
        self.started = True
        with bounded_operation(30):
            Config, Mtk = self._bindings()
            self.work = tempfile.TemporaryDirectory(prefix="couch-mtk-session-")
            loader_path = Path(self.work.name) / "approved-loader.bin"
            loader_path.write_bytes(loader)
            loader_path.chmod(0o600)
            config = Config(loglevel=logging.CRITICAL)
            config.loader = str(loader_path)
            config.stock = True
            config.skipwdt = True
            config.reconnect = False
            config.write_preloader_to_file = False
            config.hwparam_path = self.work.name
            config.vid, config.pid = self.device.idVendor, self.device.idProduct
            config.interface = self.interface.bInterfaceNumber
            self.mtk = Mtk(config=config, loglevel=logging.CRITICAL)
            mtk = self.mtk
            cdc = mtk.port.cdc
            cdc.device, cdc.interface = self.device, self.interface.bInterfaceNumber
            cdc.EP_IN, cdc.EP_OUT = self.ep_in, self.ep_out
            cdc.vid, cdc.pid = self.device.idVendor, self.device.idProduct
            cdc.connected = True
            cdc.timeout = 1000
            # Disable every discovery/reset entry point before protocol work.
            cdc.connect = self._forbidden
            cdc.detectdevices = self._forbidden
            cdc.get_interface_count = lambda: self.configuration.bNumInterfaces
            cdc.close = self._forbidden
            mtk.port.close = self._forbidden
            mtk.port.detectusbdevices = self._forbidden
            mtk.bypass_security = self._forbidden
            mtk.crasher = self._forbidden
            mtk.preloader.reset_to_brom = self._forbidden
            mtk.daloader.writeflash = self._forbidden
            mtk.daloader.formatflash = self._forbidden
            mtk.daloader.shutdown = self._forbidden
            # One handshake only: bypass Port.handshake's rediscovery loop and
            # Preloader.init's thousand retries. Exceptions stop the transaction.
            def handshake(**kwargs):
                require(mtk.port.run_handshake(retries=1), "Preloader handshake failed")
                return True
            mtk.port.handshake = handshake
            # Guard init_hwcode before upstream can take an unrelated chip's
            # initialization branches, including IoT register writes.
            init_hwcode = config.init_hwcode
            def checked_hwcode(code):
                require(code == 0x6580, "Expected MT6580 before DA startup")
                return init_hwcode(code)
            config.init_hwcode = checked_hwcode
            # The upstream helper reports an unsupported target-config command
            # as all security flags false. Require a real successful response.
            def target_config(display=False):
                preloader = mtk.preloader
                require(preloader.echo(preloader.Cmd.GET_TARGET_CONFIG.value), "Target security query unsupported")
                raw = preloader.rbyte(6)
                require(isinstance(raw, (bytes, bytearray)) and len(raw) == 6, "Short target security response")
                value, status = struct.unpack(">IH", raw)
                require(status <= 0xff, "Target security query failed")
                return {key: bool(value & mask) for key, mask in
                        (("sbc", 1), ("sla", 2), ("daa", 4), ("epp", 8), ("cert", 16),
                         ("memread", 32), ("memwrite", 64), ("cmdC8", 128))}
            mtk.preloader.get_target_config = target_config
            mtk.preloader.display = False
            require(mtk.preloader.init(display=False, directory=self.work.name), "Preloader initialization failed")
            require(config.hwcode == 0x6580 and not config.iot and not config.is_brom,
                    "Only an MT6580 preloader session is supported; BootROM requires separate validation")
            require(isinstance(config.target_config, dict)
                    and all(config.target_config.get(key) is False for key in ("sbc", "sla", "daa")),
                    "Protected target requires a separately validated loader/authentication path")
            mtk.daloader.patch = False
            # Direct unpatched DA upload; never DaHandler.configure_da.
            require(mtk.daloader.upload_da(preloader=None), "Approved DA upload failed")
            require(not mtk.daloader.patch, "Unexpected download-agent patching")
            readflash = mtk.daloader.readflash
            read_failed = False
            def bounded_read(**kwargs):
                nonlocal read_failed
                require(not read_failed, "MTK read session failed; start a new session")
                try:
                    with bounded_operation(10):
                        return readflash(**kwargs)
                except BaseException:
                    read_failed = True
                    raise
            mtk.daloader.readflash = bounded_read
            return mtk

    def close(self, reset=False):
        require(reset is False, "USB reset is disabled")
        errors = []
        if self.device is not None:
            for number in reversed(self.interfaces):
                try:
                    self.usb.util.release_interface(self.device, number)
                except Exception as error:
                    errors.append(error)
            for number in reversed(self.detached):
                try:
                    self.device.attach_kernel_driver(number)
                except Exception as error:
                    errors.append(error)
            try:
                self.usb.util.dispose_resources(self.device)
            except Exception as error:
                errors.append(error)
        self.interfaces, self.detached, self.device = [], [], None
        if self.work is not None:
            self.work.cleanup()
            self.work = None
        if self.imports is not None:
            self.imports.close()
            self.imports = None
        if errors:
            raise InstallError("USB cleanup failed; disconnect the cable before another session") from errors[0]
