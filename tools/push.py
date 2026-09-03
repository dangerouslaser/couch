#!/usr/bin/env python3
"""Push a local file to the device over the USB serial shell.

No adb, no reflash, no trip through Android. The port is opened once and held
for the whole transfer - reopening it per chunk turns seconds into minutes.

  tools/push.py stage2/stage2.sh /mnt/alpine/opt/couch/stage2.sh
"""
import base64, hashlib, os, select, sys, termios, time, glob

CHUNK = 512   # keep a line well inside the tty buffer


def open_port():
    ports = sorted(glob.glob("/dev/cu.usbmodem*"))
    if not ports:
        sys.exit("no /dev/cu.usbmodem* - is it booted into Linux?")
    fd = os.open(ports[0], os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
    a = termios.tcgetattr(fd)
    a[0] = a[1] = a[3] = 0
    a[2] = termios.CS8 | termios.CREAD | termios.CLOCAL
    a[4] = a[5] = termios.B115200
    a[6][termios.VMIN] = 0
    a[6][termios.VTIME] = 0
    termios.tcsetattr(fd, termios.TCSANOW, a)
    termios.tcflush(fd, termios.TCIOFLUSH)
    return fd


def cmd(fd, s, wait=0.2, timeout=25.0, sentinel=None):
    os.write(fd, (s + "\n").encode())
    out, deadline, last = b"", time.time() + timeout, time.time()
    while time.time() < deadline:
        if sentinel and sentinel.encode() in out.split(b"echo")[-1]:
            break
        r, _, _ = select.select([fd], [], [], 0.1)
        if r:
            try:
                c = os.read(fd, 8192)
            except BlockingIOError:
                continue
            if c:
                out += c
                last = time.time()
        elif not sentinel and out and time.time() - last > wait:
            break
    return out.decode("utf-8", "replace")


def push(local, remote):
    data = open(local, "rb").read()
    b64 = base64.b64encode(data).decode()
    total = (len(b64) + CHUNK - 1) // CHUNK
    fd = open_port()
    try:
        cmd(fd, f"mkdir -p $(dirname {remote}); rm -f {remote}.b64")
        t0 = time.time()
        for i in range(total):
            # Wait for a per-chunk sentinel rather than guessing a settle time.
            # Guessing interleaves the next command into the middle of a chunk
            # and silently corrupts the file.
            tag = f"K{i}Z"
            line = f"printf '%s' '{b64[i*CHUNK:(i+1)*CHUNK]}' >> {remote}.b64; echo {tag}"
            for attempt in range(3):
                out = cmd(fd, line, sentinel=tag, timeout=15)
                if tag in out:
                    break
            else:
                sys.exit(f"\nchunk {i} never acknowledged - transfer aborted")
            if (i + 1) % 20 == 0 or i + 1 == total:
                kb = (i + 1) * CHUNK * 3 / 4 / 1024
                print(f"  {i+1}/{total} chunks  {kb/max(time.time()-t0,0.01):.1f} KB/s",
                      end="\r", flush=True)
        print()
        out = cmd(fd, f"base64 -d {remote}.b64 > {remote} && rm -f {remote}.b64 "
                      f"&& chmod 755 {remote} && md5sum {remote}", timeout=60)
        want = hashlib.md5(data).hexdigest()
        print("MATCH" if want in out else f"MISMATCH (wanted {want})\n{out.strip()}")
    finally:
        os.close(fd)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    push(sys.argv[1], sys.argv[2])
