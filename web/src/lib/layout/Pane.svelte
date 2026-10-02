<script lang="ts">
  // One dockable pane: title bar (drag to move, ⋯ menu, hide) around its content.
  import type { Snippet } from 'svelte';
  import Icon from '../ui/Icon.svelte';
  import { PANES, type PaneId, type Side } from './layout';
  import { ui, togglePane, movePane } from '../state.svelte';

  interface Props {
    id: PaneId;
    side: Side;
    title: string;
    badge?: string;
    badgeErr?: boolean;
    dimmed?: boolean;
    onmovestart: (e: PointerEvent) => void;
    children: Snippet;
  }
  let { id, side, title, badge = '', badgeErr = false, dimmed = false, onmovestart, children }: Props = $props();
  const P = $derived(PANES[id]);
  const moves = $derived(
    (
      [
        ['left', 'Dock left', 'panel-left'],
        ['right', 'Dock right', 'panel-right'],
        ['bottom', 'Dock bottom', 'panel-bottom'],
      ] as const
    ).filter(([k]) => k !== side),
  );
</script>

<div class="pane" class:dimmed>
  <div class="bar">
    <div class="grab" onpointerdown={onmovestart} title="Drag to move" role="presentation">
      <span class="grip"><Icon name="grip-vertical" size={13} /></span>
      <span class="pi"><Icon name={P.icon} size={14} /></span>
      <span class="title">{title}</span>
      {#if badge}<span class="badge" class:err={badgeErr}>{badge}</span>{/if}
    </div>
    <button class="icon-btn" class:on={ui.paneMenu === id} title="Panel options" onclick={() => ((ui.paneMenu = ui.paneMenu === id ? null : id), (ui.menu = null))}><Icon name="ellipsis" size={14} /></button>
    <button class="icon-btn" title="Hide ({P.key})" onclick={() => togglePane(id, false)}><Icon name="x" size={14} /></button>
    {#if ui.paneMenu === id}
      <div class="menu pmenu">
        {#each moves as [k, label, icon]}
          <button class="menu-row" onclick={() => (movePane(id, k), (ui.paneMenu = null))}><span class="dim"><Icon name={icon} size={14} /></span>{label}</button>
        {/each}
        <button class="menu-row" onclick={() => (togglePane(id, false), (ui.paneMenu = null))}><span class="dim"><Icon name="eye-off" size={14} /></span><span class="grow">Hide</span><span class="key">{P.key}</span></button>
      </div>
    {/if}
  </div>
  <div class="body">{@render children()}</div>
</div>

<style>
  .pane {
    position: relative;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
  }
  .pane.dimmed {
    opacity: 0.4;
  }
  .bar {
    position: relative;
    height: 36px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 4px 0 6px;
    border-bottom: 1px solid var(--border-soft);
  }
  .grab {
    flex: 1;
    min-width: 0;
    height: 100%;
    display: flex;
    align-items: center;
    gap: 7px;
    cursor: grab;
    touch-action: none;
    padding-left: 2px;
    user-select: none;
  }
  .grip {
    color: var(--fg-faint);
  }
  .pi,
  .dim {
    color: var(--fg-dim);
  }
  .title {
    font-size: 12.5px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .badge {
    font: 11px var(--mono);
    color: var(--fg-faint);
  }
  .badge.err {
    color: var(--err);
  }
  .pmenu {
    top: 34px;
    right: 6px;
    width: 190px;
    padding: 5px;
    border-radius: 10px;
  }
  .grow {
    flex: 1;
  }
  .key {
    font: 11px var(--mono);
    color: var(--fg-faint);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
</style>
