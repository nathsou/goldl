// Global reactive state of the playground (Svelte 5 runes).
import type { Design, Diag, TestResult } from './types';

export interface Settings {
  fontSize: number;
  tabSize: number;
  lineNumbers: boolean;
  inlayHints: boolean;
  autoCloseBrackets: boolean;
  autoCompile: boolean;
  theme: 'dark' | 'light';
  palette: 'neon' | 'classic' | 'amber';
  glow: boolean;
  overlay: boolean;
  overlayLabels: boolean;
  componentOutlines: boolean;
  wires: boolean;
}

const defaults: Settings = {
  fontSize: 13,
  tabSize: 2,
  lineNumbers: true,
  inlayHints: true,
  autoCloseBrackets: true,
  autoCompile: true,
  theme: 'dark',
  palette: 'neon',
  glow: true,
  overlay: true,
  overlayLabels: true,
  componentOutlines: true,
  wires: true,
};

function load<T>(key: string, fallback: T): T {
  try {
    const v = localStorage.getItem(key);
    return v ? { ...fallback, ...JSON.parse(v) } : fallback;
  } catch {
    return fallback;
  }
}

export const settings: Settings = $state(load('goldl.settings', defaults));

export function saveSettings() {
  try {
    localStorage.setItem('goldl.settings', JSON.stringify(settings));
  } catch {}
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
  /** Generations per second while playing. */
  speed: 2000,
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
  hoverInfo: '' as string,
});

export function period(): number {
  return app.design?.period ?? 1;
}

export function cycleAt(g: number): number {
  return Math.floor(g / period());
}
