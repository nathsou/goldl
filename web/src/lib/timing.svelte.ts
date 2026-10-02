// Playback and stepping through generations, driven by the timing settings.
import { app, settings, period, cycleAt, say } from './state.svelte';

const T = () => period();

/** Playback speed in generations per second. */
export function genPerSec(): number {
  const t = settings.timing;
  return t.speedUnit === 'cycle' ? t.speed * T() : t.speed;
}
/** Step size in generations (at least 1). */
export function stepGens(): number {
  const t = settings.timing;
  return Math.max(1, Math.round(t.stepUnit === 'cycle' ? t.step * T() : t.step));
}

export function setGen(g: number) {
  app.gen = Math.max(0, g);
  if (settings.timing.loop) app.loopFrom = cycleAt(Math.floor(app.gen)) * T();
}

/** Advance playback by `dt` seconds. */
export function advance(dt: number) {
  if (!app.playing || !app.design) return;
  const dir = settings.timing.reverse ? -1 : 1;
  let g = app.gen + dir * genPerSec() * dt;
  if (settings.timing.loop) {
    const a = app.loopFrom;
    const span = T();
    g = a + ((((g - a) % span) + span) % span);
  } else if (g <= 0) {
    g = 0;
    app.playing = false;
  }
  app.gen = g;
}

export function togglePlay() {
  if (!app.design) return;
  if (!app.playing && settings.timing.loop) app.loopFrom = cycleAt(Math.floor(app.gen)) * T();
  app.playing = !app.playing;
}

export function step(d: number) {
  app.playing = false;
  setGen(Math.floor(app.gen) + d * stepGens());
}
export function stepCycle(d: number) {
  app.playing = false;
  const g = Math.floor(app.gen);
  const c = cycleAt(g);
  const phase = g - c * T();
  setGen(Math.max(0, c + d) * T() + phase);
}

export function setLoop(on: boolean) {
  settings.timing.loop = on;
  app.loopFrom = cycleAt(Math.floor(app.gen)) * T();
  if (on) say(`Looping cycle ${cycleAt(Math.floor(app.gen))}`, 'info');
}

/** Faster (k > 1) or slower, keeping the unit. */
export function scaleSpeed(k: number) {
  const t = settings.timing;
  t.speed = clampSpeed(t.speed * k, t.speedUnit);
}
export function clampSpeed(v: number, unit: 'gen' | 'cycle') {
  const gps = unit === 'cycle' ? v * T() : v;
  const lim = Math.min(Math.max(1, gps), 64 * T());
  const r = unit === 'cycle' ? lim / T() : lim;
  return Number(r.toPrecision(3));
}

export function fmtNum(v: number): string {
  if (v >= 1e9) return `${+(v / 1e9).toPrecision(3)}G`;
  if (v >= 1e6) return `${+(v / 1e6).toPrecision(3)}M`;
  if (v >= 1e4) return `${+(v / 1e3).toPrecision(3)}k`;
  return String(+v.toPrecision(3));
}
const FRACS: [number, string][] = [
  [1 / 64, '1/64'],
  [1 / 32, '1/32'],
  [1 / 16, '1/16'],
  [1 / 8, '⅛'],
  [1 / 4, '¼'],
  [1 / 2, '½'],
];
export function fmtCycles(v: number): string {
  const f = FRACS.find(([x]) => Math.abs(x - v) < 1e-9);
  return f ? f[1] : String(+v.toPrecision(3));
}
export function fmtSpeed(): string {
  const t = settings.timing;
  return t.speedUnit === 'cycle' ? `${fmtCycles(t.speed)} cyc/s` : `${fmtNum(t.speed)} gen/s`;
}
export function fmtStep(): string {
  const t = settings.timing;
  return t.stepUnit === 'cycle' ? `${fmtCycles(t.step)} cycle` : `${fmtNum(t.step)} gen`;
}

/** Parse "1/16", "0.25", "2k", "1.5M". */
export function parseAmount(s: string): number | null {
  const v = s
    .trim()
    .replace(/[_,\s]/g, '')
    .replace('⅛', '1/8')
    .replace('¼', '1/4')
    .replace('½', '1/2');
  const f = /^(\d+(?:\.\d+)?)\/(\d+(?:\.\d+)?)$/.exec(v);
  if (f) return Number(f[1]) / Number(f[2]);
  const m = /^(\d+(?:\.\d+)?(?:e\d+)?)([kmg]?)$/i.exec(v);
  if (!m) return null;
  return Number(m[1]) * ({ '': 1, k: 1e3, m: 1e6, g: 1e9 } as Record<string, number>)[m[2].toLowerCase()];
}
