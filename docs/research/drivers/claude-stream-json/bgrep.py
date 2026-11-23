"""Find a literal in the CLI binary and print context: python3 bgrep.py <literal> [before] [after] [max]

Used for the statements marked "bundle, not recorded". The binary is $CLAUDE_BIN or the claude on PATH (links resolved).
"""
import mmap, os, shutil, sys
B = os.path.realpath(os.environ.get("CLAUDE_BIN") or shutil.which("claude") or "claude")
pat = sys.argv[1].encode()
before = int(sys.argv[2]) if len(sys.argv) > 2 else 100
after = int(sys.argv[3]) if len(sys.argv) > 3 else 400
mx = int(sys.argv[4]) if len(sys.argv) > 4 else 5
with open(B, "rb") as f:
    m = mmap.mmap(f.fileno(), 0, access=mmap.ACCESS_READ)
    i, n, seen = 0, 0, set()
    while n < mx:
        i = m.find(pat, i)
        if i < 0:
            break
        s = m[max(0, i - before):i + len(pat) + after]
        txt = "".join(chr(c) if 32 <= c < 127 else "." for c in s)
        if txt not in seen:
            seen.add(txt)
            print("@%d: %s\n" % (i, txt))
            n += 1
        i += len(pat)
