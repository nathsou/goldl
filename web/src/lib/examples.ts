// Bundled example designs (from the repository's examples/ directory).
const files = import.meta.glob('../../../examples/*.goldl', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

const ORDER = ['blinker', 'half_adder', 'full_adder', 'counter', 'traffic_light', 'lfsr', 'ripple_adder', 'popcount', 'register_file', 'alu', 'cpu', 'riscv'];

// Menu labels are names, not full documentation sentences.
const TITLES: Record<string, string> = {
  blinker: 'Blinker', half_adder: 'Half adder', full_adder: 'Full adder',
  counter: '4-bit counter', traffic_light: 'Traffic light', lfsr: '8-bit LFSR',
  ripple_adder: 'Ripple-carry adder', popcount: 'Population count',
  register_file: 'Register file', alu: '8-bit ALU', cpu: 'Glider-8',
  riscv: 'RISC-V RV32I',
};

export interface Example {
  id: string;
  title: string;
  summary: string;
  src: string;
}

export const examples: Example[] = Object.entries(files)
  .map(([path, src]) => {
    const id = path.split('/').pop()!.replace('.goldl', '');
    const doc = /^\/\/\/\s*(.*)$/m.exec(src)?.[1] ?? id;
    const [title, ...rest] = doc.split(/:\s+/);
    return { id, title: TITLES[id] ?? title.replace(/\.$/, ''), summary: rest.length ? rest.join(': ') : doc, src };
  })
  .sort((a, b) => (ORDER.indexOf(a.id) + 1 || 99) - (ORDER.indexOf(b.id) + 1 || 99));
