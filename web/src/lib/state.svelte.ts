// Global reactive state of the playground (Svelte 5 runes).
import type { Design, Diag, TestResult } from './types';
import { PRESETS, clone, sanitize, type Layout, type PaneId } from './layout/layout';

export type ThemePref = 'system' | 'light' | 'dark';
export type PanelStyle = 'docked' | 'floating';

export interface Settings {
  fontSize: number;
  tabSize: number;
  lineNumbers: boolean;
  inlayHints: boolean;
  autoCloseBrackets: boolean;
  autoCompile: boolean;
  palette: 'classic' | 'neon' | 'amber';
  /** Glider halo. */
  glow: boolean;
  overlay: boolean;
  overlayLabels: boolean;
  componentOutlines: boolean;
  wires: boolean;
  /** I/O pins drawn in the universe. */
  pins: boolean;
  timing: Timing;
}

/** Playback: speed and step size, each in generations or clock cycles. */
export interface Timing {
  speed: number;
  speedUnit: 'gen' | 'cycle';
  step: number;
  stepUnit: 'gen' | 'cycle';
  reverse: boolean;
  /** Keep replaying the current clock cycle. */
  loop: boolean;
}

const defaults: Settings = {
  fontSize: 13,
  tabSize: 2,
  lineNumbers: true,
  inlayHints: true,
  autoCloseBrackets: true,
  autoCompile: true,
  palette: 'classic',
  glow: false,
  overlay: true,
  overlayLabels: true,
  componentOutlines: true,
  wires: true,
  pins: true,
  timing: { speed: 0.125, speedUnit: 'cycle', step: 1, stepUnit: 'gen', reverse: false, loop: false },
};

function load<T extends object>(key: string, fallback: T): T {
  try {
    const v = localStorage.getItem(key);
    return v ? { ...fallback, ...JSON.parse(v) } : fallback;
  } catch {
    return fallback;
  }
}

const loaded = load('goldl.settings.v2', defaults);
loaded.timing = { ...defaults.timing, ...(loaded.timing ?? {}) };
export const settings: Settings = $state(loaded);

export function saveSettings() {
  try {
    localStorage.setItem('goldl.settings.v2', JSON.stringify(settings));
  } catch {}
}

// ---- Studio layout (persisted under `goldl.layout`) ----
interface Studio {
  layout: Layout;
  panelStyle: PanelStyle;
  themePref: ThemePref;
}
function loadStudio(): Studio {
  const s = load<Partial<Studio>>('goldl.layout', {});
  return {
    layout: sanitize(s.layout) ?? clone(PRESETS[0].layout),
    panelStyle: s.panelStyle === 'floating' ? 'floating' : 'docked',
    themePref: s.themePref === 'light' || s.themePref === 'dark' ? s.themePref : 'system',
  };
}
export const studio: Studio = $state(loadStudio());
export function saveStudio() {
  try {
    localStorage.setItem('goldl.layout', JSON.stringify(studio));
  } catch {}
}

const mq = typeof matchMedia !== 'undefined' ? matchMedia('(prefers-color-scheme: dark)') : null;
export const env = $state({ sysDark: mq?.matches ?? true, vw: window.innerWidth, vh: window.innerHeight });
mq?.addEventListener('change', (e) => (env.sysDark = e.matches));
window.addEventListener('resize', () => {
  env.vw = window.innerWidth;
  env.vh = window.innerHeight;
});

export function isDark(): boolean {
  return studio.themePref === 'system' ? env.sysDark : studio.themePref === 'dark';
}

export function togglePane(id: PaneId, show?: boolean) {
  const L = studio.layout;
  const hidden = L.hidden.includes(id);
  const on = show ?? hidden;
  L.hidden = L.hidden.filter((x) => x !== id);
  if (!on) L.hidden.push(id);
}
/** Move a pane to a dock, before pane `before` (or at the end). */
export function movePane(id: PaneId, side: 'left' | 'right' | 'bottom', before: PaneId | null = null) {
  studio.layout = withPaneMoved(studio.layout, id, side, before);
}
export function withPaneMoved(L0: Layout, id: PaneId, side: 'left' | 'right' | 'bottom', before: PaneId | null): Layout {
  const L = clone(L0);
  for (const s of ['left', 'right', 'bottom'] as const) L[s].panes = L[s].panes.filter((x) => x !== id);
  const list = L[side].panes;
  const at = before === null ? -1 : list.indexOf(before);
  if (at < 0) list.push(id);
  else list.splice(at, 0, id);
  L.hidden = L.hidden.filter((x) => x !== id);
  return L;
}
/** Swap a pane with its visible neighbour in the same dock. */
export function shiftPane(id: PaneId, d: -1 | 1) {
  const L = clone(studio.layout);
  for (const s of ['left', 'right', 'bottom'] as const) {
    const vis = L[s].panes.filter((x) => !L.hidden.includes(x));
    const i = vis.indexOf(id);
    if (i < 0) continue;
    const j = i + d;
    if (j < 0 || j >= vis.length) return;
    const a = L[s].panes.indexOf(id);
    const b = L[s].panes.indexOf(vis[j]);
    [L[s].panes[a], L[s].panes[b]] = [L[s].panes[b], L[s].panes[a]];
  }
  studio.layout = L;
}
export function paneVisible(id: PaneId) {
  return !studio.layout.hidden.includes(id);
}

// ---- Transient UI state ----
export type MenuId = 'examples' | 'layout' | 'share' | 'settings' | 'layers' | 'timing' | null;
export const ui = $state({
  menu: null as MenuId,
  paneMenu: null as PaneId | null,
  /** Screen anchor of a fixed-position menu: its trigger's rectangle. */
  anchor: null as { x: number; y: number; w: number; h: number } | null,
  checkTab: 'problems' as 'problems' | 'tests',
  toast: null as { text: string; icon: string } | null,
  verifying: false,
  /** Schematic navigation: group scope, or an operator's gates. */
  schematic: { scope: 0, gates: null as number | null },
});

let toastTimer: ReturnType<typeof setTimeout> | undefined;
export function say(text: string, icon = 'info', ms = 3200) {
  ui.toast = { text, icon };
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (ui.toast = null), ms);
}

export type View = 'life' | 'schematic' | 'waves';

export const app = $state({
  src: '',
  exampleName: '',
  design: null as Design | null,
  /** Diagnostics from the language server (live) and from the last compilation. */
  diags: [] as Diag[],
  compiling: false,
  compileError: '' as string,
  compileMs: 0,
  tests: [] as TestResult[],
  /** Current generation of the Life universe. */
  gen: 0,
  playing: false,
  /** First generation of the cycle being looped (when looping). */
  loopFrom: 0,
  view: 'life' as View,
  /** Current values of the input ports (as typed by the user). */
  inputs: [] as bigint[],
  /** Bumped whenever the simulation inputs change, so views refetch. */
  simVersion: 0,
  selectedGroup: null as number | null,
  /** Source span to highlight in the editor (from schematic/overlay selection). */
  highlight: null as [number, number] | null,
  /** Request the Life view to frame a region: [x0, y0, x1, y1]. */
  frame: null as [number, number, number, number] | null,
  /** Request the editor to reveal a line (0-based). */
  revealLine: null as number | null,
});

export function period(): number {
  return app.design?.period ?? 1;
}

export function cycleAt(g: number): number {
  return Math.floor(g / period());
}

// ---- Groups: hierarchical names ----
export function groupPath(d: Design, g: number): number[] {
  const out: number[] = [];
  let cur: number | null | undefined = g;
  while (cur !== null && cur !== undefined && out.length < 64) {
    out.unshift(cur);
    cur = d.groups[cur]?.parent;
  }
  return out;
}
export function groupLabel(d: Design, g: number): string {
  const gr = d.groups[g];
  if (!gr) return '';
  return gr.label || gr.name;
}
export function groupTitle(d: Design, g: number, sep = ' › '): string {
  return groupPath(d, g)
    .map((x) => groupLabel(d, x))
    .join(sep);
}
