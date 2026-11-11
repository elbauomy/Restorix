#!/usr/bin/env python3
"""Build a small synthetic NTFS volume image containing deleted files, for testing the NTFS
scanner on machines that cannot format NTFS (macOS/Linux without ntfs-3g).

  make_ntfs_image.py <out.img> <payload-file>

Layout: 4 KiB clusters, 1 KiB MFT records. Contains:
  rec 5   root directory
  rec 6   $Bitmap
  rec 11  $Extend
  rec 30  $Extend/$UsnJrnl with a $J stream holding FILE_DELETE records
  rec 31  deleted folder "Photos"
  rec 32  deleted file  "Photos/beach.jpg" (non-resident, data = payload)
  rec 33  deleted file  "Photos/note.txt"  (resident)
  rec 34  live file     "keep.txt"
"""
import struct, sys

CL, REC = 4096, 1024
TOTAL_CL = 4096
MFT_LCN, MFT_RECS = 4, 64
BITMAP_LCN, USN_LCN, DATA_LCN = 20, 21, 100

def ft(unix):  # unix -> FILETIME
    return (unix + 11644473600) * 10_000_000

T_CREATE, T_DELETE = 1_700_000_000, 1_746_403_200  # 2023-11-14, 2025-05-05

def resident(atype, value, name="", aid=0):
    nm = name.encode("utf-16-le")
    voff = (0x18 + len(nm) + 7) & ~7
    alen = (voff + len(value) + 7) & ~7
    h = struct.pack("<IIBBHHH", atype, alen, 0, len(name), 0x18, 0, aid) + struct.pack("<IHBB", len(value), voff, 0, 0)
    b = h + nm
    b += b"\0" * (voff - len(b)) + value
    return b + b"\0" * (alen - len(b))

def runs(lst):
    out = b""
    for lcn, n, prev in lst:
        ln = n.to_bytes(4, "little").rstrip(b"\0") or b"\0"
        if lcn is None:
            out += bytes([len(ln)]) + ln
        else:
            d = lcn - prev
            ob = d.to_bytes(4, "little", signed=True)
            out += bytes([len(ob) << 4 | len(ln)]) + ln + ob
    return out + b"\0"

def nonresident(atype, rl, real, clusters, name="", aid=0):
    nm = name.encode("utf-16-le")
    roff = (0x40 + len(nm) + 7) & ~7
    alen = (roff + len(rl) + 7) & ~7
    h = struct.pack("<IIBBHHH", atype, alen, 1, len(name), 0x40, 0, aid)
    h += struct.pack("<QQHHI", 0, clusters - 1, roff, 0, 0)
    h += struct.pack("<QQQ", clusters * CL, real, real)
    b = h + nm
    b += b"\0" * (roff - len(b)) + rl
    return b + b"\0" * (alen - len(b))

def std_info():
    return resident(0x10, struct.pack("<QQQQ", ft(T_CREATE), ft(T_CREATE + 60), ft(T_CREATE + 60), ft(T_CREATE + 60)) + b"\0" * 40)

def file_name(parent, pseq, name, size=0):
    nm = name.encode("utf-16-le")
    v = struct.pack("<Q", parent | pseq << 48) + struct.pack("<QQQQ", *[ft(T_CREATE)] * 4)
    v += struct.pack("<QQII", size, size, 0, 0) + bytes([len(name), 1]) + nm
    return resident(0x30, v)

def record(seq, flags, attrs):
    body = b"".join(attrs) + struct.pack("<I", 0xFFFFFFFF) + b"\0" * 4
    hdr = b"FILE" + struct.pack("<HHQHHHHII", 0x30, 3, 0, seq, 1, 0x38, flags, 0x38 + len(body), REC) + struct.pack("<QHH", 0, 10, 0)
    hdr += b"\0" * (0x30 - len(hdr))
    usa = struct.pack("<HHH", 1, 0, 0)  # usn=1, then original bytes
    r = bytearray((hdr + usa).ljust(0x38, b"\0") + body)
    r = r.ljust(REC, b"\0")
    for i in (1, 2):  # fixup: save last 2 bytes of each sector, replace with usn
        pos = i * 512 - 2
        r[0x30 + 2 * i:0x30 + 2 * i + 2] = r[pos:pos + 2]
        r[pos:pos + 2] = struct.pack("<H", 1)
    return bytes(r)

def usn_rec(rec, seq, reason, name, t):
    nm = name.encode("utf-16-le")
    ln = (60 + len(nm) + 7) & ~7
    b = struct.pack("<IHHQQQQIIIIHH", ln, 2, 0, rec | seq << 48, 5 | 1 << 48, 0, ft(t), reason, 0, 0, 0x20, len(nm), 60) + nm
    return b.ljust(ln, b"\0")

def main():
    out, payload_path = sys.argv[1], sys.argv[2]
    payload = open(payload_path, "rb").read()
    img = bytearray(TOTAL_CL * CL)
    bs = bytearray(512)
    bs[0:11] = b"\xEB\x52\x90NTFS    "
    struct.pack_into("<HB", bs, 0x0B, 512, CL // 512)
    struct.pack_into("<B", bs, 0x15, 0xF8)
    struct.pack_into("<QQQ", bs, 0x28, TOTAL_CL * CL // 512 - 1, MFT_LCN, 2)
    struct.pack_into("<b", bs, 0x40, -10)
    struct.pack_into("<b", bs, 0x44, 1)
    bs[510:512] = b"\x55\xAA"
    img[0:512] = bs

    pay_cl = (len(payload) + CL - 1) // CL
    recs = {}
    recs[0] = record(1, 1, [std_info(), file_name(5, 5, "$MFT"), nonresident(0x80, runs([(MFT_LCN, MFT_RECS * REC // CL, 0)]), MFT_RECS * REC, MFT_RECS * REC // CL)])
    recs[5] = record(5, 3, [std_info(), file_name(5, 5, ".")])
    recs[6] = record(6, 1, [std_info(), file_name(5, 5, "$Bitmap"), nonresident(0x80, runs([(BITMAP_LCN, 1, 0)]), TOTAL_CL // 8, 1)])
    recs[11] = record(11, 3, [std_info(), file_name(5, 5, "$Extend")])
    # $J: 2 sparse clusters, then 1 real cluster of records
    journal = usn_rec(32, 1, 0x200, "beach.jpg", T_DELETE) + usn_rec(33, 1, 0x200 | 0x80000000, "note.txt", T_DELETE + 5)
    jr = runs([(None, 2, 0), (USN_LCN, 1, 0)])
    recs[30] = record(1, 1, [std_info(), file_name(11, 11, "$UsnJrnl"), nonresident(0x80, jr, 2 * CL + len(journal), 3, name="$J")])
    recs[31] = record(2, 2, [std_info(), file_name(5, 5, "Photos")])                  # deleted dir (seq bumped)
    recs[32] = record(2, 0, [std_info(), file_name(31, 1, "beach.jpg", len(payload)),
                             nonresident(0x80, runs([(DATA_LCN, pay_cl, 0)]), len(payload), pay_cl)])
    recs[33] = record(2, 0, [std_info(), file_name(31, 1, "note.txt", 26), resident(0x80, b"Deleted note: hello NTFS!\n")])
    recs[34] = record(1, 1, [std_info(), file_name(5, 5, "keep.txt", 5), resident(0x80, b"live\n")])
    for i in range(MFT_RECS):
        r = recs.get(i, b"\0" * REC)
        o = MFT_LCN * CL + i * REC
        img[o:o + REC] = r

    bitmap = bytearray(TOTAL_CL // 8)
    for c in range(0, USN_LCN + 1):
        bitmap[c // 8] |= 1 << (c % 8)
    img[BITMAP_LCN * CL:BITMAP_LCN * CL + len(bitmap)] = bitmap
    img[USN_LCN * CL:USN_LCN * CL + len(journal)] = journal
    img[DATA_LCN * CL:DATA_LCN * CL + len(payload)] = payload
    open(out, "wb").write(img)
    print(f"wrote {out}")

main()
