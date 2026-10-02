// Geometry helpers shared by the views: the staircase grid is rotated 45° in cell space.
import type { Design } from './types';

/** World (cell) position of grid cell corner (u, v) in grid units. */
export function gcToXY(u: number, v: number): [number, number] {
  return [(u + v) / 2, (u - v) / 2];
}

/** The four world-space corners of a run of blocks [i0, j0, i1, j1]. */
export function runCorners(run: [number, number, number, number] | number[], G: number): [number, number][] {
  const [i0, j0, i1, j1] = run;
  const u0 = (i0 - 0.5) * G;
  const u1 = (i1 + 0.5) * G;
  const v0 = (j0 - 0.5) * G;
  const v1 = (j1 + 0.5) * G;
  return [
    gcToXY(u0, v0),
    gcToXY(u1, v0),
    gcToXY(u1, v1),
    gcToXY(u0, v1),
  ];
}

/** Bounding box in world coordinates of everything a group (and its sub-groups) occupies. */
export function groupBox(d: Design, g: number): [number, number, number, number] | null {
  const reg = d.regions.find((r) => r.group === g);
  if (!reg || !d.grid) return null;
  let x0 = Infinity,
    y0 = Infinity,
    x1 = -Infinity,
    y1 = -Infinity;
  for (const run of reg.runs) {
    for (const [x, y] of runCorners(run, d.grid)) {
      x0 = Math.min(x0, x);
      y0 = Math.min(y0, y);
      x1 = Math.max(x1, x);
      y1 = Math.max(y1, y);
    }
  }
  return [x0, y0, x1, y1];
}

/** Is `g` inside `anc` (or equal)? */
export function within(d: Design, g: number, anc: number): boolean {
  let cur: number | null | undefined = g;
  for (let k = 0; k < 64 && cur !== null && cur !== undefined; k++) {
    if (cur === anc) return true;
    cur = d.groups[cur]?.parent;
  }
  return false;
}
