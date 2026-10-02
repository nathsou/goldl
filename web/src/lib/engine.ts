// Main-thread client of the WebAssembly worker: promise-based RPC.
import type { CompileError, Design, TestResult, TraceRow } from './types';

type Pending = { resolve: (v: any) => void; reject: (e: any) => void };

export interface Rect {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

class Engine {
  private worker: Worker;
  private next = 1;
  private pending = new Map<number, Pending>();
  /** Called when the module trapped and was restarted (state is lost). */
  onReset: (() => void) | null = null;

  constructor() {
    this.worker = new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' });
    this.worker.postMessage({ base: new URL(import.meta.env.BASE_URL, location.href).href });
    this.worker.onmessage = (e) => {
      const d = e.data;
      if (d.reset) {
        queueMicrotask(() => this.onReset?.());
        return;
      }
      const p = this.pending.get(d.id);
      if (!p) return;
      this.pending.delete(d.id);
      if ('error' in d) p.reject(new Error(d.error));
      else p.resolve(d.result);
    };
  }

  private rpc<T>(op: string, args: unknown): Promise<T> {
    const id = this.next++;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.worker.postMessage({ id, op, args });
    });
  }

  call<T = any>(req: Record<string, unknown>): Promise<T> {
    return this.rpc<T>('call', req);
  }

  compile(src: string, top?: string): Promise<Design | CompileError> {
    return this.call({ cmd: 'compile', src, top });
  }
  tests(src: string): Promise<TestResult[]> {
    return this.call({ cmd: 'tests', src });
  }
  setInput(port: number, value: bigint, from: number): Promise<unknown> {
    return this.call({ cmd: 'set_input', port, value: Number(value), from });
  }
  trace(from: number, to: number, rids: number[]): Promise<TraceRow[]> {
    return this.call({ cmd: 'trace', from, to, rids });
  }
  rle(gen: number): Promise<{ rle: string }> {
    return this.call({ cmd: 'rle', gen });
  }
  verify(gen: number): Promise<{ gen: number; cells: number; mismatches: number; ok: boolean }> {
    return this.call({ cmd: 'verify', gen });
  }
  lsp(msg: unknown): Promise<any[]> {
    return this.call({ cmd: 'lsp', msg });
  }
  cells(g: number, r: Rect, max: number): Promise<Int32Array> {
    return this.rpc('cells', { g, ...r, max });
  }
  gliders(g: number, r: Rect, max: number): Promise<Float64Array> {
    return this.rpc('gliders', { g, ...r, max });
  }
  components(): Promise<Int32Array> {
    return this.rpc('components', {});
  }
  wires(): Promise<Float64Array> {
    return this.rpc('wires', {});
  }
  active(g: number, r: Rect): Promise<Int32Array> {
    return this.rpc('active', { g, ...r });
  }
}

export const engine = new Engine();
