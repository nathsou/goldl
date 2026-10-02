<script lang="ts">
  // Problems (live diagnostics and compile errors) and test bench results.
  import Icon from './ui/Icon.svelte';
  import { app, ui } from './state.svelte';
  import { live, gotoLine } from './actions.svelte';

  const errs = $derived(live.diags.filter((d) => d.severity <= 2));
  const nErr = $derived(live.diags.filter((d) => d.severity === 1).length);
  const failed = $derived(app.tests.filter((t) => !t.passed).length);
  const file = $derived(`${(app.exampleName || app.design?.name || 'main').toLowerCase()}.goldl`);
</script>

<div class="tabs">
  <button class:on={ui.checkTab === 'problems'} onclick={() => (ui.checkTab = 'problems')}>Problems <span class="b" class:err={nErr > 0}>{errs.length}</span></button>
  <button class:on={ui.checkTab === 'tests'} onclick={() => (ui.checkTab = 'tests')}>Tests <span class="b" class:err={failed > 0}>{app.tests.length ? `${app.tests.length - failed}/${app.tests.length}` : '0'}</span></button>
</div>
<div class="list">
  {#if ui.checkTab === 'problems'}
    {#each errs as d}
      <div class="card" class:warn={d.severity === 2}>
        <span class="ic"><Icon name={d.severity === 1 ? 'circle-x' : 'triangle-alert'} /></span>
        <div class="txt">
          <span class="msg">{d.message}</span>
          <button class="loc" onclick={() => gotoLine(d.range.start.line)}>{file}:{d.range.start.line + 1}:{d.range.start.character + 1}</button>
        </div>
      </div>
    {/each}
    {#if app.compileError && errs.length === 0}
      <div class="card">
        <span class="ic"><Icon name="circle-x" /></span>
        <div class="txt"><span class="msg">{app.compileError}</span></div>
      </div>
    {/if}
    {#if errs.length === 0 && !app.compileError}
      <div class="none"><span class="ok"><Icon name="circle-check" /></span>No problems</div>
    {/if}
  {:else}
    {#if app.tests.length === 0}
      <div class="none">No test benches. Add one with <code>test "name" for Module {'{ … }'}</code>.</div>
    {/if}
    {#each app.tests as t}
      <button class="test" onclick={() => gotoLine((t.failLine ?? t.line) as number)}>
        <span class={t.passed ? 'ok' : 'bad'}><Icon name={t.passed ? 'circle-check' : 'circle-x'} /></span>
        <span class="name">{t.name}{#if t.message}<span class="why"> — {t.message}</span>{/if}</span>
        <span class="cyc">{t.cycles} cycles</span>
      </button>
    {/each}
  {/if}
</div>

<style>
  .tabs {
    display: flex;
    gap: 2px;
    padding: 6px 8px 0;
  }
  .tabs button {
    height: 26px;
    padding: 0 9px;
    border-radius: 6px;
    color: var(--fg-dim);
    font-size: 12.5px;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .tabs button.on {
    background: var(--hover);
    color: var(--fg);
  }
  .b {
    font: 11px var(--mono);
    color: var(--fg-faint);
  }
  .b.err {
    color: var(--err);
  }
  .list {
    flex: 1;
    overflow: auto;
    padding: 8px;
    font-size: 12.5px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .card {
    display: flex;
    gap: 10px;
    padding: 9px 10px;
    border-radius: 8px;
    background: var(--err-soft);
    align-items: flex-start;
  }
  .card .ic {
    color: var(--err);
    margin-top: 1px;
  }
  .card.warn {
    background: color-mix(in oklab, var(--warn) 10%, transparent);
  }
  .card.warn .ic {
    color: var(--warn);
  }
  .txt {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .msg {
    line-height: 1.4;
    white-space: pre-wrap;
  }
  .loc {
    align-self: flex-start;
    color: var(--fg-dim);
    font-size: 12px;
  }
  .loc:hover {
    color: var(--fg);
  }
  .none {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px;
    color: var(--fg-dim);
  }
  .ok {
    color: var(--ok);
  }
  .bad {
    color: var(--err);
  }
  .test {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border-radius: 8px;
    text-align: left;
  }
  .test:hover {
    background: var(--hover);
  }
  .name {
    flex: 1;
    min-width: 0;
    white-space: normal;
  }
  .why {
    color: var(--fg-dim);
  }
  .cyc {
    color: var(--fg-faint);
    font: 11.5px var(--mono);
  }
  code {
    font-family: var(--mono);
    background: var(--code-bg);
    padding: 0 4px;
    border-radius: 3px;
  }
</style>
