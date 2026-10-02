<script lang="ts">
  // Transport controls (time travel through the Life universe) and circuit inputs.
  import { app, period, cycleAt } from './state.svelte';
  import { engine } from './engine';

  let jump = $state('');
  let jumpError = $state(false);
  const T = $derived(period());
  const g = $derived(Math.floor(app.gen));
  const cycle = $derived(cycleAt(g));
  const phase = $derived(((g % T) + T) % T);

  // Speed slider: log scale, generations per second.
  const speedExp = $derived(Math.log10(Math.max(1, app.speed)));
  const fmtSpeed = (s: number) => (s >= 1e6 ? `${(s / 1e6).toFixed(s >= 1e7 ? 0 : 1)}M` : s >= 1e3 ? `${(s / 1e3).toFixed(s >= 1e4 ? 0 : 1)}k` : String(s));

  function setGen(v: number) {
    app.gen = Math.max(0, v);
  }
  function stepCycle(d: number) {
    app.playing = false;
    setGen((cycle + d) * T + (d > 0 ? 0 : 0) + Math.min(phase, T - 1));
  }
  function stepGen(d: number) {
    app.playing = false;
    setGen(Math.floor(app.gen) + d);
  }
  function doJump() {
    const s = jump.trim().replace(/[_,\s]/g, '');
    let v: number | null = null;
    const m = /^(?:c|cycle)[:=]?(\d+)$/i.exec(s);
    if (m) v = Number(m[1]) * T;
    else if (/^\d+(\.\d+)?(e\d+)?$/i.test(s)) v = Math.floor(Number(s));
    else if (/^\d+(\.\d+)?[kmg]$/i.test(s)) v = Math.floor(Number(s.slice(0, -1)) * { k: 1e3, m: 1e6, g: 1e9 }[s.slice(-1).toLowerCase() as 'k' | 'm' | 'g']);
    jumpError = v === null || !Number.isFinite(v);
    if (!jumpError) {
      app.playing = false;
      setGen(v!);
      jump = '';
    }
  }

  // Inputs apply from the next cycle on (gliders of the current cycle are already in flight).
  async function setInput(port: number, value: bigint) {
    const d = app.design;
    if (!d) return;
    const w = d.inputs[port].width;
    const mask = (1n << BigInt(w)) - 1n;
    value &= mask;
    app.inputs[port] = value;
    const from = Math.max(0, cycle + (phase > 0 ? 1 : 0));
    await engine.setInput(port, value, from);
    app.simVersion++;
  }
  function toggleBit(port: number, bit: number) {
    setInput(port, (app.inputs[port] ?? 0n) ^ (1n << BigInt(bit)));
  }
  function parseVal(s: string): bigint | null {
    try {
      const t = s.trim().replace(/_/g, '');
      if (/^0x[0-9a-f]+$/i.test(t) || /^0b[01]+$/i.test(t) || /^\d+$/.test(t)) return BigInt(t);
    } catch {}
    return null;
  }
  const fmtG = (v: number) => v.toLocaleString('en-US');
</script>

<div class="simbar">
  <div class="transport">
    <button class="icon" title="Previous cycle" onclick={() => stepCycle(-1)} disabled={!app.design}>⏮</button>
    <button class="icon" title="Previous generation" onclick={() => stepGen(-1)} disabled={!app.design}>◀</button>
    <button class="play" class:on={app.playing} title="Play / pause (space)" onclick={() => (app.playing = !app.playing)} disabled={!app.design}>
      {app.playing ? '❚❚' : '▶'}
    </button>
    <button class="icon" title="Next generation" onclick={() => stepGen(1)} disabled={!app.design}>▶</button>
    <button class="icon" title="Next cycle" onclick={() => stepCycle(1)} disabled={!app.design}>⏭</button>
    <label class="speed" title="Generations per second">
      <input type="range" min="0" max="9" step="0.05" value={speedExp} oninput={(e) => (app.speed = Math.round(10 ** Number((e.target as HTMLInputElement).value)))} />
      <span>{fmtSpeed(app.speed)} gen/s</span>
    </label>
  </div>
  <div class="time">
    <div class="readout">
      <span class="lbl">generation</span>
      <span class="big">{fmtG(g)}</span>
      <span class="lbl">cycle</span>
      <span class="big c">{cycle}</span>
      <span class="lbl">phase</span>
      <span class="mid">{fmtG(phase)} / {fmtG(T)}</span>
    </div>
    <div class="progress" title="Position within the clock cycle">
      <div class="fill" style="width:{(phase / T) * 100}%"></div>
    </div>
  </div>
  <form class="jump" onsubmit={(e) => (e.preventDefault(), doJump())}>
    <input placeholder="jump: 700000, 2.5M, c:12" bind:value={jump} class:err={jumpError} disabled={!app.design} />
  </form>
  {#if app.design && app.design.inputs.length}
    <div class="inputs">
      {#each app.design.inputs as p, i}
        <div class="port">
          <span class="pname">{p.name}</span>
          {#if p.width <= 8}
            <div class="bits">
              {#each Array(p.width) as _, b}
                {@const bit = p.width - 1 - b}
                <button class="bit" class:on={((app.inputs[i] ?? 0n) >> BigInt(bit)) & 1n} title="{p.name}[{bit}]" onclick={() => toggleBit(i, bit)}>
                  {((app.inputs[i] ?? 0n) >> BigInt(bit)) & 1n}
                </button>
              {/each}
            </div>
          {/if}
          {#if p.width > 1}
            <input
              class="num"
              value={String(app.inputs[i] ?? 0n)}
              onchange={(e) => {
                const v = parseVal((e.target as HTMLInputElement).value);
                if (v !== null) setInput(i, v);
              }}
            />
          {/if}
        </div>
      {/each}
      <span class="note">inputs take effect from the next clock cycle</span>
    </div>
  {/if}
</div>

<style>
  .simbar {
    display: flex;
    align-items: center;
    gap: 18px;
    padding: 6px 14px;
    background: var(--panel);
    border-top: 1px solid var(--border);
    font: 12px var(--sans);
    color: var(--fg);
    flex-wrap: wrap;
    min-height: 54px;
  }
  .transport {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .icon {
    width: 30px;
    height: 28px;
    padding: 0;
  }
  .play {
    width: 42px;
    height: 34px;
    border-radius: 50%;
    font-size: 14px;
    background: var(--accent);
    color: var(--accent-ink);
    border: 0;
    margin: 0 4px;
    box-shadow: 0 0 14px var(--accent-soft);
  }
  .play.on {
    background: var(--accent-2);
  }
  .speed {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-left: 10px;
    color: var(--fg-dim);
  }
  .speed input {
    width: 110px;
  }
  .speed span {
    font-family: var(--mono);
    min-width: 76px;
  }
  .time {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 330px;
  }
  .readout {
    display: flex;
    align-items: baseline;
    gap: 6px;
  }
  .lbl {
    color: var(--fg-faint);
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .big {
    font: 600 15px var(--mono);
    margin-right: 8px;
  }
  .big.c {
    color: var(--accent);
  }
  .mid {
    font: 12px var(--mono);
    color: var(--fg-dim);
  }
  .progress {
    height: 4px;
    background: var(--border-soft);
    border-radius: 2px;
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: linear-gradient(90deg, var(--accent), var(--accent-2));
  }
  .jump input {
    width: 190px;
    font-family: var(--mono);
  }
  .jump input.err {
    border-color: var(--err);
  }
  .inputs {
    display: flex;
    align-items: center;
    gap: 14px;
    flex-wrap: wrap;
  }
  .port {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .pname {
    font: 600 12px var(--mono);
    color: var(--syn-port);
  }
  .bits {
    display: flex;
    gap: 2px;
  }
  .bit {
    width: 20px;
    height: 24px;
    padding: 0;
    font: 12px var(--mono);
  }
  .bit.on {
    background: var(--accent);
    color: var(--accent-ink);
    border-color: var(--accent);
  }
  .num {
    width: 70px;
    font-family: var(--mono);
  }
  .note {
    color: var(--fg-faint);
    font-size: 11px;
  }
</style>
