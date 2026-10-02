<script lang="ts">
  // Inputs, the selected region (with its place in the design hierarchy), and design statistics.
  import Icon from './ui/Icon.svelte';
  import Switch from './ui/Switch.svelte';
  import { app, ui, groupLabel, groupPath } from './state.svelte';
  import { setInput, parseValue, lineOf } from './actions.svelte';
  import { groupBox, within } from './geom';

  const fmt = (n: number) => n.toLocaleString('en-US');
  const d = $derived(app.design);

  const sel = $derived.by(() => {
    const dd = d;
    const g = app.selectedGroup;
    if (!dd || g === null || !dd.groups[g]) return null;
    const gr = dd.groups[g];
    let gates = 0;
    let other = 0;
    const B = dd.blocks.data;
    for (let k = 0; k < B.length; k += 7) {
      if (!within(dd, B[k + 4], g)) continue;
      const kind = dd.blocks.kinds[B[k + 5]] ?? '';
      if (kind.startsWith('AND') || kind === 'NOT') gates++;
      else other++;
    }
    const children = dd.groups.map((x, i) => ({ x, i })).filter(({ x }) => x.parent === g);
    return { g, gr, gates, other, path: groupPath(dd, g), line: gr.span ? lineOf(gr.span[0]) + 1 : null, children };
  });

  function pick(g: number) {
    const dd = d;
    if (!dd) return;
    app.selectedGroup = g;
    const sp = dd.groups[g]?.span;
    if (sp) app.highlight = sp;
  }
  function frame(g: number) {
    if (!d) return;
    const b = groupBox(d, g);
    if (b) {
      app.frame = b;
      app.view = 'life';
    }
  }
  function openSchematic(g: number) {
    ui.schematic = { scope: g, gates: null };
    app.view = 'schematic';
  }
</script>

<div class="insp">
  <section>
    <span class="caption">Inputs</span>
    {#if d && d.inputs.length}
      {#each d.inputs as p, i}
        {@const v = app.inputs[i] ?? 0n}
        <div class="port">
          <span class="pname">{p.name} <span class="ty">{p.width === 1 ? 'bit' : `bits<${p.width}>`}</span></span>
          {#if p.width === 1}
            <button class="sw" title="{p.name} = {v ? 0 : 1}" onclick={() => setInput(i, v ^ 1n)}><Switch on={v === 1n} large /></button>
          {:else}
            <input
              class="num mono"
              value={String(v)}
              title="Decimal, 0x… or 0b…"
              onchange={(e) => {
                const x = parseValue((e.target as HTMLInputElement).value);
                if (x !== null) setInput(i, x);
              }}
            />
          {/if}
        </div>
        {#if p.width > 1 && p.width <= 16}
          <div class="bits">
            {#each Array(p.width) as _, b}
              {@const bit = p.width - 1 - b}
              {@const on = ((v >> BigInt(bit)) & 1n) === 1n}
              <button class="bit" class:on title="{p.name}[{bit}]" onclick={() => setInput(i, v ^ (1n << BigInt(bit)))}>{on ? 1 : 0}</button>
            {/each}
          </div>
        {/if}
      {/each}
      <span class="caption">Applies from the next clock cycle.</span>
    {:else}
      <span class="muted">{d ? 'This design has no inputs.' : 'Compile a design to drive its inputs.'}</span>
    {/if}
  </section>

  <section>
    <span class="caption">Selection</span>
    {#if sel && d}
      <div class="crumbs">
        {#each sel.path as g, k}
          {#if k}<span class="sep"><Icon name="chevron-right" size={11} /></span>{/if}
          <button class="crumb" class:cur={g === sel.g} onclick={() => pick(g)} title={d.groups[g].kind}>{groupLabel(d, g)}</button>
        {/each}
      </div>
      <div class="selrow">
        <span class="sname mono">{groupLabel(d, sel.g)}</span>
        <button class="icon-btn" title="Clear selection" onclick={() => (app.selectedGroup = null)}><Icon name="x" size={14} /></button>
      </div>
      <span class="meta">{sel.gr.kind} · {fmt(sel.gates)} gate{sel.gates === 1 ? '' : 's'}{sel.other ? ` · ${fmt(sel.other)} other blocks` : ''}{sel.line ? ` · line ${sel.line}` : ''}</span>
      <div class="acts">
        <button class="act" onclick={() => frame(sel.g)}><Icon name="crosshair" size={13} />Frame</button>
        <button class="act" onclick={() => openSchematic(sel.g)}><Icon name="network" size={13} />Schematic</button>
      </div>
      {#if sel.children.length}
        <span class="caption sub">Contains</span>
        <div class="kids">
          {#each sel.children as c}
            <button class="kid" onclick={() => pick(c.i)}><span class="mono">{groupLabel(d, c.i)}</span><span class="kk">{c.x.kind}</span></button>
          {/each}
        </div>
      {/if}
    {:else}
      <span class="muted">Click a region in the universe to see what it implements.</span>
    {/if}
  </section>

  {#if d}
    {@const s = d.stats}
    <section class="grid">
      <span class="caption full">Design</span>
      {#each [[fmt(s.aigAnds), 'AND gates'], [fmt(s.crossings), 'glider crossings'], [fmt(s.regBits), 'register bits'], [fmt(s.components), 'Life components'], [fmt(s.period), 'generations / cycle'], [`${fmt(s.width)} × ${fmt(s.height)}`, 'cells, w × h']] as [v, l]}
        <div class="stat"><span class="v mono">{v}</span><span class="l">{l}</span></div>
      {/each}
    </section>
  {/if}
</div>

<style>
  .insp {
    flex: 1;
    overflow: auto;
  }
  section {
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    border-bottom: 1px solid var(--border-soft);
  }
  .port {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .pname {
    font: 500 13px var(--mono);
  }
  .ty {
    color: var(--fg-faint);
    font-weight: 400;
  }
  .sw {
    display: flex;
  }
  .num {
    width: 86px;
    height: 26px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    font-size: 12px;
    text-align: right;
  }
  .num:focus {
    border-color: var(--accent);
  }
  .bits {
    display: flex;
    gap: 2px;
    flex-wrap: wrap;
    margin-top: -2px;
  }
  .bit {
    width: 22px;
    height: 22px;
    border-radius: 5px;
    border: 1px solid var(--border);
    font: 500 11.5px var(--mono);
    color: var(--fg-dim);
  }
  .bit.on {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-ink);
  }
  .muted {
    color: var(--fg-dim);
    font-size: 12.5px;
    line-height: 1.45;
  }
  .crumbs {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px;
    font-size: 11.5px;
  }
  .crumb {
    color: var(--fg-dim);
    font-family: var(--mono);
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    padding: 1px 3px;
    border-radius: 4px;
  }
  .crumb:hover {
    background: var(--hover);
    color: var(--fg);
  }
  .crumb.cur {
    color: var(--fg);
  }
  .sep {
    color: var(--fg-faint);
  }
  .selrow {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
  }
  .sname {
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    color: var(--fg-dim);
    font-size: 12.5px;
  }
  .acts {
    display: flex;
    gap: 6px;
  }
  .act {
    height: 26px;
    padding: 0 9px;
    border: 1px solid var(--border);
    border-radius: 6px;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--fg);
  }
  .act:hover {
    background: var(--hover);
  }
  .sub {
    margin-top: 2px;
  }
  .kids {
    display: flex;
    flex-direction: column;
  }
  .kid {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    padding: 4px 6px;
    margin: 0 -6px;
    border-radius: 6px;
    font-size: 12px;
    text-align: left;
  }
  .kid:hover {
    background: var(--hover);
  }
  .kid .mono {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .kk {
    color: var(--fg-faint);
    font-size: 11.5px;
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px 10px;
    border-bottom: 0;
  }
  .full {
    grid-column: 1 / -1;
  }
  .stat {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .v {
    font-weight: 500;
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .l {
    font-size: 11.5px;
    color: var(--fg-dim);
  }
</style>
