<script lang="ts">
  // The Game of Life universe: WebGL cells, components, glider paths and gliders, with an
  // abstraction overlay (design hierarchy down to single gates) and the physical I/O pins.
  import { onMount } from 'svelte';
  import { app, settings, period, cycleAt, isDark, ui, groupLabel, groupTitle } from '../state.svelte';
  import { engine } from '../engine';
  import { advance } from '../timing.svelte';
  import { Batch, Renderer, type Camera } from './renderer';
  import { oklch, hex, css, type RGB } from '../color';
  import { runCorners, within } from '../geom';
  import Icon from '../ui/Icon.svelte';
  import Switch from '../ui/Switch.svelte';
  import type { Design } from '../types';

  let { insets = { l: 0, r: 0, t: 0, b: 0 } }: { insets?: { l: number; r: number; t: number; b: number } } = $props();

  let wrap: HTMLDivElement;
  let canvas: HTMLCanvasElement;
  let overlay: HTMLCanvasElement;
  let renderer: Renderer | null = null;
  let glError = $state('');

  let cam: Camera = { cx: 0, cy: 0, zoom: 0.001 };
  let zoomLabel = $state('');
  let canvasW = $state(800);
  let tip: { name: string; kind: string; x: number; y: number } | null = $state(null);

  let comps: Int32Array | null = null;
  let wires: Float64Array | null = null;
  let data: { cells: Int32Array | null; gliders: Float64Array | null; active: Int32Array | null } = { cells: null, gliders: null, active: null };
  let lastKey = '';
  let inflight = false;
  let designRef: Design | null = null;
  /** Port values in the current cycle (for the pins). */
  let ioVals: { inputs: bigint[]; outputs: bigint[]; regs: bigint[] } = { inputs: [], outputs: [], regs: [] };

  const KIND_NAMES = ['Snark', 'reflector', 'Syringe', 'eater'];

  interface Pal {
    bg: RGB;
    cell: RGB;
    kinds: RGB[];
    row: RGB;
    col: RGB;
    loop: RGB;
    laneA: number;
    loopA: number;
    g1: RGB;
    g2: RGB;
    active: RGB;
    additive: boolean;
  }
  function palette(): Pal {
    const dark = isDark();
    if (settings.palette === 'neon')
      return {
        bg: dark ? hex('#05070d') : hex('#f6f7fb'),
        cell: dark ? hex('#e9f7ff') : hex('#0d1424'),
        kinds: ['#7c6cff', '#3fa7ff', '#ffb347', '#ff5c7a'].map(hex),
        row: dark ? hex('#38e8c6') : hex('#0d9d84'),
        col: dark ? hex('#ff4fd8') : hex('#c2189b'),
        loop: dark ? hex('#e9f7ff') : hex('#0d1424'),
        laneA: 0.3,
        loopA: 0.5,
        g1: dark ? hex('#38e8c6') : hex('#0d9d84'),
        g2: dark ? hex('#ff4fd8') : hex('#c2189b'),
        active: hex('#9ff6ff'),
        additive: dark,
      };
    if (settings.palette === 'amber')
      return {
        bg: dark ? hex('#0b0703') : hex('#fbf4e6'),
        cell: dark ? hex('#ffd27a') : hex('#3a2200'),
        kinds: ['#c4772a', '#e09a3c', '#f2bd5a', '#b0502a'].map(hex),
        row: hex('#ffb347'),
        col: hex('#ffb347'),
        loop: hex('#ffd27a'),
        laneA: 0.25,
        loopA: 0.5,
        g1: dark ? hex('#ffd27a') : hex('#3a2200'),
        g2: dark ? hex('#ffd27a') : hex('#3a2200'),
        active: hex('#fff1c2'),
        additive: dark,
      };
    const ink = dark ? hex('#ffffff') : hex('#000000');
    return {
      bg: dark ? hex('#0d0d0f') : hex('#f6f6f5'),
      cell: dark ? hex('#f1f1f1') : hex('#161616'),
      kinds: dark
        ? [oklch(0.6, 0.07, 285), oklch(0.62, 0.07, 230), oklch(0.68, 0.07, 80), oklch(0.6, 0.08, 20)]
        : [oklch(0.55, 0.08, 285), oklch(0.56, 0.08, 230), oklch(0.62, 0.09, 75), oklch(0.55, 0.09, 20)],
      row: ink,
      col: ink,
      loop: ink,
      laneA: 0.2,
      loopA: dark ? 0.28 : 0.32,
      g1: dark ? hex('#ffffff') : hex('#111111'),
      g2: dark ? hex('#ffffff') : hex('#111111'),
      active: dark ? hex('#ffffff') : hex('#000000'),
      additive: false,
    };
  }
  /** Overlay colours from the CSS tokens. */
  function ovColors() {
    const cs = getComputedStyle(wrap);
    const dark = isDark();
    return {
      region: dark ? 'rgba(255,255,255,0.32)' : 'rgba(0,0,0,0.28)',
      regionFill: dark ? 'rgba(255,255,255,0.025)' : 'rgba(0,0,0,0.02)',
      hoverFill: dark ? 'rgba(255,255,255,0.06)' : 'rgba(0,0,0,0.05)',
      block: dark ? 'rgba(255,255,255,0.16)' : 'rgba(0,0,0,0.14)',
      accent: cs.getPropertyValue('--accent').trim(),
      accentSoft: cs.getPropertyValue('--accent-soft').trim(),
      raised: cs.getPropertyValue('--raised').trim(),
      fg: cs.getPropertyValue('--fg').trim(),
      dim: cs.getPropertyValue('--fg-dim').trim(),
      faint: cs.getPropertyValue('--fg-faint').trim(),
      border: cs.getPropertyValue('--border').trim(),
      sans: cs.getPropertyValue('--sans').trim() || 'system-ui',
      mono: cs.getPropertyValue('--mono').trim() || 'monospace',
    };
  }

  const W = () => wrap?.clientWidth ?? 800;
  const H = () => wrap?.clientHeight ?? 600;

  function viewRect(pad = 0.15) {
    const w = W() / cam.zoom;
    const h = H() / cam.zoom;
    return { x0: cam.cx - w * (0.5 + pad), y0: cam.cy - h * (0.5 + pad), x1: cam.cx + w * (0.5 + pad), y1: cam.cy + h * (0.5 + pad) };
  }

  /** Camera framing world box `b` inside the part of the canvas not covered by floating panes. */
  function camFor(b: [number, number, number, number], maxZoom = 24): Camera {
    const w = Math.max(10, b[2] - b[0]);
    const h = Math.max(10, b[3] - b[1]);
    const aw = Math.max(80, W() - insets.l - insets.r);
    const ah = Math.max(80, H() - insets.t - insets.b);
    const zoom = Math.min((aw * 0.9) / w, (ah * 0.9) / h, maxZoom);
    // Centre of the free area, in screen offsets from the canvas centre.
    const ox = (insets.l - insets.r) / 2;
    const oy = (insets.t - insets.b) / 2;
    return { cx: (b[0] + b[2]) / 2 - ox / zoom, cy: (b[1] + b[3]) / 2 - oy / zoom, zoom };
  }
  function animateTo(target: Camera) {
    const from = { ...cam };
    const t0 = performance.now();
    const step = () => {
      const t = Math.min(1, (performance.now() - t0) / 450);
      const e = t * t * (3 - 2 * t);
      const lz = Math.log(from.zoom) + (Math.log(target.zoom) - Math.log(from.zoom)) * e;
      const z = Math.exp(lz);
      // Interpolate the screen-space path so the motion feels straight at every scale.
      cam = { cx: from.cx + (target.cx - from.cx) * e, cy: from.cy + (target.cy - from.cy) * e, zoom: z };
      if (t < 1) requestAnimationFrame(step);
    };
    step();
  }
  function fit(b: [number, number, number, number], animate = true) {
    const target = camFor(b);
    if (animate) animateTo(target);
    else cam = target;
  }

  // ---- Static per-design data: block polygons ----
  interface BlockPoly {
    pts: [number, number][];
    box: [number, number, number, number];
    group: number;
    kind: string;
    origin: number;
  }
  let blockPolys: BlockPoly[] = [];
  /** A maximal run of consecutive staircase blocks inside one group (and its sub-groups). */
  interface Run {
    g: number;
    depth: number;
    idx: number[];
    box: [number, number, number, number];
    /** Outline in world units relative to box[0], box[1]. */
    path: Path2D;
    top: [number, number];
  }
  let runs: Run[] = [];
  function buildRuns(d: Design) {
    const open = new Map<number, Run>();
    runs = [];
    blockPolys.forEach((b, k) => {
      let g: number | null | undefined = b.group;
      for (let guard = 0; g !== null && g !== undefined && guard < 64; guard++) {
        const r = open.get(g);
        if (r && r.idx[r.idx.length - 1] === k - 1) r.idx.push(k);
        else {
          const nr: Run = { g, depth: groupDepth(d, g), idx: [k], box: [0, 0, 0, 0], path: new Path2D(), top: [0, 0] };
          open.set(g, nr);
          runs.push(nr);
        }
        g = d.groups[g]?.parent;
      }
    });
    for (const r of runs) {
      const bs = r.idx.map((k) => blockPolys[k]);
      r.box = [Math.min(...bs.map((b) => b.box[0])), Math.min(...bs.map((b) => b.box[1])), Math.max(...bs.map((b) => b.box[2])), Math.max(...bs.map((b) => b.box[3]))];
      const p = new Path2D();
      for (const b of bs) {
        b.pts.forEach(([x, y], i) => (i ? p.lineTo(x - r.box[0], y - r.box[1]) : p.moveTo(x - r.box[0], y - r.box[1])));
        p.closePath();
      }
      r.path = p;
      let top: [number, number] = [0, Infinity];
      for (const b of bs) for (const q of b.pts) if (q[1] < top[1]) top = q;
      r.top = top;
    }
  }
  function buildBlocks(d: Design) {
    blockPolys = [];
    const G = d.grid ?? 128;
    const B = d.blocks?.data ?? [];
    for (let k = 0; k < B.length; k += 7) {
      const pts = runCorners([B[k], B[k + 1], B[k + 2], B[k + 3]], G);
      const xs = pts.map((p) => p[0]);
      const ys = pts.map((p) => p[1]);
      blockPolys.push({ pts, box: [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)], group: B[k + 4], kind: d.blocks.kinds[B[k + 5]] ?? '', origin: B[k + 6] });
    }
    buildRuns(d);
  }

  // New design: load components and frame it.
  $effect(() => {
    const d = app.design;
    if (d === designRef) return;
    designRef = d;
    comps = null;
    wires = null;
    data = { cells: null, gliders: null, active: null };
    lastKey = '';
    if (!d?.bbox) return;
    buildBlocks(d);
    engine.components().then((c) => {
      if (designRef === d) comps = c;
    });
    engine.wires().then((w) => {
      if (designRef === d) wires = w;
    });
    if (wrap) fit(d.bbox, false);
  });

  $effect(() => {
    const f = app.frame;
    if (f && wrap) {
      fit(f);
      app.frame = null;
    }
  });

  // Port values for the pins, per cycle.
  $effect(() => {
    const d = app.design;
    const c = cycleAt(Math.floor(app.gen));
    void app.simVersion;
    if (!d) return;
    engine.trace(c, c + 1, []).then((rows) => {
      const r = rows[0];
      if (!r || designRef !== d) return;
      ioVals = { inputs: r.inputs.map(BigInt), outputs: r.outputs.map(BigInt), regs: r.regs.map(BigInt) };
    });
  });

  function groupDepth(d: Design, g: number) {
    let k = 0;
    let p = d.groups[g]?.parent;
    while (p !== null && p !== undefined && k < 64) {
      k++;
      p = d.groups[p]?.parent;
    }
    return k;
  }

  function toScreen(x: number, y: number): [number, number] {
    return [(x - cam.cx) * cam.zoom + W() / 2, (y - cam.cy) * cam.zoom + H() / 2];
  }
  function toWorld(sx: number, sy: number): [number, number] {
    return [(sx - W() / 2) / cam.zoom + cam.cx, (sy - H() / 2) / cam.zoom + cam.cy];
  }

  interface ShownRegion {
    g: number;
    run: Run;
    size: number;
    depth: number;
  }
  /** Regions worth drawing at the current zoom (sorted shallow to deep). */
  function visibleRegions(d: Design): ShownRegion[] {
    const out: ShownRegion[] = [];
    const w = W();
    const h = H();
    const vr = viewRect(0);
    for (const r of runs) {
      if (r.depth === 0 && d.groups.length > 1) continue;
      if (r.box[2] < vr.x0 || r.box[0] > vr.x1 || r.box[3] < vr.y0 || r.box[1] > vr.y1) continue;
      const size = Math.max(r.box[2] - r.box[0], r.box[3] - r.box[1]) * cam.zoom;
      if (size < 28 || size > 40 * Math.max(w, h)) continue;
      out.push({ g: r.g, run: r, size, depth: r.depth });
    }
    return out.sort((a, b) => a.depth - b.depth);
  }
  /** Is world point (x, y) inside one of the run's blocks? */
  function inRun(r: Run, x: number, y: number) {
    if (x < r.box[0] || x > r.box[2] || y < r.box[1] || y > r.box[3]) return false;
    return r.idx.some((k) => {
      const b = blockPolys[k];
      return x >= b.box[0] && x <= b.box[2] && y >= b.box[1] && y <= b.box[3] && inPoly(x, y, b.pts);
    });
  }
  /** Fill and stroke a run's outline in world space. */
  function strokeRun(ctx: CanvasRenderingContext2D, r: Run, dpr: number, fill: string, stroke: string, width: number, dash: number[]) {
    const z = cam.zoom;
    ctx.save();
    ctx.setTransform(dpr * z, 0, 0, dpr * z, dpr * ((r.box[0] - cam.cx) * z + W() / 2), dpr * ((r.box[1] - cam.cy) * z + H() / 2));
    ctx.fillStyle = fill;
    ctx.fill(r.path);
    ctx.lineWidth = width / z;
    ctx.strokeStyle = stroke;
    ctx.setLineDash(dash.map((x) => x / z));
    ctx.stroke(r.path);
    ctx.restore();
  }

  interface ShownBlock {
    b: BlockPoly;
    pts: [number, number][];
    size: number;
  }
  /** Single blocks (gates, fan-outs, delays) once they are large on screen. */
  function visibleBlocks(): ShownBlock[] {
    const out: ShownBlock[] = [];
    const r = viewRect(0);
    for (const b of blockPolys) {
      if (b.box[2] < r.x0 || b.box[0] > r.x1 || b.box[3] < r.y0 || b.box[1] > r.y1) continue;
      const size = (b.box[2] - b.box[0]) * cam.zoom;
      if (size < 70) continue;
      out.push({ b, pts: b.pts.map(([x, y]) => toScreen(x, y)), size });
      if (out.length > 400) break;
    }
    return out;
  }

  let hoverGroup = -1;

  function poly(ctx: CanvasRenderingContext2D, pts: [number, number][]) {
    ctx.beginPath();
    pts.forEach(([x, y], k) => (k ? ctx.lineTo(x, y) : ctx.moveTo(x, y)));
    ctx.closePath();
  }
  function pill(ctx: CanvasRenderingContext2D, x: number, y: number, text: string, font: string, bg: string, fg: string, border?: string, align: 'center' | 'left' | 'right' = 'center') {
    ctx.font = font;
    const tw = ctx.measureText(text).width;
    const w = tw + 12;
    const h = 18;
    const x0 = align === 'center' ? x - w / 2 : align === 'left' ? x : x - w;
    ctx.beginPath();
    ctx.roundRect(x0, y - h / 2, w, h, 5);
    ctx.fillStyle = bg;
    ctx.fill();
    if (border) {
      ctx.strokeStyle = border;
      ctx.lineWidth = 1;
      ctx.stroke();
    }
    ctx.fillStyle = fg;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.fillText(text, x0 + 6, y + 0.5);
    return [x0, y - h / 2, x0 + w, y + h / 2] as const;
  }

  function drawOverlay() {
    const ctx = overlay.getContext('2d')!;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = W();
    const h = H();
    if (overlay.width !== Math.round(w * dpr) || overlay.height !== Math.round(h * dpr)) {
      overlay.width = Math.round(w * dpr);
      overlay.height = Math.round(h * dpr);
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    const d = app.design;
    if (!d) return;
    const C = ovColors();
    const labels: (readonly [number, number, number, number])[] = [];
    const free = (r: readonly [number, number, number, number]) => !labels.some((l) => r[0] < l[2] && r[2] > l[0] && r[1] < l[3] && r[3] > l[1]);
    if (settings.overlay) {
      const regs = visibleRegions(d);
      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      for (const r of regs) {
        const sel = app.selectedGroup === r.g;
        strokeRun(ctx, r.run, dpr, sel ? C.accentSoft : hoverGroup === r.g ? C.hoverFill : C.regionFill, sel ? C.accent : C.region, sel ? 1.5 : 1, sel ? [] : [5, 4]);
      }
      // Gate-level detail.
      const blocks = visibleBlocks();
      for (const sb of blocks) {
        poly(ctx, sb.pts);
        const sel = app.selectedGroup !== null && within(d, sb.b.group, app.selectedGroup);
        ctx.strokeStyle = sel ? C.accent : C.block;
        ctx.lineWidth = 1;
        ctx.stroke();
      }
      if (settings.overlayLabels) {
        // Deepest labels win the space; draw them first.
        // One label per group: on its largest visible run.
        const best = new Map<number, ShownRegion>();
        for (const r of regs) if (!best.has(r.g) || best.get(r.g)!.size < r.size) best.set(r.g, r);
        for (const r of [...regs].reverse()) {
          if (r.size < 90 || best.get(r.g) !== r) continue;
          const top = toScreen(r.run.top[0], r.run.top[1]);
          const name = groupLabel(d, r.g);
          const text = name.length > 40 ? name.slice(0, 39) + '…' : name;
          ctx.font = `500 11px ${C.sans}`;
          const tw = ctx.measureText(text).width + 12;
          const rect = [top[0] - tw / 2, top[1] - 26, top[0] + tw / 2, top[1] - 8] as const;
          if (!free(rect)) continue;
          labels.push(pill(ctx, top[0], top[1] - 17, text, `500 11px ${C.sans}`, C.raised, app.selectedGroup === r.g ? C.accent : C.fg, C.border));
        }
        for (const sb of blocks) {
          if (sb.size < 110) continue;
          const bottom = sb.pts.reduce((a, b) => (b[1] > a[1] ? b : a));
          const op = sb.b.origin >= 0 ? d.rtlOps[sb.b.origin] : '';
          const text = op ? `${sb.b.kind} · ${op}` : sb.b.kind;
          ctx.font = `11px ${C.sans}`;
          const tw = ctx.measureText(text).width + 12;
          const rect = [bottom[0] - tw / 2, bottom[1] + 4, bottom[0] + tw / 2, bottom[1] + 22] as const;
          if (!free(rect)) continue;
          labels.push(pill(ctx, bottom[0], bottom[1] + 13, text, `11px ${C.sans}`, C.raised, C.dim, C.border));
        }
      }
    }
    if (settings.pins) drawPins(ctx, d, C, free, labels);
  }

  function bitOf(v: bigint | undefined, b: number) {
    return v !== undefined && ((v >> BigInt(b)) & 1n) === 1n;
  }
  function drawPins(ctx: CanvasRenderingContext2D, d: Design, C: ReturnType<typeof ovColors>, free: (r: readonly [number, number, number, number]) => boolean, labels: (readonly [number, number, number, number])[]) {
    const w = W();
    const h = H();
    const showRegs = cam.zoom > 0.02;
    // Bits of one port whose pads sit close together share a single label.
    const clusters = new Map<string, { xs: number[]; ys: number[]; last: [number, number, number, number] }>();
    for (const p of d.pins ?? []) {
      if (p.kind !== 'in' && p.kind !== 'out') continue;
      const [sx, sy] = toScreen(p.x, p.y);
      const k = `${p.kind}:${p.port}`;
      const c = clusters.get(k) ?? { xs: [], ys: [], last: [sx, sy, p.dir[0], p.dir[1]] };
      c.xs.push(sx);
      c.ys.push(sy);
      if (p.kind === 'out' ? sx + sy > c.last[0] + c.last[1] : sx + sy < c.last[0] + c.last[1]) c.last = [sx, sy, p.dir[0], p.dir[1]];
      clusters.set(k, c);
    }
    const merged = new Set<string>();
    for (const [k, c] of clusters) {
      if (c.xs.length < 2) continue;
      const spread = Math.max(Math.max(...c.xs) - Math.min(...c.xs), Math.max(...c.ys) - Math.min(...c.ys));
      if (spread < 24 * c.xs.length) merged.add(k);
    }
    const fmtPort = (v: bigint | undefined, w: number) => (v === undefined ? '?' : w <= 4 ? String(v) : '0x' + v.toString(16));
    for (const k of merged) {
      const [kind, port] = k.split(':');
      const c = clusters.get(k)!;
      const pi = Number(port);
      const info = kind === 'in' ? d.inputs[pi] : d.outputs[pi];
      const v = kind === 'in' ? ioVals.inputs[pi] : ioVals.outputs[pi];
      const [sx, sy, dx, dy] = c.last;
      const n = Math.hypot(dx, dy) || 1;
      const side = kind === 'out' ? 1 : -1;
      const lx = sx + side * (dx / n) * 22;
      const ly = sy + side * (dy / n) * 22;
      const text = `${info?.name ?? '?'}[${(info?.width ?? 1) - 1}:0] = ${fmtPort(v, info?.width ?? 1)}`;
      const font = `500 11px ${C.mono}`;
      ctx.font = font;
      const tw = ctx.measureText(text).width + 12;
      const align = lx < sx ? 'right' : 'left';
      // Ports leaving at the same place stack their labels.
      for (let k = 0; k < 6; k++) {
        const y = ly + k * 22;
        const x0 = align === 'right' ? lx - tw : lx;
        const rect = [x0, y - 9, x0 + tw, y + 9] as const;
        if (!free(rect)) continue;
        labels.push(pill(ctx, lx, y, text, font, C.raised, C.fg, C.border, align));
        break;
      }
    }
    for (const p of d.pins ?? []) {
      if (p.kind === 'one' && cam.zoom < 0.05) continue;
      if ((p.kind === 'reg-q' || p.kind === 'reg-d') && !showRegs) continue;
      const [sx, sy] = toScreen(p.x, p.y);
      if (sx < -40 || sy < -40 || sx > w + 40 || sy > h + 40) continue;
      const width = p.kind === 'in' ? d.inputs[p.port]?.width : p.kind === 'out' ? d.outputs[p.port]?.width : p.kind === 'one' ? 1 : d.regs[p.port]?.width;
      // The constant gun is not an RTL register: it always holds 1.
      const gun = (p.kind === 'reg-q' || p.kind === 'reg-d') && p.port >= d.regs.length;
      const val = p.kind === 'in' ? bitOf(ioVals.inputs[p.port], p.bit) : p.kind === 'out' ? bitOf(ioVals.outputs[p.port], p.bit) : p.kind === 'one' || gun ? true : bitOf(ioVals.regs[p.port], p.bit);
      const io = p.kind === 'in' || p.kind === 'out';
      // The pad: a rotated square on the glider lane.
      const r = io ? 6 : 4;
      ctx.save();
      ctx.translate(sx, sy);
      ctx.rotate(Math.PI / 4);
      ctx.beginPath();
      ctx.rect(-r, -r, 2 * r, 2 * r);
      ctx.fillStyle = val ? C.accent : C.raised;
      ctx.fill();
      ctx.lineWidth = 1.5;
      ctx.strokeStyle = val ? C.accent : io ? C.fg : C.dim;
      ctx.stroke();
      ctx.restore();
      // Direction tick: the stream leaves the pad along `dir`.
      const [dx, dy] = p.dir;
      const n = Math.hypot(dx, dy) || 1;
      ctx.beginPath();
      ctx.moveTo(sx + (dx / n) * (r + 3), sy + (dy / n) * (r + 3));
      ctx.lineTo(sx + (dx / n) * (r + 11), sy + (dy / n) * (r + 11));
      ctx.strokeStyle = io ? C.fg : C.dim;
      ctx.lineWidth = 1.5;
      ctx.stroke();
      if ((!io && cam.zoom < 0.08) || merged.has(`${p.kind}:${p.port}`)) continue;
      const name = p.kind === 'one' ? '1' : gun ? 'constant gun' : width && width > 1 ? `${p.name}[${p.bit}]` : p.name;
      const text = `${name}${p.inv ? ' (¬)' : ''}${p.kind === 'reg-q' ? ' Q' : p.kind === 'reg-d' ? ' D' : ''} = ${val ? 1 : 0}`;
      // Label upstream of an input, downstream of an output.
      const side = p.kind === 'out' || p.kind === 'reg-d' ? 1 : -1;
      const lx = sx + side * (dx / n) * (r + 16);
      const ly = sy + side * (dy / n) * (r + 16);
      const font = `500 11px ${C.mono}`;
      ctx.font = font;
      const tw = ctx.measureText(text).width + 12;
      const align = lx < sx ? 'right' : 'left';
      const x0 = align === 'right' ? lx - tw : lx;
      const rect = [x0, ly - 9, x0 + tw, ly + 9] as const;
      if (!free(rect)) continue;
      labels.push(pill(ctx, lx, ly, text, font, val ? C.accent : C.raised, val ? (isDark() ? '#0c1220' : '#ffffff') : io ? C.fg : C.dim, val ? undefined : C.border, align));
    }
  }

  // ---- Data and drawing ----
  const cellsB = new Batch(1 << 14);
  const compB = new Batch(1 << 14);
  const glowB = new Batch(1 << 12);
  const actB = new Batch(1 << 10);
  const wireB = new Batch(1 << 14);

  function fetchData() {
    const d = app.design;
    if (!d || !d.bbox || inflight) return;
    const g = Math.floor(app.gen);
    const r = viewRect();
    const cellsMode = cam.zoom >= 0.35;
    const key = `${g}|${Math.round(r.x0)}|${Math.round(r.y0)}|${Math.round(r.x1)}|${Math.round(r.y1)}|${cellsMode}|${app.simVersion}`;
    if (key === lastKey) return;
    lastKey = key;
    inflight = true;
    const reqs: Promise<unknown>[] = [engine.gliders(g, r, 60000)];
    if (cellsMode) reqs.push(engine.cells(g, r, 700000));
    else reqs.push(Promise.resolve(null));
    reqs.push(cam.zoom > 0.003 && !cellsMode && settings.glow ? engine.active(g, r) : Promise.resolve(null));
    Promise.all(reqs)
      .then(([gl, cells, act]) => {
        data = { gliders: gl as Float64Array, cells: cells as Int32Array | null, active: act as Int32Array | null };
      })
      .catch(() => {})
      .finally(() => (inflight = false));
  }

  function draw() {
    if (!renderer) return;
    const pal = palette();
    renderer.resize();
    renderer.begin(pal.bg);
    const cx = cam.cx;
    const cy = cam.cy;
    const z = cam.zoom;
    const cellsVisible = data.cells && data.cells[0] >= 0 && z >= 0.35;
    if (app.design) {
      // Glider paths: the circuit's wiring (lanes, and the register loops around it).
      if (wires && settings.wires) {
        wireB.reset();
        const n = wires[0];
        const r = viewRect(0.05);
        // Thousands of lanes overlap at far zoom: keep them light.
        const k = cellsVisible ? 0.5 : Math.min(1, Math.max(0.3, z * 300));
        for (let i = 0; i < n; i++) {
          const o = 1 + 6 * i;
          const x0 = wires[o];
          const y0 = wires[o + 1];
          const x1 = wires[o + 2];
          const y1 = wires[o + 3];
          if (Math.max(x0, x1) < r.x0 || Math.min(x0, x1) > r.x1 || Math.max(y0, y1) < r.y0 || Math.min(y0, y1) > r.y1) continue;
          const dir = wires[o + 4];
          const c = dir === 8 ? pal.row : dir === 2 ? pal.col : pal.loop;
          const a = (dir === 8 || dir === 2 ? pal.laneA : pal.loopA) * k;
          wireB.push(x0 - cx, y0 - cy, x1 - cx, y1 - cy, c[0], c[1], c[2], a);
        }
        renderer.drawSegments(wireB, cam, 1.4, false);
      }
      // Components (overview).
      if (comps && (!cellsVisible || z < 2.5) && settings.componentOutlines) {
        compB.reset();
        const n = comps[0];
        const a = cellsVisible ? 0.3 : 0.9;
        const r = viewRect(0.05);
        for (let k = 0; k < n; k++) {
          const o = 1 + 6 * k;
          const x0 = comps[o];
          const y0 = comps[o + 1];
          const x1 = comps[o + 2];
          const y1 = comps[o + 3];
          if (x1 < r.x0 || x0 > r.x1 || y1 < r.y0 || y0 > r.y1) continue;
          const c = pal.kinds[comps[o + 4]] ?? pal.cell;
          compB.push(x0 - cx, y0 - cy, x1 + 1 - cx, y1 + 1 - cy, c[0], c[1], c[2], a);
        }
        renderer.draw(compB, cam, cellsVisible ? 2 : 0, 1.2, false, 0.06);
      }
      // Reacting components (halo layer).
      if (data.active && comps && settings.glow && !cellsVisible) {
        actB.reset();
        const n = data.active[0];
        for (let k = 1; k <= n; k++) {
          const o = 1 + 6 * data.active[k];
          const pad = 40;
          const a = 0.45;
          actB.push(comps[o] - pad - cx, comps[o + 1] - pad - cy, comps[o + 2] + pad - cx, comps[o + 3] + pad - cy, pal.active[0], pal.active[1], pal.active[2], a);
        }
        renderer.draw(actB, cam, 1, 14, pal.additive);
      }
      // Cells.
      if (cellsVisible) {
        const c = data.cells!;
        cellsB.reset();
        const n = c[0];
        for (let k = 0; k < n; k++) {
          const x = c[1 + 2 * k] - cx;
          const y = c[2 + 2 * k] - cy;
          cellsB.push(x + 0.06, y + 0.06, x + 0.94, y + 0.94, pal.cell[0], pal.cell[1], pal.cell[2], 1);
        }
        renderer.draw(cellsB, cam, 0, z < 1 ? 1 : 0);
      }
      // Gliders: dots in the overview, an optional halo.
      if (data.gliders) {
        const gl = data.gliders;
        const n = gl[0];
        glowB.reset();
        const halo = settings.glow;
        const rad = halo ? Math.max(6 / z, 7) : Math.max(2.6 / z, 2.5);
        for (let k = 0; k < n; k++) {
          const o = 1 + 4 * k;
          const x = gl[o] - cx;
          const y = gl[o + 1] - cy;
          const dir = gl[o + 2];
          const col = dir === 2 ? pal.g2 : pal.g1;
          const a = cellsVisible ? (halo ? 0.35 : 0) : 0.95;
          if (a > 0) glowB.push(x - rad, y - rad, x + rad, y + rad, col[0], col[1], col[2], a);
        }
        renderer.draw(glowB, cam, halo ? 1 : 3, halo ? 9 : 5, halo && pal.additive);
      }
    }
    drawOverlay();
    zoomLabel = z >= 1 ? `${z.toFixed(z >= 10 ? 0 : 1)} px/cell` : `1 px : ${Math.round(1 / z).toLocaleString('en-US')} cells`;
  }

  let raf = 0;
  let lastT = 0;
  function frame(t: number) {
    const dt = lastT ? Math.min(0.1, (t - lastT) / 1000) : 0;
    lastT = t;
    advance(dt);
    fetchData();
    draw();
    raf = requestAnimationFrame(frame);
  }

  onMount(() => {
    try {
      renderer = new Renderer(canvas);
    } catch (e) {
      glError = String(e);
    }
    if (app.design?.bbox) fit(app.design.bbox, false);
    raf = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(raf);
  });

  // ---- Interaction ----
  function inPoly(x: number, y: number, pts: [number, number][]) {
    let inside = false;
    for (let i = 0, j = pts.length - 1; i < pts.length; j = i++) {
      const [xi, yi] = pts[i];
      const [xj, yj] = pts[j];
      if (yi > y !== yj > y && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) inside = !inside;
    }
    return inside;
  }
  /** What is under the pointer: the deepest region, or a single block when zoomed in. */
  function hit(sx: number, sy: number): { g: number; block?: BlockPoly } | null {
    const d = app.design;
    if (!d || !settings.overlay) return null;
    for (const sb of visibleBlocks()) if (inPoly(sx, sy, sb.pts)) return { g: sb.b.group, block: sb.b };
    const [wx, wy] = toWorld(sx, sy);
    const regs = visibleRegions(d).reverse();
    for (const r of regs) if (inRun(r.run, wx, wy)) return { g: r.g };
    return null;
  }

  let drag: { x: number; y: number; cx: number; cy: number; moved: boolean } | null = null;
  function onpointerdown(e: PointerEvent) {
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    drag = { x: e.clientX, y: e.clientY, cx: cam.cx, cy: cam.cy, moved: false };
    ui.menu = null;
  }
  function onpointermove(e: PointerEvent) {
    const rect = wrap.getBoundingClientRect();
    if (drag) {
      const dx = e.clientX - drag.x;
      const dy = e.clientY - drag.y;
      if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true;
      cam = { ...cam, cx: drag.cx - dx / cam.zoom, cy: drag.cy - dy / cam.zoom };
      tip = null;
      return;
    }
    const sx = e.clientX - rect.left;
    const sy = e.clientY - rect.top;
    const h = hit(sx, sy);
    const d = app.design;
    hoverGroup = h?.g ?? -1;
    if (!h || !d) {
      tip = null;
      return;
    }
    if (h.block) {
      const op = h.block.origin >= 0 ? d.rtlOps[h.block.origin] : '';
      tip = { name: `${groupTitle(d, h.g)} › ${h.block.kind}`, kind: op ? `part of a ${op}` : '', x: sx + 14, y: sy + 14 };
    } else tip = { name: groupTitle(d, h.g), kind: d.groups[h.g].kind, x: sx + 14, y: sy + 14 };
  }
  function onpointerup(e: PointerEvent) {
    if (drag && !drag.moved) select(e);
    drag = null;
  }
  function select(e: PointerEvent) {
    const d = app.design;
    if (!d) return;
    const rect = wrap.getBoundingClientRect();
    const h = hit(e.clientX - rect.left, e.clientY - rect.top);
    if (!h) {
      app.selectedGroup = null;
      return;
    }
    app.selectedGroup = h.g;
    const sp = d.groups[h.g]?.span;
    if (sp) app.highlight = sp;
  }
  function onwheel(e: WheelEvent) {
    e.preventDefault();
    const rect = wrap.getBoundingClientRect();
    zoomAt(e.clientX - rect.left, e.clientY - rect.top, Math.exp(-e.deltaY * (e.deltaMode === 1 ? 0.05 : 0.0016)));
  }
  function zoomAt(sx: number, sy: number, f: number) {
    const [wx, wy] = toWorld(sx, sy);
    const zoom = Math.min(64, Math.max(1e-6, cam.zoom * f));
    cam = { zoom, cx: wx - (sx - W() / 2) / zoom, cy: wy - (sy - H() / 2) / zoom };
  }
  function zoomStep(f: number) {
    const target = { ...cam, zoom: Math.min(64, Math.max(1e-6, cam.zoom * f)) };
    animateTo(target);
  }
  function ondblclick(e: MouseEvent) {
    const rect = wrap.getBoundingClientRect();
    const [wx, wy] = toWorld(e.clientX - rect.left, e.clientY - rect.top);
    const z = Math.min(64, cam.zoom * 4);
    animateTo({ cx: wx, cy: wy, zoom: z });
  }

  export function fitAll() {
    if (app.design?.bbox) fit(app.design.bbox);
  }
  export function zoomTo(pxPerCell: number) {
    // Aim at the component closest to the view centre (inside the selection, if any).
    let cx = cam.cx;
    let cy = cam.cy;
    const d = app.design;
    if (comps && d) {
      let best = Infinity;
      for (let k = 0; k < comps[0]; k++) {
        const o = 1 + 6 * k;
        if (app.selectedGroup !== null && !within(d, comps[o + 5], app.selectedGroup)) continue;
        const x = (comps[o] + comps[o + 2]) / 2;
        const y = (comps[o + 1] + comps[o + 3]) / 2;
        const dd = (x - cam.cx) ** 2 + (y - cam.cy) ** 2;
        if (dd < best) {
          best = dd;
          cx = x;
          cy = y;
        }
      }
    }
    animateTo({ cx, cy, zoom: pxPerCell });
  }

  const kindColors = $derived.by(() => {
    void settings.palette;
    return palette().kinds.map((c) => css(c));
  });
  const LAYERS = [
    ['overlay', 'Abstraction overlay'],
    ['overlayLabels', 'Labels'],
    ['pins', 'I/O pins'],
    ['wires', 'Glider paths'],
    ['glow', 'Glider halo'],
  ] as const;
</script>

<div class="life" bind:this={wrap} bind:clientWidth={canvasW}>
  <canvas bind:this={canvas} class="gl" {onpointerdown} {onpointermove} {onpointerup} onpointerleave={() => ((tip = null), (hoverGroup = -1))} {onwheel} {ondblclick}></canvas>
  <canvas bind:this={overlay} class="ov"></canvas>
  {#if glError}
    <div class="msg">This view needs WebGL2: {glError}</div>
  {:else if app.design?.stats.layoutError}
    <div class="msg">Layout failed: {app.design.stats.layoutError}</div>
  {/if}
  {#if app.design}
    {#if canvasW - insets.l - insets.r > 680}
      <div class="legend" style="top:{insets.t || 12}px; left:{insets.l + 12}px">
        {#each KIND_NAMES as k, i}
          <span><i style="background:{kindColors[i]}"></i>{k}</span>
        {/each}
      </div>
    {/if}
    <div class="toolbar" style="top:{insets.t || 12}px; right:{insets.r + 12}px">
      <button class="tb" data-menu class:on={ui.menu === 'layers'} onclick={() => (ui.menu = ui.menu === 'layers' ? null : 'layers')} title="Layers"><span class="dim"><Icon name="layers" size={14} /></span>Layers</button>
      <span class="vsep"></span>
      <button class="tbi" title="Zoom out" onclick={() => zoomStep(1 / 1.6)}><Icon name="minus" /></button>
      <button class="tb zl mono" title="Fit (F)" onclick={fitAll}>{zoomLabel}</button>
      <button class="tbi" title="Zoom in" onclick={() => zoomStep(1.6)}><Icon name="plus" /></button>
      <button class="tb" title="Zoom to cells" onclick={() => zoomTo(4)}>Cells</button>
      {#if ui.menu === 'layers'}
        <div class="menu layers">
          {#each LAYERS as [k, label]}
            <button class="menu-row between" onclick={() => (settings[k] = !settings[k])}><span>{label}</span><Switch on={settings[k]} /></button>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
  {#if tip}
    <div class="tip" style="left:{tip.x}px; top:{tip.y}px"><span class="mono">{tip.name}</span>{#if tip.kind}<span class="k">{tip.kind}</span>{/if}</div>
  {/if}
</div>

<style>
  .life {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
    background: var(--canvas);
    touch-action: none;
  }
  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }
  .gl {
    cursor: grab;
  }
  .gl:active {
    cursor: grabbing;
  }
  .ov {
    pointer-events: none;
  }
  .legend {
    position: absolute;
    display: flex;
    gap: 12px;
    font-size: 11.5px;
    color: var(--fg-dim);
    padding: 5px 9px;
    border-radius: 8px;
    background: var(--hud);
    pointer-events: none;
    z-index: 15;
  }
  .legend span {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .legend i {
    width: 8px;
    height: 8px;
    border-radius: 2px;
  }
  .toolbar {
    position: absolute;
    display: flex;
    padding: 3px;
    gap: 2px;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: var(--shadow-sm);
    z-index: 30;
  }
  .tb {
    height: 28px;
    padding: 0 8px;
    border-radius: 7px;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
  }
  .tbi {
    width: 28px;
    height: 28px;
    border-radius: 7px;
    display: grid;
    place-items: center;
    color: var(--fg-dim);
  }
  .tb:hover,
  .tb.on,
  .tbi:hover {
    background: var(--hover);
  }
  .zl {
    font-size: 11.5px;
    min-width: 104px;
    justify-content: center;
  }
  .vsep {
    width: 1px;
    margin: 4px 2px;
    background: var(--border);
  }
  .dim {
    color: var(--fg-dim);
    display: inline-flex;
  }
  .layers {
    top: 38px;
    left: 0;
    width: 230px;
    padding: 6px;
  }
  .between {
    justify-content: space-between;
  }
  .tip {
    position: absolute;
    display: flex;
    gap: 8px;
    padding: 5px 9px;
    border-radius: 7px;
    background: var(--fg);
    color: var(--bg);
    font-size: 12px;
    pointer-events: none;
    z-index: 30;
    max-width: 70%;
  }
  .tip .k {
    opacity: 0.7;
    white-space: nowrap;
  }
  .msg {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: var(--fg-dim);
    font-size: 14px;
    pointer-events: none;
  }
</style>
