<script lang="ts">
  // Top bar: design picker, compile, status, view switcher, layout, verify, share, settings.
  import { tick } from 'svelte';
  import Icon from './ui/Icon.svelte';
  import Switch from './ui/Switch.svelte';
  import { app, ui, settings, studio, env, togglePane, paneVisible, type MenuId } from './state.svelte';
  import { compile, loadExample, copyLink, downloadRle, verify, live } from './actions.svelte';
  import { examples } from './examples';
  import { PRESETS, PANES, PANE_IDS, clone, presetOf, type Layout } from './layout/layout';

  // Counts are measured, not hard-coded. Edited designs must not overwrite the
  // bundled example's count.
  const counts = $state<Record<string, number>>({});
  let exampleSearch = $state('');
  let exampleInput: HTMLInputElement | undefined = $state();
  let examplePicker: HTMLButtonElement | undefined = $state();
  const filteredExamples = $derived(examples.filter((ex) =>
    `${ex.title} ${ex.summary} ${ex.id}`.toLowerCase().includes(exampleSearch.trim().toLowerCase())));
  $effect(() => {
    const ex = examples.find((e) => e.id === app.exampleName);
    if (app.design && !app.compiling && !app.compileError && ex?.src === app.src) {
      counts[ex.id] = app.design.stats.components;
    }
  });

  const wide = $derived(env.vw >= 1100);
  const nErr = $derived(live.diags.filter((d) => d.severity === 1).length);
  const firstErr = $derived(live.diags.find((d) => d.severity === 1));
  const designName = $derived(app.design?.name ?? (examples.find((e) => e.id === app.exampleName)?.title || 'Untitled'));
  const status = $derived.by(() => {
    if (app.compiling) return { text: 'Compiling…', cls: '', dot: 'faint' };
    if (nErr || (app.compileError && !app.design)) {
      const line = firstErr ? firstErr.range.start.line + 1 : null;
      return { text: `${nErr || 1} error${nErr > 1 ? 's' : ''}${line ? ` · line ${line}` : ''}`, cls: 'err', dot: 'err' };
    }
    if (app.compileError) return { text: 'Layout failed', cls: 'err', dot: 'err' };
    if (!app.design) return { text: 'Not compiled', cls: '', dot: 'faint' };
    const ms = `${Math.round(app.compileMs)} ms`;
    return { text: wide ? `${app.design.stats.components.toLocaleString('en-US')} components · ${ms}` : ms, cls: '', dot: 'ok' };
  });
  const preset = $derived(presetOf(studio.layout));

  async function open(m: MenuId) {
    ui.menu = ui.menu === m ? null : m;
    ui.paneMenu = null;
    if (ui.menu === 'examples') {
      exampleSearch = '';
      await tick();
      exampleInput?.focus();
    }
  }
  function exampleKeys(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.stopPropagation();
      ui.menu = null;
      examplePicker?.focus();
    } else if (event.target === exampleInput && event.key === 'Enter' && filteredExamples.length) {
      event.preventDefault();
      loadExample(filteredExamples[0].id);
      examplePicker?.focus();
    }
  }
  function showChecks() {
    togglePane('checks', true);
    ui.checkTab = nErr || app.compileError ? 'problems' : 'tests';
  }
  function applyPreset(L: Layout) {
    studio.layout = clone(L);
    ui.menu = null;
  }
  /** Mini-map blocks of a preset. */
  function blocks(L: Layout) {
    const vis = (id: string) => !L.hidden.includes(id as never);
    const lp = L.left.panes.filter(vis);
    const rp = L.right.panes.filter(vis);
    const bp = L.bottom.panes.filter(vis);
    const lw = lp.length ? (L.left.size > 500 ? 46 : 34) : 0;
    const rw = rp.length ? 24 : 0;
    const bh = bp.length ? 30 : 0;
    const b: { l: number; t: number; w: number; h: number; code: boolean }[] = [];
    lp.forEach((id, i) => b.push({ l: 0, t: (i * 100) / lp.length, w: lw, h: 100 / lp.length, code: id === 'code' }));
    rp.forEach((id, i) => b.push({ l: 100 - rw, t: (i * 100) / rp.length, w: rw, h: 100 / rp.length, code: id === 'code' }));
    bp.forEach((id, i) => b.push({ l: lw + (i * (100 - lw - rw)) / bp.length, t: 100 - bh, w: (100 - lw - rw) / bp.length, h: bh, code: id === 'code' }));
    return b;
  }
  const VIEWS = [
    ['life', 'Universe', '1'],
    ['schematic', 'Schematic', '2'],
    ['waves', 'Waveforms', '3'],
  ] as const;
  const THEMES = [
    ['light', 'Light', 'sun'],
    ['dark', 'Dark', 'moon'],
    ['system', 'System', 'monitor'],
  ] as const;
  const PALS = [
    ['classic', 'Classic'],
    ['neon', 'Neon'],
    ['amber', 'Amber'],
  ] as const;
  const TOGGLES = [
    ['lineNumbers', 'Line numbers'],
    ['inlayHints', 'Inlay type hints'],
    ['autoCompile', 'Compile as I type'],
    ['componentOutlines', 'Component outlines'],
  ] as const;
  const KEYS = [
    ['Space', 'Play / pause'],
    ['⌘↵', 'Compile'],
    ['1 2 3', 'Switch view'],
    ['⌘B ⌘J ⌘I', 'Toggle code, problems, inspector'],
    ['⌥1–4', 'Layout presets'],
    ['F', 'Fit universe'],
    ['Scroll', 'Zoom'],
  ];
</script>

<header>
  <div class="brand" title="GoLDL: a hardware description language for the Game of Life">
    <svg viewBox="0 0 3 3" width="14" height="14"><rect x="1" y="0" width="1" height="1" /><rect x="2" y="1" width="1" height="1" /><rect x="0" y="2" width="3" height="1" /></svg>
    <span>GoLDL</span>
  </div>
  <div class="anchor">
    <button bind:this={examplePicker} data-menu class="picker" onclick={() => open('examples')} title="Examples" aria-label="Choose example" aria-expanded={ui.menu === 'examples'} aria-controls="example-picker">
      <span class="dot {status.dot}"></span><span class="dn">{designName}</span><span class="dim"><Icon name="chevrons-up-down" size={13} /></span>
    </button>
    {#if ui.menu === 'examples'}
      <div id="example-picker" class="menu exmenu" role="region" aria-label="Examples">
        <div class="caption mh"><span>Examples</span><span title="Measured after compiling the bundled example">Components</span></div>
        <input bind:this={exampleInput} bind:value={exampleSearch} class="exsearch" onkeydown={exampleKeys} aria-label="Find an example" placeholder="Find an example…" />
        <div class="exlist">
          {#each filteredExamples as ex}
            <button class="menu-row example-row" onkeydown={exampleKeys} class:cur={ex.id === app.exampleName} onclick={() => loadExample(ex.id)} title={`${ex.title}: ${ex.summary}`} aria-current={ex.id === app.exampleName ? 'true' : undefined}>
              <span class="extext"><span class="extitle">{ex.title}</span><span class="exsummary">{ex.summary}</span></span>
              <span class="cnt mono" title={counts[ex.id] === undefined ? 'Compile to measure' : `${counts[ex.id].toLocaleString('en-US')} components`}>{counts[ex.id]?.toLocaleString('en-US') ?? '—'}</span>
            </button>
          {:else}
            <p class="empty-examples">No examples match “{exampleSearch}”.</p>
          {/each}
        </div>
      </div>
    {/if}
  </div>
  <button class="compile" onclick={() => ((ui.menu = null), compile())} disabled={app.compiling} title="Compile (⌘↵)"><Icon name="play" size={13} />Compile</button>
  <button class="status {status.cls}" onclick={showChecks}>
    {#if app.compiling}<span class="spinner"></span>{/if}
    <span class="st">{status.text}</span>
  </button>
  <div class="flex"></div>
  <div class="views">
    {#each VIEWS as [v, label, key]}
      <button class:on={app.view === v} title="{label} ({key})" onclick={() => (app.view = v)}>{label}</button>
    {/each}
  </div>
  <div class="flex"></div>
  <div class="anchor">
    <button data-menu class="btn" class:on={ui.menu === 'layout'} onclick={() => open('layout')} title="Layout">
      <span class="dim"><Icon name="layout-panel-left" /></span>
      {#if wide}<span>{preset?.name ?? 'Custom'}</span>{/if}
      <span class="dim"><Icon name="chevron-down" size={13} /></span>
    </button>
    {#if ui.menu === 'layout'}
      <div class="menu lmenu">
        <span class="caption sec">Presets</span>
        <div class="presets">
          {#each PRESETS as p}
            <button class="preset" class:cur={preset?.id === p.id} onclick={() => applyPreset(p.layout)}>
              <div class="mini">
                {#each blocks(p.layout) as b}
                  <div class:code={b.code} style="left:calc({b.l}% + 2px); top:calc({b.t}% + 2px); width:calc({b.w}% - 4px); height:calc({b.h}% - 4px)"></div>
                {/each}
              </div>
              <span class="pn"><span>{p.name}</span><span class="key mono">{p.key}</span></span>
            </button>
          {/each}
        </div>
        <span class="caption sec bt">Panels</span>
        {#each PANE_IDS as id}
          <button class="menu-row toggle" onclick={() => togglePane(id)}>
            <span class="dim"><Icon name={PANES[id].icon} /></span><span class="grow">{PANES[id].label}</span><span class="key mono">{PANES[id].key}</span>
            <Switch on={paneVisible(id)} />
          </button>
        {/each}
        <div class="rowx bt">
          <span>Panel style</span>
          <div class="seg">
            <button class:on={studio.panelStyle === 'docked'} onclick={() => (studio.panelStyle = 'docked')}>Docked</button>
            <button class:on={studio.panelStyle === 'floating'} onclick={() => (studio.panelStyle = 'floating')}>Floating</button>
          </div>
        </div>
        <div class="hint">Drag a panel by its title to dock it left, right or bottom. Drag the edges to resize.</div>
      </div>
    {/if}
  </div>
  <button class="btn" onclick={verify} disabled={!app.design || ui.verifying} title="Run the real Life rules with HashLife and compare every cell">
    <span class="dim"><Icon name="shield-check" /></span>{#if wide}<span>Verify</span>{/if}
  </button>
  <div class="anchor">
    <button data-menu class="btn" class:on={ui.menu === 'share'} onclick={() => open('share')} title="Share">
      <span class="dim"><Icon name="share" /></span>{#if wide}<span>Share</span>{/if}
    </button>
    {#if ui.menu === 'share'}
      <div class="menu smenu">
        <button class="menu-row" onclick={copyLink}><span class="dim"><Icon name="link" /></span><span class="two"><span>Copy link</span><span class="caption">The source is encoded in the URL</span></span></button>
        <button class="menu-row" onclick={downloadRle} disabled={!app.design}><span class="dim"><Icon name="download" /></span><span class="two"><span>Download RLE</span><span class="caption">Generation {Math.floor(app.gen).toLocaleString('en-US')} · self-contained{app.design?.inputs.length ? ', 64 cycles of inputs' : ''}</span></span></button>
      </div>
    {/if}
  </div>
  <div class="anchor">
    <button data-menu class="icon-btn gear" class:on={ui.menu === 'settings'} onclick={() => open('settings')} title="Settings"><Icon name="settings" /></button>
    {#if ui.menu === 'settings'}
      <div class="menu setmenu">
        <span class="caption sec">Theme</span>
        <div class="themes">
          {#each THEMES as [v, label, icon]}
            <button class:cur={studio.themePref === v} onclick={() => (studio.themePref = v)}><span class="dim"><Icon name={icon} /></span>{label}</button>
          {/each}
        </div>
        <span class="caption note">{studio.themePref === 'system' ? `Following your system: ${env.sysDark ? 'dark' : 'light'}` : 'Fixed regardless of your system setting'}</span>
        <div class="rowx bt">
          <span>Universe colours</span>
          <div class="seg">
            {#each PALS as [v, label]}
              <button class:on={settings.palette === v} onclick={() => (settings.palette = v)}>{label}</button>
            {/each}
          </div>
        </div>
        <div class="rowx">
          <span>Editor font size</span>
          <div class="stepper">
            <button onclick={() => (settings.fontSize = Math.max(10, settings.fontSize - 1))}>−</button>
            <span class="mono">{settings.fontSize}</span>
            <button onclick={() => (settings.fontSize = Math.min(20, settings.fontSize + 1))}>+</button>
          </div>
        </div>
        {#each TOGGLES as [k, label]}
          <button class="menu-row tg" onclick={() => (settings[k] = !settings[k])}><span class="grow">{label}</span><Switch on={settings[k]} /></button>
        {/each}
        <div class="keys bt">
          {#each KEYS as [k, l]}<span class="mono">{k}</span><span>{l}</span>{/each}
        </div>
      </div>
    {/if}
  </div>
</header>

<style>
  header {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 48px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 10px 0 14px;
    background: var(--panel);
    border-bottom: 1px solid var(--border);
    z-index: 40;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-right: 6px;
    font-weight: 600;
    font-size: 14px;
    flex-shrink: 0;
  }
  .brand svg {
    fill: var(--fg);
  }
  .anchor {
    position: relative;
    flex-shrink: 0;
  }
  .picker {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 30px;
    padding: 0 9px;
    border-radius: 7px;
    background: var(--hover);
  }
  .dn {
    font-weight: 500;
    max-width: min(24vw, 240px);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--fg-faint);
  }
  .dot.ok {
    background: var(--ok);
  }
  .dot.err {
    background: var(--err);
  }
  .dim {
    color: var(--fg-dim);
    display: inline-flex;
  }
  .compile {
    height: 30px;
    padding: 0 11px;
    border-radius: 7px;
    background: var(--accent);
    color: var(--accent-ink);
    font-weight: 500;
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  .status {
    height: 30px;
    padding: 0 9px;
    border-radius: 7px;
    color: var(--fg-dim);
    font-size: 12.5px;
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    flex-shrink: 1;
    overflow: hidden;
  }
  .status:hover {
    background: var(--hover);
  }
  .status.err {
    color: var(--err);
  }
  .st {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .flex {
    flex: 1;
    min-width: 8px;
  }
  .views {
    flex-shrink: 0;
    display: flex;
    padding: 2px;
    gap: 1px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 9px;
  }
  .views button {
    height: 26px;
    padding: 0 12px;
    border-radius: 7px;
    color: var(--fg-dim);
    font-size: 12.5px;
  }
  .views button.on {
    background: var(--raised);
    color: var(--fg);
    box-shadow: var(--seg-on);
  }
  .gear {
    width: 30px;
    height: 30px;
    border-radius: 7px;
  }
  .exmenu {
    top: 38px;
    left: 0;
    width: 360px;
    max-width: calc(100vw - 120px);
    padding: 6px;
    max-height: calc(100dvh - 70px);
    overflow: hidden;
  }
  .mh {
    padding: 6px 10px;
    display: flex;
    justify-content: space-between;
    gap: 12px;
    flex-shrink: 0;
  }
  .exsearch {
    margin: 4px 4px 8px;
    padding: 8px 10px;
    min-width: 0;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 6px;
    flex-shrink: 0;
  }
  .exsearch:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  .exlist { overflow-y: auto; overflow-x: hidden; min-height: 0; }
  .example-row {
    width: 100%;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: start;
  }
  .example-row:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .extext { min-width: 0; display: grid; gap: 3px; }
  .extitle { white-space: normal; overflow-wrap: anywhere; line-height: 1.35; }
  .exsummary {
    color: var(--fg-dim);
    font-size: 11px;
    line-height: 1.4;
    white-space: normal;
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    overflow: hidden;
  }
  .empty-examples { margin: 12px 10px; color: var(--fg-dim); overflow-wrap: anywhere; }
  .between {
    justify-content: space-between;
  }
  .cur {
    background: var(--hover);
  }
  .cnt {
    font-size: 11px;
    color: var(--fg-faint);
    white-space: nowrap;
    line-height: 1.6;
  }
  .lmenu {
    top: 38px;
    right: 0;
    width: 300px;
  }
  .sec {
    padding: 12px 14px 6px;
  }
  .bt {
    border-top: 1px solid var(--border-soft);
  }
  .presets {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
    padding: 0 10px 10px;
  }
  .preset {
    display: flex;
    flex-direction: column;
    gap: 7px;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: 9px;
    text-align: left;
  }
  .preset.cur {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .mini {
    position: relative;
    width: 100%;
    height: 46px;
    border-radius: 5px;
    background: var(--canvas);
    border: 1px solid var(--border-soft);
    overflow: hidden;
  }
  .mini div {
    position: absolute;
    border-radius: 2px;
    background: var(--border);
  }
  .mini div.code {
    background: var(--accent-soft);
  }
  .pn {
    display: flex;
    justify-content: space-between;
    font-size: 12px;
  }
  .key {
    font-size: 11px;
    color: var(--fg-faint);
  }
  .toggle {
    border-radius: 0;
    padding: 7px 14px;
  }
  .grow {
    flex: 1;
  }
  .rowx {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 10px 14px;
    margin-top: 6px;
  }
  .hint {
    padding: 0 14px 12px;
    font-size: 11.5px;
    color: var(--fg-faint);
    line-height: 1.45;
  }
  .smenu {
    top: 38px;
    right: 0;
    width: 250px;
    padding: 6px;
  }
  .two {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .setmenu {
    top: 38px;
    right: 0;
    width: 320px;
    padding: 6px 0;
  }
  .setmenu .sec {
    padding: 10px 16px 8px;
  }
  .themes {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr;
    gap: 6px;
    padding: 0 12px 6px;
  }
  .themes button {
    height: 54px;
    border: 1px solid var(--border);
    border-radius: 9px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    font-size: 12px;
  }
  .themes button.cur {
    border-color: var(--fg);
    background: var(--hover);
  }
  .note {
    padding: 2px 16px 8px;
  }
  .setmenu .rowx {
    padding: 8px 16px;
    margin: 0;
  }
  .stepper {
    display: flex;
    align-items: center;
    border: 1px solid var(--border);
    border-radius: 7px;
  }
  .stepper button {
    width: 26px;
    height: 24px;
    color: var(--fg-dim);
  }
  .stepper span {
    font-size: 12px;
    width: 22px;
    text-align: center;
  }
  .tg {
    border-radius: 0;
    padding: 7px 16px;
  }
  .keys {
    margin-top: 6px;
    padding: 10px 16px 6px;
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 14px;
    font-size: 12px;
    color: var(--fg-dim);
  }
  .keys .mono {
    color: var(--fg);
  }
</style>
