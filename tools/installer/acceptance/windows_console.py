"""Bounded real Windows console test. The only permitted menu action is Cancel."""
import ctypes as C
from ctypes import wintypes as W
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time


def run(launcher, output):
    kernel = C.WinDLL('kernel32', use_last_error=True)
    class Coord(C.Structure):
        _fields_ = [('X', C.c_short), ('Y', C.c_short)]
    class Startup(C.Structure):
        _fields_ = [('cb', W.DWORD), ('reserved', W.LPWSTR), ('desktop', W.LPWSTR),
                    ('title', W.LPWSTR), ('x', W.DWORD), ('y', W.DWORD),
                    ('xs', W.DWORD), ('ys', W.DWORD), ('xc', W.DWORD), ('yc', W.DWORD),
                    ('fill', W.DWORD), ('flags', W.DWORD), ('show', W.WORD),
                    ('reserved_size', W.WORD), ('reserved_bytes', C.c_void_p),
                    ('stdin', W.HANDLE), ('stdout', W.HANDLE), ('stderr', W.HANDLE)]
    class Extended(C.Structure):
        _fields_ = [('startup', Startup), ('attributes', C.c_void_p)]
    class Process(C.Structure):
        _fields_ = [('process', W.HANDLE), ('thread', W.HANDLE), ('pid', W.DWORD), ('tid', W.DWORD)]
    def declare(name, result, *args):
        fn = getattr(kernel, name); fn.restype = result; fn.argtypes = args; return fn
    close = declare('CloseHandle', W.BOOL, W.HANDLE)
    pipe = declare('CreatePipe', W.BOOL, C.POINTER(W.HANDLE), C.POINTER(W.HANDLE), C.c_void_p, W.DWORD)
    pseudo = declare('CreatePseudoConsole', C.c_long, Coord, W.HANDLE, W.HANDLE, W.DWORD, C.POINTER(W.HANDLE))
    close_pseudo = declare('ClosePseudoConsole', None, W.HANDLE)
    init = declare('InitializeProcThreadAttributeList', W.BOOL, C.c_void_p, W.DWORD, W.DWORD, C.POINTER(C.c_size_t))
    update = declare('UpdateProcThreadAttribute', W.BOOL, C.c_void_p, W.DWORD, C.c_size_t, C.c_void_p, C.c_size_t, C.c_void_p, C.c_void_p)
    delete = declare('DeleteProcThreadAttributeList', None, C.c_void_p)
    create = declare('CreateProcessW', W.BOOL, W.LPCWSTR, W.LPWSTR, C.c_void_p, C.c_void_p, W.BOOL, W.DWORD, C.c_void_p, W.LPCWSTR, C.POINTER(Extended), C.POINTER(Process))
    read = declare('ReadFile', W.BOOL, W.HANDLE, C.c_void_p, W.DWORD, C.POINTER(W.DWORD), C.c_void_p)
    write = declare('WriteFile', W.BOOL, W.HANDLE, C.c_void_p, W.DWORD, C.POINTER(W.DWORD), C.c_void_p)
    wait = declare('WaitForSingleObject', W.DWORD, W.HANDLE, W.DWORD)
    exit_code = declare('GetExitCodeProcess', W.BOOL, W.HANDLE, C.POINTER(W.DWORD))
    def checked(ok):
        if not ok: raise C.WinError(C.get_last_error())
    incoming, writer, reader_handle, outgoing, console = [W.HANDLE() for _ in range(5)]
    checked(pipe(C.byref(incoming), C.byref(writer), None, 0))
    checked(pipe(C.byref(reader_handle), C.byref(outgoing), None, 0))
    process = Process(); attributes = None
    transcript = bytearray(); overflow = threading.Event()
    try:
        if pseudo(Coord(120, 40), incoming, outgoing, 0, C.byref(console)) != 0:
            raise RuntimeError('Could not create Windows pseudoconsole')
        size = C.c_size_t(); init(None, 1, 0, C.byref(size))
        attributes = C.create_string_buffer(size.value)
        checked(init(attributes, 1, 0, C.byref(size)))
        checked(update(attributes, 0, 0x20016, console, C.sizeof(console), None, None))
        startup = Extended(); startup.startup.cb = C.sizeof(startup)
        # Null standard handles + STARTF_USESTDHANDLES prevent the CI parent's
        # redirected handles from bypassing the pseudoconsole (Microsoft #15814).
        startup.startup.flags = 0x100
        startup.attributes = C.cast(attributes, C.c_void_p)
        command = C.create_unicode_buffer(subprocess.list2cmdline([
            str(Path(os.environ['SystemRoot'])/'System32/WindowsPowerShell/v1.0/powershell.exe'),
            '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', str(launcher)]))
        checked(create(None, command, None, None, False, 0x80000, None, str(launcher.parent), C.byref(startup), C.byref(process)))
        close(incoming); incoming = W.HANDLE(); close(outgoing); outgoing = W.HANDLE()
        def drain():
            buffer = C.create_string_buffer(8192); count = W.DWORD()
            while read(reader_handle, buffer, len(buffer), C.byref(count), None) and count.value:
                if len(transcript) + count.value > 4*1024*1024:
                    overflow.set()
                else:
                    transcript.extend(buffer.raw[:count.value])
        threading.Thread(target=drain, daemon=True).start()
        deadline = time.monotonic()+120; sent = False
        state = Path(os.environ['LOCALAPPDATA'])/'CouchInstaller'
        while wait(process.process, 50) == 258:
            if state.exists(): raise RuntimeError('Cancel smoke unexpectedly created an installer session')
            if overflow.is_set() or time.monotonic() > deadline:
                raise RuntimeError('Launcher output/deadline bound exceeded')
            if not sent and b'Reinstall existing Couch' in transcript and b'Cancel' in transcript:
                time.sleep(0.2)
                for key in (b'\x1b[B', b'\x1b[B', b'\x1b[B', b'\r'):
                    count = W.DWORD(); checked(write(writer, key, len(key), C.byref(count), None))
                    if count.value != len(key): raise RuntimeError('Short console input')
                    time.sleep(0.1)
                sent = True
        status = W.DWORD(); checked(exit_code(process.process, C.byref(status)))
        if not sent or status.value != 0 or state.exists():
            raise RuntimeError(f'Launcher did not safely cancel: sent={sent}, exit={status.value}')
        return {'cancel_selected': True, 'exit_code': status.value, 'session_created': False,
                'device_access': False, 'transport': 'ConPTY', 'launcher_modified': False}
    finally:
        if process.process and wait(process.process, 0) == 258:
            subprocess.run(['taskkill', '/PID', str(process.pid), '/T', '/F'], timeout=15, check=False, capture_output=True)
        if console: close_pseudo(console)
        for handle in (incoming, writer, reader_handle, outgoing, process.thread, process.process):
            if handle: close(handle)
        if attributes is not None: delete(attributes)
        output.write_bytes(transcript)


if __name__ == '__main__':
    result = run(Path(sys.argv[1]).resolve(), Path(sys.argv[2]))
    Path(sys.argv[3]).write_text(json.dumps(result, indent=2)+'\n')
