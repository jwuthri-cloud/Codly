"""Turn KayKit Character Animations (CC0, Kay Lousberg) clips into our characters' joints.

    python3 -I scripts/kaykit-clips.py KayKit_AnimatedCharacter_v1.2.glb > src/office/gfx/clips.json

KayKit's rig is rigid parts like ours (Body, Head, armLeft, armRight; no legs), so each clip
maps onto joints of gfx/motion.ts: body bob/pitch/yaw/roll, head pitch/yaw/roll and each arm's
pitch and out. Angles are relative to KayKit's BasePose (arms down), our relaxed stance.
"""

import json
import math
import struct
import sys

# KayKit name → ours, and the frames a pose loops (a matching pair: raise and lower play only as a reaction).
CLIPS = {"Cheer": ("cheer", 17, 35), "Wave": ("wave", 15, 25), "Dance": ("dance", 0, 50)}
FPS = 30
# Our neck height over KayKit's (body bob scale).
BOB = 0.32 / 0.712


def qmul(a, b):
    ax, ay, az, aw = a
    bx, by, bz, bw = b
    return (aw * bx + ax * bw + ay * bz - az * by, aw * by - ax * bz + ay * bw + az * bx, aw * bz + ax * by - ay * bx + az * bw, aw * bw - ax * bx - ay * by - az * bz)


def qinv(q):
    return (-q[0], -q[1], -q[2], q[3])


def rot(q, v):
    x, y, z, _ = qmul(qmul(q, (v[0], v[1], v[2], 0)), qinv(q))
    return (x, y, z)


def slerp(a, b, t):
    d = sum(x * y for x, y in zip(a, b))
    if d < 0:
        b, d = tuple(-x for x in b), -d
    if d > 0.9995:
        r = tuple(x + (y - x) * t for x, y in zip(a, b))
    else:
        th = math.acos(d)
        s = math.sin(th)
        r = tuple((math.sin((1 - t) * th) * x + math.sin(t * th) * y) / s for x, y in zip(a, b))
    n = math.sqrt(sum(x * x for x in r))
    return tuple(x / n for x in r)


def euler_xyz(q):
    """three.js Euler 'XYZ' from a quaternion."""
    x, y, z, w = q
    m11, m12, m13 = 1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)
    m22, m23, m33 = 1 - 2 * (x * x + z * z), 2 * (y * z - x * w), 1 - 2 * (x * x + y * y)
    ey = math.asin(max(-1, min(1, m13)))
    if abs(m13) < 0.9999999:
        return math.atan2(-m23, m33), ey, math.atan2(-m12, m11)
    return math.atan2(2 * (y * z + x * w), m22), ey, 0


def main(path):
    b = open(path, "rb").read()
    n = struct.unpack("<I", b[12:16])[0]
    j = json.loads(b[20 : 20 + n])
    binary = b[20 + n + 8 :]

    def read(i):
        a = j["accessors"][i]
        v = j["bufferViews"][a["bufferView"]]
        k = {"SCALAR": 1, "VEC3": 3, "VEC4": 4}[a["type"]]
        off = v.get("byteOffset", 0) + a.get("byteOffset", 0)
        flat = struct.unpack_from("<%df" % (a["count"] * k), binary, off)
        return [flat[i : i + k] if k > 1 else flat[i] for i in range(0, len(flat), k)]

    names = {i: nd.get("name") for i, nd in enumerate(j["nodes"])}

    def tracks(anim):
        out = {}
        for c in anim["channels"]:
            s = anim["samplers"][c["sampler"]]
            out[(names[c["target"]["node"]], c["target"]["path"])] = (read(s["input"]), read(s["output"]))
        return out

    def sample(tr, key, t):
        times, vals = tr[key]
        if t <= times[0] or len(times) == 1:
            return vals[0]
        for i in range(1, len(times)):
            if t <= times[i]:
                f = (t - times[i - 1]) / (times[i] - times[i - 1])
                a, c = vals[i - 1], vals[i]
                return slerp(a, c, f) if len(a) == 4 else tuple(x + (y - x) * f for x, y in zip(a, c))
        return vals[-1]

    anims = {a["name"]: a for a in j["animations"]}
    rest = tracks(anims["BasePose"])
    r = {k: sample(rest, (k, "rotation"), 0) for k in ("Body", "Head", "armLeft", "armRight")}
    y0 = sample(rest, ("Body", "translation"), 0)[1]

    def arm(q, q0):
        # The arm's turn from rest, in the body's frame, applied to our arm hanging down.
        d = qmul(q, qinv(q0))
        vx, vy, vz = rot(d, (0, -1, 0))
        return math.atan2(-vz, -vy), math.asin(max(-1, min(1, vx)))

    out = {}
    for src, (dst, a0, a1) in CLIPS.items():
        tr = tracks(anims[src])
        dur = max(max(times) for times, _ in tr.values())
        ch = {k: [] for k in (0, 1, 13, 20, 2, 3, 4, 5, 6, 8, 9)}
        for f in range(int(round(dur * FPS)) + 1):
            t = f / FPS
            bq = qmul(sample(tr, ("Body", "rotation"), t), qinv(r["Body"]))
            ch[0].append((sample(tr, ("Body", "translation"), t)[1] - y0) * BOB)
            ch[1], ch[13], ch[20] = [c + [v] for c, v in zip((ch[1], ch[13], ch[20]), euler_xyz(bq))]
            hq = qmul(sample(tr, ("Head", "rotation"), t), qinv(r["Head"]))
            ch[2], ch[3], ch[4] = [c + [v] for c, v in zip((ch[2], ch[3], ch[4]), euler_xyz(hq))]
            a, o = arm(sample(tr, ("armLeft", "rotation"), t), r["armLeft"])
            ch[5].append(a)
            ch[6].append(o)
            a, o = arm(sample(tr, ("armRight", "rotation"), t), r["armRight"])
            ch[8].append(a)
            ch[9].append(-o)
        a1 = min(a1, len(ch[0]) - 1)
        for vs in ch.values():  # close the loop: spread any drift over it
            gap = vs[a1] - vs[a0]
            for i in range(a0, a1 + 1):
                vs[i] -= gap * (i - a0) / (a1 - a0)
        out[dst] = {"fps": FPS, "loop": [a0, a1], "j": {str(k): [round(v, 3) for v in vs] for k, vs in ch.items()}}
    json.dump(out, sys.stdout, separators=(",", ":"))


if __name__ == "__main__":
    main(sys.argv[1])
