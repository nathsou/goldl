<script lang="ts">
  // Transport: step, play, an editable cycle/generation readout, a scrubber over clock cycles
  // (Shift-drag scrubs inside the current cycle), the timing popover and "go to".
  import Icon from './ui/Icon.svelte';
  import Switch from './ui/Switch.svelte';
  import { app, ui, settings, period, cycleAt, paneVisible } from './state.svelte';
  import { setInput, parseJump, parseValue } from './actions.svelte';
  import { togglePlay, step, stepCycle, setGen, setLoop, fmtSpeed, fmtStep, fmtNum, fmtCycles, clampSpeed, parseAmount, stepGens, genPerSec } from './timing.svelte';

  let { width, floating }: { width: number; floating: boolean } = $props();

  let jumpErr = $state(false);
  let editing: 'gen' | 'cycle' | null = $state(null);
  const T = $derived(period());
  const g = $derived(Math.floor(app.gen));
  const cycle = $derived(cycleAt(g));
  const phase = $derived(((g % T) + T) % T);
  const span = $derived(16 * Math.max(1, Math.ceil((cycle + 1) / 16)));
  const pct = $derived(Math.min(100, ((cycle + phase / T) / span) * 100));
  const ticks = $derived([0, 1, 2, 3, 4].map((k) => (span / 4) * k));
  const narrow = $derived(!floating && width < 600);
  const tiny = $derived(!floating && width < 440);
  const tm = $derived(settings.timing);
  const fmt = (n: number) => n.toLocaleString('en-US');

  // ---- Scrubber ----
  let scrub: { fine: boolean; c0: number } | null = null;
  let track: HTMLDivElement;
  let fineScrub = $state(false);
  function scrubTo(e: PointerEvent) {
    const r = track.getBoundingClientRect();
    const f = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
    if (scrub?.fine) setGen(scrub.c0 * T + Math.floor(f * (T - 1)));
    else setGen(Math.floor(f * span * T));
  }
  function onscrubwheel(e: WheelEvent) {
    e.preventDefault();
    const d = (e.deltaY || e.deltaX) > 0 ? 1 : -1;
    if (e.shiftKey) stepCycle(d);
    else step(d);
  }

  // ---- Editable readout ----
  function commitEdit(e: Event) {
    const inp = e.currentTarget as HTMLInputElement;
    const v = inp.value.trim();
    if (editing === 'cycle') {
      const c = parseAmount(v);
      if (c !== null) setGen(Math.floor(c) * T + phase);
    } else {
      const x = parseJump(v);
      if (x !== null) setGen(x);
    }
    app.playing = false;
    editing = null;
  }
  function focusSelect(node: HTMLInputElement) {
    node.focus();
    node.select();
  }

  function jump(e: SubmitEvent) {
    e.preventDefault();
    const inp = (e.currentTarget as HTMLFormElement).elements.namedItem('jump') as HTMLInputElement;
    const v = parseJump(inp.value);
    jumpErr = v === null || !Number.isFinite(v);
    if (!jumpErr) {
      app.playing = false;
      setGen(v!);
      inp.value = '';
      inp.blur();
    }
  }

  // ---- Timing popover ----
  let speedBtn: HTMLButtonElement;
  let pop = $state({ x: 0, bottom: 0 });
  function openTiming() {
    if (ui.menu === 'timing') {
      ui.menu = null;
      return;
    }
    const r = speedBtn.getBoundingClientRect();
    pop = { x: Math.max(8, Math.min(window.innerWidth - 328, r.right - 320)), bottom: window.innerHeight - r.top + 8 };
    ui.menu = 'timing';
    ui.paneMenu = null;
  }
  const maxLog = $derived(Math.log10(64 * T));
  const speedLog = $derived(Math.log10(Math.max(1, genPerSec())));
  function setSpeedFromLog(x: number) {
    const gps = 10 ** x;
    settings.timing.speed = clampSpeed(tm.speedUnit === 'cycle' ? gps / T : gps, tm.speedUnit);
  }
  function setSpeedUnit(u: 'gen' | 'cycle') {
    if (u === tm.speedUnit) return;
    const gps = genPerSec();
    settings.timing.speedUnit = u;
    settings.timing.speed = clampSpeed(u === 'cycle' ? gps / T : gps, u);
  }
  function setStepUnit(u: 'gen' | 'cycle') {
    if (u === tm.stepUnit) return;
    const n = stepGens();
    settings.timing.stepUnit = u;
    settings.timing.step = u === 'cycle' ? Number((n / T).toPrecision(3)) : n;
  }
  const SPEEDS: [number, 'gen' | 'cycle', string][] = [
    [1, 'gen', '1 gen/s'],
    [60, 'gen', '60 gen/s'],
    [1000, 'gen', '1k gen/s'],
    [0.125, 'cycle', '⅛ cyc/s'],
    [0.5, 'cycle', '½ cyc/s'],
    [1, 'cycle', '1 cyc/s'],
    [4, 'cycle', '4 cyc/s'],
  ];
  const STEPS: [number, 'gen' | 'cycle', string][] = [
    [1, 'gen', '1 gen'],
    [10, 'gen', '10'],
    [100, 'gen', '100'],
    [1000, 'gen', '1k'],
    [1 / 16, 'cycle', '1/16 cyc'],
    [1 / 4, 'cycle', '¼ cyc'],
  ];
  const isOn = (v: number, u: string, cv: number, cu: string) => u === cu && Math.abs(v - cv) < 1e-9;
</script>

<div class="transport" class:floating>
  <div class="steps">
    <button class="st w30" title="Previous cycle (Shift+←)" onclick={() => stepCycle(-1)} disabled={!app.design}><Icon name="skip-back" size={14} /></button>
    <button class="st" title="Back {fmtStep()} (←)" onclick={() => step(-1)} disabled={!app.design}><Icon name="chevron-left" size={14} /></button>
    <button class="play" class:rev={tm.reverse} title="{app.playing ? 'Pause' : tm.reverse ? 'Play backwards' : 'Play'} (Space)" onclick={togglePlay} disabled={!app.design}>
      <Icon name={app.playing ? 'pause' : 'play'} size={15} />
    </button>
    <button class="st" title="Forward {fmtStep()} (→)" onclick={() => step(1)} disabled={!app.design}><Icon name="chevron-right" size={14} /></button>
    <button class="st w30" title="Next cycle (Shift+→)" onclick={() => stepCycle(1)} disabled={!app.design}><Icon name="skip-forward" size={14} /></button>
  </div>
  <div class="readout">
    {#if editing === 'cycle'}
      <input class="edit mono big" value={String(cycle)} use:focusSelect onkeydown={(e) => (e.key === 'Enter' ? commitEdit(e) : e.key === 'Escape' && (editing = null))} onblur={() => (editing = null)} />
    {:else}
      <button class="cyc" title="Click to go to a cycle" onclick={() => (editing = 'cycle')} disabled={!app.design}>cycle {cycle}<span class="ph">&nbsp;· {Math.floor((phase / T) * 100)}%</span></button>
    {/if}
    {#if editing === 'gen'}
      <input class="edit mono" value={String(g)} use:focusSelect onkeydown={(e) => (e.key === 'Enter' ? commitEdit(e) : e.key === 'Escape' && (editing = null))} onblur={() => (editing = null)} />
    {:else}
      <button class="gen" title="Click to go to a generation (700k, 2.5M, c:12)" onclick={() => (editing = 'gen')} disabled={!app.design}>gen {fmt(g)}</button>
    {/if}
  </div>
  {#if !tiny}
    <div
      class="scrub"
      class:fixed={floating}
      class:fine={fineScrub}
      bind:this={track}
      role="slider"
      aria-valuenow={cycle}
      tabindex="-1"
      title="Drag to move through cycles · Shift-drag inside the current cycle · wheel steps"
      onwheel={onscrubwheel}
      onpointerdown={(e) => {
        if (!app.design || e.button !== 0) return;
        e.preventDefault();
        scrub = { fine: e.shiftKey, c0: cycle };
        fineScrub = e.shiftKey;
        app.playing = false;
        (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
        scrubTo(e);
      }}
      onpointermove={(e) => scrub && scrubTo(e)}
      onpointerup={() => ((scrub = null), (fineScrub = false))}
      onpointercancel={() => ((scrub = null), (fineScrub = false))}
      onlostpointercapture={() => ((scrub = null), (fineScrub = false))}
    >
      <div class="rail"></div>
      {#if fineScrub}
        <div class="fill" style="width:{(phase / T) * 100}%"></div>
        <span class="tick" style="left:0">cycle {cycle}</span>
        <span class="tick" style="left:100%">{cycle + 1}</span>
        <div class="thumb" style="left:{(phase / T) * 100}%"></div>
      {:else}
        <div class="fill" style="width:{pct}%"></div>
        {#each ticks as t}
          <span class="tick" style="left:{(t / span) * 100}%">{t}</span>
        {/each}
        <div class="thumb" style="left:{pct}%"></div>
      {/if}
    </div>
  {/if}
  <button class="speed mono" class:on={ui.menu === 'timing'} bind:this={speedBtn} data-menu onclick={openTiming} title="Speed, direction and step size">
    {#if tm.reverse}<span class="rv">◀</span>{/if}{fmtSpeed()}{#if tm.loop}<span class="rv">⟲</span>{/if}<Icon name="chevron-down" size={12} />
  </button>
  {#if !narrow}
    <form onsubmit={jump}>
      <input name="jump" class="goto mono" class:err={jumpErr} placeholder="Go to…" title="700k, 2.5M or c:12" disabled={!app.design} oninput={() => (jumpErr = false)} />
    </form>
  {/if}
  {#if app.design && app.design.inputs.length && !paneVisible('inspector')}
    <div class="chips">
      {#each app.design.inputs.slice(0, 4) as p, i}
        {@const v = app.inputs[i] ?? 0n}
        <span class="cn mono">{p.name}</span>
        {#if p.width === 1}
          <button class="chip mono" class:on={v === 1n} title="Applies from the next cycle" onclick={() => setInput(i, v ^ 1n)}>{v}</button>
        {:else}
          <input
            class="chipnum mono"
            value={String(v)}
            onchange={(e) => {
              const x = parseValue((e.target as HTMLInputElement).value);
              if (x !== null) setInput(i, x);
            }}
          />
        {/if}
      {/each}
    </div>
  {/if}
</div>

{#if ui.menu === 'timing'}
  <div class="menu timing" style="left:{pop.x}px; bottom:{pop.bottom}px">
    <div class="sec">
      <div class="head"><span class="caption">Speed</span><span class="val mono">{fmtSpeed()}</span></div>
      <input class="slider" type="range" min="0" max={maxLog} step="0.01" value={speedLog} oninput={(e) => setSpeedFromLog(Number((e.target as HTMLInputElement).value))} />
      <div class="row">
        <input
          class="num mono"
          value={tm.speedUnit === 'cycle' ? fmtCycles(tm.speed) : fmtNum(tm.speed)}
          onchange={(e) => {
            const v = parseAmount((e.target as HTMLInputElement).value);
            if (v !== null && v > 0) settings.timing.speed = clampSpeed(v, tm.speedUnit);
          }}
        />
        <div class="seg">
          <button class:on={tm.speedUnit === 'gen'} onclick={() => setSpeedUnit('gen')}>gen/s</button>
          <button class:on={tm.speedUnit === 'cycle'} onclick={() => setSpeedUnit('cycle')}>cycles/s</button>
        </div>
      </div>
      <div class="chipsrow">
        {#each SPEEDS as [v, u, label]}
          <button class="pchip mono" class:on={isOn(v, u, tm.speed, tm.speedUnit)} onclick={() => ((settings.timing.speedUnit = u), (settings.timing.speed = v))}>{label}</button>
        {/each}
      </div>
      <span class="caption">{fmt(Math.round(genPerSec()))} generations per second · one cycle is {fmt(T)} generations</span>
    </div>
    <div class="sec">
      <div class="row between">
        <span>Direction</span>
        <div class="seg">
          <button class:on={!tm.reverse} onclick={() => (settings.timing.reverse = false)}>Forward</button>
          <button class:on={tm.reverse} onclick={() => (settings.timing.reverse = true)}>Reverse</button>
        </div>
      </div>
      <button class="menu-row tgl" onclick={() => setLoop(!tm.loop)}><span class="grow">Loop the current cycle</span><Switch on={tm.loop} /></button>
    </div>
    <div class="sec">
      <div class="head"><span class="caption">Step size (‹ › and ← →)</span><span class="val mono">{fmtStep()}</span></div>
      <div class="row">
        <input
          class="num mono"
          value={tm.stepUnit === 'cycle' ? fmtCycles(tm.step) : fmtNum(tm.step)}
          title="e.g. 1, 250, 2k, or 1/16 (cycles)"
          onchange={(e) => {
            const v = parseAmount((e.target as HTMLInputElement).value);
            if (v !== null && v > 0) settings.timing.step = tm.stepUnit === 'gen' ? Math.max(1, Math.round(v)) : v;
          }}
        />
        <div class="seg">
          <button class:on={tm.stepUnit === 'gen'} onclick={() => setStepUnit('gen')}>gen</button>
          <button class:on={tm.stepUnit === 'cycle'} onclick={() => setStepUnit('cycle')}>cycles</button>
        </div>
      </div>
      <div class="chipsrow">
        {#each STEPS as [v, u, label]}
          <button class="pchip mono" class:on={isOn(v, u, tm.step, tm.stepUnit)} onclick={() => ((settings.timing.stepUnit = u), (settings.timing.step = v))}>{label}</button>
        {/each}
      </div>
    </div>
    <div class="sec">
      <div class="head"><span class="caption">Position in cycle {cycle}</span><span class="val mono">{fmt(phase)} / {fmt(T)}</span></div>
      <input class="slider" type="range" min="0" max={T - 1} step="1" value={phase} disabled={!app.design} oninput={(e) => ((app.playing = false), setGen(cycle * T + Number((e.target as HTMLInputElement).value)))} />
    </div>
    <div class="keys">
      <span class="mono">← →</span><span>step</span><span class="mono">⇧← ⇧→</span><span>cycle</span>
      <span class="mono">[ ]</span><span>slower / faster</span><span class="mono">⇧Space</span><span>reverse</span>
    </div>
  </div>
{/if}

<style>
  .transport {
    height: 100%;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 0 12px 0 8px;
    background: var(--panel);
    border-top: 1px solid var(--border);
    overflow: hidden;
  }
  .transport.floating {
    border: 1px solid var(--border);
    border-radius: 14px;
    box-shadow: var(--shadow);
  }
  .steps {
    display: flex;
    align-items: center;
    gap: 1px;
    flex-shrink: 0;
  }
  .st {
    width: 26px;
    height: 30px;
    border-radius: 7px;
    color: var(--fg-dim);
    display: grid;
    place-items: center;
  }
  .st.w30 {
    width: 30px;
  }
  .st:hover:not(:disabled) {
    background: var(--hover);
    color: var(--fg);
  }
  .play {
    width: 36px;
    height: 36px;
    border-radius: 10px;
    background: var(--fg);
    color: var(--bg);
    display: grid;
    place-items: center;
    margin: 0 3px;
  }
  .play.rev :global(svg) {
    transform: scaleX(-1);
  }
  .readout {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    min-width: 120px;
    flex-shrink: 0;
  }
  .cyc,
  .gen {
    border-radius: 4px;
    padding: 0 3px;
    margin-left: -3px;
    text-align: left;
  }
  .cyc {
    font: 500 13.5px var(--mono);
  }
  .ph {
    color: var(--fg-faint);
    font-weight: 400;
    font-size: 11.5px;
  }
  .gen {
    font: 11px var(--mono);
    color: var(--fg-dim);
  }
  .cyc:hover:not(:disabled),
  .gen:hover:not(:disabled) {
    background: var(--hover);
  }
  .edit {
    width: 120px;
    height: 20px;
    padding: 0 4px;
    border: 1px solid var(--accent);
    border-radius: 4px;
    background: var(--bg);
    font-size: 11px;
  }
  .edit.big {
    font-size: 13px;
  }
  .scrub {
    flex: 1;
    min-width: 80px;
    height: 34px;
    position: relative;
    cursor: pointer;
    touch-action: none;
    user-select: none;
    -webkit-user-select: none;
  }
  .scrub.fixed {
    flex: none;
    width: 260px;
  }
  .rail,
  .fill {
    position: absolute;
    left: 0;
    top: 12px;
    height: 4px;
    border-radius: 2px;
  }
  .rail {
    right: 0;
    background: var(--border-soft);
  }
  .fill {
    background: var(--accent);
  }
  .fine .fill {
    background: var(--warn);
  }
  .tick {
    position: absolute;
    top: 21px;
    transform: translateX(-50%);
    font: 10px var(--mono);
    color: var(--fg-faint);
    pointer-events: none;
    white-space: nowrap;
  }
  .fine .tick:first-of-type {
    transform: none;
  }
  .fine .tick:last-of-type {
    transform: translateX(-100%);
  }
  .thumb {
    position: absolute;
    top: 7px;
    width: 14px;
    height: 14px;
    margin-left: -7px;
    border-radius: 50%;
    background: var(--raised);
    border: 2px solid var(--accent);
  }
  .speed {
    height: 30px;
    padding: 0 8px 0 10px;
    border: 1px solid var(--border);
    border-radius: 7px;
    background: var(--bg);
    font-size: 11.5px;
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
    color: var(--fg);
  }
  .speed:hover,
  .speed.on {
    border-color: var(--fg-faint);
  }
  .rv {
    color: var(--accent);
  }
  form {
    margin: 0;
    flex-shrink: 0;
  }
  .goto {
    width: 96px;
    height: 30px;
    padding: 0 9px;
    border: 1px solid var(--border);
    border-radius: 7px;
    background: var(--bg);
    font-size: 12px;
  }
  .goto:focus {
    border-color: var(--accent);
  }
  .goto.err {
    border-color: var(--err);
  }
  .chips {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-left: 10px;
    border-left: 1px solid var(--border);
    flex-shrink: 0;
  }
  .cn {
    font-size: 12px;
    color: var(--fg-dim);
  }
  .chip {
    width: 26px;
    height: 26px;
    border-radius: 6px;
    border: 1px solid var(--border);
    color: var(--fg-dim);
    font-weight: 500;
    font-size: 12px;
  }
  .chip.on {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-ink);
  }
  .chipnum {
    width: 52px;
    height: 26px;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    font-size: 12px;
  }
  /* Timing popover (fixed: the bar clips its overflow). */
  .timing {
    position: fixed;
    width: 320px;
    padding: 4px 0;
    z-index: 70;
  }
  .sec {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border-soft);
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }
  .val {
    font-size: 12px;
    font-weight: 500;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .between {
    justify-content: space-between;
  }
  .num {
    flex: 1;
    min-width: 0;
    height: 26px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    font-size: 12px;
  }
  .num:focus {
    border-color: var(--accent);
  }
  .slider {
    width: 100%;
    accent-color: var(--accent);
    margin: 0;
  }
  .chipsrow {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .pchip {
    height: 22px;
    padding: 0 7px;
    border: 1px solid var(--border);
    border-radius: 6px;
    font-size: 11px;
    color: var(--fg-dim);
  }
  .pchip:hover {
    background: var(--hover);
    color: var(--fg);
  }
  .pchip.on {
    border-color: var(--accent);
    background: var(--accent-soft);
    color: var(--fg);
  }
  .tgl {
    margin: 0 -8px;
    padding: 6px 8px;
  }
  .grow {
    flex: 1;
  }
  .keys {
    display: grid;
    grid-template-columns: auto 1fr auto 1fr;
    gap: 4px 10px;
    padding: 10px 14px 8px;
    font-size: 11.5px;
    color: var(--fg-dim);
  }
  .keys .mono {
    color: var(--fg);
  }
</style>
