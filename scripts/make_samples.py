#!/usr/bin/env python3
"""Generate sample files (PNG, JPEG via sips, MP4, text) used to build test disk images."""
import os, struct, sys, zlib, random, subprocess

out = sys.argv[1]
os.makedirs(out, exist_ok=True)
random.seed(7)

def png(path, w, h):
    raw = b"".join(b"\x00" + bytes(random.getrandbits(8) for _ in range(w * 3)) for _ in range(h))
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")
    open(path, "wb").write(data)

def mp4(path, payload):
    ftyp = b"isom" + struct.pack(">I", 512) + b"isomiso2mp41"
    ftyp = struct.pack(">I", 8 + len(ftyp)) + b"ftyp" + ftyp
    mdat = struct.pack(">I", 8 + len(payload)) + b"mdat" + payload
    # mvhd v0: creation time 2024-03-01 (seconds since 1904)
    t = 1709251200 + 2082844800
    mvhd_body = b"\x00\x00\x00\x00" + struct.pack(">IIII", t, t, 1000, 5000) + b"\x00" * 80
    mvhd = struct.pack(">I", 8 + len(mvhd_body)) + b"mvhd" + mvhd_body
    moov = struct.pack(">I", 8 + len(mvhd)) + b"moov" + mvhd
    open(path, "wb").write(ftyp + mdat + moov)

png(os.path.join(out, "holiday.png"), 320, 240)
png(os.path.join(out, "tmp_big.png"), 900, 700)
subprocess.run(["sips", "-s", "format", "jpeg", os.path.join(out, "tmp_big.png"), "--out", os.path.join(out, "beach photo.jpg")], capture_output=True, check=True)
subprocess.run(["sips", "-s", "format", "heic", os.path.join(out, "tmp_big.png"), "--out", os.path.join(out, "IMG_0042.heic")], capture_output=True)
os.remove(os.path.join(out, "tmp_big.png"))
mp4(os.path.join(out, "clip.mp4"), bytes(random.getrandbits(8) for _ in range(300_000)))
open(os.path.join(out, "notes.txt"), "w").write("Meeting notes\n" * 400)
for f in sorted(os.listdir(out)):
    print(f, os.path.getsize(os.path.join(out, f)))
