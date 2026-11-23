"""Quieter timeline: python3 tlq.py <scenario> [maxlen]. Uuids are shortened to 4 characters."""
import json, re, sys, io, contextlib
import tl
buf = io.StringIO()
with contextlib.redirect_stdout(buf):
    tl.main(sys.argv[1], int(sys.argv[2]) if len(sys.argv) > 2 else 300, skip=("thinking_tokens", "commands_changed"))
for line in buf.getvalue().splitlines():
    if "rate_limit_event" in line or '"argv"' in line:
        continue
    line = re.sub(r"([0-9a-f]{4})[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", r"\1", line)
    line = re.sub(r"toolu_\w{18}(\w{4})", r"tu_\1", line)
    print(line[:int(sys.argv[2]) + 40 if len(sys.argv) > 2 else 340])
