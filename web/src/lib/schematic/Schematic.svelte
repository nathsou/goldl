<script lang="ts">
  // Logic diagram of the word-level RTL with ANSI/IEEE distinctive-shape symbols.
  import { app, cycleAt } from '../state.svelte';
  import { engine } from '../engine';
  import type { SNode, SWire } from '../types';

  let svg: SVGSVGElement;
  let wrap: HTMLDivElement;
  let view = $state({ x: 0, y: 0, k: 1 });
  let values: Map<number, bigint> = $state(new Map());
  let hover: { x: number; y: number; text: string } | null = $state(null);
  let fitted: unknown = null;

  const sch = $derived(app.design?.schematic ?? null);
  const cycle = $derived(cycleAt(Math.floor(app.gen)));

  // Values of every wire for the current cycle.
  $effect(() => {
    const s = sch;
    const c = cycle;
    void app.simVersion;
    if (!s) return;
    const rids = [...new Set(s.wires.map((w) => w.rid).concat(s.nodes.filter((n) => n.rid !== null).map((n) => n.rid!)))];
    engine.trace(c, c + 1, rids).then((rows) => {
      const m = new Map<number, bigint>();
      rids.forEach((r, i) => m.set(r, BigInt(rows[0]?.values[i] ?? '0')));
      values = m;
    });
  });

  $effect(() => {
    if (sch && fitted !== sch && wrap) {
      fitted = sch;
      fit();
    }
  });

  function fit() {
    if (!sch || !wrap) return;
    const [w, h] = sch.size;
    const k = Math.min(wrap.clientWidth / (w + 40), wrap.clientHeight / (h + 40), 2.2);
    view = { k, x: (wrap.clientWidth - w * k) / 2, y: Math.max(10, (wrap.clientHeight - h * k) / 2) };
  }

  let drag: { x: number; y: number; vx: number; vy: number } | null = null;
  function onpointerdown(e: PointerEvent) {
    if ((e.target as Element).closest('.node')) return;
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    drag = { x: e.clientX, y: e.clientY, vx: view.x, vy: view.y };
  }
  function onpointermove(e: PointerEvent) {
    if (!drag) return;
    view = { ...view, x: drag.vx + e.clientX - drag.x, y: drag.vy + e.clientY - drag.y };
  }
  function onwheel(e: WheelEvent) {
    e.preventDefault();
    const r = wrap.getBoundingClientRect();
    const sx = e.clientX - r.left;
    const sy = e.clientY - r.top;
    const f = Math.exp(-e.deltaY * 0.0015);
    const k = Math.min(8, Math.max(0.05, view.k * f));
    view = { k, x: sx - ((sx - view.x) * k) / view.k, y: sy - ((sy - view.y) * k) / view.k };
  }

  const fmt = (v: bigint | undefined, w: number) => (v === undefined ? '?' : w === 1 ? String(v) : w <= 4 ? String(v) : '0x' + v.toString(16));

  function wireClass(w: SWire) {
    const v = values.get(w.rid);
    if (w.width > 1) return 'bus';
    return v ? 'hi' : 'lo';
  }

  function selectNode(n: SNode) {
    app.selectedGroup = n.group;
    if (n.span[1] > n.span[0]) app.highlight = n.span;
  }

  function locate(n: SNode) {
    selectNode(n);
    const d = app.design;
    if (!d) return;
    const reg = d.regions.find((r) => r.group === n.group);
    if (!reg || !d.grid) return;
    const G = d.grid;
    let x0 = Infinity,
      y0 = Infinity,
      x1 = -Infinity,
      y1 = -Infinity;
    for (const [i0, j0, i1, j1] of reg.runs) {
      for (const [u, v] of [
        [(i0 - 0.5) * G, (j0 - 0.5) * G],
        [(i1 + 0.5) * G, (j0 - 0.5) * G],
        [(i0 - 0.5) * G, (j1 + 0.5) * G],
        [(i1 + 0.5) * G, (j1 + 0.5) * G],
      ]) {
        const x = (u + v) / 2;
        const y = (u - v) / 2;
        x0 = Math.min(x0, x);
        y0 = Math.min(y0, y);
        x1 = Math.max(x1, x);
        y1 = Math.max(y1, y);
      }
    }
    app.frame = [x0, y0, x1, y1];
    app.view = 'life';
  }

  function path(points: [number, number][]) {
    return points.map(([x, y], i) => `${i ? 'L' : 'M'}${x},${y}`).join(' ');
  }

  // Gate outlines (local coordinates, origin at the node's top-left).
  function gatePath(n: SNode): string {
    const { w, h } = n;
    switch (n.kind) {
      case 'and':
        return `M0,0 H${w * 0.5} A${w * 0.5},${h / 2} 0 0 1 ${w * 0.5},${h} H0 Z`;
      case 'or':
      case 'xor':
        return `M0,0 Q${w * 0.55},0 ${w},${h / 2} Q${w * 0.55},${h} 0,${h} Q${w * 0.28},${h / 2} 0,0 Z`;
      case 'not':
        return `M0,0 L${w - 8},${h / 2} L0,${h} Z`;
      case 'mux':
        return `M0,0 L${w},${h * 0.18} L${w},${h * 0.82} L0,${h} Z`;
      case 'input':
        return `M0,0 H${w - 9} L${w},${h / 2} L${w - 9},${h} H0 Z`;
      case 'output':
        return `M9,0 H${w} V${h} H9 L0,${h / 2} Z`;
      default:
        return `M0,0 H${w} V${h} H0 Z`;
    }
  }

  const groupBoxes = $derived.by(() => {
    if (!sch || !app.design) return [];
    const m = new Map<number, [number, number, number, number]>();
    for (const n of sch.nodes) {
      if (n.kind === 'const' || n.kind === 'input' || n.kind === 'output' || n.group === 0) continue;
      const b = m.get(n.group);
      m.set(n.group, b ? [Math.min(b[0], n.x), Math.min(b[1], n.y), Math.max(b[2], n.x + n.w), Math.max(b[3], n.y + n.h)] : [n.x, n.y, n.x + n.w, n.y + n.h]);
    }
    return [...m.entries()]
      .filter(([g]) => app.design!.groups[g])
      .map(([g, b]) => ({ g, name: app.design!.groups[g].name, b }));
  });
</script>

<div class="schematic" bind:this={wrap} {onwheel} role="presentation">
  {#if sch}
    <svg bind:this={svg} {onpointerdown} {onpointermove} onpointerup={() => (drag = null)} role="presentation">
      <defs>
        <filter id="glow" x="-50%" y="-50%" width="200%" height="200%">
          <feGaussianBlur stdDeviation="2.2" result="b" />
          <feMerge><feMergeNode in="b" /><feMergeNode in="SourceGraphic" /></feMerge>
        </filter>
      </defs>
      <g transform="translate({view.x},{view.y}) scale({view.k})">
        {#each groupBoxes as gb}
          <g class="group" class:sel={app.selectedGroup === gb.g}>
            <rect x={gb.b[0] - 10} y={gb.b[1] - 22} width={gb.b[2] - gb.b[0] + 20} height={gb.b[3] - gb.b[1] + 32} rx="8" style="--h:{(gb.g * 137.508) % 360}" />
            <text x={gb.b[0] - 4} y={gb.b[1] - 9} style="--h:{(gb.g * 137.508) % 360}">{gb.name}</text>
          </g>
        {/each}
        {#each sch.wires as w}
          <path
            class="wire {wireClass(w)}"
            class:fb={w.feedback}
            class:busw={w.width > 1}
            d={path(w.points)}
            role="presentation"
            onpointerenter={(e) => (hover = { x: e.offsetX + 12, y: e.offsetY + 12, text: `${fmt(values.get(w.rid), w.width)}  (${w.width} bit${w.width > 1 ? 's' : ''})` })}
            onpointerleave={() => (hover = null)}
          />
        {/each}
        {#each sch.nodes as n}
          {@const v = n.rid !== null ? values.get(n.rid) : undefined}
          <g
            class="node k-{n.kind}"
            class:sel={app.selectedGroup === n.group && n.group !== 0}
            transform="translate({n.x},{n.y})"
            role="button"
            tabindex="-1"
            onclick={() => selectNode(n)}
            ondblclick={() => locate(n)}
            onkeydown={() => {}}
            onpointerenter={(e) => (hover = { x: e.offsetX + 12, y: e.offsetY + 12, text: `${n.kind}${n.label ? ' ' + n.label : ''} = ${fmt(v, n.width)} · double-click to locate in Life` })}
            onpointerleave={() => (hover = null)}
          >
            {#if n.kind === 'dff'}
              <rect width={n.w} height={n.h} rx="3" class="body" />
              <path d="M0,{n.h - 14} L9,{n.h - 8} L0,{n.h - 2}" class="clk" />
              <text x="5" y={n.h * 0.3 + 4} class="pin">D</text>
              <text x={n.w - 5} y={n.h * 0.3 + 4} class="pin" text-anchor="end">Q</text>
              <text x={n.w / 2} y="-5" class="name" text-anchor="middle">{n.label}</text>
              <text x={n.w / 2} y={n.h - 6} class="val" text-anchor="middle">{fmt(v, n.width)}</text>
            {:else if n.kind === 'bus' || n.kind === 'concat'}
              <rect width={n.w} height={n.h} rx="2" class="bar" />
              <text x={n.w / 2} y="-4" class="tiny" text-anchor="middle">{n.label || '{…}'}</text>
            {:else if n.kind === 'const'}
              <rect width={n.w} height={n.h} rx="9" class="const" />
              <text x={n.w / 2} y={n.h / 2 + 4} class="cval" text-anchor="middle">{n.label}</text>
            {:else}
              <path d={gatePath(n)} class="body" class:on={n.width === 1 && v === 1n} />
              {#if n.kind === 'xor'}
                <path d="M-6,0 Q{n.w * 0.28 - 6},{n.h / 2} -6,{n.h}" class="xorback" />
              {/if}
              {#if n.kind === 'not'}
                <circle cx={n.w - 4} cy={n.h / 2} r="4" class="bubble" />
              {/if}
              {#if n.kind === 'input' || n.kind === 'output'}
                <text x={n.kind === 'input' ? 7 : 15} y={n.h / 2 + 4} class="port">{n.label}</text>
              {:else if n.kind === 'mux'}
                {#each n.inNames as nm, i}
                  <text x="4" y={n.ins[i][1] - n.y + 3} class="pin small">{nm}</text>
                {/each}
              {:else if n.label}
                <text x={n.kind === 'and' || n.kind === 'or' || n.kind === 'xor' ? n.w * 0.4 : n.w / 2} y={n.h / 2 + 5} class="sym" text-anchor="middle">{n.label}</text>
              {/if}
            {/if}
          </g>
        {/each}
      </g>
    </svg>
    <div class="bar-top">
      <span>{app.design?.name} · cycle {cycle}</span>
      <button onclick={fit}>Fit</button>
    </div>
    {#if hover}
      <div class="tip" style="left:{hover.x}px; top:{hover.y}px">{hover.text}</div>
    {/if}
  {:else}
    <div class="empty">Compile a design to see its schematic.</div>
  {/if}
</div>

<style>
  .schematic {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
    background: var(--sch-bg);
    background-image: radial-gradient(var(--sch-dot) 1px, transparent 1px);
    background-size: 22px 22px;
  }
  svg {
    width: 100%;
    height: 100%;
    cursor: grab;
  }
  .wire {
    fill: none;
    stroke-width: 1.3;
    stroke-linejoin: round;
  }
  .wire.lo {
    stroke: var(--wire-lo);
  }
  .wire.hi {
    stroke: var(--wire-hi);
    filter: url(#glow);
  }
  .wire.bus {
    stroke: var(--wire-bus);
  }
  .wire.busw {
    stroke-width: 3;
  }
  .wire.fb {
    stroke-dasharray: 6 3;
  }
  .wire:hover {
    stroke: var(--accent);
    stroke-width: 3.5;
  }
  .node {
    cursor: pointer;
  }
  .body {
    fill: var(--gate-fill);
    stroke: var(--gate-stroke);
    stroke-width: 1.5;
  }
  .body.on {
    stroke: var(--wire-hi);
  }
  .node.sel .body,
  .node.sel .bar {
    stroke: var(--accent);
    stroke-width: 2.5;
  }
  .node:hover .body {
    stroke: var(--accent);
  }
  .xorback,
  .clk {
    fill: none;
    stroke: var(--gate-stroke);
    stroke-width: 1.5;
  }
  .bubble {
    fill: var(--gate-fill);
    stroke: var(--gate-stroke);
    stroke-width: 1.5;
  }
  .bar {
    fill: var(--gate-stroke);
    stroke: var(--gate-stroke);
  }
  .const {
    fill: var(--const-fill);
    stroke: var(--const-stroke);
  }
  text {
    font-family: var(--mono);
    fill: var(--fg);
    pointer-events: none;
  }
  .sym {
    font-size: 15px;
    font-weight: 600;
  }
  .pin {
    font-size: 10px;
    fill: var(--fg-dim);
  }
  .small {
    font-size: 8.5px;
  }
  .name {
    font-size: 11px;
    font-weight: 600;
    fill: var(--syn-register);
  }
  .val {
    font-size: 10px;
    fill: var(--wire-hi);
  }
  .cval {
    font-size: 10px;
    fill: var(--syn-number);
  }
  .tiny {
    font-size: 8.5px;
    fill: var(--fg-dim);
  }
  .port {
    font-size: 11px;
    font-weight: 600;
    fill: var(--syn-port);
  }
  .group rect {
    fill: hsla(var(--h), 70%, 55%, 0.05);
    stroke: hsla(var(--h), 70%, 60%, 0.35);
    stroke-dasharray: 4 3;
  }
  .group text {
    font: 600 11px var(--sans);
    fill: hsla(var(--h), 80%, 70%, 0.9);
  }
  .group.sel rect {
    stroke: var(--accent);
    stroke-dasharray: none;
    fill: hsla(var(--h), 70%, 55%, 0.12);
  }
  .bar-top {
    position: absolute;
    top: 8px;
    left: 10px;
    right: 10px;
    display: flex;
    justify-content: space-between;
    align-items: center;
    font: 12px var(--sans);
    color: var(--fg-dim);
    pointer-events: none;
  }
  .bar-top button {
    pointer-events: auto;
  }
  .tip {
    position: absolute;
    background: var(--panel-raised);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 4px 8px;
    font: 12px var(--mono);
    color: var(--fg);
    pointer-events: none;
    box-shadow: var(--shadow);
  }
  .empty {
    display: grid;
    place-items: center;
    height: 100%;
    color: var(--fg-dim);
    font: 14px var(--sans);
  }
</style>
