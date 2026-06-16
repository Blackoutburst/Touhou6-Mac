#!/usr/bin/env python3
"""
Extract files from a PC-98 Anex86 .hdi hard-disk image (FAT12/16, Shift-JIS names).

Used to pull the original Touhou PC-98 game data out of the .hdi disk images that
ship with the Neko Project 21 emulator bundle, so the formats can be reverse-engineered
for a native Rust reimplementation (the TH04 port — see docs/TH4_PORTING.md).

Brings nothing copyrighted into the repo: output is gitignored.

Usage:
    python3 tools/pc98_hdi_extract.py path/to/th04j.hdi [outdir]

Notes on the format:
  - Anex86 HDI = 4096-byte header (geometry) followed by the raw disk image.
  - PC-98 disks boot via an IPL ("IPL1" signature at byte 4 of the disk) and place
    the DOS FAT partition's boot sector on a later cylinder. We locate it by scanning
    cylinder boundaries for the "FAT1" filesystem-type string at boot-sector offset +54.
  - Sector size is often 1024 bytes on PC-98 (not 512); we read it from the BPB.
"""
import struct, os, sys


def read_hdi(path):
    raw = open(path, "rb").read()
    # Anex86 header: <reserved, hddtype, headersize, hddsize, sectorsize, spt, heads, cyls>
    _, _, hsize, _, _, spt, heads, _ = struct.unpack("<8I", raw[:32])
    disk = raw[hsize:]
    cyl_bytes = spt * heads * 512
    return disk, cyl_bytes


def find_fat_boot(disk, cyl_bytes):
    for cyl in range(0, 16):
        o = cyl * cyl_bytes
        if disk[o + 54:o + 57] == b"FAT":
            return o
    raise SystemExit("No FAT boot sector found (not a DOS-formatted PC-98 disk?)")


def extract(img, outdir):
    disk, cyl_bytes = read_hdi(img)
    part = find_fat_boot(disk, cyl_bytes)
    bs = disk[part:part + 1024]
    bps = struct.unpack("<H", bs[11:13])[0]
    spc = bs[13]
    resv = struct.unpack("<H", bs[14:16])[0]
    nfat = bs[16]
    root_ent = struct.unpack("<H", bs[17:19])[0]
    spf = struct.unpack("<H", bs[22:24])[0]

    fat_start = part + resv * bps
    root_start = fat_start + nfat * spf * bps
    data_start = root_start + root_ent * 32
    fat = disk[fat_start:fat_start + spf * bps]

    def fat12(n):
        i = n + n // 2
        v = fat[i] | (fat[i + 1] << 8)
        return (v >> 4) if (n & 1) else (v & 0xFFF)

    def chain(start, size):
        out = bytearray()
        c = start
        while 2 <= c < 0xFF8:
            o = data_start + (c - 2) * spc * bps
            out += disk[o:o + spc * bps]
            c = fat12(c)
        return bytes(out[:size]) if size else bytes(out)

    def listdir(db, path=""):
        r = []
        for i in range(0, len(db), 32):
            e = db[i:i + 32]
            if e[0] == 0:
                break
            if e[0] == 0xE5 or (e[11] & 0x0F) == 0x0F or (e[11] & 0x08):
                continue
            stem, ext = e[0:8].rstrip(b" "), e[8:11].rstrip(b" ")
            nm = stem + (b"." + ext if ext else b"")
            try:
                name = nm.decode("shift-jis")
            except UnicodeDecodeError:
                name = nm.decode("latin1")
            r.append((path + name, bool(e[11] & 0x10),
                      struct.unpack("<H", e[26:28])[0],
                      struct.unpack("<I", e[28:32])[0]))
        return r

    files = []

    def walk(entries):
        for nm, isdir, cl, sz in entries:
            if nm.split("/")[-1] in (".", ".."):
                continue
            if isdir:
                walk(listdir(chain(cl, 0), nm + "/"))
            else:
                files.append((nm, cl, sz))

    walk(listdir(disk[root_start:data_start]))
    os.makedirs(outdir, exist_ok=True)
    for nm, cl, sz in files:
        open(os.path.join(outdir, nm.replace("/", "__")), "wb").write(chain(cl, sz))
    return files


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    img = sys.argv[1]
    out = sys.argv[2] if len(sys.argv) > 2 else "_extracted_" + os.path.splitext(os.path.basename(img))[0]
    fs = extract(img, out)
    for nm, _, sz in sorted(fs, key=lambda x: -x[2]):
        print(f"{sz:>9}  {nm}")
    print(f"\n{len(fs)} files -> {out}/")
