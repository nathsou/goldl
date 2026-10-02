/// <reference lib="webworker" />
// Hosts the GoLDL WebAssembly module (raw C ABI, no bindings) off the main thread.

interface Exports {
  memory: WebAssembly.Memory;
  goldl_init(): void;
  goldl_alloc(n: number): number;
  goldl_free(p: number, n: number): void;
  goldl_call(p: number, n: number): number;
  goldl_last_panic(): number;
  goldl_cells(g: number, x0: number, y0: number, x1: number, y1: number, max: number): number;
  goldl_gliders(g: number, x0: number, y0: number, x1: number, y1: number, max: number): number;
  goldl_components(): number;
  goldl_wires(): number;
  goldl_active(g: number, x0: number, y0: number, x1: number, y1: number): number;
}

let module: WebAssembly.Module | null = null;
let ex: Exports | null = null;
const enc = new TextEncoder();
const dec = new TextDecoder();

async function instantiate() {
  if (!module) {
    const url = new URL('goldl.wasm', (self as any).GOLDL_BASE ?? self.location.href);
    module = await WebAssembly.compileStreaming(fetch(url));
  }
  const inst = await WebAssembly.instantiate(module, {});
  ex = inst.exports as unknown as Exports;
  ex.goldl_init();
}

function readOut(p: number): string {
  const m = new Uint8Array(ex!.memory.buffer);
  const n = m[p] | (m[p + 1] << 8) | (m[p + 2] << 16) | (m[p + 3] << 24);
  return dec.decode(m.subarray(p + 4, p + 4 + n));
}

function call(req: unknown): unknown {
  const bytes = enc.encode(JSON.stringify(req));
  const p = ex!.goldl_alloc(bytes.length);
  new Uint8Array(ex!.memory.buffer, p, bytes.length).set(bytes);
  const r = ex!.goldl_call(p, bytes.length);
  const text = readOut(r);
  ex!.goldl_free(p, bytes.length);
  return JSON.parse(text);
}

function i32(p: number, per: number): Int32Array {
  const view = new Int32Array(ex!.memory.buffer, p, 1);
  const n = view[0];
  if (n < 0) return new Int32Array([-1]);
  return new Int32Array(ex!.memory.buffer.slice(p, p + 4 * (1 + n * per)));
}

function f64(p: number, per: number): Float64Array {
  const n = new Float64Array(ex!.memory.buffer, p, 1)[0];
  return new Float64Array(ex!.memory.buffer.slice(p, p + 8 * (1 + n * per)));
}

type Req = { id: number; op: string; args: any };

self.onmessage = async (e: MessageEvent<Req | { base: string }>) => {
  if ('base' in e.data) {
    (self as any).GOLDL_BASE = e.data.base;
    return;
  }
  const { id, op, args } = e.data;
  try {
    if (!ex) await instantiate();
    let result: unknown;
    const transfer: Transferable[] = [];
    switch (op) {
      case 'call':
        result = call(args);
        break;
      case 'cells': {
        const a = i32(ex!.goldl_cells(args.g, args.x0, args.y0, args.x1, args.y1, args.max), 2);
        result = a;
        transfer.push(a.buffer);
        break;
      }
      case 'gliders': {
        const a = f64(ex!.goldl_gliders(args.g, args.x0, args.y0, args.x1, args.y1, args.max), 4);
        result = a;
        transfer.push(a.buffer);
        break;
      }
      case 'components': {
        const a = i32(ex!.goldl_components(), 6);
        result = a;
        transfer.push(a.buffer);
        break;
      }
      case 'wires': {
        const a = f64(ex!.goldl_wires(), 6);
        result = a;
        transfer.push(a.buffer);
        break;
      }
      case 'active': {
        const a = i32(ex!.goldl_active(args.g, args.x0, args.y0, args.x1, args.y1), 1);
        result = a;
        transfer.push(a.buffer);
        break;
      }
      default:
        throw new Error('unknown op ' + op);
    }
    (self as any).postMessage({ id, result }, transfer);
  } catch (err) {
    let message = String(err);
    if (err instanceof WebAssembly.RuntimeError && ex) {
      try {
        message = 'internal error: ' + readOut(ex.goldl_last_panic());
      } catch {}
      // The instance is poisoned after a trap: start afresh.
      ex = null;
      (self as any).postMessage({ reset: true });
    }
    (self as any).postMessage({ id, error: message });
  }
};
