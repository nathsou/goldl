export interface Port {
  name: string;
  width: number;
}

export interface Diag {
  message: string;
  severity: 'error' | 'warning' | 'info';
  from: [number, number];
  to: [number, number];
  span: [number, number];
}

export interface Group {
  name: string;
  /** Short label (a name or a source snippet such as `r + 1`). */
  label: string;
  kind: string;
  parent: number | null;
  span?: [number, number] | null;
}

/** A physical I/O point of the pattern. */
export interface Pin {
  kind: 'in' | 'out' | 'one' | 'reg-q' | 'reg-d';
  name: string;
  port: number;
  bit: number;
  inv: boolean;
  x: number;
  y: number;
  dir: [number, number];
}

export interface Region {
  group: number;
  runs: [number, number, number, number][];
}

export interface SNode {
  id: number;
  kind: string;
  label: string;
  rid: number | null;
  group: number;
  span: [number, number];
  x: number;
  y: number;
  w: number;
  h: number;
  width: number;
  ins: [number, number][];
  inNames: string[];
  out: [number, number];
}

export interface SWire {
  points: [number, number][];
  width: number;
  rid: number | null;
  feedback: boolean;
  src: number;
  dst: number;
}

export interface SchematicData {
  nodes: SNode[];
  wires: SWire[];
  groups: Group[];
  size: [number, number];
  /** Gate-level views: node values of their own netlist (ids are local). */
  local?: boolean;
  values?: string[];
  gates?: number;
  error?: string;
}

export interface Stats {
  rtlNodes: number;
  regBits: number;
  aigAnds: number;
  crossings: number;
  splitters: number;
  delays: number;
  nets: number;
  components: number;
  period: number;
  width: number;
  height: number;
  cells: number;
  layoutError: string | null;
}

export interface Design {
  ok: true;
  name: string;
  diags: Diag[];
  inputs: Port[];
  outputs: Port[];
  regs: { name: string; width: number; group: number; init: string }[];
  probes: { name: string; rid: number; group: number; width: number }[];
  groups: Group[];
  regions: Region[];
  stats: Stats;
  schematic: SchematicData;
  /** Staircase blocks: flat [i0, j0, i1, j1, group, kind, origin] and the kind strings. */
  blocks: { data: number[]; kinds: string[] };
  /** Description of every RTL node, e.g. "4-bit adder". */
  rtlOps: string[];
  pins: Pin[];
  period?: number;
  bbox?: [number, number, number, number];
  grid?: number;
}

export interface CompileError {
  ok: false;
  diags: Diag[];
}

export interface TraceRow {
  inputs: string[];
  outputs: string[];
  regs: string[];
  values: string[];
}

export interface TestResult {
  name: string;
  passed: boolean;
  cycles: number;
  line: number;
  message?: string;
  failLine?: number;
}
