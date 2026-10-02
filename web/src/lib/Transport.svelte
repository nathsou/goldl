<script lang="ts">
  // Transport: step, play, the cycle readout, a scrubber over clock cycles, speed and "go to".
  import Icon from './ui/Icon.svelte';
  import { app, period, cycleAt, paneVisible } from './state.svelte';
  import { setInput, parseJump, parseValue } from './actions.svelte';

  let { width, floating }: { width: number; floating: boolean } = $props();

  let jumpErr = $state(false);
  const T = $derived(period());
  const g = $derived(Math.floor(app.gen));
  const cycle = $derived(cycleAt(g));
  const phase = $derived(((g % T) + T) % T);
  const span = $derived(16 * Math.max(1, Math.ceil((cycle + 1) / 16)));
  const pct = $derived(Math.min(100, ((cycle + phase / T) / span) * 100));
  const ticks = $derived([0, 1, 2, 3, 4].map((k) => (span / 4) * k));
  const narrow = $derived(!floating && width < 560);
  const tiny = $derived(!floating && width < 420);
  const SPEEDS: [number, string][] = [
    [0.125, '⅛'],
    [0.5, '½'],
    [1, '1'],
    [4, '4'],
  ];

  function stepCycle(dc: number) {
    app.playing = false;
    app.gen = Math.max(0, (cycle + dc) * T + phase);
  }
  function stepGen(dg: number) {
    app.playing = false;
    app.gen = Math.max(0, g + dg);
  }
  let scrubbing = false;
  let track: HTMLDivElement;
  function scrubTo(e: PointerEvent) {
    const r = track.getBoundingClientRect();
    const f = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
    app.gen = Math.floor(f * span * T);
  }
  function jump(e: SubmitEvent) {
    e.preventDefault();
    const inp = (e.currentTarget as HTMLFormElement).elements.namedItem('jump') as HTMLInputElement;
    const v = parseJump(inp.value);
    jumpErr = v === null || !Number.isFinite(v);
    if (!jumpErr) {
      app.playing = false;
      app.gen = Math.max(0, v!);
      inp.value = '';
      inp.blur();
    }
  }
  const fmt = (n: number) => n.toLocaleString('en-US');
</script>

<div class="transport" class:floating>
  <div class="steps">
    <button class="st w30" title="Previous cycle" onclick={() => stepCycle(-1)} disabled={!app.design}><Icon name="skip-back" size={14} /></button>
    <button class="st" title="Previous generation" onclick={() => stepGen(-1)} disabled={!app.design}><Icon name="chevron-left" size={14} /></button>
    <button class="play" title="Play / pause (Space)" onclick={() => (app.playing = !app.playing)} disabled={!app.design}>
      <Icon name={app.playing ? 'pause' : 'play'} size={15} />
    </button>
    <button class="st" title="Next generation" onclick={() => stepGen(1)} disabled={!app.design}><Icon name="chevron-right" size={14} /></button>
    <button class="st w30" title="Next cycle" onclick={() => stepCycle(1)} disabled={!app.design}><Icon name="skip-forward" size={14} /></button>
  </div>
  <div class="readout" title="Phase {fmt(phase)} / {fmt(T)} generations">
    <span class="cyc">cycle {cycle}</span>
    <span class="gen">gen {fmt(g)}</span>
  </div>
  {#if !tiny}
    <div
      class="scrub"
      class:fixed={floating}
      bind:this={track}
      role="slider"
      aria-valuenow={cycle}
      tabindex="-1"
      onpointerdown={(e) => {
        if (!app.design) return;
        scrubbing = true;
        app.playing = false;
        (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
        scrubTo(e);
      }}
      onpointermove={(e) => scrubbing && scrubTo(e)}
      onpointerup={() => (scrubbing = false)}
    >
      <div class="rail"></div>
      <div class="fill" style="width:{pct}%"></div>
      {#each ticks as t}
        <span class="tick" style="left:{(t / span) * 100}%">{t}</span>
      {/each}
      <div class="thumb" style="left:{pct}%"></div>
    </div>
  {/if}
  {#if !narrow}
    <div class="seg" title="Clock cycles per second">
      {#each SPEEDS as [v, label]}
        <button class="mono" class:on={app.cps === v} onclick={() => (app.cps = v)}>{label}</button>
      {/each}
    </div>
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
  .readout {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 92px;
    flex-shrink: 0;
  }
  .cyc {
    font: 500 13.5px var(--mono);
  }
  .gen {
    font: 11px var(--mono);
    color: var(--fg-dim);
  }
  .scrub {
    flex: 1;
    min-width: 80px;
    height: 34px;
    position: relative;
    cursor: pointer;
    touch-action: none;
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
  .tick {
    position: absolute;
    top: 21px;
    transform: translateX(-50%);
    font: 10px var(--mono);
    color: var(--fg-faint);
    pointer-events: none;
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
  .seg > button {
    font-size: 11.5px;
    padding: 3px 7px;
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
</style>
