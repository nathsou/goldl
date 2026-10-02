<script lang="ts">
  import { onMount } from 'svelte';
  import { app, settings, period } from '../state.svelte';
  import { engine } from '../engine';
  import { Batch, Renderer, type Camera } from './renderer';
  import type { Design } from '../types';

  let wrap: HTMLDivElement;
  let canvas: HTMLCanvasElement;
  let overlay: HTMLCanvasElement;
  let renderer: Renderer | null = null;
  let glError = $state('');

  let cam: Camera = { cx: 0, cy: 0, zoom: 0.001 };
  let hud = $state({ x: 0, y: 0, zoom: 0.001, mode: '', cells: 0, gliders: 0 });
  let hoverRegion: { name: string; kind: string; x: number; y: number } | null = $state(null);

  let comps: Int32Array | null = null;
  let wires: Float64Array | null = null;
  let data: { cells: Int32Array | null; gliders: Float64Array | null; active: Int32Array | null } = { cells: null, gliders: null, active: null };
  let lastKey = '';
  let inflight = false;
  let designRef: Design | null = null;

  const KIND_NAMES = ['Snark', 'colour-changing reflector', 'Syringe duplicator', 'eater'];

  interface Pal {
    bg: [number, number, number];
    cell: [number, number, number];
    kinds: [number, number, number][];
    row: [number, number, number];
    col: [number, number, number];
    other: [number, number, number];
    active: [number, number, number];
    text: string;
  }
  const hex = (h: string): [number, number, number] => [parseInt(h.slice(1, 3), 16) / 255, parseInt(h.slice(3, 5), 16) / 255, parseInt(h.slice(5, 7), 16) / 255];
  function palette(): Pal {
    const light = settings.theme === 'light';
    if (settings.palette === 'classic')
      return { bg: light ? hex('#f4f4ee') : hex('#000000'), cell: light ? hex('#111111') : hex('#ffffff'), kinds: ['#8a8a8a', '#a0a0a0', '#b8b8b8', '#707070'].map(hex), row: hex('#9ad'), col: hex('#d9a'), other: hex('#ccc'), active: hex('#ffffff'), text: light ? '#222' : '#ddd' };
    if (settings.palette === 'amber')
      return { bg: light ? hex('#fbf4e6') : hex('#0b0703'), cell: light ? hex('#3a2200') : hex('#ffd27a'), kinds: ['#c4772a', '#e09a3c', '#f2bd5a', '#b0502a'].map(hex), row: hex('#ffb347'), col: hex('#ff7a3d'), other: hex('#ffe08a'), active: hex('#fff1c2'), text: light ? '#3a2200' : '#ffd27a' };
    return {
      bg: light ? hex('#f6f7fb') : hex('#05070d'),
      cell: light ? hex('#0d1424') : hex('#e9f7ff'),
      kinds: (light ? ['#6c5ce7', '#2f86d6', '#d98a1c', '#d6335c'] : ['#7c6cff', '#3fa7ff', '#ffb347', '#ff5c7a']).map(hex),
      row: hex('#38e8c6'),
      col: hex('#ff4fd8'),
      other: hex('#ffd36b'),
      active: hex('#9ff6ff'),
      text: light ? '#1a2238' : '#d8e6ff',
    };
  }

  function viewRect(pad = 0.15) {
    const w = (wrap?.clientWidth ?? 800) / cam.zoom;
    const h = (wrap?.clientHeight ?? 600) / cam.zoom;
    return { x0: cam.cx - w * (0.5 + pad), y0: cam.cy - h * (0.5 + pad), x1: cam.cx + w * (0.5 + pad), y1: cam.cy + h * (0.5 + pad) };
  }

  function fit(b: [number, number, number, number], animate = true) {
    const w = Math.max(10, b[2] - b[0]);
    const h = Math.max(10, b[3] - b[1]);
    const zx = (wrap.clientWidth * 0.9) / w;
    const zy = (wrap.clientHeight * 0.9) / h;
    const target = { cx: (b[0] + b[2]) / 2, cy: (b[1] + b[3]) / 2, zoom: Math.min(zx, zy, 24) };
    if (!animate) {
      cam = target;
      return;
    }
    const from = { ...cam };
    const t0 = performance.now();
    const step = () => {
      const t = Math.min(1, (performance.now() - t0) / 450);
      const e = t * t * (3 - 2 * t);
      const lz = Math.log(from.zoom) + (Math.log(target.zoom) - Math.log(from.zoom)) * e;
      cam = { cx: from.cx + (target.cx - from.cx) * e, cy: from.cy + (target.cy - from.cy) * e, zoom: Math.exp(lz) };
      if (t < 1) requestAnimationFrame(step);
    };
    step();
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

  function groupDepth(d: Design, g: number) {
    let k = 0;
    let p = d.groups[g]?.parent;
    while (p !== null && p !== undefined && k < 32) {
      k++;
      p = d.groups[p]?.parent;
    }
    return k;
  }

  /** Screen polygon of a run of staircase blocks (gc rectangle, rotated grid). */
  function runPoly(run: [number, number, number, number], G: number) {
    const [i0, j0, i1, j1] = run;
    const u0 = (i0 - 0.5) * G;
    const u1 = (i1 + 0.5) * G;
    const v0 = (j0 - 0.5) * G;
    const v1 = (j1 + 0.5) * G;
    const pts: [number, number][] = [
      [u0, v0],
      [u1, v0],
      [u1, v1],
      [u0, v1],
    ].map(([u, v]) => [(u + v) / 2, (u - v) / 2]);
    return pts.map(([x, y]) => toScreen(x, y));
  }

  function toScreen(x: number, y: number): [number, number] {
    return [(x - cam.cx) * cam.zoom + wrap.clientWidth / 2, (y - cam.cy) * cam.zoom + wrap.clientHeight / 2];
  }
  function toWorld(sx: number, sy: number): [number, number] {
    return [(sx - wrap.clientWidth / 2) / cam.zoom + cam.cx, (sy - wrap.clientHeight / 2) / cam.zoom + cam.cy];
  }

  function hue(g: number) {
    return (g * 137.508) % 360;
  }

  // Regions worth drawing at the current zoom (deepest first for hit testing).
  function visibleRegions(d: Design) {
    const out: { g: number; pts: [number, number][]; size: number; depth: number }[] = [];
    const G = d.grid ?? 128;
    const W = wrap.clientWidth;
    const H = wrap.clientHeight;
    for (const r of d.regions) {
      const depth = groupDepth(d, r.group);
      if (depth === 0 && d.regions.length > 1) continue;
      for (const run of r.runs) {
        const pts = runPoly(run, G);
        const xs = pts.map((p) => p[0]);
        const ys = pts.map((p) => p[1]);
        const minx = Math.min(...xs);
        const maxx = Math.max(...xs);
        const miny = Math.min(...ys);
        const maxy = Math.max(...ys);
        if (maxx < 0 || minx > W || maxy < 0 || miny > H) continue;
        const size = Math.max(maxx - minx, maxy - miny);
        if (size < 28 || size > 40 * Math.max(W, H)) continue;
        out.push({ g: r.group, pts, size, depth });
      }
    }
    return out;
  }

  function drawOverlay() {
    const ctx = overlay.getContext('2d')!;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const W = wrap.clientWidth;
    const H = wrap.clientHeight;
    if (overlay.width !== Math.round(W * dpr) || overlay.height !== Math.round(H * dpr)) {
      overlay.width = Math.round(W * dpr);
      overlay.height = Math.round(H * dpr);
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, W, H);
    const d = app.design;
    if (!d || !settings.overlay) return;
    const light = settings.theme === 'light';
    const regs = visibleRegions(d).sort((a, b) => a.depth - b.depth);
    for (const r of regs) {
      const h = hue(r.g);
      const sel = app.selectedGroup === r.g;
      // Fade huge (enclosing) regions and tiny ones; emphasise the middle of the range.
      const fill = sel ? 0.16 : Math.max(0.025, 0.09 - r.depth * 0.012);
      ctx.beginPath();
      r.pts.forEach(([x, y], k) => (k ? ctx.lineTo(x, y) : ctx.moveTo(x, y)));
      ctx.closePath();
      ctx.fillStyle = `hsla(${h}, 85%, ${light ? 45 : 62}%, ${fill})`;
      ctx.fill();
      ctx.lineWidth = sel ? 2.5 : Math.max(0.8, 2 - r.depth * 0.35);
      ctx.strokeStyle = `hsla(${h}, 90%, ${light ? 40 : 68}%, ${sel ? 0.95 : 0.55})`;
      ctx.setLineDash(r.depth > 1 ? [5, 4] : []);
      ctx.stroke();
      ctx.setLineDash([]);
      if (settings.overlayLabels && r.size > 110) {
        const top = r.pts.reduce((a, b) => (b[1] < a[1] ? b : a));
        const name = d.groups[r.g]?.name ?? '';
        const fs = Math.min(18, Math.max(10, r.size / 22));
        ctx.font = `600 ${fs}px var(--sans, system-ui)`;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'bottom';
        const tw = ctx.measureText(name).width;
        ctx.fillStyle = light ? 'rgba(255,255,255,0.8)' : 'rgba(5,7,13,0.72)';
        ctx.fillRect(top[0] - tw / 2 - 6, top[1] - fs - 8, tw + 12, fs + 6);
        ctx.fillStyle = `hsl(${h}, 90%, ${light ? 32 : 75}%)`;
        ctx.fillText(name, top[0], top[1] - 4);
      }
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
    reqs.push(cam.zoom > 0.003 && !cellsMode ? engine.active(g, r) : Promise.resolve(null));
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
    // Glider paths: the circuit's wiring.
    if (wires && settings.wires) {
      wireB.reset();
      const n = wires[0];
      const r = viewRect(0.05);
      const a = cellsVisible ? 0.1 : 0.32;
      for (let k = 0; k < n; k++) {
        const o = 1 + 6 * k;
        const x0 = wires[o];
        const y0 = wires[o + 1];
        const x1 = wires[o + 2];
        const y1 = wires[o + 3];
        if (Math.max(x0, x1) < r.x0 || Math.min(x0, x1) > r.x1 || Math.max(y0, y1) < r.y0 || Math.min(y0, y1) > r.y1) continue;
        const dir = wires[o + 4];
        const c = dir === 8 ? pal.row : dir === 2 ? pal.col : pal.other;
        wireB.push(x0 - cx, y0 - cy, x1 - cx, y1 - cy, c[0], c[1], c[2], a);
      }
      renderer.drawSegments(wireB, cam, cellsVisible ? 1.5 : 1.6);
    }
    // Components (overview).
    if (comps && (!cellsVisible || z < 2.5) && settings.componentOutlines) {
      compB.reset();
      const n = comps[0];
      const a = cellsVisible ? 0.25 : 0.75;
      const r = viewRect(0.05);
      for (let k = 0; k < n; k++) {
        const o = 1 + 6 * k;
        const x0 = comps[o];
        const y0 = comps[o + 1];
        const x1 = comps[o + 2];
        const y1 = comps[o + 3];
        if (x1 < r.x0 || x0 > r.x1 || y1 < r.y0 || y0 > r.y1) continue;
        const c = pal.kinds[comps[o + 4]] ?? pal.cell;
        const sel = app.selectedGroup !== null && comps[o + 5] === app.selectedGroup;
        compB.push(x0 - cx, y0 - cy, x1 + 1 - cx, y1 + 1 - cy, sel ? 1 : c[0], sel ? 1 : c[1], sel ? 1 : c[2], sel ? 0.95 : a);
      }
      renderer.draw(compB, cam, cellsVisible ? 2 : 0, 1.2, false, 0.06);
    }
    // Reacting components.
    if (data.active && comps && settings.glow && !cellsVisible) {
      actB.reset();
      const n = data.active[0];
      for (let k = 1; k <= n; k++) {
        const o = 1 + 6 * data.active[k];
        const pad = 40;
        actB.push(comps[o] - pad - cx, comps[o + 1] - pad - cy, comps[o + 2] + pad - cx, comps[o + 3] + pad - cy, pal.active[0], pal.active[1], pal.active[2], 0.55);
      }
      renderer.draw(actB, cam, 1, 14, true);
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
    // Gliders.
    if (data.gliders) {
      const gl = data.gliders;
      const n = gl[0];
      glowB.reset();
      const rad = Math.max(6 / z, 7);
      for (let k = 0; k < n; k++) {
        const o = 1 + 4 * k;
        const x = gl[o] - cx;
        const y = gl[o + 1] - cy;
        const dir = gl[o + 2];
        const col = dir === 8 ? pal.row : dir === 2 ? pal.col : pal.other;
        const a = cellsVisible ? 0.35 : 0.95;
        glowB.push(x - rad, y - rad, x + rad, y + rad, col[0], col[1], col[2], a);
      }
      if (settings.glow || !cellsVisible) renderer.draw(glowB, cam, 1, 9, true);
    }
    drawOverlay();
    hud.zoom = z;
    hud.mode = cellsVisible ? 'cells' : 'overview';
    hud.cells = cellsVisible ? data.cells![0] : 0;
    hud.gliders = data.gliders ? data.gliders[0] : 0;
  }

  let raf = 0;
  let lastT = 0;
  function frame(t: number) {
    const dt = lastT ? Math.min(0.1, (t - lastT) / 1000) : 0;
    lastT = t;
    if (app.playing && app.design) app.gen = Math.max(0, app.gen + app.speed * dt);
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
  let drag: { x: number; y: number; cx: number; cy: number; moved: boolean } | null = null;
  function onpointerdown(e: PointerEvent) {
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    drag = { x: e.clientX, y: e.clientY, cx: cam.cx, cy: cam.cy, moved: false };
  }
  function onpointermove(e: PointerEvent) {
    const rect = wrap.getBoundingClientRect();
    const [wx, wy] = toWorld(e.clientX - rect.left, e.clientY - rect.top);
    hud.x = Math.floor(wx);
    hud.y = Math.floor(wy);
    if (drag) {
      const dx = e.clientX - drag.x;
      const dy = e.clientY - drag.y;
      if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true;
      cam = { ...cam, cx: drag.cx - dx / cam.zoom, cy: drag.cy - dy / cam.zoom };
      hoverRegion = null;
      return;
    }
    hoverRegion = null;
    const d = app.design;
    if (!d || !settings.overlay) return;
    const sx = e.clientX - rect.left;
    const sy = e.clientY - rect.top;
    const regs = visibleRegions(d).sort((a, b) => b.depth - a.depth);
    for (const r of regs) {
      if (inPoly(sx, sy, r.pts)) {
        const g = d.groups[r.g];
        hoverRegion = { name: g.name, kind: g.kind, x: sx + 14, y: sy + 14 };
        break;
      }
    }
  }
  function inPoly(x: number, y: number, pts: [number, number][]) {
    let inside = false;
    for (let i = 0, j = pts.length - 1; i < pts.length; j = i++) {
      const [xi, yi] = pts[i];
      const [xj, yj] = pts[j];
      if (yi > y !== yj > y && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) inside = !inside;
    }
    return inside;
  }
  function onpointerup(e: PointerEvent) {
    if (drag && !drag.moved) select(e);
    drag = null;
  }
  function select(e: PointerEvent) {
    const d = app.design;
    if (!d) return;
    const rect = wrap.getBoundingClientRect();
    const sx = e.clientX - rect.left;
    const sy = e.clientY - rect.top;
    const regs = settings.overlay ? visibleRegions(d).sort((a, b) => b.depth - a.depth) : [];
    for (const r of regs) {
      if (inPoly(sx, sy, r.pts)) {
        app.selectedGroup = r.g;
        // Source span of the group, from its schematic nodes.
        const spans = d.schematic.nodes.filter((n) => n.group === r.g && n.span[1] > n.span[0]).map((n) => n.span);
        if (spans.length) app.highlight = [Math.min(...spans.map((s) => s[0])), Math.max(...spans.map((s) => s[1]))];
        return;
      }
    }
    app.selectedGroup = null;
  }
  function onwheel(e: WheelEvent) {
    e.preventDefault();
    const rect = wrap.getBoundingClientRect();
    const sx = e.clientX - rect.left;
    const sy = e.clientY - rect.top;
    const [wx, wy] = toWorld(sx, sy);
    const f = Math.exp(-e.deltaY * (e.deltaMode === 1 ? 0.05 : 0.0016));
    const zoom = Math.min(64, Math.max(1e-6, cam.zoom * f));
    cam = { zoom, cx: wx - (sx - wrap.clientWidth / 2) / zoom, cy: wy - (sy - wrap.clientHeight / 2) / zoom };
  }
  function ondblclick(e: MouseEvent) {
    const rect = wrap.getBoundingClientRect();
    const [wx, wy] = toWorld(e.clientX - rect.left, e.clientY - rect.top);
    const z = Math.min(64, cam.zoom * 4);
    const w = wrap.clientWidth / z;
    const h = wrap.clientHeight / z;
    fit([wx - w / 2, wy - h / 2, wx + w / 2, wy + h / 2]);
  }

  export function fitAll() {
    if (app.design?.bbox) fit(app.design.bbox);
  }
  export function zoomTo(pxPerCell: number) {
    // Aim at the component closest to the view centre (or the selected group).
    let cx = cam.cx;
    let cy = cam.cy;
    if (comps) {
      let best = Infinity;
      for (let k = 0; k < comps[0]; k++) {
        const o = 1 + 6 * k;
        if (app.selectedGroup !== null && comps[o + 5] !== app.selectedGroup) continue;
        const x = (comps[o] + comps[o + 2]) / 2;
        const y = (comps[o + 1] + comps[o + 3]) / 2;
        const d = (x - cam.cx) ** 2 + (y - cam.cy) ** 2;
        if (d < best) {
          best = d;
          cx = x;
          cy = y;
        }
      }
    }
    const target = { cx, cy, zoom: pxPerCell };
    const w = wrap.clientWidth / target.zoom / 0.9;
    const h = wrap.clientHeight / target.zoom / 0.9;
    fit([cx - w / 2, cy - h / 2, cx + w / 2, cy + h / 2]);
  }

  const fmtZoom = (z: number) => (z >= 1 ? `${z.toFixed(z >= 10 ? 0 : 1)} px/cell` : `1 px : ${Math.round(1 / z).toLocaleString()} cells`);
</script>

<div class="life" bind:this={wrap}>
  <canvas
    bind:this={canvas}
    class="gl"
    {onpointerdown}
    {onpointermove}
    {onpointerup}
    onpointerleave={() => (hoverRegion = null)}
    {onwheel}
    {ondblclick}
  ></canvas>
  <canvas bind:this={overlay} class="ov"></canvas>
  {#if glError}
    <div class="error">This view needs WebGL2: {glError}</div>
  {/if}
  {#if hoverRegion}
    <div class="tip" style="left:{hoverRegion.x}px; top:{hoverRegion.y}px">
      <b>{hoverRegion.name}</b><span>{hoverRegion.kind}</span>
    </div>
  {/if}
  <div class="hud">
    <span>({hud.x.toLocaleString()}, {hud.y.toLocaleString()})</span>
    <span>{fmtZoom(hud.zoom)}</span>
    <span>{hud.mode}{hud.mode === 'cells' ? ` · ${hud.cells.toLocaleString()} cells` : ''} · {hud.gliders.toLocaleString()} gliders</span>
  </div>
  <div class="legend">
    {#each KIND_NAMES as k, i}
      <span><i style="background: rgb({palette().kinds[i].map((x) => Math.round(x * 255)).join(',')})"></i>{k}</span>
    {/each}
    <span><i class="dot" style="background:#38e8c6"></i>glider →↘</span>
    <span><i class="dot" style="background:#ff4fd8"></i>glider →↗</span>
  </div>
  {#if !app.design}
    <div class="empty">Compile a design to see it run in the Game of Life.</div>
  {:else if app.design.stats.layoutError}
    <div class="empty">Layout failed: {app.design.stats.layoutError}</div>
  {/if}
</div>

<style>
  .life {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
    background: var(--life-bg);
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
  .hud {
    position: absolute;
    left: 10px;
    bottom: 8px;
    display: flex;
    gap: 14px;
    font: 11.5px var(--mono);
    color: var(--fg-dim);
    background: var(--hud-bg);
    padding: 3px 10px;
    border-radius: 6px;
    pointer-events: none;
  }
  .legend {
    position: absolute;
    right: 10px;
    bottom: 8px;
    display: flex;
    gap: 12px;
    font: 11px var(--sans);
    color: var(--fg-dim);
    background: var(--hud-bg);
    padding: 3px 10px;
    border-radius: 6px;
    pointer-events: none;
  }
  .legend i {
    display: inline-block;
    width: 9px;
    height: 9px;
    margin-right: 5px;
    border-radius: 2px;
    vertical-align: -1px;
  }
  .legend i.dot {
    border-radius: 50%;
    box-shadow: 0 0 6px currentColor;
  }
  .tip {
    position: absolute;
    display: flex;
    flex-direction: column;
    gap: 1px;
    background: var(--panel-raised);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
    border-radius: 6px;
    padding: 5px 9px;
    font: 12px var(--sans);
    color: var(--fg);
    pointer-events: none;
  }
  .tip span {
    color: var(--fg-dim);
    font-size: 11px;
  }
  .empty,
  .error {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: var(--fg-dim);
    font: 14px var(--sans);
    pointer-events: none;
  }
</style>
