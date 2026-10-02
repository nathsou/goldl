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
  kind: string;
  parent: number | null;
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
  rid: number;
  feedback: boolean;
  src: number;
  dst: number;
}

export interface SchematicData {
  nodes: SNode[];
  wires: SWire[];
  groups: Group[];
  size: [number, number];
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
