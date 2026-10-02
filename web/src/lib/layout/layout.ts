// Dockable panes: the layout model, presets, and the geometry of every dock, pane and handle.
export type PaneId = 'code' | 'checks' | 'inspector';
export type Side = 'left' | 'right' | 'bottom';
export const PANE_IDS: PaneId[] = ['code', 'checks', 'inspector'];
export const SIDES: Side[] = ['left', 'right', 'bottom'];

export interface Layout {
  left: { panes: PaneId[]; size: number };
  right: { panes: PaneId[]; size: number };
  bottom: { panes: PaneId[]; size: number };
  hidden: PaneId[];
  w: Record<PaneId, number>;
}

export const PANES: Record<PaneId, { label: string; icon: 'file-code' | 'list-checks' | 'panel-right'; key: string }> = {
  code: { label: 'Code', icon: 'file-code', key: '⌘B' },
  checks: { label: 'Problems & tests', icon: 'list-checks', key: '⌘J' },
  inspector: { label: 'Inspector', icon: 'panel-right', key: '⌘I' },
};

export const PRESETS: { id: string; name: string; key: string; layout: Layout }[] = [
  { id: 'default', name: 'Default', key: '⌥1', layout: { left: { panes: ['code', 'checks'], size: 460 }, right: { panes: ['inspector'], size: 280 }, bottom: { panes: [], size: 190 }, hidden: [], w: { code: 3, checks: 1, inspector: 1 } } },
  { id: 'write', name: 'Write', key: '⌥2', layout: { left: { panes: ['code'], size: 620 }, right: { panes: ['inspector'], size: 280 }, bottom: { panes: ['checks'], size: 170 }, hidden: ['inspector'], w: { code: 1, checks: 1, inspector: 1 } } },
  { id: 'explore', name: 'Explore', key: '⌥3', layout: { left: { panes: ['code', 'checks'], size: 460 }, right: { panes: ['inspector'], size: 300 }, bottom: { panes: [], size: 190 }, hidden: ['code', 'checks'], w: { code: 3, checks: 1, inspector: 1 } } },
  { id: 'focus', name: 'Focus', key: '⌥4', layout: { left: { panes: ['code', 'checks'], size: 460 }, right: { panes: ['inspector'], size: 280 }, bottom: { panes: [], size: 190 }, hidden: ['code', 'checks', 'inspector'], w: { code: 3, checks: 1, inspector: 1 } } },
];

export const clone = <T>(o: T): T => JSON.parse(JSON.stringify(o));

/** Canonical form for comparing layouts with presets. */
function canon(L: Layout) {
  return JSON.stringify({ left: L.left, right: L.right, bottom: L.bottom, hidden: [...L.hidden].sort(), w: { code: L.w.code, checks: L.w.checks, inspector: L.w.inspector } });
}
export function presetOf(L: Layout) {
  const c = canon(L);
  return PRESETS.find((p) => canon(p.layout) === c) ?? null;
}

/** A layout read from storage, repaired so that every pane sits in exactly one dock. */
export function sanitize(x: unknown): Layout | null {
  try {
    const L = x as Layout;
    const seen = new Set<PaneId>();
    for (const s of SIDES) {
      if (!L[s] || !Array.isArray(L[s].panes) || typeof L[s].size !== 'number') return null;
      L[s].panes = L[s].panes.filter((p) => PANE_IDS.includes(p) && !seen.has(p) && (seen.add(p), true));
    }
    for (const p of PANE_IDS) if (!seen.has(p)) L.left.panes.push(p);
    L.hidden = (L.hidden ?? []).filter((p) => PANE_IDS.includes(p));
    L.w = Object.assign({ code: 1, checks: 1, inspector: 1 }, L.w ?? {});
    return L;
  } catch {
    return null;
  }
}

export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}
export interface Split {
  box: Box;
  vertical: boolean;
  a: PaneId;
  b: PaneId;
  /** Length of the dock along the stacking direction (px) and the sum of its weights. */
  len: number;
  sum: number;
}
export interface Geometry {
  floating: boolean;
  gap: number;
  docks: { side: Side; box: Box }[];
  panes: Partial<Record<PaneId, Box & { side: Side }>>;
  splits: Split[];
  edges: { side: Side; box: Box; size: number }[];
  /** Space taken by the docks (incl. gaps) on each side. */
  cl: number;
  cr: number;
  cb: number;
  /** The view area (docked) / the canvas area (floating). */
  view: Box;
  transport: Box;
  centerW: number;
}

export const HEADER = 48;
export const TRANSPORT = 56;

export function geometry(L: Layout, floating: boolean, VW: number, VH: number): Geometry {
  const vis = (id: PaneId) => !L.hidden.includes(id);
  const lp = L.left.panes.filter(vis);
  const rp = L.right.panes.filter(vis);
  const bp = L.bottom.panes.filter(vis);
  const g = floating ? 10 : 0;
  let LW = lp.length ? Math.min(L.left.size, Math.round(VW * 0.4)) : 0;
  let RW = rp.length ? Math.min(L.right.size, Math.round(VW * 0.28)) : 0;
  const over = LW + RW + 2 * g * ((LW ? 1 : 0) + (RW ? 1 : 0)) + 380 - VW;
  if (over > 0) {
    const tot = LW + RW || 1;
    LW = LW ? Math.max(220, Math.round(LW - over * (LW / tot))) : 0;
    RW = RW ? Math.max(200, Math.round(RW - over * (RW / tot))) : 0;
  }
  const BH = bp.length ? Math.min(L.bottom.size, Math.max(110, VH - HEADER - 200)) : 0;
  const cl = LW ? LW + 2 * g : 0;
  const cr = RW ? RW + 2 * g : 0;
  const cb = BH ? BH + 2 * g : 0;
  const top = HEADER + g;
  const hs = floating ? 10 : 5;
  const geo: Geometry = { floating, gap: g, docks: [], panes: {}, splits: [], edges: [], cl, cr, cb, view: { x: 0, y: 0, w: 0, h: 0 }, transport: { x: 0, y: 0, w: 0, h: 0 }, centerW: floating ? VW : VW - cl - cr };
  const stack = (side: Side, list: PaneId[], box: Box, vertical: boolean) => {
    if (!list.length) return;
    geo.docks.push({ side, box });
    const len = (vertical ? box.h : box.w) - hs * (list.length - 1);
    const sum = list.reduce((a, id) => a + (L.w[id] || 1), 0);
    let pos = vertical ? box.y : box.x;
    list.forEach((id, i) => {
      if (i > 0) {
        const hb = vertical ? { x: box.x, y: pos, w: box.w, h: hs } : { x: pos, y: box.y, w: hs, h: box.h };
        geo.splits.push({ box: hb, vertical, a: list[i - 1], b: id, len: vertical ? box.h : box.w, sum });
        pos += hs;
      }
      const sz = Math.round((len * (L.w[id] || 1)) / sum);
      const size = i === list.length - 1 ? (vertical ? box.y + box.h : box.x + box.w) - pos : sz;
      geo.panes[id] = vertical ? { x: box.x, y: pos, w: box.w, h: size, side } : { x: pos, y: box.y, w: size, h: box.h, side };
      pos += size;
    });
  };
  stack('left', lp, { x: g, y: top, w: LW, h: VH - top - g }, true);
  stack('right', rp, { x: VW - g - RW, y: top, w: RW, h: VH - top - g }, true);
  const bx = cl || g;
  stack('bottom', bp, { x: bx, y: VH - g - BH, w: VW - bx - (cr || g), h: BH }, false);
  if (LW) geo.edges.push({ side: 'left', box: { x: g + LW - 3, y: top, w: 6, h: VH - top - g }, size: LW });
  if (RW) geo.edges.push({ side: 'right', box: { x: VW - g - RW - 3, y: top, w: 6, h: VH - top - g }, size: RW });
  if (BH) geo.edges.push({ side: 'bottom', box: { x: bx, y: VH - g - BH - 3, w: VW - bx - (cr || g), h: 6 }, size: L.bottom.size });
  if (floating) {
    geo.view = { x: cl || 10, y: HEADER + 10, w: VW - (cl || 10) - (cr || 10), h: VH - HEADER - 10 - ((cb || 10) + 76) };
    const tw = Math.min(VW - cl - cr - 20, 900);
    const cx = cl + (VW - cl - cr) / 2;
    geo.transport = { x: cx - tw / 2, y: VH - (cb || 10) - 6 - 54, w: tw, h: 54 };
  } else {
    geo.view = { x: cl, y: HEADER, w: VW - cl - cr, h: VH - HEADER - cb - TRANSPORT };
    geo.transport = { x: cl, y: VH - cb - TRANSPORT, w: VW - cl - cr, h: TRANSPORT };
  }
  return geo;
}

export function zoneAt(x: number, y: number, W: number, H: number): Side | null {
  if (y < HEADER) return null;
  if (x < W * 0.28) return 'left';
  if (x > W * 0.72) return 'right';
  if (y > H * 0.58) return 'bottom';
  return null;
}
