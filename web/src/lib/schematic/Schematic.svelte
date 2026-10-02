<script lang="ts">
  // Logic diagram with ANSI/IEEE distinctive-shape symbols. It is hierarchical: a scope shows
  // the operators of one group with its sub-groups collapsed into boxes; double-click a box to
  // enter it, and an operator (adder, multiplexer, comparator…) to see its gates.
  import { app, ui, cycleAt, groupLabel, groupPath } from '../state.svelte';
  import { engine } from '../engine';
  import { groupBox } from '../geom';
  import Icon from '../ui/Icon.svelte';
  import type { SNode, SWire, SchematicData } from '../types';

  let wrap: HTMLDivElement;
  let view = $state({ x: 0, y: 0, k: 1 });
  let values: Map<number, bigint> = $state(new Map());
  let hover: { x: number; y: number; text: string; sub: string } | null = $state(null);
  let flat = $state(false);
  let sch: SchematicData | null = $state(null);
  let loading = $state(false);
  let fitted: unknown = null;

  const cycle = $derived(cycleAt(Math.floor(app.gen)));
  const nav = $derived(ui.schematic);
  const local = $derived(!!sch?.local);

  // Load the diagram for the current navigation state.
  let seq = 0;
  $effect(() => {
    const d = app.design;
    const n = nav;
    const f = flat;
    const c = n.gates !== null ? cycle : 0;
    void (n.gates !== null && app.simVersion);
    if (!d) {
      sch = null;
      return;
    }
    if (f && n.gates === null) {
      sch = d.schematic;
      return;
    }
    const my = ++seq;
    loading = true;
    const p = n.gates !== null ? engine.gates(n.gates, c) : engine.scope(n.scope);
    p.then((s) => {
      if (my !== seq) return;
      if (s.error) {
        ui.schematic = { scope: n.scope, gates: null };
        return;
      }
      sch = s;
    }).finally(() => my === seq && (loading = false));
  });

  // Values of every wire for the current cycle (design nodes; gate views carry their own).
  $effect(() => {
    const s = sch;
    const c = cycle;
    void app.simVersion;
    if (!s) return;
    if (s.local) {
      const m = new Map<number, bigint>();
      (s.values ?? []).forEach((v, i) => m.set(i, BigInt(v)));
      values = m;
      return;
    }
    const rids = [...new Set([...s.wires.map((w) => w.rid), ...s.nodes.map((n) => n.rid)].filter((r): r is number => r !== null))];
    engine.trace(c, c + 1, rids).then((rows) => {
      const m = new Map<number, bigint>();
      rids.forEach((r, i) => m.set(r, BigInt(rows[0]?.values[i] ?? '0')));
      values = m;
    });
  });

  $effect(() => {
    const s = sch;
    const key = s && `${flat}|${nav.scope}|${nav.gates}|${s.size}`;
    if (s && fitted !== key && wrap) {
      fitted = key;
      fit();
    }
  });

  function fit() {
    if (!sch || !wrap) return;
    const [w, h] = sch.size;
    const k = Math.min(wrap.clientWidth / (w + 60), (wrap.clientHeight - 60) / (h + 60), 1.5);
    view = { k, x: (wrap.clientWidth - w * k) / 2, y: Math.max(56, (wrap.clientHeight - h * k) / 2 + 20) };
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
    zoomAt(e.clientX - r.left, e.clientY - r.top, Math.exp(-e.deltaY * 0.0015));
  }
  function zoomAt(sx: number, sy: number, f: number) {
    const k = Math.min(8, Math.max(0.05, view.k * f));
    view = { k, x: sx - ((sx - view.x) * k) / view.k, y: sy - ((sy - view.y) * k) / view.k };
  }

  const fmt = (v: bigint | undefined, w: number) => (v === undefined ? '?' : w === 1 ? String(v) : w <= 4 ? String(v) : '0x' + v.toString(16));
  const val = (rid: number | null) => (rid === null ? undefined : values.get(rid));

  function wireClass(w: SWire) {
    if (w.width > 1) return 'bus';
    const v = val(w.rid);
    return v === undefined ? 'unk' : v ? 'hi' : 'lo';
  }

  /** Operators that have a gate-level view. */
  function drillable(n: SNode) {
    if (local || n.rid === null) return false;
    if (n.kind === 'box') return true;
    if (['arith', 'cmp', 'shift', 'mux'].includes(n.kind)) return true;
    if (['and', 'or', 'xor', 'not'].includes(n.kind)) return n.width > 1 || n.inNames.length === 0;
    return false;
  }
  function open(n: SNode) {
    if (n.kind === 'box') {
      flat = false;
      ui.schematic = { scope: n.group, gates: null };
    } else if (drillable(n) && n.rid !== null) {
      ui.schematic = { scope: nav.scope, gates: n.rid };
    }
    hover = null;
  }
  function selectNode(n: SNode) {
    if (local) return;
    const d = app.design;
    if (n.kind === 'box' || (n.group !== 0 && d?.groups[n.group])) {
      app.selectedGroup = n.group;
      const sp = d?.groups[n.group]?.span;
      if (sp) app.highlight = sp;
    } else if (n.span[1] > n.span[0]) app.highlight = n.span;
  }
  function locate() {
    const d = app.design;
    const g = app.selectedGroup ?? nav.scope;
    if (!d) return;
    const b = groupBox(d, g);
    if (b) {
      app.frame = b;
      app.view = 'life';
    }
  }

  const crumbs = $derived(app.design ? groupPath(app.design, nav.scope) : []);
  const gateTitle = $derived(nav.gates !== null && app.design ? (app.design.rtlOps[nav.gates] ?? 'operator') : '');

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
    if (!sch || !app.design || !flat || local) return [];
    const m = new Map<number, [number, number, number, number]>();
    for (const n of sch.nodes) {
      if (n.kind === 'const' || n.kind === 'input' || n.kind === 'output' || n.group === 0) continue;
      const b = m.get(n.group);
      m.set(n.group, b ? [Math.min(b[0], n.x), Math.min(b[1], n.y), Math.max(b[2], n.x + n.w), Math.max(b[3], n.y + n.h)] : [n.x, n.y, n.x + n.w, n.y + n.h]);
    }
    return [...m.entries()].filter(([g]) => app.design!.groups[g]).map(([g, b]) => ({ g, name: groupLabel(app.design!, g), b }));
  });

  function nodeTip(n: SNode, e: PointerEvent) {
    const v = val(n.rid);
    const d = app.design;
    let text = '';
    let sub = '';
    if (n.kind === 'box' && d) {
      text = groupLabel(d, n.group);
      sub = `${d.groups[n.group]?.kind ?? 'group'} · double-click to open`;
    } else {
      const what = !local && n.rid !== null && d ? (d.rtlOps[n.rid] ?? n.kind) : n.kind.toUpperCase();
      text = `${what}${n.label && !['input', 'output'].includes(n.kind) ? ' ' + n.label : n.kind === 'input' || n.kind === 'output' ? ' ' + n.label : ''} = ${fmt(v, n.width)}`;
      sub = drillable(n) ? 'double-click for its gates' : '';
    }
    hover = { x: e.offsetX + 14, y: e.offsetY + 14, text, sub };
  }
</script>

<div class="schematic" bind:this={wrap} {onwheel} role="presentation">
  {#if sch}
    <svg {onpointerdown} {onpointermove} onpointerup={() => (drag = null)} role="presentation">
      <g transform="translate({view.x},{view.y}) scale({view.k})">
        {#each groupBoxes as gb}
          <g class="group" class:sel={app.selectedGroup === gb.g}>
            <rect x={gb.b[0] - 10} y={gb.b[1] - 22} width={gb.b[2] - gb.b[0] + 20} height={gb.b[3] - gb.b[1] + 32} rx="8" />
            <text x={gb.b[0] - 4} y={gb.b[1] - 9}>{gb.name}</text>
          </g>
        {/each}
        {#each sch.wires as w}
          <path
            class="wire {wireClass(w)}"
            class:fb={w.feedback}
            class:busw={w.width > 1}
            d={path(w.points)}
            role="presentation"
            onpointerenter={(e) => (hover = { x: e.offsetX + 14, y: e.offsetY + 14, text: `${fmt(val(w.rid), w.width)}`, sub: `${w.width} bit${w.width > 1 ? 's' : ''}` })}
            onpointerleave={() => (hover = null)}
          />
        {/each}
        {#each sch.nodes as n}
          {@const v = val(n.rid)}
          <g
            class="node k-{n.kind}"
            class:sel={!local && app.selectedGroup !== null && app.selectedGroup === n.group && n.group !== 0}
            class:drill={drillable(n)}
            transform="translate({n.x},{n.y})"
            role="button"
            tabindex="-1"
            onclick={() => selectNode(n)}
            ondblclick={() => open(n)}
            onkeydown={(e) => e.key === 'Enter' && open(n)}
            onpointerenter={(e) => nodeTip(n, e)}
            onpointerleave={() => (hover = null)}
          >
            {#if n.kind === 'box'}
              <rect width={n.w} height={n.h} rx="6" class="boxr" />
              <text x={n.w / 2} y={n.h / 2 + 4} class="boxl" text-anchor="middle">{n.label}</text>
              <text x="6" y="-5" class="tiny">{app.design?.groups[n.group]?.kind ?? ''}</text>
              <path d="M{n.w - 12},5 h7 v7 M{n.w - 5},5 l-6,6" class="openmark" />
            {:else if n.kind === 'dff'}
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
                <text x={n.kind === 'input' ? 7 : 15} y={n.h / 2 + 4} class="port" class:on={n.width === 1 && v === 1n}>{n.label}</text>
              {:else if n.kind === 'mux'}
                {#each n.inNames as nm, i}
                  <text x="4" y={n.ins[i][1] - n.y + 3} class="pin small">{nm}</text>
                {/each}
              {:else if n.label}
                <text x={n.kind === 'and' || n.kind === 'or' || n.kind === 'xor' ? n.w * 0.4 : n.w / 2} y={n.h / 2 + 5} class="sym" text-anchor="middle">{n.label}</text>
              {/if}
              {#if drillable(n)}
                <path d="M{n.w - 11},{n.h - 4} h-4 M{n.w - 13},{n.h - 6} v4" class="openmark" />
              {/if}
            {/if}
          </g>
        {/each}
      </g>
    </svg>
  {:else if !app.design}
    <div class="empty">Compile a design to see its schematic.</div>
  {/if}

  {#if app.design}
    <div class="crumbs">
      {#if !flat || nav.gates !== null}
        {#each crumbs as g, k}
          {#if k}<span class="sep"><Icon name="chevron-right" size={12} /></span>{/if}
          <button class="crumb" class:cur={g === nav.scope && nav.gates === null} onclick={() => (ui.schematic = { scope: g, gates: null })}>{groupLabel(app.design, g)}</button>
        {/each}
        {#if nav.gates !== null}
          <span class="sep"><Icon name="chevron-right" size={12} /></span>
          <span class="crumb cur">gates of the {gateTitle}{sch?.gates ? ` · ${sch.gates}` : ''}</span>
        {/if}
      {:else}
        <span class="crumb cur">{app.design.name} · all operators</span>
      {/if}
      <span class="cyc mono">cycle {cycle}</span>
      {#if loading}<span class="spinner"></span>{/if}
    </div>
    <div class="toolbar">
      <div class="seg">
        <button class:on={!flat} onclick={() => (flat = false)} title="Sub-groups collapsed into boxes">Hierarchy</button>
        <button class:on={flat} onclick={() => ((flat = true), (ui.schematic = { scope: 0, gates: null }))} title="Every operator of the design">Flat</button>
      </div>
      <span class="vsep"></span>
      {#if nav.gates !== null || nav.scope !== 0}
        <button class="tbi" title="Up one level" onclick={() => (ui.schematic = nav.gates !== null ? { scope: nav.scope, gates: null } : { scope: app.design?.groups[nav.scope]?.parent ?? 0, gates: null })}><Icon name="arrow-up-left" size={14} /></button>
      {/if}
      <button class="tbi" title="Show the selection in the universe" onclick={locate}><Icon name="crosshair" size={14} /></button>
      <button class="tbi" title="Zoom out" onclick={() => zoomAt(wrap.clientWidth / 2, wrap.clientHeight / 2, 1 / 1.4)}><Icon name="minus" /></button>
      <button class="tb mono" title="Fit" onclick={fit}>{Math.round(view.k * 100)}%</button>
      <button class="tbi" title="Zoom in" onclick={() => zoomAt(wrap.clientWidth / 2, wrap.clientHeight / 2, 1.4)}><Icon name="plus" /></button>
    </div>
  {/if}
  {#if hover}
    <div class="tip" style="left:{hover.x}px; top:{hover.y}px"><span class="mono">{hover.text}</span>{#if hover.sub}<span class="k">{hover.sub}</span>{/if}</div>
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
  .wire.lo,
  .wire.unk {
    stroke: var(--wire-lo);
  }
  .wire.hi {
    stroke: var(--wire-hi);
    stroke-width: 1.8;
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
  .body,
  .boxr {
    fill: var(--gate-fill);
    stroke: var(--gate-stroke);
    stroke-width: 1.5;
  }
  .boxr {
    stroke-dasharray: none;
  }
  .body.on {
    stroke: var(--wire-hi);
  }
  .node.sel .body,
  .node.sel .bar,
  .node.sel .boxr {
    stroke: var(--accent);
    stroke-width: 2.2;
  }
  .node:hover .body,
  .node:hover .boxr {
    stroke: var(--accent);
  }
  .node.k-box:hover .boxr {
    fill: var(--accent-soft);
  }
  .openmark {
    fill: none;
    stroke: var(--fg-faint);
    stroke-width: 1.2;
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
    font-weight: 500;
  }
  .boxl {
    font-size: 11.5px;
    font-weight: 500;
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
    font-weight: 500;
    fill: var(--syn-register);
  }
  .val {
    font-size: 10px;
    fill: var(--accent);
  }
  .cval {
    font-size: 10px;
    fill: var(--syn-number);
  }
  .tiny {
    font-size: 9px;
    fill: var(--fg-faint);
    font-family: var(--sans);
  }
  .port {
    font-size: 11px;
    font-weight: 500;
    fill: var(--syn-port);
  }
  .port.on {
    fill: var(--accent);
  }
  .group rect {
    fill: transparent;
    stroke: var(--frame);
    stroke-dasharray: 4 3;
  }
  .group text {
    font: 500 11px var(--sans);
    fill: var(--fg-dim);
  }
  .group.sel rect {
    stroke: var(--accent);
    stroke-dasharray: none;
    fill: var(--accent-soft);
  }
  .crumbs {
    position: absolute;
    top: 12px;
    left: 12px;
    right: 360px;
    display: flex;
    align-items: center;
    gap: 2px;
    flex-wrap: wrap;
    padding: 4px 6px;
    border-radius: 8px;
    background: var(--hud);
    font-size: 12px;
    width: fit-content;
    max-width: calc(100% - 380px);
  }
  .crumb {
    padding: 2px 5px;
    border-radius: 5px;
    color: var(--fg-dim);
    font-family: var(--mono);
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  button.crumb:hover {
    background: var(--hover);
    color: var(--fg);
  }
  .crumb.cur {
    color: var(--fg);
  }
  .sep {
    color: var(--fg-faint);
    display: inline-flex;
  }
  .cyc {
    margin-left: 8px;
    color: var(--fg-faint);
    font-size: 11.5px;
  }
  .toolbar {
    position: absolute;
    top: 12px;
    right: 12px;
    display: flex;
    align-items: center;
    padding: 3px;
    gap: 2px;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: var(--shadow-sm);
  }
  .toolbar .seg {
    border: 0;
    background: transparent;
  }
  .tb {
    height: 28px;
    padding: 0 8px;
    border-radius: 7px;
    font-size: 11.5px;
    min-width: 54px;
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
  .tbi:hover {
    background: var(--hover);
  }
  .vsep {
    width: 1px;
    align-self: stretch;
    margin: 4px 2px;
    background: var(--border);
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
    white-space: nowrap;
  }
  .tip .k {
    opacity: 0.7;
  }
  .empty {
    display: grid;
    place-items: center;
    height: 100%;
    color: var(--fg-dim);
    font-size: 14px;
  }
</style>
