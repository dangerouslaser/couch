#!/usr/bin/env python3
"""Send a command to the remote's USB serial shell and print what comes back."""
import os, sys, termios, time, select, glob

def port():
    p = sorted(glob.glob("/dev/cu.usbmodem*"))
    if not p: sys.exit("no /dev/cu.usbmodem* present")
    return p[0]

# Commands that sleep go quiet mid-run, so the idle "settle" window has to be
# longer than the longest sleep or we cut the reply off. SER_SETTLE raises it.
def run(cmd, settle=float(os.environ.get("SER_SETTLE", 1.5)),
        timeout=float(os.environ.get("SER_TIMEOUT", 6.0))):
    fd = os.open(port(), os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
    try:
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
