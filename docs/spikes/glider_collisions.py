#!/usr/bin/env python3
"""Planning spike: characterize 90-degree glider/glider interactions in Conway's Life.

Throwaway, dependency-free reference for the numbers quoted in docs/PLAN.md section 2.6.
The real characterizer will live in Rust (crates/goldl-tech) and must reproduce these results.

Setup: glider A moves SE (+1,+1), glider B moves SW (-1,+1); y grows downwards.
For two perpendicular lanes, the only free parameters are:
  dt     = arrival time of A at the lane intersection minus arrival time of B (generations)
  parity = whether the two lanes intersect on a cell (0) or between cells (1)

Run: python3 docs/spikes/glider_collisions.py   (takes ~10 s)
"""
from collections import Counter


def step(cells):
    cnt = Counter()
    for (x, y) in cells:
        for dx in (-1, 0, 1):
            for dy in (-1, 0, 1):
                if dx or dy:
                    cnt[(x + dx, y + dy)] += 1
    return frozenset(c for c, n in cnt.items() if n == 3 or (n == 2 and c in cells))


def run(cells, n):
    for _ in range(n):
        cells = step(cells)
    return cells


def shift(cells, dx, dy):
    return frozenset((x + dx, y + dy) for x, y in cells)


SE = frozenset({(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)})  # moves (+1,+1) every 4 gens
SW = frozenset({(1, 0), (0, 1), (0, 2), (1, 2), (2, 2)})  # moves (-1,+1) every 4 gens


def first_contact_and_end(a, b, limit=900):
    """Generation of first deviation from independent evolution, and generation the joint pattern dies (if it does)."""
    s, ia, ib = a | b, a, b
    first = None
    for t in range(1, limit):
        s, ia, ib = step(s), step(ia), step(ib)
        if first is None and s != (ia | ib):
            first = t
        if first is not None and not s:
            return first, t, s
    return first, None, s


def perpendicular_table(dist=24):
    table = {}
    for dy in range(-14, 15):
        for dx in (dist, dist + 1):
            for k in range(4):
                dt = 4 * dy + k          # exact: B is SW advanced k gens, placed at (dx, dy)
                parity = (dx + dy) % 2
                first, end, rest = first_contact_and_end(SE, shift(run(SW, k), dx, dy))
                if first is None:
                    cls = "pass"
                elif end is not None:
                    cls = "vanish (+%d gens after contact)" % (end - first)
                else:
                    cls = "debris (pop %d)" % len(rest)
                table[(dt, parity)] = cls
    return table


def parallel_min_spacing():
    def interacts(a, b, steps=24):
        s, ia, ib = a | b, a, b
        for _ in range(steps):
            s, ia, ib = step(s), step(ia), step(ib)
            if s != (ia | ib):
                return True
        return False

    for sep in range(0, 12):
        if not any(interacts(SE, shift(run(SE, k), along + sep, along))
                   for k in range(4) for along in range(-6, 7)):
            return sep
    return None


if __name__ == "__main__":
    t = perpendicular_table()
    inter = {k: v for k, v in t.items() if v != "pass"}
    print("perpendicular (dt, parity) classes examined:", len(t), "interacting:", len(inter))
    print("interaction window: |dt| <=", max(abs(k[0]) for k in inter), "-> any |dt| beyond that passes cleanly")
    for k in sorted(k for k, v in inter.items() if v.startswith("vanish")):
        print("  clean annihilation at dt=%+d parity=%d: %s" % (k[0], k[1], t[k]))
    print("minimum safe spacing for parallel same-direction lanes (x-y units):", parallel_min_spacing())
