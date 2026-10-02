// Lexical highlighting of GoLDL source (synchronous; semantic tokens refine identifiers).

export const KEYWORDS = new Set(['module', 'fn', 'let', 'reg', 'mem', 'const', 'enum', 'if', 'else', 'match', 'for', 'in', 'test', 'step', 'assert']);
const TYPES = new Set(['bit', 'bits', 'uint']);
const CONSTS = new Set(['true', 'false']);
export const BUILTINS = new Set(['zext', 'sext', 'trunc', 'cat', 'rep', 'clog2', 'width', 'any', 'all', 'parity', 'mux', 'slt', 'sle', 'sgt', 'sge']);

export interface Tok {
  start: number;
  end: number;
  cls: string;
}

/** Token classes per line. `inComment` carries block-comment state across lines. */
export function lexLine(line: string, inComment: boolean): { toks: Tok[]; inComment: boolean } {
  const toks: Tok[] = [];
  let i = 0;
  const n = line.length;
  if (inComment) {
    const e = line.indexOf('*/');
    if (e < 0) return { toks: [{ start: 0, end: n, cls: 'comment' }], inComment: true };
    toks.push({ start: 0, end: e + 2, cls: 'comment' });
    i = e + 2;
  }
  while (i < n) {
    const c = line[i];
    if (c === ' ' || c === '\t') {
      i++;
      continue;
    }
    if (c === '/' && line[i + 1] === '/') {
      toks.push({ start: i, end: n, cls: line[i + 2] === '/' ? 'doc' : 'comment' });
      break;
    }
    if (c === '/' && line[i + 1] === '*') {
      const e = line.indexOf('*/', i + 2);
      if (e < 0) {
        toks.push({ start: i, end: n, cls: 'comment' });
        return { toks, inComment: true };
      }
      toks.push({ start: i, end: e + 2, cls: 'comment' });
      i = e + 2;
      continue;
    }
    if (c === '"') {
      let j = i + 1;
      while (j < n && line[j] !== '"') j += line[j] === '\\' ? 2 : 1;
      toks.push({ start: i, end: Math.min(j + 1, n), cls: 'string' });
      i = j + 1;
      continue;
    }
    if (/[0-9]/.test(c)) {
      const m = /^(0x[0-9a-fA-F_]+|0b[01_]+|[0-9_]+)/.exec(line.slice(i))!;
      toks.push({ start: i, end: i + m[0].length, cls: 'number' });
      i += m[0].length;
      continue;
    }
    if (/[A-Za-z_]/.test(c)) {
      const m = /^[A-Za-z_][A-Za-z0-9_]*/.exec(line.slice(i))!;
      const w = m[0];
      let cls = 'ident';
      if (KEYWORDS.has(w)) cls = 'keyword';
      else if (TYPES.has(w)) cls = 'type';
      else if (CONSTS.has(w)) cls = 'number';
      else if (BUILTINS.has(w) && line[i + w.length] === '(') cls = 'builtin';
      else if (w === '_') cls = 'operator';
      else if (/^[A-Z][A-Z0-9_]+$/.test(w)) cls = 'constant';
      else if (/^[A-Z]/.test(w)) cls = 'typename';
      toks.push({ start: i, end: i + w.length, cls });
      i += w.length;
      continue;
    }
    if (c === '#' && line[i + 1] === '[') {
      const e = line.indexOf(']', i);
      toks.push({ start: i, end: e < 0 ? n : e + 1, cls: 'attr' });
      i = e < 0 ? n : e + 1;
      continue;
    }
    if ('()[]{}'.includes(c)) {
      toks.push({ start: i, end: i + 1, cls: 'bracket' });
      i++;
      continue;
    }
    const op = /^(=>|->|\.\.=|\.\.|::|==|!=|<=|>=|&&|\|\||<<|>>>|>>|\+\+|[-+*/%&|^~!<>=.,:;?])/.exec(line.slice(i));
    if (op) {
      toks.push({ start: i, end: i + op[0].length, cls: 'operator' });
      i += op[0].length;
      continue;
    }
    i++;
  }
  return { toks, inComment: false };
}

export function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

/** Map semantic token types to CSS classes. */
export function semanticClass(t: string): string {
  switch (t) {
    case 'class':
      return 'typename';
    case 'function':
      return 'function';
    case 'parameter':
      return 'port';
    case 'property':
      return 'register';
    case 'macro':
      return 'constant';
    case 'enumMember':
      return 'constant';
    case 'type':
      return 'type';
    default:
      return 'ident';
  }
}

/** Render one line to HTML, with optional semantic classes for identifier columns. */
export function renderLine(line: string, toks: Tok[], sem: Map<number, string> | undefined): string {
  let out = '';
  let pos = 0;
  for (const t of toks) {
    if (t.start > pos) out += escapeHtml(line.slice(pos, t.start));
    let cls = t.cls;
    if (cls === 'ident' && sem) cls = sem.get(t.start) ?? cls;
    out += `<span class="t-${cls}">${escapeHtml(line.slice(t.start, t.end))}</span>`;
    pos = t.end;
  }
  if (pos < line.length) out += escapeHtml(line.slice(pos));
  return out;
}

/** Index of the bracket matching the one at `i` (or -1). */
export function matchBracket(text: string, i: number): number {
  const pairs: Record<string, string> = { '(': ')', '[': ']', '{': '}' };
  const rev: Record<string, string> = { ')': '(', ']': '[', '}': '{' };
  const c = text[i];
  if (pairs[c]) {
    let d = 0;
    for (let j = i; j < text.length; j++) {
      if (text[j] === c) d++;
      else if (text[j] === pairs[c] && --d === 0) return j;
    }
  } else if (rev[c]) {
    let d = 0;
    for (let j = i; j >= 0; j--) {
      if (text[j] === c) d++;
      else if (text[j] === rev[c] && --d === 0) return j;
    }
  }
  return -1;
}

/** Expand an LSP snippet: returns the text and the caret offset within it. */
export function expandSnippet(s: string): { text: string; caret: number } {
  let text = '';
  let caret = -1;
  let first = -1;
  const re = /\$\{(\d+):([^}]*)\}|\$(\d+)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(s))) {
    text += s.slice(last, m.index);
    const idx = Number(m[1] ?? m[3]);
    if (idx === 0) caret = text.length;
    else if (first < 0 && idx === 1) first = text.length;
    text += m[2] ?? '';
    last = m.index + m[0].length;
  }
  text += s.slice(last);
  return { text, caret: first >= 0 ? first : caret >= 0 ? caret : text.length };
}
