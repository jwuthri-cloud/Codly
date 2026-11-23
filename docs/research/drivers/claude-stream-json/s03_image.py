"""Scenario 3: a user message with a base64 PNG image block (16x16 solid red)."""
import base64, struct, zlib
from drv import Session

def png(w, h, rgb):
    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
    row = b"\x00" + bytes(rgb) * w
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(row * h)) + chunk(b"IEND", b"")

data = base64.b64encode(png(16, 16, (255, 0, 0))).decode()
print("png base64:", data)
s = Session("image-input", extra=["--tools", ""], hard_timeout=90)
s.user([{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": data}},
        {"type": "text", "text": "What colour is this square? One word."}])
print("result:", s.wait_results(1, 60))
# control: a blue square, so the answer cannot be a guess
data2 = base64.b64encode(png(16, 16, (0, 0, 255))).decode()
s.user([{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": data2}},
        {"type": "text", "text": "And this one? One word."}])
print("result:", s.wait_results(2, 60))
# broken image: what does a rejected image look like?
s.user([{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "aGVsbG8gd29ybGQ="}},
        {"type": "text", "text": "And this one? One word."}])
print("result:", s.wait_results(3, 60))
s.user("Reply with the single word: alive")
print("result:", s.wait_results(4, 60))
print("exit:", s.close())
