<script lang="ts">
  import { onMount } from 'svelte';
  import Editor from './lib/editor/Editor.svelte';
  import LifeView from './lib/life/LifeView.svelte';
  import Schematic from './lib/schematic/Schematic.svelte';
  import Waves from './lib/Waves.svelte';
  import Header from './lib/Header.svelte';
  import Transport from './lib/Transport.svelte';
  import Inspector from './lib/Inspector.svelte';
  import Checks from './lib/Checks.svelte';
  import Pane from './lib/layout/Pane.svelte';
  import Icon from './lib/ui/Icon.svelte';
  import { app, settings, saveSettings, studio, saveStudio, env, ui, isDark, togglePane, movePane, withPaneMoved, say } from './lib/state.svelte';
  import { togglePlay, step, stepCycle, scaleSpeed, fmtSpeed } from './lib/timing.svelte';
  import { compile, scheduleAutoCompile, decodeShare, loadExample, live } from './lib/actions.svelte';
  import { examples } from './lib/examples';
  import { PANES, PANE_IDS, PRESETS, HEADER, clone, geometry, type PaneId, type Side, type Box } from './lib/layout/layout';
  import type { IconName } from './lib/ui/icons';

  let life: LifeView | undefined = $state();

  $effect(() => {
    document.documentElement.dataset.theme = isDark() ? 'dark' : 'light';
  });
  $effect(() => {
    void JSON.stringify(settings);
    saveSettings();
  });
  $effect(() => {
    void JSON.stringify(studio);
    saveStudio();
  });

  const geo = $derived(geometry(studio.layout, studio.panelStyle === 'floating', env.vw, env.vh));
  const fl = $derived(studio.panelStyle === 'floating');
  /** Where the universe canvas sits, and the part of it not covered by floating docks. */
  const canvasBox = $derived(fl ? { x: 0, y: HEADER, w: env.vw, h: env.vh - HEADER } : geo.view);
  const insets = $derived(fl ? { l: geo.cl, r: geo.cr, t: 12, b: (geo.cb || 12) + 70 } : { l: 0, r: 0, t: 0, b: 0 });
  const centerX = $derived(geo.cl + (env.vw - geo.cl - geo.cr) / 2);

  const nErr = $derived(live.diags.filter((d) => d.severity === 1).length);
  const hasError = $derived(nErr > 0 || (!!app.compileError && !app.compiling));
  const empty = $derived(!app.design && !app.compiling);
  const canvasOpacity = $derived(app.compiling ? 0.45 : hasError && app.design ? 0.55 : 1);
  const fileName = $derived(`${(app.exampleName || app.design?.name || 'untitled').toLowerCase()}.goldl`);

  // ---- Source ----
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  function onSourceChange(text: string) {
    app.src = text;
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      try {
        localStorage.setItem('goldl.src', app.src);
      } catch {}
    }, 400);
    scheduleAutoCompile();
  }

  // ---- Dragging: pane moves, dock edges, splits ----
  type Drag =
    | { type: 'move'; id: PaneId; x: number; y: number; moved: boolean }
    | { type: 'dock'; side: Side; size0: number; x: number; y: number }
    | { type: 'split'; a: PaneId; b: PaneId; wa: number; wb: number; sum: number; len: number; vertical: boolean; x: number; y: number };
  let drag: Drag | null = null;
  interface Target {
    side: Side;
    before: PaneId | null;
    label: string;
    box: Box;
  }
  let moving: { id: PaneId; x: number; y: number; target: Target | null } | null = $state(null);
  let resizing = $state(false);
  const SIDE_NAME: Record<Side, string> = { left: 'Left dock', right: 'Right dock', bottom: 'Bottom dock' };

  /** Where a pane dragged to (x, y) would land, with the resulting box. */
  function targetAt(id: PaneId, x: number, y: number): Target | null {
    const L = studio.layout;
    const vis = (p: PaneId) => !L.hidden.includes(p);
    let side: Side | null = null;
    let before: PaneId | null = null;
    let label = '';
    for (const pid of PANE_IDS) {
      const b = geo.panes[pid];
      if (!b || x < b.x || x > b.x + b.w || y < b.y || y > b.y + b.h) continue;
      if (pid === id) return null;
      const vertical = b.side !== 'bottom';
      const first = vertical ? y < b.y + b.h / 2 : x < b.x + b.w / 2;
      side = b.side;
      if (first) before = pid;
      else {
        const list = L[b.side].panes;
        const rest = list.slice(list.indexOf(pid) + 1).filter((p) => p !== id && vis(p));
        before = rest[0] ?? null;
      }
      label = `${SIDE_NAME[b.side]} · ${first ? (vertical ? 'above' : 'left of') : vertical ? 'below' : 'right of'} ${PANES[pid].label}`;
      break;
    }
    if (!side) {
      const W = env.vw;
      const H = env.vh;
      const has = (s: Side) => L[s].panes.some((p) => p !== id && vis(p));
      if (x < (has('left') ? 56 : W * 0.2)) side = 'left';
      else if (x > W - (has('right') ? 56 : W * 0.2)) side = 'right';
      else if (y > H - (has('bottom') ? 70 : H * 0.28)) side = 'bottom';
      if (!side) return null;
      label = has(side) ? `${SIDE_NAME[side]} · at the end` : SIDE_NAME[side];
    }
    const next = withPaneMoved(L, id, side, before);
    if (JSON.stringify(next) === JSON.stringify(L)) return null;
    const box = geometry(next, fl, env.vw, env.vh).panes[id];
    return box ? { side, before, label, box } : null;
  }

  function startDrag(e: PointerEvent, d: Drag) {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    drag = d;
    ui.paneMenu = null;
    ui.menu = null;
    if (d.type !== 'move') resizing = true;
  }
  function onpointermove(e: PointerEvent) {
    const d = drag;
    if (!d) return;
    const dx = e.clientX - d.x;
    const dy = e.clientY - d.y;
    const L = studio.layout;
    if (d.type === 'move') {
      if (!d.moved && Math.abs(dx) + Math.abs(dy) > 4) d.moved = true;
      if (d.moved) moving = { id: d.id, x: e.clientX, y: e.clientY, target: targetAt(d.id, e.clientX, e.clientY) };
    } else if (d.type === 'dock') {
      const v = d.side === 'left' ? d.size0 + dx : d.side === 'right' ? d.size0 - dx : d.size0 - dy;
      L[d.side].size = Math.round(Math.min(d.side === 'bottom' ? 480 : 900, Math.max(d.side === 'bottom' ? 110 : 220, v)));
    } else {
      const tot = d.wa + d.wb;
      const f = ((d.vertical ? dy : dx) / d.len) * d.sum;
      const a = Math.max(0.15 * tot, Math.min(0.85 * tot, d.wa + f));
      L.w[d.a] = a;
      L.w[d.b] = tot - a;
    }
  }
  function onpointerup() {
    const d = drag;
    drag = null;
    resizing = false;
    if (d?.type === 'move') {
      const t = moving?.target;
      moving = null;
      if (t) movePane(d.id, t.side, t.before);
    }
  }
  function cancelDrag() {
    drag = null;
    moving = null;
    resizing = false;
  }
  const DEFAULT_SIZE: Record<Side, number> = { left: 460, right: 280, bottom: 190 };

  // Menus close on any press outside a menu or its trigger.
  $effect(() => {
    const down = (e: PointerEvent) => {
      if (!ui.menu && !ui.paneMenu) return;
      const t = e.target as Element | null;
      if (t?.closest?.('.menu, [data-menu]')) return;
      ui.menu = null;
      ui.paneMenu = null;
    };
    window.addEventListener('pointerdown', down, true);
    return () => window.removeEventListener('pointerdown', down, true);
  });

  // ---- Keyboard ----
  function onkeydown(e: KeyboardEvent) {
    const t = e.target as HTMLElement;
    const typing = t.tagName === 'TEXTAREA' || t.tagName === 'INPUT' || t.tagName === 'SELECT';
    const mod = e.metaKey || e.ctrlKey;
    if (e.key === 'Escape') {
      ui.menu = null;
      ui.paneMenu = null;
      if (drag) cancelDrag();
    }
    if (mod && e.key === 'Enter' && !typing) compile();
    if (mod && !e.shiftKey && !e.altKey) {
      const id = ({ b: 'code', j: 'checks', i: 'inspector' } as Record<string, PaneId>)[e.key.toLowerCase()];
      if (id) {
        e.preventDefault();
        togglePane(id);
        return;
      }
    }
    if (e.altKey && /^Digit[1-4]$/.test(e.code)) {
      e.preventDefault();
      studio.layout = clone(PRESETS[Number(e.code.slice(5)) - 1].layout);
      return;
    }
    if (typing || mod || e.altKey) return;
    if ((e.target as HTMLElement).closest?.('.timing') && e.key.startsWith('Arrow')) return;
    if (e.key === ' ') {
      e.preventDefault();
      if (e.shiftKey) settings.timing.reverse = !settings.timing.reverse;
      togglePlay();
    }
    if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
      if (!app.design) return;
      e.preventDefault();
      const d = e.key === 'ArrowRight' ? 1 : -1;
      if (e.shiftKey) stepCycle(d);
      else step(d);
    }
    if (e.key === '[' || e.key === ']') {
      scaleSpeed(e.key === ']' ? 2 : 0.5);
      say(`Speed ${fmtSpeed()}`, 'info', 1200);
    }
    if (e.key === 'f' && app.view === 'life') life?.fitAll();
    if (/^[1-3]$/.test(e.key)) app.view = (['life', 'schematic', 'waves'] as const)[Number(e.key) - 1];
  }

  onMount(async () => {
    const m = /#code=([A-Za-z0-9_-]+)/.exec(location.hash);
    let src: string | null = null;
    if (m) {
      try {
        src = await decodeShare(m[1]);
      } catch {
        say('Could not decode the shared link', 'circle-x');
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
    } else {
      app.exampleName = examples.find((e) => e.src === src)?.id ?? '';
    }
    app.src = src;
    compile();
  });

  const box = (b: { x: number; y: number; w: number; h: number }) => `left:${b.x}px;top:${b.y}px;width:${b.w}px;height:${b.h}px`;
  const STARTERS = [
    ['counter', 'Counter', '801 components'],
    ['traffic_light', 'Traffic light', '1,726 components'],
    ['alu', '8-bit ALU', '8,612 components'],
  ];
</script>

<svelte:window {onkeydown} {onpointermove} {onpointerup} />

<div class="studio" class:noselect={!!moving || resizing} class:resizing>
  <!-- The universe (kept mounted so its camera survives view switches). -->
  <div class="canvas" style="{box(canvasBox)}; visibility:{app.view === 'life' ? 'visible' : 'hidden'}">
    <div class="fade" style="opacity:{canvasOpacity}"><LifeView bind:this={life} {insets} /></div>
  </div>
  {#if app.view !== 'life'}
    <div class="viewarea" class:floating={fl} style="{box(geo.view)}; opacity:{canvasOpacity}">
      {#if app.view === 'schematic'}<Schematic />{:else}<Waves />{/if}
    </div>
  {/if}

  <Header />

  <!-- Docks and panes. -->
  {#if !fl}
    {#each geo.docks as d}
      <div class="dockbg {d.side}" style={box(d.box)}></div>
    {/each}
  {/if}
  {#each PANE_IDS as id (id)}
    {@const r = geo.panes[id]}
    <div class="pane" class:floating={fl} class:menuopen={ui.paneMenu === id} style={r ? box(r) : 'display:none'}>
      {#if r}
        {#if id === 'code'}
          <Pane {id} side={r.side} title={fileName} dimmed={moving?.id === id} onmovestart={(e) => startDrag(e, { type: 'move', id, x: e.clientX, y: e.clientY, moved: false })}>
            <div class="editor-wrap">
              <Editor
                bind:value={app.src}
                fontSize={settings.fontSize}
                tabSize={settings.tabSize}
                lineNumbers={settings.lineNumbers}
                inlayHints={settings.inlayHints}
                autoClose={settings.autoCloseBrackets}
                highlight={app.highlight}
                focusRequest={app.editorFocus}
                onchange={onSourceChange}
                ondiagnostics={(d) => (live.diags = d)}
                onrun={compile}
              />
            </div>
          </Pane>
        {:else if id === 'checks'}
          <Pane {id} side={r.side} title={PANES.checks.label} badge={nErr ? String(nErr) : ''} badgeErr dimmed={moving?.id === id} onmovestart={(e) => startDrag(e, { type: 'move', id, x: e.clientX, y: e.clientY, moved: false })}>
            <Checks />
          </Pane>
        {:else}
          <Pane {id} side={r.side} title={PANES.inspector.label} dimmed={moving?.id === id} onmovestart={(e) => startDrag(e, { type: 'move', id, x: e.clientX, y: e.clientY, moved: false })}>
            <Inspector />
          </Pane>
        {/if}
      {/if}
    </div>
  {/each}
  {#each geo.splits as s}
    <div
      class="split"
      class:v={s.vertical}
      class:floating={fl}
      style={box(s.box)}
      role="separator"
      onpointerdown={(e) => startDrag(e, { type: 'split', a: s.a, b: s.b, wa: studio.layout.w[s.a] || 1, wb: studio.layout.w[s.b] || 1, sum: s.sum, len: s.len, vertical: s.vertical, x: e.clientX, y: e.clientY })}
    ><i></i></div>
  {/each}
  {#each geo.edges as ed}
    <div
      class="edge {ed.side}"
      style={box(ed.box)}
      role="separator"
      title="Drag to resize · double-click to reset"
      ondblclick={() => (studio.layout[ed.side].size = DEFAULT_SIZE[ed.side])}
      onpointerdown={(e) => startDrag(e, { type: 'dock', side: ed.side, size0: ed.size, x: e.clientX, y: e.clientY })}
    ></div>
  {/each}

  <!-- State banner, empty state. -->
  {#if !empty && (app.compiling || hasError)}
    <button class="banner" style="left:{centerX}px" onclick={() => (togglePane('checks', true), (ui.checkTab = 'problems'))}>
      {#if app.compiling}
        <span class="spinner"></span>Compiling {app.design?.name ?? ''} · elaboration → layout
      {:else}
        <span class="warn"><Icon name="triangle-alert" /></span>{app.design ? `Last successful build · ${nErr || 1} error${nErr > 1 ? 's' : ''} to fix` : app.compileError.split('\n')[0]}
      {/if}
    </button>
  {/if}
  {#if empty && app.view === 'life'}
    <div class="emptycard" style="left:{centerX}px">
      <span class="et">An empty universe</span>
      <span class="eb">{app.compileError ? 'The design did not compile yet: fix the problems, or start from an example.' : 'Compile a module to lay it out as gliders, reflectors and duplicators, or start from an example.'}</span>
      <div class="starters">
        {#each STARTERS as [id, title, sub]}
          <button onclick={() => loadExample(id)}><span class="two"><span>{title}</span><span class="sub">{sub}</span></span><span class="dim"><Icon name="arrow-right" /></span></button>
        {/each}
      </div>
    </div>
  {/if}

  <div class="transport" class:floating={fl} style={fl ? `left:${(geo.cl || 0) + 10}px; right:${(geo.cr || 0) + 10}px; bottom:${(geo.cb || 10) + 6}px; height:54px` : box(geo.transport)}>
    <Transport width={geo.centerW} floating={fl} />
  </div>

  {#if moving}
    <div class="dragveil"></div>
    {#each ['left', 'right', 'bottom'] as const as sd}
      {#if !studio.layout[sd].panes.some((p) => p !== moving!.id && !studio.layout.hidden.includes(p))}
        <div class="hint {sd}" class:on={moving.target?.side === sd}>{SIDE_NAME[sd]}</div>
      {/if}
    {/each}
    {#if moving.target}
      <div class="preview" style={box(moving.target.box)}><span>{moving.target.label}</span></div>
    {/if}
    <div class="ghost" style="left:{Math.min(moving.x + 12, env.vw - 260)}px; top:{Math.min(moving.y + 10, env.vh - 50)}px">
      <span class="dim"><Icon name={PANES[moving.id].icon} /></span>{PANES[moving.id].label}
      <span class="gk">{moving.target ? 'release to dock' : 'Esc cancels'}</span>
    </div>
  {/if}

  {#if ui.toast}
    <div class="toast" style="left:{centerX}px; bottom:{(fl ? (geo.cb || 10) + 76 : geo.cb + 56) + 14}px">
      <Icon name={ui.toast.icon as IconName} />{ui.toast.text}
    </div>
  {/if}
</div>

<style>
  .studio {
    position: relative;
    height: 100vh;
    overflow: hidden;
    background: var(--canvas);
    color: var(--fg);
  }
  .studio.noselect {
    user-select: none;
  }
  .studio:not(.resizing) .pane,
  .studio:not(.resizing) .split,
  .studio:not(.resizing) .dockbg {
    transition:
      left 0.18s ease,
      top 0.18s ease,
      width 0.18s ease,
      height 0.18s ease;
  }
  .canvas,
  .viewarea {
    position: absolute;
  }
  .fade {
    width: 100%;
    height: 100%;
    transition: opacity 0.2s;
  }
  .viewarea {
    overflow: hidden;
    background: var(--canvas);
  }
  .viewarea.floating {
    border-radius: 12px;
    border: 1px solid var(--border);
  }
  .dockbg {
    position: absolute;
    background: var(--panel);
    z-index: 19;
  }
  .dockbg.left {
    border-right: 1px solid var(--border);
  }
  .dockbg.right {
    border-left: 1px solid var(--border);
  }
  .dockbg.bottom {
    border-top: 1px solid var(--border);
  }
  .pane {
    position: absolute;
    background: var(--panel);
    z-index: 20;
    overflow: hidden;
  }
  .pane.menuopen {
    z-index: 46;
  }
  .pane.floating {
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: var(--shadow-sm);
  }
  .editor-wrap {
    flex: 1;
    min-height: 0;
  }
  .split {
    position: absolute;
    z-index: 21;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: col-resize;
    touch-action: none;
  }
  .split.v {
    cursor: row-resize;
  }
  .split i {
    width: 1px;
    height: 100%;
    background: var(--border);
  }
  .split.v i {
    width: 100%;
    height: 1px;
  }
  .split.floating i {
    display: none;
  }
  .edge {
    position: absolute;
    z-index: 25;
    cursor: col-resize;
    touch-action: none;
  }
  .edge.bottom {
    cursor: row-resize;
  }
  .edge:hover,
  .split:hover {
    background: var(--accent-soft);
  }
  .transport {
    position: absolute;
    z-index: 22;
  }
  .transport.floating {
    width: fit-content;
    max-width: calc(100% - 20px);
    margin: 0 auto;
  }
  .banner {
    position: absolute;
    top: 104px;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 12px;
    border-radius: 9px;
    background: var(--raised);
    border: 1px solid var(--border);
    box-shadow: var(--shadow-sm);
    font-size: 12.5px;
    z-index: 16;
    max-width: 60vw;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .warn {
    color: var(--warn);
    display: inline-flex;
  }
  .emptycard {
    position: absolute;
    top: 50%;
    transform: translate(-50%, -50%);
    width: 360px;
    padding: 20px;
    background: var(--raised);
    border: 1px solid var(--border);
    border-radius: 14px;
    box-shadow: var(--shadow);
    display: flex;
    flex-direction: column;
    gap: 12px;
    z-index: 16;
  }
  .et {
    font-size: 15px;
    font-weight: 600;
  }
  .eb {
    color: var(--fg-dim);
    line-height: 1.5;
  }
  .starters {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .starters button {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 9px 12px;
    border: 1px solid var(--border);
    border-radius: 9px;
    text-align: left;
  }
  .starters button:hover {
    background: var(--hover);
  }
  .two {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .sub {
    font-size: 11.5px;
    color: var(--fg-dim);
  }
  .dim {
    color: var(--fg-dim);
    display: inline-flex;
  }
  .dragveil {
    position: absolute;
    inset: 48px 0 0 0;
    z-index: 58;
    cursor: grabbing;
  }
  .hint {
    position: absolute;
    z-index: 59;
    border: 2px dashed var(--border);
    border-radius: 12px;
    display: grid;
    place-items: center;
    color: var(--fg-faint);
    font-size: 12.5px;
    font-weight: 500;
    pointer-events: none;
    background: color-mix(in oklab, var(--panel) 40%, transparent);
  }
  .hint.on {
    opacity: 0;
  }
  .hint.left {
    left: 10px;
    top: 58px;
    bottom: 10px;
    width: 18%;
  }
  .hint.right {
    right: 10px;
    top: 58px;
    bottom: 10px;
    width: 18%;
  }
  .hint.bottom {
    left: 22%;
    right: 22%;
    bottom: 10px;
    height: 24%;
  }
  .preview {
    position: absolute;
    z-index: 60;
    border: 2px solid var(--accent);
    border-radius: 10px;
    background: var(--accent-soft);
    box-shadow: 0 0 0 4px color-mix(in oklab, var(--accent) 12%, transparent);
    display: grid;
    place-items: center;
    pointer-events: none;
    transition:
      left 0.14s ease,
      top 0.14s ease,
      width 0.14s ease,
      height 0.14s ease;
  }
  .preview span {
    padding: 5px 10px;
    border-radius: 7px;
    background: var(--raised);
    border: 1px solid var(--border);
    font-size: 12px;
    font-weight: 500;
    box-shadow: var(--shadow-sm);
  }
  .gk {
    color: var(--fg-faint);
    font-size: 11.5px;
    margin-left: 4px;
  }
  .ghost {
    position: absolute;
    z-index: 61;
    pointer-events: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-radius: 9px;
    background: var(--raised);
    border: 1px solid var(--accent);
    box-shadow: var(--shadow);
    font-size: 12.5px;
  }
  .toast {
    position: absolute;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 9px 14px;
    border-radius: 10px;
    background: var(--fg);
    color: var(--bg);
    font-size: 12.5px;
    z-index: 45;
    white-space: nowrap;
    max-width: 80vw;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
