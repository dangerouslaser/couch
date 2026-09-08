#!/usr/bin/env python3
"""Use Ollie's USB serial connection (SER_HOST=local for a local cable)."""
import os, sys, termios, time, select, glob, fcntl, shlex, subprocess
from pathlib import Path

def port():
    explicit = os.environ.get("SER_PORT")
    if explicit:
        return explicit
    p = sorted(glob.glob("/dev/serial/by-id/usb-Android_Android_*-if00"))
    if not p:
        p = sorted(glob.glob("/dev/ttyACM*") + glob.glob("/dev/cu.usbmodem*"))
    if len(p) != 1:
        sys.exit(f"expected one serial remote, found {len(p)}; set SER_PORT explicitly")
    return p[0]

# Commands that sleep go quiet mid-run, so the idle "settle" window has to be
# longer than the longest sleep or we cut the reply off. SER_SETTLE raises it.
def run(cmd, settle=float(os.environ.get("SER_SETTLE", 1.5)),
        timeout=float(os.environ.get("SER_TIMEOUT", 6.0))):
    host = os.environ.get("SER_HOST", "ollie")
    if host != "local":
        env = ["SER_HOST=local", f"SER_SETTLE={settle}", f"SER_TIMEOUT={timeout}"]
        if os.environ.get("SER_PORT"):
            env.append(f"SER_PORT={os.environ['SER_PORT']}")
        remote = shlex.join(["env", *env, "python3", "-", cmd])
        result = subprocess.run(
            ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", host, remote],
            input=Path(__file__).read_text(), text=True, capture_output=True, check=True,
        )
        return result.stdout
    fd = os.open(port(), os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
    try:
        # Lock the actual device inode, including when callers use a by-id alias.
        # Another sercmd reader must not consume this command's response.
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        a = termios.tcgetattr(fd)
        a[0] = a[1] = a[3] = 0                       # iflag oflag lflag: raw
        a[2] = termios.CS8 | termios.CREAD | termios.CLOCAL
        a[4] = a[5] = termios.B115200
        a[6][termios.VMIN] = 0
        a[6][termios.VTIME] = 0
        termios.tcsetattr(fd, termios.TCSANOW, a)
        termios.tcflush(fd, termios.TCIOFLUSH)

        os.write(fd, (cmd + "\n").encode())
        out, deadline, last = b"", time.time() + timeout, time.time()
        while time.time() < deadline:
            r, _, _ = select.select([fd], [], [], 0.2)
            if r:
                try:
                    chunk = os.read(fd, 4096)
                except BlockingIOError:
                    continue
                if chunk:
                    out += chunk
                    last = time.time()
            elif out and time.time() - last > settle:
                break
        return out.decode("utf-8", "replace")
    finally:
        os.close(fd)

if __name__ == "__main__":
    print(run(" ".join(sys.argv[1:]) if len(sys.argv) > 1 else "uname -a"), end="")
