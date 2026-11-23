"""F20: one PNG of 1 to 3 MB as an image block on stream-json.

The PNG is made here with zlib + struct: RGB noise from random.Random(20) (so it does not compress), 800x800 (about
1.9 MB) and 1000x1000 (about 3.0 MB). The image is never written to disk; the sanitizer removes the base64 text.
"""
import base64, random, struct, zlib
from drv import Session


def noise_png(w, h, seed):
    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
    rnd = random.Random(seed)
    raw = b"".join(b"\x00" + rnd.randbytes(3 * w) for _ in range(h))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 1)) + chunk(b"IEND", b"")


s = Session("r4-big-image", extra=["--tools", ""], hard_timeout=150)
n = 0
for w in (800, 1000):
    png = noise_png(w, w, 20)
    data = base64.b64encode(png).decode()
    s.note(image={"width": w, "height": w, "png_bytes": len(png), "base64_chars": len(data)})
    print("png %dx%d: %d bytes, base64 %d chars" % (w, w, len(png), len(data)))
    s.user([{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": data}},
            {"type": "text", "text": "Is this picture a photo or random noise? One word."}])
    n += 1
    print("result:", s.wait_results(n, 90))
print("cost:", s.cost(), "exit:", s.close())
