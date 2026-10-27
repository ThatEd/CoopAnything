import sys
import struct

d = open(sys.argv[1], 'rb').read()
a, b, c, e = struct.unpack_from('<IIII', d, 0)
print('u32s:', hex(a), hex(b), hex(c), hex(e), 'filesize', hex(len(d)))
p = 16
n = 0
start = p
while n < a and p + 2 <= len(d):
    ln = struct.unpack_from('<H', d, p)[0]
    if ln == 0 or ln > 512 or p + 2 + ln > len(d):
        print('anomaly at', hex(p), 'len', ln)
        break
    p += 2 + ln
    n += 1
print('parsed', n, 'of', a, 'names, ended at', hex(p), 'table bytes', p - start)
print('next u32s:', [hex(x) for x in struct.unpack_from('<IIII', d, p)])
