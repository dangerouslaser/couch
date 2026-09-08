import struct, sys, hashlib
# Repack a boot image, swapping only the zImage. The device tree, ramdisk,
# load addresses, page size and cmdline are carried over verbatim from the
# known-good image so the only variable under test is our kernel.
src, newz, out = sys.argv[1], sys.argv[2], sys.argv[3]
d = open(src,'rb').read()
hdr = bytearray(d[:1024])
ks,ka,rs,ra,ss,sa,tags,pgsz,dtsz,uid = struct.unpack('<10I', d[8:48])
def pages(n): return (n+pgsz-1)//pgsz*pgsz
kern = d[pgsz:pgsz+ks]
rd   = d[pgsz+pages(ks):pgsz+pages(ks)+rs]
# Split stock kernel blob at the appended DTB.
i = kern.find(b'\xd0\x0d\xfe\xed', 6900000)
tot = struct.unpack('>I', kern[i+4:i+8])[0]
assert i+tot == len(kern), (i, tot, len(kern))
dtb = kern[i:]
z = open(newz,'rb').read()
assert z[0x24:0x28] == b'\x18\x28\x6f\x01', "not a zImage"
nk = z + dtb
print(f"stock zImage {i}  dtb {len(dtb)}  new zImage {len(z)}  new kernel {len(nk)}")
struct.pack_into('<I', hdr, 8, len(nk))
# id[8] is a SHA1 over the parts; the bootloader on this platform does not
# verify it, but keep it consistent rather than stale.
sha = hashlib.sha1()
for part, size in ((nk, len(nk)), (rd, len(rd)), (b'', 0)):
    sha.update(part); sha.update(struct.pack('<I', size))
struct.pack_into('<20s', hdr, 576, sha.digest())
img = bytes(hdr) + b'\0'*(pgsz-1024) + nk + b'\0'*(pages(len(nk))-len(nk)) + rd + b'\0'*(pages(len(rd))-len(rd))
open(out,'wb').write(img)
print("wrote", out, len(img), "bytes")
