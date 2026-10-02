// Bundled example designs (from the repository's examples/ directory).
const files = import.meta.glob('../../../examples/*.goldl', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

const ORDER = ['blinker', 'half_adder', 'full_adder', 'counter', 'traffic_light', 'lfsr', 'ripple_adder', 'popcount', 'register_file', 'alu', 'cpu'];

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
    return { id, title: title.replace(/\.$/, ''), summary: rest.join(': '), src };
  })
  .sort((a, b) => (ORDER.indexOf(a.id) + 1 || 99) - (ORDER.indexOf(b.id) + 1 || 99));
