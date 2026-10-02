<script lang="ts">
  // Waveforms of inputs, outputs, registers and named signals over clock cycles.
  import { app, cycleAt, period } from './state.svelte';
  import { engine } from './engine';
  import type { TraceRow } from './types';

  let canvas: HTMLCanvasElement;
  let wrap: HTMLDivElement;
  let rows: TraceRow[] = $state([]);
  let showProbes = $state(true);
  let cellW = $state(34);
  let scrollX = $state(0);
  let radix: 'hex' | 'dec' = $state('hex');
  let size = $state({ w: 600, h: 300 });

  const cycle = $derived(cycleAt(Math.floor(app.gen)));
  const nCycles = $derived(Math.max(cycle + 12, 32));

  interface Sig {
    name: string;
    width: number;
    kind: 'in' | 'out' | 'reg' | 'probe';
    get: (r: TraceRow) => bigint;
  }

  const signals = $derived.by((): Sig[] => {
    const d = app.design;
    if (!d) return [];
    const s: Sig[] = [];
    d.inputs.forEach((p, i) => s.push({ name: p.name, width: p.width, kind: 'in', get: (r) => BigInt(r.inputs[i]) }));
    d.outputs.forEach((p, i) => s.push({ name: p.name, width: p.width, kind: 'out', get: (r) => BigInt(r.outputs[i]) }));
    d.regs.forEach((p, i) => s.push({ name: p.name, width: p.width, kind: 'reg', get: (r) => BigInt(r.regs[i]) }));
    if (showProbes) {
      const seen = new Set([...d.inputs, ...d.outputs, ...d.regs].map((p) => p.name));
      d.probes.forEach((p, i) => {
        if (!seen.has(p.name)) {
          seen.add(p.name);
          s.push({ name: p.name, width: p.width, kind: 'probe', get: (r) => BigInt(r.values[i] ?? '0') });
        }
      });
    }
    return s;
  });

  $effect(() => {
    const d = app.design;
    void app.simVersion;
    const n = nCycles;
    if (!d) return;
    engine.trace(0, n, d.probes.map((p) => p.rid)).then((r) => (rows = r));
  });

  $effect(() => {
    if (!wrap) return;
    const ro = new ResizeObserver(() => (size = { w: wrap.clientWidth, h: wrap.clientHeight }));
    ro.observe(wrap);
    return () => ro.disconnect();
  });

  const NAME_W = 120;
  const ROW_H = 26;
  const HEAD = 24;

  function fmt(v: bigint, w: number) {
    if (w === 1) return String(v);
    return radix === 'hex' ? v.toString(16).toUpperCase() : v.toString();
  }

  $effect(() => {
    // Redraw on any dependency change.
    const sigs = signals;
    const rs = rows;
    const cw = cellW;
    const sx = scrollX;
    const cur = cycle;
    const phase = (app.gen % period()) / period();
    void size;
    void radix;
    if (!canvas) return;
    const css = getComputedStyle(canvas);
    const col = (v: string) => css.getPropertyValue(v).trim() || '#888';
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const W = wrap.clientWidth;
    const H = Math.max(wrap.clientHeight, HEAD + sigs.length * ROW_H + 10);
    canvas.width = W * dpr;
    canvas.height = H * dpr;
    canvas.style.height = H + 'px';
    const ctx = canvas.getContext('2d')!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, W, H);
    ctx.font = `11px ${col('--mono')}`;
    const x0 = NAME_W - sx;
    // Cycle grid.
    ctx.fillStyle = col('--fg-faint');
    ctx.strokeStyle = col('--border-soft');
    ctx.lineWidth = 1;
    for (let c = 0; c < rs.length; c++) {
      const x = x0 + c * cw;
      if (x < NAME_W - cw || x > W) continue;
      ctx.beginPath();
      ctx.moveTo(x + 0.5, HEAD - 4);
      ctx.lineTo(x + 0.5, H);
      ctx.stroke();
      if (cw > 22 || c % 5 === 0) ctx.fillText(String(c), x + 3, 14);
    }
    // Current position marker.
    const cx = x0 + (cur + phase) * cw;
    ctx.fillStyle = col('--accent-soft');
    ctx.fillRect(x0 + cur * cw, HEAD - 4, cw, H);
    ctx.strokeStyle = col('--accent');
    ctx.beginPath();
    ctx.moveTo(cx + 0.5, HEAD - 6);
    ctx.lineTo(cx + 0.5, H);
    ctx.stroke();
    // Signals.
    sigs.forEach((s, k) => {
      const y = HEAD + k * ROW_H;
      const color = s.kind === 'in' ? col('--syn-port') : s.kind === 'out' ? col('--accent') : s.kind === 'reg' ? col('--syn-register') : col('--syn-function');
      ctx.strokeStyle = color;
      ctx.fillStyle = color;
      ctx.lineWidth = 1.5;
      const top = y + 5;
      const bot = y + ROW_H - 6;
      let prev: bigint | null = null;
      for (let c = 0; c < rs.length; c++) {
        const x = x0 + c * cw;
        if (x > W) break;
        const v = s.get(rs[c]);
        if (x + cw < NAME_W) {
          prev = v;
          continue;
        }
        if (s.width === 1) {
          const yv = v ? top : bot;
          ctx.beginPath();
          if (prev !== null && prev !== v) {
            ctx.moveTo(x, prev ? top : bot);
            ctx.lineTo(x, yv);
          } else ctx.moveTo(x, yv);
          ctx.lineTo(x + cw, yv);
          ctx.stroke();
          if (v) {
            ctx.globalAlpha = 0.12;
            ctx.fillRect(x, top, cw, bot - top);
            ctx.globalAlpha = 1;
          }
        } else {
          const change = prev === null || prev !== v;
          const mid = (top + bot) / 2;
          ctx.beginPath();
          if (change) {
            ctx.moveTo(x, mid);
            ctx.lineTo(x + 3, top);
            ctx.lineTo(x + cw, top);
            ctx.moveTo(x, mid);
            ctx.lineTo(x + 3, bot);
            ctx.lineTo(x + cw, bot);
          } else {
            ctx.moveTo(x, top);
            ctx.lineTo(x + cw, top);
            ctx.moveTo(x, bot);
            ctx.lineTo(x + cw, bot);
          }
          ctx.stroke();
          if (change) {
            // Label spans until the next change.
            let e = c + 1;
            while (e < rs.length && s.get(rs[e]) === v) e++;
            const span = (e - c) * cw - 8;
            const label = fmt(v, s.width);
            if (ctx.measureText(label).width < span) {
              ctx.fillStyle = col('--fg');
              ctx.fillText(label, x + 6, mid + 4);
              ctx.fillStyle = color;
            }
          }
        }
        prev = v;
      }
    });
    // Names column.
    ctx.fillStyle = col('--panel');
    ctx.fillRect(0, 0, NAME_W, H);
    ctx.strokeStyle = col('--border-soft');
    ctx.beginPath();
    ctx.moveTo(NAME_W + 0.5, 0);
    ctx.lineTo(NAME_W + 0.5, H);
    ctx.stroke();
    sigs.forEach((s, k) => {
      const y = HEAD + k * ROW_H;
      ctx.fillStyle = col('--fg-dim');
      ctx.font = `10px ${col('--sans')}`;
      ctx.fillText(s.kind, 8, y + ROW_H / 2 + 4);
      ctx.fillStyle = col('--fg');
      ctx.font = `12px ${col('--mono')}`;
      ctx.fillText(s.name.length > 11 ? s.name.slice(0, 10) + '…' : s.name, 40, y + ROW_H / 2 + 4);
      const cv = rs[cur] ? fmt(s.get(rs[cur]), s.width) : '';
      ctx.fillStyle = col('--accent');
      ctx.font = `10px ${col('--mono')}`;
      ctx.textAlign = 'right';
      ctx.fillText(cv, NAME_W - 6, y + ROW_H / 2 + 4);
      ctx.textAlign = 'left';
    });
  });

  function onclick(e: MouseEvent) {
    const r = canvas.getBoundingClientRect();
    const x = e.clientX - r.left;
    if (x < NAME_W) return;
    const c = Math.floor((x - NAME_W + scrollX) / cellW);
    if (c >= 0) app.gen = c * period() + period() * 0.5;
  }
  function onwheel(e: WheelEvent) {
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault();
      cellW = Math.min(120, Math.max(4, cellW * Math.exp(-e.deltaY * 0.002)));
    } else if (Math.abs(e.deltaX) > Math.abs(e.deltaY) || e.shiftKey) {
      e.preventDefault();
      scrollX = Math.max(0, scrollX + (e.shiftKey ? e.deltaY : e.deltaX));
    }
  }
  // Follow the cursor.
  $effect(() => {
    const x = (cycle + 1) * cellW;
    const w = size.w - NAME_W;
    if (x - scrollX > w) scrollX = x - w + cellW * 2;
    else if (cycle * cellW < scrollX) scrollX = Math.max(0, cycle * cellW - cellW);
  });
</script>

<div class="waves">
  <div class="tools">
    <label><input type="checkbox" bind:checked={showProbes} /> named signals</label>
    <label>radix <select bind:value={radix}><option value="hex">hex</option><option value="dec">dec</option></select></label>
    <span class="hint">click to jump to a cycle · shift+wheel to scroll · ctrl+wheel to zoom</span>
  </div>
  <div class="scroll" bind:this={wrap} {onwheel}>
    {#if app.design}
      <canvas bind:this={canvas} {onclick}></canvas>
    {:else}
      <div class="empty">Compile a design to see its waveforms.</div>
    {/if}
  </div>
</div>

<style>
  .waves {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--panel);
  }
  .tools {
    display: flex;
    gap: 16px;
    align-items: center;
    padding: 6px 12px;
    font: 12px var(--sans);
    color: var(--fg-dim);
    border-bottom: 1px solid var(--border-soft);
  }
  .hint {
    margin-left: auto;
    color: var(--fg-faint);
  }
  .scroll {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    min-height: 0;
  }
  canvas {
    display: block;
    width: 100%;
    cursor: pointer;
  }
  .empty {
    display: grid;
    place-items: center;
    height: 100%;
    color: var(--fg-dim);
    font: 14px var(--sans);
  }
</style>
