<script lang="ts">
  import { onMount } from 'svelte';
  import Editor from './lib/editor/Editor.svelte';
  import LifeView from './lib/life/LifeView.svelte';
  import Schematic from './lib/schematic/Schematic.svelte';
  import Waves from './lib/Waves.svelte';
  import SimBar from './lib/SimBar.svelte';
  import { app, settings, saveSettings, period } from './lib/state.svelte';
  import { engine } from './lib/engine';
  import { lsp, type LspDiag } from './lib/lsp';
  import { examples } from './lib/examples';
  import type { Design } from './lib/types';

  let life: LifeView | undefined = $state();
  let leftW = $state(Math.min(620, Math.round(window.innerWidth * 0.4)));
  let infoTab: 'problems' | 'tests' | 'stats' = $state('problems');
  let infoOpen = $state(true);
  let showSettings = $state(false);
  let showAbout = $state(false);
  let lspDiags: LspDiag[] = $state([]);
  let toast: { text: string; kind: 'ok' | 'err' | 'info' } | null = $state(null);
  let verifying = $state(false);
  let toastTimer: ReturnType<typeof setTimeout> | undefined;

  function say(text: string, kind: 'ok' | 'err' | 'info' = 'info', ms = 4000) {
    toast = { text, kind };
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), ms);
  }

  $effect(() => {
    document.documentElement.dataset.theme = settings.theme;
    saveSettings();
    void JSON.stringify(settings);
  });

  // ---- Source persistence and sharing ----
  async function encodeShare(src: string): Promise<string> {
    const cs = new CompressionStream('deflate-raw');
    const buf = await new Response(new Blob([src]).stream().pipeThrough(cs)).arrayBuffer();
    let s = '';
    new Uint8Array(buf).forEach((b) => (s += String.fromCharCode(b)));
    return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
  }
  async function decodeShare(code: string): Promise<string> {
    const bin = atob(code.replace(/-/g, '+').replace(/_/g, '/'));
    const bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0));
    const ds = new DecompressionStream('deflate-raw');
    return await new Response(new Blob([bytes]).stream().pipeThrough(ds)).text();
  }
  async function share() {
    const code = await encodeShare(app.src);
    const url = `${location.origin}${location.pathname}#code=${code}`;
    history.replaceState(null, '', url);
    try {
      await navigator.clipboard.writeText(url);
      say('Link copied to the clipboard', 'ok');
    } catch {
      say('Link is in the address bar', 'info');
    }
  }

  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  function onSourceChange(text: string) {
    app.src = text;
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      try {
        localStorage.setItem('goldl.src', text);
      } catch {}
    }, 400);
    scheduleAutoCompile();
  }

  function loadExample(id: string) {
    const ex = examples.find((e) => e.id === id);
    if (!ex) return;
    app.src = ex.src;
    app.exampleName = id;
    try {
      localStorage.setItem('goldl.src', ex.src);
    } catch {}
    history.replaceState(null, '', location.pathname);
    compile();
  }

  // ---- Compilation ----
  let compileSeq = 0;
  let autoTimer: ReturnType<typeof setTimeout> | undefined;
  function scheduleAutoCompile() {
    if (!settings.autoCompile) return;
    clearTimeout(autoTimer);
    autoTimer = setTimeout(() => {
      if (!lspDiags.some((d) => d.severity === 1)) compile();
    }, 1200);
  }

  async function compile() {
    const seq = ++compileSeq;
    const src = app.src;
    app.compiling = true;
    const t0 = performance.now();
    try {
      const r = await engine.compile(src);
      if (seq !== compileSeq) return;
      app.compileMs = performance.now() - t0;
      if (r.ok) {
        const prev = app.design;
        const d = r as Design;
        const same = prev && prev.name === d.name && prev.period === d.period;
        app.design = d;
        app.compileError = d.stats.layoutError ?? '';
        app.inputs = d.inputs.map(() => 0n);
        if (!same) {
          app.gen = 0;
          app.playing = false;
          // About one clock cycle every eight seconds.
          app.speed = Math.max(1, Math.round((d.period ?? 8000) / 8));
        } else {
          // Inputs were reset with the new session.
          app.gen = Math.min(app.gen, (period() || 1) * 64);
        }
        app.simVersion++;
        app.selectedGroup = null;
      } else {
        app.compileError = r.diags.map((d) => d.message).join('\n') || 'compilation failed';
        infoTab = 'problems';
        infoOpen = true;
      }
      engine.tests(src).then((t) => seq === compileSeq && (app.tests = t));
    } catch (e) {
      app.compileError = String(e);
      say(String(e), 'err', 8000);
    } finally {
      if (seq === compileSeq) app.compiling = false;
    }
  }

  engine.onReset = () => {
    say('The compiler crashed and was restarted', 'err', 6000);
    lsp.reset(app.src);
  };

  async function downloadRle() {
    if (!app.design) return;
    say('Building the pattern…', 'info');
    const g = Math.floor(app.gen);
    const { rle } = await engine.rle(g);
    const a = document.createElement('a');
    a.href = URL.createObjectURL(new Blob([rle], { type: 'text/plain' }));
    a.download = `${app.design.name.toLowerCase()}-gen${g}.rle`;
    a.click();
    URL.revokeObjectURL(a.href);
    say('Pattern downloaded (open it in Golly)', 'ok');
  }

  async function verify() {
    if (!app.design || verifying) return;
    verifying = true;
    const g = Math.floor(app.gen);
    say(`Running the real Life rules with HashLife up to generation ${g.toLocaleString()}…`, 'info', 60000);
    try {
      const r = await engine.verify(g);
      say(r.ok ? `✓ Verified: all ${r.cells.toLocaleString()} cells at generation ${r.gen.toLocaleString()} match the logic simulation` : `✗ ${r.mismatches} cells differ at generation ${r.gen}`, r.ok ? 'ok' : 'err', 8000);
    } catch (e) {
      say(String(e), 'err');
    } finally {
      verifying = false;
    }
  }

  // ---- Layout ----
  let dragging = false;
  function startDrag(e: PointerEvent) {
    dragging = true;
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
  }
  function onDrag(e: PointerEvent) {
    if (dragging) leftW = Math.min(window.innerWidth - 320, Math.max(280, e.clientX));
  }

  function onkeydown(e: KeyboardEvent) {
    const t = e.target as HTMLElement;
    const typing = t.tagName === 'TEXTAREA' || t.tagName === 'INPUT' || t.tagName === 'SELECT';
    if (!typing && e.key === ' ') {
      e.preventDefault();
      if (app.design) app.playing = !app.playing;
    }
    if (!typing && e.key === 'f' && app.view === 'life') life?.fitAll();
    if (!typing && /^[1-3]$/.test(e.key)) app.view = (['life', 'schematic', 'waves'] as const)[Number(e.key) - 1];
  }

  onMount(async () => {
    const m = /#code=([A-Za-z0-9_-]+)/.exec(location.hash);
    let src: string | null = null;
    if (m) {
      try {
        src = await decodeShare(m[1]);
      } catch {
        say('Could not decode the shared link', 'err');
      }
    }
    if (!src) {
      try {
        src = localStorage.getItem('goldl.src');
      } catch {}
    }
    if (!src) {
      const ex = examples.find((e) => e.id === 'counter') ?? examples[0];
      src = ex.src;
      app.exampleName = ex.id;
    }
    app.src = src;
    compile();
  });

  const errCount = $derived(lspDiags.filter((d) => d.severity === 1).length);
  const warnCount = $derived(lspDiags.filter((d) => d.severity === 2).length);
  const testsFailed = $derived(app.tests.filter((t) => !t.passed).length);
  const fmt = (n: number) => n.toLocaleString('en-US');

  function gotoLine(line: number) {
    // Highlight the whole line in the editor.
    const lines = app.src.split('\n');
    let off = 0;
    for (let i = 0; i < line; i++) off += new TextEncoder().encode(lines[i] + '\n').length;
    app.highlight = [off, off + new TextEncoder().encode(lines[line] ?? '').length];
  }
</script>

<svelte:window {onkeydown} onpointermove={onDrag} onpointerup={() => (dragging = false)} />

<div class="shell">
  <header>
    <div class="brand" title="GoLDL: a hardware description language for the Game of Life">
      <svg viewBox="0 0 3 3" width="18" height="18"><rect x="1" y="0" width="1" height="1" /><rect x="2" y="1" width="1" height="1" /><rect x="0" y="2" width="3" height="1" /></svg>
      <span class="name">GoLDL</span>
      <span class="tag">circuits in the Game of Life</span>
    </div>
    <select class="examples" value={app.exampleName} onchange={(e) => loadExample((e.target as HTMLSelectElement).value)} title="Load an example">
      <option value="" disabled>Examples…</option>
      {#each examples as ex}
        <option value={ex.id}>{ex.title}</option>
      {/each}
    </select>
    <button class="primary" onclick={compile} disabled={app.compiling} title="Compile (Ctrl+Enter)">
      {#if app.compiling}<span class="spin"></span> Compiling…{:else}▶ Compile{/if}
    </button>
    <span class="status">
      {#if app.design && !app.compileError}
        <span class="ok">●</span> {app.design.name} · {fmt(app.design.stats.components)} components · period {fmt(app.design.stats.period)} gen · {Math.round(app.compileMs)} ms
      {:else if app.compileError}
        <span class="bad">●</span> {app.compileError.split('\n')[0]}
      {/if}
    </span>
    <div class="actions">
      <button onclick={verify} disabled={!app.design || verifying} title="Check the reconstruction against the real Life rules (HashLife)">{verifying ? 'Verifying…' : '✓ Verify'}</button>
      <button onclick={downloadRle} disabled={!app.design} title="Download the pattern at the current generation">⇩ RLE</button>
      <button onclick={share} title="Copy a link to this design">⤴ Share</button>
      <button onclick={() => (showAbout = !showAbout)} title="About">?</button>
      <button onclick={() => (showSettings = !showSettings)} title="Settings">⚙</button>
    </div>
  </header>

  <main>
    <section class="left" style="width:{leftW}px">
      <div class="editor-wrap">
        <Editor
          bind:value={app.src}
          fontSize={settings.fontSize}
          tabSize={settings.tabSize}
          lineNumbers={settings.lineNumbers}
          inlayHints={settings.inlayHints}
          autoClose={settings.autoCloseBrackets}
          highlight={app.highlight}
          onchange={onSourceChange}
          ondiagnostics={(d) => (lspDiags = d)}
          onrun={compile}
        />
      </div>
      <div class="info" class:open={infoOpen}>
        <div class="info-tabs">
          <button class:sel={infoTab === 'problems'} onclick={() => ((infoTab = 'problems'), (infoOpen = true))}>Problems <span class="badge" class:e={errCount}>{errCount + warnCount}</span></button>
          <button class:sel={infoTab === 'tests'} onclick={() => ((infoTab = 'tests'), (infoOpen = true))}>Tests <span class="badge" class:e={testsFailed}>{app.tests.length ? `${app.tests.length - testsFailed}/${app.tests.length}` : 0}</span></button>
          <button class:sel={infoTab === 'stats'} onclick={() => ((infoTab = 'stats'), (infoOpen = true))}>Statistics</button>
          <button class="collapse" onclick={() => (infoOpen = !infoOpen)}>{infoOpen ? '▾' : '▴'}</button>
        </div>
        {#if infoOpen}
          <div class="info-body">
            {#if infoTab === 'problems'}
              {#if lspDiags.length === 0 && !app.compileError}
                <div class="muted">No problems.</div>
              {/if}
              {#each lspDiags as d}
                <button class="row" onclick={() => gotoLine(d.range.start.line)}>
                  <span class={d.severity === 1 ? 'e' : 'w'}>{d.severity === 1 ? '●' : '▲'}</span>
                  <span class="msg">{d.message}</span>
                  <span class="loc">{d.range.start.line + 1}:{d.range.start.character + 1}</span>
                </button>
              {/each}
              {#if app.compileError && lspDiags.length === 0}
                <div class="row e">{app.compileError}</div>
              {/if}
            {:else if infoTab === 'tests'}
              {#if app.tests.length === 0}
                <div class="muted">No test benches. Add one with <code>test "name" for Module {'{ ... }'}</code>.</div>
              {/if}
              {#each app.tests as t}
                <button class="row" onclick={() => gotoLine(t.failLine ?? t.line)}>
                  <span class={t.passed ? 'okc' : 'e'}>{t.passed ? '✓' : '✗'}</span>
                  <span class="msg">{t.name}{t.message ? ' — ' + t.message : ''}</span>
                  <span class="loc">{t.cycles} cycles</span>
                </button>
              {/each}
            {:else if app.design}
              {@const s = app.design.stats}
              <div class="stats">
                <div><b>{fmt(s.rtlNodes)}</b><span>RTL operators</span></div>
                <div><b>{fmt(s.regBits)}</b><span>register bits</span></div>
                <div><b>{fmt(s.aigAnds)}</b><span>AND gates (AIG)</span></div>
                <div><b>{fmt(s.crossings)}</b><span>glider crossings</span></div>
                <div><b>{fmt(s.splitters)}</b><span>splitters</span></div>
                <div><b>{fmt(s.delays)}</b><span>delay loops</span></div>
                <div><b>{fmt(s.components)}</b><span>Life components</span></div>
                <div><b>{fmt(s.cells)}</b><span>live cells (still)</span></div>
                <div><b>{fmt(s.period)}</b><span>generations / cycle</span></div>
                <div><b>{fmt(s.width)} × {fmt(s.height)}</b><span>pattern size</span></div>
              </div>
            {:else}
              <div class="muted">Compile a design to see statistics.</div>
            {/if}
          </div>
        {/if}
      </div>
    </section>
    <div class="divider" onpointerdown={startDrag} role="separator" aria-orientation="vertical"></div>
    <section class="right">
      <nav>
        <button class:sel={app.view === 'life'} onclick={() => (app.view = 'life')}>Game of Life</button>
        <button class:sel={app.view === 'schematic'} onclick={() => (app.view = 'schematic')}>Schematic</button>
        <button class:sel={app.view === 'waves'} onclick={() => (app.view = 'waves')}>Waveforms</button>
        {#if app.view === 'life'}
          <div class="view-tools">
            <label title="Show what groups of cells implement"><input type="checkbox" bind:checked={settings.overlay} /> abstraction overlay</label>
            <label><input type="checkbox" bind:checked={settings.overlayLabels} /> labels</label>
            <label><input type="checkbox" bind:checked={settings.wires} /> paths</label>
            <label><input type="checkbox" bind:checked={settings.glow} /> glow</label>
            <button onclick={() => life?.fitAll()} title="Fit (F)">Fit</button>
            <button onclick={() => life?.zoomTo(4)} title="Zoom to cells">Cells</button>
          </div>
        {/if}
      </nav>
      <div class="view">
        <div class="pane" class:hidden={app.view !== 'life'}><LifeView bind:this={life} /></div>
        {#if app.view === 'schematic'}
          <div class="pane"><Schematic /></div>
        {:else if app.view === 'waves'}
          <div class="pane"><Waves /></div>
        {/if}
      </div>
    </section>
  </main>

  <SimBar />

  {#if showSettings}
    <div class="popover settings" role="dialog">
      <h3>Settings</h3>
      <label>Theme <select bind:value={settings.theme}><option value="dark">dark</option><option value="light">light</option></select></label>
      <label>Life palette <select bind:value={settings.palette}><option value="neon">neon</option><option value="classic">classic</option><option value="amber">amber</option></select></label>
      <label>Editor font size <input type="number" min="9" max="24" bind:value={settings.fontSize} /></label>
      <label>Tab size <input type="number" min="1" max="8" bind:value={settings.tabSize} /></label>
      <label><input type="checkbox" bind:checked={settings.lineNumbers} /> Line numbers</label>
      <label><input type="checkbox" bind:checked={settings.inlayHints} /> Inlay type hints</label>
      <label><input type="checkbox" bind:checked={settings.autoCloseBrackets} /> Auto-close brackets</label>
      <label><input type="checkbox" bind:checked={settings.autoCompile} /> Compile automatically</label>
      <label><input type="checkbox" bind:checked={settings.componentOutlines} /> Component outlines in the Life view</label>
      <button onclick={() => (showSettings = false)}>Close</button>
    </div>
  {/if}

  {#if showAbout}
    <div class="popover about" role="dialog">
      <h3>GoLDL</h3>
      <p>A hardware description language whose circuits are compiled to the Game of Life. Signals are gliders: one glider (or none) per wire per clock cycle. Gates are glider collisions, wires are lanes turned by Snarks and colour-changing reflectors, fan-out uses Syringe duplicators, and registers are glider loops around the circuit.</p>
      <p>The simulation is hierarchical: the logic is simulated cycle by cycle, and the exact state of every cell at any generation is reconstructed from it, so you can jump straight to generation 700,000 or 7,000,000,000. <b>Verify</b> runs the real Life rules with HashLife and checks every cell.</p>
      <p>Keys: <code>Space</code> play/pause · <code>1 2 3</code> views · <code>F</code> fit · <code>Ctrl+Enter</code> compile · drag/scroll to pan and zoom · double-click to zoom in · click a region to select it.</p>
      <button onclick={() => (showAbout = false)}>Close</button>
    </div>
  {/if}

  {#if toast}
    <div class="toast {toast.kind}">{toast.text}</div>
  {/if}
</div>

<style>
  .shell {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 14px;
    height: 48px;
    background: var(--panel);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-right: 6px;
  }
  .brand svg {
    fill: var(--accent);
    filter: drop-shadow(0 0 6px var(--accent));
  }
  .brand .name {
    font: 700 17px var(--sans);
    letter-spacing: 0.02em;
  }
  .brand .tag {
    font-size: 12px;
    color: var(--fg-faint);
  }
  .examples {
    width: 220px;
  }
  .primary {
    background: var(--accent);
    color: var(--accent-ink);
    border-color: var(--accent);
    font-weight: 600;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .spin {
    width: 12px;
    height: 12px;
    border: 2px solid currentColor;
    border-right-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .status {
    font-size: 12px;
    color: var(--fg-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }
  .ok {
    color: var(--ok);
  }
  .bad {
    color: var(--err);
  }
  .actions {
    display: flex;
    gap: 6px;
  }
  main {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .left {
    display: flex;
    flex-direction: column;
    min-width: 280px;
    border-right: 1px solid var(--border);
  }
  .editor-wrap {
    flex: 1;
    min-height: 0;
  }
  .divider {
    width: 5px;
    margin-left: -3px;
    cursor: col-resize;
    z-index: 5;
  }
  .divider:hover {
    background: var(--accent-soft);
  }
  .right {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  nav {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 6px 10px 0;
    background: var(--panel);
    border-bottom: 1px solid var(--border);
  }
  nav > button {
    border: 0;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    background: none;
    padding: 6px 14px 8px;
    color: var(--fg-dim);
  }
  nav > button.sel {
    color: var(--fg);
    border-bottom-color: var(--accent);
  }
  .view-tools {
    margin-left: auto;
    display: flex;
    gap: 12px;
    align-items: center;
    font-size: 12px;
    color: var(--fg-dim);
    padding-bottom: 4px;
  }
  .view-tools button {
    padding: 2px 10px;
  }
  .view {
    flex: 1;
    position: relative;
    min-height: 0;
  }
  .pane {
    position: absolute;
    inset: 0;
  }
  .pane.hidden {
    visibility: hidden;
    pointer-events: none;
  }
  .info {
    border-top: 1px solid var(--border);
    background: var(--panel);
    display: flex;
    flex-direction: column;
    max-height: 34%;
  }
  .info.open {
    height: 190px;
  }
  .info-tabs {
    display: flex;
    gap: 2px;
    padding: 2px 8px;
  }
  .info-tabs button {
    border: 0;
    background: none;
    font-size: 12px;
    color: var(--fg-dim);
    padding: 4px 10px;
  }
  .info-tabs button.sel {
    color: var(--fg);
  }
  .collapse {
    margin-left: auto;
  }
  .badge {
    background: var(--border);
    border-radius: 8px;
    padding: 0 6px;
    font-size: 10.5px;
    margin-left: 4px;
  }
  .badge.e {
    background: var(--err);
    color: #fff;
  }
  .info-body {
    flex: 1;
    overflow: auto;
    padding: 2px 8px 8px;
    font-size: 12.5px;
  }
  .row {
    display: flex;
    gap: 8px;
    width: 100%;
    text-align: left;
    background: none;
    border: 0;
    border-radius: 4px;
    padding: 3px 6px;
    align-items: baseline;
  }
  .row:hover {
    background: var(--line-hl);
  }
  .row .msg {
    flex: 1;
    white-space: pre-wrap;
  }
  .row .loc {
    color: var(--fg-faint);
    font-family: var(--mono);
    font-size: 11px;
  }
  .e {
    color: var(--err);
  }
  .w {
    color: var(--warn);
  }
  .okc {
    color: var(--ok);
  }
  .muted {
    color: var(--fg-faint);
    padding: 6px;
  }
  code {
    font-family: var(--mono);
    background: var(--code-bg);
    padding: 0 4px;
    border-radius: 3px;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
    gap: 8px;
    padding: 6px;
  }
  .stats div {
    display: flex;
    flex-direction: column;
    background: var(--panel-raised);
    border: 1px solid var(--border-soft);
    border-radius: 8px;
    padding: 8px 10px;
  }
  .stats b {
    font: 600 15px var(--mono);
    color: var(--accent);
  }
  .stats span {
    font-size: 11px;
    color: var(--fg-dim);
  }
  .popover {
    position: fixed;
    top: 54px;
    right: 14px;
    z-index: 50;
    width: 340px;
    background: var(--panel-raised);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: var(--shadow);
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 9px;
    font-size: 13px;
  }
  .popover.about {
    width: 460px;
    line-height: 1.5;
  }
  .popover h3 {
    margin: 0 0 4px;
    font-size: 15px;
  }
  .popover p {
    margin: 0;
    color: var(--fg-dim);
  }
  .popover label {
    display: flex;
    align-items: center;
    gap: 8px;
    justify-content: space-between;
  }
  .popover label:has(input[type='checkbox']) {
    justify-content: flex-start;
  }
  .popover input[type='number'] {
    width: 70px;
  }
  .popover > button {
    align-self: flex-end;
    margin-top: 4px;
  }
  .toast {
    position: fixed;
    bottom: 76px;
    left: 50%;
    transform: translateX(-50%);
    background: var(--panel-raised);
    border: 1px solid var(--border);
    border-left: 3px solid var(--accent);
    box-shadow: var(--shadow);
    padding: 8px 14px;
    border-radius: 8px;
    font-size: 13px;
    z-index: 60;
    max-width: 70vw;
  }
  .toast.err {
    border-left-color: var(--err);
  }
  .toast.ok {
    border-left-color: var(--ok);
  }
  @media (max-width: 900px) {
    .brand .tag,
    .status {
      display: none;
    }
  }
</style>
