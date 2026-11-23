"""List-price cost of the recordings: per raw file, the sum over sessions of the last total_cost_usd (the CLI's running
total per session id).  python3 cost.py [raw dir]"""
import glob, json, os, sys
OUT = os.path.abspath(os.environ.get("CAPTURE_DIR") or os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
total = 0.0
for p in sorted(glob.glob(os.path.join(sys.argv[1] if len(sys.argv) > 1 else os.path.join(OUT, "raw"), "*.ndjson"))):
    last = {}
    for l in open(p):
        r = json.loads(l)
        m = r["msg"]
        if r["dir"] == "recv" and isinstance(m, dict) and m.get("type") == "result":
            last[m.get("session_id")] = m.get("total_cost_usd") or 0.0
    c = sum(last.values())
    total += c
    print("%-55s %.5f" % (os.path.basename(p), c))
print("%-55s %.5f" % ("TOTAL", total))
