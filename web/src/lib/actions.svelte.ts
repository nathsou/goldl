// Commands shared by the header, panes and transport bar.
import { app, settings, period, cycleAt, say, ui, togglePane } from './state.svelte';
import { engine } from './engine';
import { lsp, type LspDiag } from './lsp';
import { examples } from './examples';
import type { Design } from './types';

/** Live diagnostics from the language server. */
export const live = $state({ diags: [] as LspDiag[], caret: { line: 0, col: 0 } });

let compileSeq = 0;
let autoTimer: ReturnType<typeof setTimeout> | undefined;

export function scheduleAutoCompile() {
  if (!settings.autoCompile) return;
  clearTimeout(autoTimer);
  autoTimer = setTimeout(() => {
    if (!live.diags.some((d) => d.severity === 1)) compile();
  }, 1200);
}

export async function compile() {
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
        ui.schematic = { scope: 0, gates: null };
      } else {
        app.gen = Math.min(app.gen, (period() || 1) * 64);
      }
      app.simVersion++;
      app.selectedGroup = null;
    } else {
      app.compileError = r.diags.map((d) => d.message).join('\n') || 'compilation failed';
      ui.checkTab = 'problems';
    }
    engine.tests(src).then((t) => seq === compileSeq && (app.tests = t));
  } catch (e) {
    app.compileError = String(e);
    say(String(e), 'circle-x', 8000);
  } finally {
    if (seq === compileSeq) app.compiling = false;
  }
}

engine.onReset = () => {
  say('The compiler crashed and was restarted', 'circle-x', 6000);
  lsp.reset(app.src);
};

export function loadExample(id: string) {
  const ex = examples.find((e) => e.id === id);
  if (!ex) return;
  app.src = ex.src;
  app.exampleName = id;
  try {
    localStorage.setItem('goldl.src', ex.src);
  } catch {}
  history.replaceState(null, '', location.pathname);
  ui.menu = null;
  compile();
}

// ---- Sharing ----
async function encodeShare(src: string): Promise<string> {
  const cs = new CompressionStream('deflate-raw');
  const buf = await new Response(new Blob([src]).stream().pipeThrough(cs)).arrayBuffer();
  let s = '';
  new Uint8Array(buf).forEach((b) => (s += String.fromCharCode(b)));
  return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}
export async function decodeShare(code: string): Promise<string> {
  const bin = atob(code.replace(/-/g, '+').replace(/_/g, '/'));
  const bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0));
  const ds = new DecompressionStream('deflate-raw');
  return await new Response(new Blob([bytes]).stream().pipeThrough(ds)).text();
}
export async function copyLink() {
  ui.menu = null;
  const code = await encodeShare(app.src);
  const url = `${location.origin}${location.pathname}#code=${code}`;
  history.replaceState(null, '', url);
  try {
    await navigator.clipboard.writeText(url);
    say('Link copied to the clipboard', 'link');
  } catch {
    say('The link is in the address bar', 'link');
  }
}

export async function downloadRle() {
  ui.menu = null;
  if (!app.design) return;
  const g = Math.floor(app.gen);
  say('Building the pattern…', 'loader', 30000);
  const { rle } = await engine.rle(g);
  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([rle], { type: 'text/plain' }));
  const name = `${app.design.name.toLowerCase()}-gen${g}.rle`;
  a.download = name;
  a.click();
  URL.revokeObjectURL(a.href);
  say(`${name} downloaded`, 'download');
}

export async function verify() {
  if (!app.design || ui.verifying) return;
  ui.verifying = true;
  const g = Math.floor(app.gen);
  say(`Running HashLife up to generation ${g.toLocaleString('en-US')}…`, 'loader', 600000);
  try {
    const r = await engine.verify(g);
    say(r.ok ? `Verified: all ${r.cells.toLocaleString('en-US')} cells at generation ${r.gen.toLocaleString('en-US')} match` : `${r.mismatches} cells differ at generation ${r.gen.toLocaleString('en-US')}`, r.ok ? 'circle-check' : 'circle-x', 6000);
  } catch (e) {
    say(String(e), 'circle-x');
  } finally {
    ui.verifying = false;
  }
}

// ---- Inputs: take effect from the next clock cycle ----
export async function setInput(port: number, value: bigint) {
  const d = app.design;
  if (!d) return;
  const w = d.inputs[port].width;
  value &= (1n << BigInt(w)) - 1n;
  app.inputs[port] = value;
  const g = Math.floor(app.gen);
  const c = cycleAt(g);
  const from = Math.max(0, c + (g % period() > 0 ? 1 : 0));
  await engine.setInput(port, value, from);
  app.simVersion++;
  say(`${d.inputs[port].name} = ${w > 4 ? '0x' + value.toString(16) : value} from cycle ${from}`, 'toggle-right');
}

export function parseValue(s: string): bigint | null {
  try {
    const t = s.trim().replace(/_/g, '');
    if (/^0x[0-9a-f]+$/i.test(t) || /^0b[01]+$/i.test(t) || /^\d+$/.test(t)) return BigInt(t);
  } catch {}
  return null;
}

/** Parse a "go to" target: 700000, 2.5M, 700k, c:12. */
export function parseJump(s: string): number | null {
  const v = s.trim().replace(/[_,\s]/g, '');
  const m = /^(?:c|cycle)[:=]?(\d+)$/i.exec(v);
  if (m) return Number(m[1]) * period();
  if (/^\d+(\.\d+)?(e\d+)?$/i.test(v)) return Math.floor(Number(v));
  if (/^\d+(\.\d+)?[kmg]$/i.test(v)) return Math.floor(Number(v.slice(0, -1)) * { k: 1e3, m: 1e6, g: 1e9 }[v.slice(-1).toLowerCase() as 'k' | 'm' | 'g']);
  return null;
}

/** Highlight a 0-based line in the editor and make sure the code pane shows. */
export function gotoLine(line: number) {
  togglePane('code', true);
  const lines = app.src.split('\n');
  let off = 0;
  const enc = new TextEncoder();
  for (let i = 0; i < line; i++) off += enc.encode(lines[i] + '\n').length;
  app.highlight = [off, off + enc.encode(lines[line] ?? '').length];
  app.revealLine = line;
}

/** 0-based line of a byte offset in the source. */
export function lineOf(byteOff: number): number {
  const bytes = new TextEncoder().encode(app.src);
  let n = 0;
  for (let i = 0; i < Math.min(byteOff, bytes.length); i++) if (bytes[i] === 10) n++;
  return n;
}
