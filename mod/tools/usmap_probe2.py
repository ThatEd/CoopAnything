"""Parse the UE4SS Mappings.usmap in F1 Manager 24's Win64 folder.

The file is NOT a standard UnrealMappingsDumper .usmap: there is no 0xC263 magic and
name lengths are 1-byte prefixed. Layout found by parsing:

  u32 name_count
  name_count x (u8 len, len bytes ascii)
  u32 ?? ...
"""
import sys
import struct

d = open(sys.argv[1], 'rb').read()
p = 0


def u32():
    global p
    v = struct.unpack_from('<I', d, p)[0]
    p += 4
    return v


def i32():
    global p
    v = struct.unpack_from('<i', d, p)[0]
    p += 4
    return v


def u8():
    global p
    v = d[p]
    p += 1
    return v


names_n = u32()
names = []
for _ in range(names_n):
    ln = u8()
    names.append(d[p:p + ln].decode('utf-8', 'replace'))
    p += ln
print('names %d parsed, p=0x%X' % (len(names), p))
print('sample:', names[:3], names[-3:])
for i in range(8):
    save = p
    v = u32()
    print('  u32 @0x%X = %d (0x%X)' % (save, v, v))
