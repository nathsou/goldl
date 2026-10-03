<script lang="ts">
  // A lightweight code editor: a transparent <textarea> (native input, selection, IME and
  // undo) layered over syntax-highlighted lines, with language-server features.
  import { onMount, tick } from 'svelte';
  import { lsp, type CompletionItem, type InlayHint, type LspDiag, type Range } from '../lsp';
  import { expandSnippet, lexLine, matchBracket, renderLine, semanticClass, escapeHtml } from './highlight';

  interface Props {
    value: string;
    fontSize: number;
    tabSize: number;
    lineNumbers: boolean;
    inlayHints: boolean;
    autoClose: boolean;
    /** Byte span to highlight (e.g. selected in the schematic). */
    highlight: [number, number] | null;
    focusRequest?: number;
    onchange?: (text: string) => void;
    ondiagnostics?: (d: LspDiag[]) => void;
    onrun?: () => void;
  }
  let { value = $bindable(''), fontSize, tabSize, lineNumbers, inlayHints, autoClose, highlight, focusRequest = 0, onchange, ondiagnostics, onrun }: Props = $props();

  let ta: HTMLTextAreaElement;
  let scroller: HTMLDivElement;
  let measure: HTMLSpanElement;
  let charW = $state(7.8);
  const lineH = $derived(Math.round(fontSize * 1.7));
  const lines = $derived(value.split('\n'));
  const gutterW = $derived(lineNumbers ? Math.max(46, String(lines.length).length * charW + 26) : 16);

  let semantic: Map<number, Map<number, string>> = $state(new Map());
  let semVersion = $state(-1);
  let docVersion = $state(0);
  let diags: LspDiag[] = $state([]);
  let hints: InlayHint[] = $state([]);
  let occurrences: Range[] = $state([]);
  let caret = $state({ line: 0, col: 0, offset: 0 });
  let focused = $state(false);
  $effect(() => {
    if (!focusRequest) return;
    tick().then(() => {
      ta?.focus();
      ta?.setSelectionRange(0, 0);
      if (scroller) { scroller.scrollTop = 0; scroller.scrollLeft = 0; }
    });
  });
  let bracketPair: [number, number] | null = $state(null);

  // Hover tooltip.
  let tip: { x: number; y: number; html: string } | null = $state(null);
  let hoverTimer: ReturnType<typeof setTimeout> | undefined;
  // Completion popup.
  let comp: { items: CompletionItem[]; index: number; from: number; x: number; y: number } | null = $state(null);
  let allItems: CompletionItem[] = [];
  // Signature help.
  let sig: { x: number; y: number; html: string } | null = $state(null);

  const html = $derived.by(() => {
    let inComment = false;
    let inAssembly = false;
    const useSem = semVersion === docVersion;
    return lines.map((l, k) => {
      const r = lexLine(l, inComment, inAssembly);
      inComment = r.inComment;
      inAssembly = r.inAssembly;
      return renderLine(l, r.toks, useSem ? semantic.get(k) : undefined) || ' ';
    });
  });

  const hintsByLine = $derived.by(() => {
    const m = new Map<number, string[]>();
    if (!inlayHints) return m;
    for (const h of hints) {
      const line = lines[h.position.line] ?? '';
      const before = line.slice(0, h.position.character);
      const name = /([A-Za-z_][A-Za-z0-9_]*)\s*$/.exec(before)?.[1] ?? '';
      const arr = m.get(h.position.line) ?? [];
      arr.push(name + h.label);
      m.set(h.position.line, arr);
    }
    return m;
  });

  // Inline error lens: the first error message on the line where it starts.
  const lensByLine = $derived.by(() => {
    const m = new Map<number, string>();
    for (const d of diags) {
      if (d.severity !== 1 || m.has(d.range.start.line)) continue;
      const msg = d.message.split('\n')[0];
      m.set(d.range.start.line, msg.length > 90 ? msg.slice(0, 89) + '…' : msg);
    }
    return m;
  });

  const diagLines = $derived.by(() => {
    const m = new Map<number, number>();
    for (const d of diags) {
      for (let l = d.range.start.line; l <= d.range.end.line; l++) m.set(l, Math.min(m.get(l) ?? 9, d.severity));
    }
    return m;
  });

  function offsetToPos(text: string, off: number) {
    let line = 0;
    let ls = 0;
    for (let i = 0; i < off && i < text.length; i++) {
      if (text.charCodeAt(i) === 10) {
        line++;
        ls = i + 1;
      }
    }
    return { line, character: off - ls };
  }
  function posToOffset(text: string, line: number, ch: number) {
    let off = 0;
    for (let l = 0; l < line; l++) {
      const e = text.indexOf('\n', off);
      if (e < 0) return text.length;
      off = e + 1;
    }
    return Math.min(off + ch, text.length);
  }
  /** UTF-8 byte offset (from the compiler) → string index. */
  function byteToIndex(text: string, b: number) {
    const enc = new TextEncoder();
    let bytes = 0;
    for (let i = 0; i < text.length; i++) {
      if (bytes >= b) return i;
      bytes += enc.encode(text[i]).length;
    }
    return text.length;
  }

  function measureChar() {
    if (measure) charW = measure.getBoundingClientRect().width / 100 || 7.8;
  }
  $effect(() => {
    void fontSize;
    tick().then(measureChar);
  });

  // ---- Language server synchronisation ----
  let syncTimer: ReturnType<typeof setTimeout> | undefined;
  let opened = false;
  async function sync() {
    if (!opened) {
      opened = true;
      await lsp.open(value);
      docVersion = lsp.docVersion;
    } else {
      docVersion = await lsp.change(value);
    }
    refreshSemantic();
  }
  async function refreshSemantic() {
    const v = lsp.docVersion;
    const [st, ih] = await Promise.all([lsp.semanticTokens(), lsp.inlayHints()]);
    if (v !== lsp.docVersion) return;
    const m = new Map<number, Map<number, string>>();
    for (const [line, ch, , type] of st.tokens) {
      if (type === 'keyword' || type === 'number' || type === 'string' || type === 'comment' || type === 'operator') continue;
      let lm = m.get(line);
      if (!lm) m.set(line, (lm = new Map()));
      lm.set(ch, semanticClass(type));
    }
    semantic = m;
    semVersion = st.version;
    hints = ih;
  }
  function scheduleSync() {
    clearTimeout(syncTimer);
    syncTimer = setTimeout(sync, 180);
  }

  onMount(() => {
    measureChar();
    lsp.onDiagnostics = (d) => {
      diags = d;
      ondiagnostics?.(d);
    };
    sync();
    const ro = new ResizeObserver(() => measureChar());
    ro.observe(scroller);
    return () => ro.disconnect();
  });

  // External value changes (examples, formatting) must reach the server too.
  let lastSynced = '';
  $effect(() => {
    if (value !== lastSynced) {
      lastSynced = value;
      scheduleSync();
    }
  });

  // ---- Caret & decorations ----
  let hlTimer: ReturnType<typeof setTimeout> | undefined;
  function updateCaret() {
    if (!ta) return;
    const off = ta.selectionStart;
    const p = offsetToPos(value, off);
    caret = { line: p.line, col: p.character, offset: off };
    // Bracket matching (before or at the caret).
    let b = -1;
    let at = -1;
    for (const i of [off, off - 1]) {
      if (i >= 0 && '()[]{}'.includes(value[i] ?? '')) {
        b = matchBracket(value, i);
        at = i;
        if (b >= 0) break;
      }
    }
    bracketPair = b >= 0 ? [at, b] : null;
    clearTimeout(hlTimer);
    hlTimer = setTimeout(async () => {
      if (ta.selectionStart !== ta.selectionEnd) {
        occurrences = [];
        return;
      }
      occurrences = await lsp.highlights(p);
    }, 200);
    updateSignature();
  }

  async function updateSignature() {
    const off = ta.selectionStart;
    const before = value.slice(Math.max(0, off - 200), off);
    // Only inside an open call.
    let depth = 0;
    let inside = false;
    for (let i = before.length - 1; i >= 0; i--) {
      const c = before[i];
      if (c === ')') depth++;
      else if (c === '(') {
        if (depth === 0) {
          inside = /[A-Za-z_0-9]\s*$/.test(before.slice(0, i));
          break;
        }
        depth--;
      } else if (c === '\n' || c === '{' || c === '}') break;
    }
    if (!inside || !focused) {
      sig = null;
      return;
    }
    const r = await lsp.signatureHelp(offsetToPos(value, off));
    if (!r || !r.signatures?.length) {
      sig = null;
      return;
    }
    const s = r.signatures[0];
    const p = caretXY();
    sig = { x: p.x, y: p.y - lineH - 6, html: `<code>${escapeHtml(s.label)}</code>${s.documentation ? ' — ' + escapeHtml(s.documentation) : ''}` };
  }

  function caretXY() {
    return { x: gutterW + caret.col * charW, y: caret.line * lineH + lineH };
  }

  function rangeBoxes(r: Range): { x: number; y: number; w: number }[] {
    const out = [];
    for (let l = r.start.line; l <= r.end.line; l++) {
      const s = l === r.start.line ? r.start.character : (lines[l]?.match(/^\s*/)?.[0].length ?? 0);
      const e = l === r.end.line ? r.end.character : (lines[l]?.length ?? 0);
      out.push({ x: gutterW + s * charW, y: l * lineH, w: Math.max(charW * 0.8, (e - s) * charW) });
    }
    return out;
  }

  const highlightRange = $derived.by((): Range | null => {
    if (!highlight) return null;
    const a = byteToIndex(value, highlight[0]);
    const b = byteToIndex(value, highlight[1]);
    return { start: offsetToPos(value, a), end: offsetToPos(value, b) };
  });

  $effect(() => {
    const r = highlightRange;
    if (r && scroller) {
      const y = r.start.line * lineH;
      if (y < scroller.scrollTop || y > scroller.scrollTop + scroller.clientHeight - 2 * lineH) {
        scroller.scrollTo({ top: Math.max(0, y - scroller.clientHeight / 3), behavior: 'smooth' });
      }
    }
  });

  function scrollCaretIntoView() {
    const y = caret.line * lineH;
    if (y < scroller.scrollTop) scroller.scrollTop = y - lineH;
    else if (y + lineH > scroller.scrollTop + scroller.clientHeight) scroller.scrollTop = y - scroller.clientHeight + 2 * lineH;
    const x = gutterW + caret.col * charW;
    if (x > scroller.scrollLeft + scroller.clientWidth - 20) scroller.scrollLeft = x - scroller.clientWidth + 60;
    else if (x < scroller.scrollLeft + gutterW) scroller.scrollLeft = Math.max(0, x - gutterW - 40);
  }

  // ---- Editing ----
  function edit(start: number, end: number, text: string, selStart?: number, selEnd?: number) {
    ta.focus();
    ta.setSelectionRange(start, end);
    let ok = false;
    try {
      ok = text === '' ? document.execCommand('delete') : document.execCommand('insertText', false, text);
    } catch {}
    if (!ok) {
      ta.setRangeText(text, start, end, 'end');
      value = ta.value;
      onchange?.(value);
    }
    if (selStart !== undefined) ta.setSelectionRange(selStart, selEnd ?? selStart);
    updateCaret();
  }

  function oninput() {
    value = ta.value;
    onchange?.(value);
    updateCaret();
    maybeComplete();
  }

  const PAIRS: Record<string, string> = { '(': ')', '[': ']', '{': '}', '"': '"' };

  function lineStart(off: number) {
    return value.lastIndexOf('\n', off - 1) + 1;
  }

  function onkeydown(e: KeyboardEvent) {
    const mod = e.ctrlKey || e.metaKey;
    const s = ta.selectionStart;
    const en = ta.selectionEnd;
    if (comp) {
      if (e.key === 'ArrowDown') {
        comp.index = (comp.index + 1) % comp.items.length;
        e.preventDefault();
        return;
      }
      if (e.key === 'ArrowUp') {
        comp.index = (comp.index - 1 + comp.items.length) % comp.items.length;
        e.preventDefault();
        return;
      }
      if (e.key === 'Enter' || e.key === 'Tab') {
        e.preventDefault();
        acceptCompletion(comp.items[comp.index]);
        return;
      }
      if (e.key === 'Escape') {
        comp = null;
        e.preventDefault();
        return;
      }
    }
    if (e.key === 'Escape') {
      tip = null;
      sig = null;
    }
    if (mod && e.key === 'Enter') {
      e.preventDefault();
      onrun?.();
      return;
    }
    if ((mod && e.key === 's') || (e.shiftKey && e.altKey && (e.key === 'F' || e.key === 'f'))) {
      e.preventDefault();
      format();
      return;
    }
    if (mod && e.key === ' ') {
      e.preventDefault();
      openCompletion(true);
      return;
    }
    if (e.key === 'F12' || (mod && e.key === 'b')) {
      e.preventDefault();
      gotoDefinition(s);
      return;
    }
    if (e.key === 'F2') {
      e.preventDefault();
      rename(s);
      return;
    }
    if (mod && e.key === '/') {
      e.preventDefault();
      toggleComment();
      return;
    }
    if (e.altKey && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
      e.preventDefault();
      moveLines(e.key === 'ArrowUp' ? -1 : 1);
      return;
    }
    const unit = ' '.repeat(tabSize);
    if (e.key === 'Tab') {
      e.preventDefault();
      const ls = lineStart(s);
      if (s === en && !e.shiftKey) {
        const col = s - ls;
        edit(s, en, ' '.repeat(tabSize - (col % tabSize)));
      } else {
        const le = value.indexOf('\n', en - (en > s && value[en - 1] === '\n' ? 1 : 0));
        const end = le < 0 ? value.length : le;
        const block = value.slice(ls, end);
        const out = block
          .split('\n')
          .map((l) => (e.shiftKey ? l.replace(new RegExp(`^ {1,${tabSize}}`), '') : l.length ? unit + l : l))
          .join('\n');
        edit(ls, end, out, ls, ls + out.length);
      }
      return;
    }
    if (e.key === 'Enter' && !mod) {
      e.preventDefault();
      const ls = lineStart(s);
      const indent = /^\s*/.exec(value.slice(ls, s))![0];
      const before = value.slice(ls, s).trimEnd();
      const after = value[en];
      const opens = /[{(\[]$/.test(before);
      if (opens && after && '})]'.includes(after)) {
        const ins = '\n' + indent + unit + '\n' + indent;
        edit(s, en, ins, s + 1 + indent.length + unit.length);
      } else {
        const ins = '\n' + indent + (opens ? unit : '');
        edit(s, en, ins);
      }
      return;
    }
    if (e.key === 'Backspace' && s === en && s > 0) {
      const ls = lineStart(s);
      const before = value.slice(ls, s);
      if (before.length > 0 && /^ +$/.test(before)) {
        e.preventDefault();
        const n = before.length % tabSize || tabSize;
        edit(s - n, s, '');
        return;
      }
      if (autoClose && PAIRS[value[s - 1]] === value[s]) {
        e.preventDefault();
        edit(s - 1, s + 1, '');
        return;
      }
    }
    if (autoClose && !mod && e.key.length === 1) {
      const c = e.key;
      if (')]}"'.includes(c) && value[s] === c && s === en) {
        e.preventDefault();
        ta.setSelectionRange(s + 1, s + 1);
        updateCaret();
        return;
      }
      if (PAIRS[c] && (c !== '"' || s !== en || !/\w/.test(value[s - 1] ?? ''))) {
        e.preventDefault();
        if (s !== en) {
          edit(s, en, c + value.slice(s, en) + PAIRS[c], s + 1, en + 1);
        } else {
          const next = value[s] ?? '';
          if (next === '' || /[\s)\]},;]/.test(next)) edit(s, en, c + PAIRS[c], s + 1);
          else edit(s, en, c);
        }
        return;
      }
      if (c === '}' && s === en) {
        const ls = lineStart(s);
        const before = value.slice(ls, s);
        if (/^ +$/.test(before) && before.length >= tabSize) {
          e.preventDefault();
          edit(s - tabSize, s, '}');
          return;
        }
      }
    }
  }

  function toggleComment() {
    const s = ta.selectionStart;
    const en = ta.selectionEnd;
    const ls = lineStart(s);
    const le0 = value.indexOf('\n', Math.max(en - 1, s));
    const le = le0 < 0 ? value.length : le0;
    const block = value.slice(ls, le).split('\n');
    const all = block.every((l) => !l.trim() || /^\s*\/\//.test(l));
    const ind = Math.min(...block.filter((l) => l.trim()).map((l) => /^\s*/.exec(l)![0].length));
    const out = block.map((l) => (!l.trim() ? l : all ? l.replace(/^(\s*)\/\/ ?/, '$1') : l.slice(0, ind) + '// ' + l.slice(ind))).join('\n');
    edit(ls, le, out, ls, ls + out.length);
  }

  function moveLines(dir: number) {
    const s = ta.selectionStart;
    const en = ta.selectionEnd;
    const ls = lineStart(s);
    const le0 = value.indexOf('\n', Math.max(en - 1, s));
    const le = le0 < 0 ? value.length : le0;
    const block = value.slice(ls, le);
    if (dir < 0) {
      if (ls === 0) return;
      const ps = lineStart(ls - 1);
      const prev = value.slice(ps, ls - 1);
      edit(ps, le, block + '\n' + prev, s - (ls - ps), en - (ls - ps));
    } else {
      if (le >= value.length) return;
      const ne0 = value.indexOf('\n', le + 1);
      const ne = ne0 < 0 ? value.length : ne0;
      const next = value.slice(le + 1, ne);
      edit(ls, ne, next + '\n' + block, s + next.length + 1, en + next.length + 1);
    }
  }

  async function format() {
    const edits = await lsp.formatting(tabSize);
    if (!edits.length) return;
    const line = caret.line;
    const col = caret.col;
    const text = edits[0].newText;
    edit(0, value.length, text);
    const off = posToOffset(text, line, col);
    ta.setSelectionRange(off, off);
    updateCaret();
  }

  async function gotoDefinition(off: number) {
    const r = await lsp.definition(offsetToPos(value, off));
    if (!r) return;
    const a = posToOffset(value, r.range.start.line, r.range.start.character);
    const b = posToOffset(value, r.range.end.line, r.range.end.character);
    ta.focus();
    ta.setSelectionRange(a, b);
    updateCaret();
    scrollCaretIntoView();
  }

  async function rename(off: number) {
    const m = /[A-Za-z_][A-Za-z0-9_]*$/.exec(value.slice(0, off));
    const m2 = /^[A-Za-z0-9_]*/.exec(value.slice(off));
    const old = (m?.[0] ?? '') + (m2?.[0] ?? '');
    const name = prompt('Rename symbol', old);
    if (!name || name === old || !/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) return;
    const edits = await lsp.rename(offsetToPos(value, off), name);
    if (!edits.length) return;
    let text = value;
    const ofs = edits.map((e) => [posToOffset(value, e.range.start.line, e.range.start.character), posToOffset(value, e.range.end.line, e.range.end.character)]);
    ofs.sort((a, b) => b[0] - a[0]);
    for (const [a, b] of ofs) text = text.slice(0, a) + name + text.slice(b);
    edit(0, value.length, text);
  }

  // ---- Completion ----
  let compTimer: ReturnType<typeof setTimeout> | undefined;
  function wordStart(off: number) {
    let i = off;
    while (i > 0 && /[A-Za-z0-9_]/.test(value[i - 1])) i--;
    return i;
  }
  function maybeComplete() {
    const off = ta.selectionStart;
    const ch = value[off - 1] ?? '';
    if (ch === '.' || (/[A-Za-z_]/.test(ch) && off - wordStart(off) >= 1)) {
      clearTimeout(compTimer);
      compTimer = setTimeout(() => openCompletion(false), comp ? 0 : 60);
    } else {
      comp = null;
    }
  }
  async function openCompletion(force: boolean) {
    const off = ta.selectionStart;
    const from = wordStart(off);
    // Inside comments: no completion.
    const ls = lineStart(off);
    if (/\/\//.test(value.slice(ls, off))) return;
    if (!comp || force) {
      await sync();
      allItems = await lsp.completion(offsetToPos(value, off));
    }
    const prefix = value.slice(from, off).toLowerCase();
    let items = allItems.filter((i) => i.label.toLowerCase().startsWith(prefix));
    items = items.concat(allItems.filter((i) => !i.label.toLowerCase().startsWith(prefix) && prefix.length > 1 && i.label.toLowerCase().includes(prefix)));
    if (!force && items.length === 1 && items[0].label.toLowerCase() === prefix) items = [];
    if (!items.length) {
      comp = null;
      return;
    }
    const p = offsetToPos(value, from);
    comp = { items: items.slice(0, 60), index: 0, from, x: gutterW + p.character * charW, y: (p.line + 1) * lineH };
  }
  function acceptCompletion(it: CompletionItem) {
    if (!comp) return;
    const off = ta.selectionStart;
    const from = comp.from;
    comp = null;
    const raw = it.insertText ?? it.label;
    if (it.insertTextFormat === 2) {
      // Re-indent multi-line snippets to the current line.
      const ls = lineStart(from);
      const indent = /^\s*/.exec(value.slice(ls, from))![0];
      const { text, caret: c } = expandSnippet(raw.replace(/\n/g, '\n' + indent));
      edit(from, off, text, from + c);
    } else {
      edit(from, off, raw);
    }
  }
  const kindIcon = (k?: number) =>
    ({ 3: 'ƒ', 5: '◆', 6: '▪', 7: '▣', 9: '⊞', 13: 'E', 14: 'k', 15: '⌗', 20: 'e', 21: 'π', 25: 'T' } as Record<number, string>)[k ?? 0] ?? '·';

  // ---- Hover ----
  function posFromMouse(e: MouseEvent) {
    const r = scroller.getBoundingClientRect();
    const x = e.clientX - r.left + scroller.scrollLeft - gutterW;
    const y = e.clientY - r.top + scroller.scrollTop;
    const line = Math.floor(y / lineH);
    const character = Math.max(0, Math.round(x / charW - 0.3));
    return { line, character, x: e.clientX - r.left + scroller.scrollLeft, y: e.clientY - r.top + scroller.scrollTop };
  }
  function onmousemove(e: MouseEvent) {
    clearTimeout(hoverTimer);
    if (e.buttons) return;
    const p = posFromMouse(e);
    hoverTimer = setTimeout(async () => {
      if (p.line >= lines.length || p.character > (lines[p.line]?.length ?? 0)) {
        tip = null;
        return;
      }
      const parts: string[] = [];
      for (const d of diags) {
        const inside =
          (p.line > d.range.start.line || (p.line === d.range.start.line && p.character >= d.range.start.character)) &&
          (p.line < d.range.end.line || (p.line === d.range.end.line && p.character <= d.range.end.character + 1));
        if (inside) parts.push(`<div class="tip-diag sev${d.severity}">${escapeHtml(d.message)}</div>`);
      }
      const h = await lsp.hover({ line: p.line, character: p.character });
      if (h?.contents?.value) parts.push(markdown(h.contents.value));
      tip = parts.length ? { x: p.x, y: (p.line + 1) * lineH + 4, html: parts.join('') } : null;
    }, 350);
  }
  function markdown(md: string): string {
    return md
      .split(/```(?:goldl)?\n?/)
      .map((part, i) => (i % 2 === 1 ? `<pre class="tip-code">${colorize(part.trimEnd())}</pre>` : `<div class="tip-doc">${escapeHtml(part).replace(/`([^`]+)`/g, '<code>$1</code>')}</div>`))
      .join('');
  }
  function colorize(code: string) {
    let inC = false;
    return code
      .split('\n')
      .map((l) => {
        const r = lexLine(l, inC);
        inC = r.inComment;
        return renderLine(l, r.toks, undefined);
      })
      .join('\n');
  }
  function onclick(e: MouseEvent) {
    comp = null;
    if (e.ctrlKey || e.metaKey) {
      gotoDefinition(ta.selectionStart);
    }
  }
</script>

<div class="editor" style="--fs:{fontSize}px; --lh:{lineH}px; --cw:{charW}px; --gw:{gutterW}px; --tab:{tabSize}">
  <span class="measure" bind:this={measure}>{'M'.repeat(100)}</span>
  <div class="scroller" bind:this={scroller} onmousemove={onmousemove} onmouseleave={() => (clearTimeout(hoverTimer), (tip = null))} role="presentation">
    <div class="content" style="height:{lines.length * lineH + 200}px; min-width:{gutterW + Math.max(...lines.map((l) => l.length)) * charW + 300}px">
      {#if focused}
        <div class="current-line" style="top:{caret.line * lineH}px"></div>
      {/if}
      {#if highlightRange}
        {#each rangeBoxes(highlightRange) as b}
          <div class="hl-span" style="left:{b.x}px; top:{b.y}px; width:{b.w}px"></div>
        {/each}
      {/if}
      {#each occurrences as r}
        {#each rangeBoxes(r) as b}
          <div class="occ" style="left:{b.x}px; top:{b.y}px; width:{b.w}px"></div>
        {/each}
      {/each}
      {#if bracketPair}
        {#each bracketPair as off}
          {@const p = offsetToPos(value, off)}
          <div class="bracket" style="left:{gutterW + p.character * charW}px; top:{p.line * lineH}px"></div>
        {/each}
      {/if}
      <pre class="code" aria-hidden="true">{#each html as h, k}<div class="line" style="top:{k * lineH}px">{@html h}{#if hintsByLine.get(k)}<span class="inlay">{'  ' + hintsByLine.get(k)!.join('  ')}</span>{/if}{#if lensByLine.get(k)}<span class="lens">{lensByLine.get(k)}</span>{/if}</div>{/each}</pre>
      {#each diags as d}
        {#each rangeBoxes(d.range) as b}
          <div class="squiggle sev{d.severity}" style="left:{b.x}px; top:{b.y + lineH - 4}px; width:{b.w}px"></div>
        {/each}
      {/each}
      <div class="gutter" style="width:{gutterW}px">
        {#each lines as _, k}
          <div class="ln sev{diagLines.get(k) ?? 0}" class:active={k === caret.line} style="top:{k * lineH}px">
            {#if lineNumbers}{k + 1}{:else if diagLines.has(k)}<span class="gdot sev{diagLines.get(k)}"></span>{/if}
          </div>
        {/each}
      </div>
      <textarea
        bind:this={ta}
        {value}
        spellcheck="false"
        autocapitalize="off"
        autocomplete="off"
        wrap="off"
        aria-label="GoLDL source"
        {oninput}
        {onkeydown}
        {onclick}
        onkeyup={updateCaret}
        onmouseup={updateCaret}
        onselect={updateCaret}
        onfocus={() => ((focused = true), updateCaret())}
        onblur={() => ((focused = false), setTimeout(() => (comp = null), 150), (sig = null))}
        style="left:{gutterW}px; width:calc(100% - {gutterW}px); height:{lines.length * lineH + 200}px"
      ></textarea>
      {#if comp}
        <div class="completion" style="left:{comp.x}px; top:{comp.y}px">
          {#each comp.items as it, i}
            <button class="item" class:sel={i === comp.index} onmousedown={(e) => (e.preventDefault(), acceptCompletion(it))}>
              <span class="kind k{it.kind}">{kindIcon(it.kind)}</span>
              <span class="label">{it.label}</span>
              {#if it.detail}<span class="detail">{it.detail}</span>{/if}
            </button>
          {/each}
        </div>
      {/if}
      {#if tip}
        <div class="tooltip" style="left:{Math.max(0, tip.x - 20)}px; top:{tip.y}px">{@html tip.html}</div>
      {/if}
      {#if sig && !comp}
        <div class="tooltip sig" style="left:{sig.x}px; top:{Math.max(0, sig.y)}px">{@html sig.html}</div>
      {/if}
    </div>
  </div>
  <div class="status">
    <span title="Ctrl+Enter compile · Ctrl+S format · F12 definition · F2 rename · Ctrl+Space complete">GoLDL · UTF-8</span>
    <span class="pos">Ln {caret.line + 1}, Col {caret.col + 1}</span>
  </div>
</div>

<style>
  .editor {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--editor-bg);
    font-family: var(--mono);
    font-size: var(--fs);
    line-height: var(--lh);
  }
  .measure {
    position: absolute;
    visibility: hidden;
    white-space: pre;
    font: inherit;
  }
  .scroller {
    position: relative;
    flex: 1;
    overflow: auto;
    min-height: 0;
    /* The sticky gutter covers the left edge: keep the caret clear of it. */
    scroll-padding-left: calc(var(--gw) + 8px);
  }
  .content {
    position: relative;
  }
  .code {
    position: absolute;
    left: var(--gw);
    top: 0;
    margin: 0;
    font: inherit;
    line-height: inherit;
    color: var(--fg);
    pointer-events: none;
    tab-size: var(--tab);
  }
  .line {
    position: absolute;
    left: 0;
    white-space: pre;
    height: var(--lh);
  }
  textarea {
    position: absolute;
    top: 0;
    margin: 0;
    padding: 0;
    border: 0;
    outline: none;
    resize: none;
    overflow: hidden;
    background: transparent;
    color: transparent;
    caret-color: var(--caret);
    font: inherit;
    line-height: inherit;
    white-space: pre;
    tab-size: var(--tab);
    z-index: 2;
  }
  textarea::selection {
    background: var(--selection);
    color: transparent;
  }
  .gutter {
    position: sticky;
    left: 0;
    top: 0;
    height: 100%;
    background: var(--editor-bg);
    z-index: 3;
    pointer-events: none;
  }
  .ln {
    position: absolute;
    right: 16px;
    left: 0;
    text-align: right;
    color: var(--fg-faint);
    height: var(--lh);
  }
  .ln.active {
    color: var(--fg-dim);
  }
  .ln.sev1 {
    color: var(--err);
  }
  .ln.sev2 {
    color: var(--warn);
  }
  .lens {
    margin-left: 16px;
    padding: 0 7px;
    border-radius: 4px;
    background: var(--err-soft);
    color: var(--err);
    font: 12px var(--sans);
  }
  .gdot {
    position: absolute;
    left: 6px;
    top: calc(var(--lh) / 2 - 3px);
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }
  .gdot.sev1 {
    background: var(--err);
  }
  .gdot.sev2 {
    background: var(--warn);
  }
  .current-line {
    position: absolute;
    left: 0;
    right: 0;
    height: var(--lh);
    background: var(--line-hl);
  }
  .occ {
    position: absolute;
    height: var(--lh);
    background: var(--occ);
    border-radius: 2px;
  }
  .hl-span {
    position: absolute;
    height: var(--lh);
    background: var(--accent-soft);
    outline: 1px solid var(--accent);
    border-radius: 2px;
  }
  .bracket {
    position: absolute;
    width: var(--cw);
    height: var(--lh);
    outline: 1px solid var(--fg-dim);
    border-radius: 2px;
  }
  .squiggle {
    position: absolute;
    height: 4px;
    background-repeat: repeat-x;
    background-size: 6px 4px;
    pointer-events: none;
  }
  .squiggle.sev1,
  .squiggle.sev2,
  .squiggle.sev3 {
    background: var(--err);
    -webkit-mask: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='6' height='4'%3E%3Cpath d='M0 3 L1.5 1 L3 3 L4.5 1 L6 3' fill='none' stroke='black' stroke-width='1.1'/%3E%3C/svg%3E") repeat-x 0 0 / 6px 4px;
    mask: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='6' height='4'%3E%3Cpath d='M0 3 L1.5 1 L3 3 L4.5 1 L6 3' fill='none' stroke='black' stroke-width='1.1'/%3E%3C/svg%3E") repeat-x 0 0 / 6px 4px;
  }
  .squiggle.sev2,
  .squiggle.sev3 {
    background: var(--warn);
  }
  .inlay {
    color: var(--fg-faint);
    font-style: italic;
  }
  .completion {
    position: absolute;
    z-index: 10;
    min-width: 260px;
    max-width: 520px;
    max-height: 260px;
    overflow-y: auto;
    background: var(--panel-raised);
    border: 1px solid var(--border);
    border-radius: 6px;
    box-shadow: var(--shadow);
    padding: 3px;
  }
  .item {
    display: flex;
    gap: 8px;
    align-items: baseline;
    width: 100%;
    border: 0;
    background: none;
    color: var(--fg);
    font: inherit;
    font-size: 0.95em;
    padding: 1px 8px;
    border-radius: 4px;
    text-align: left;
    cursor: pointer;
  }
  .item.sel {
    background: var(--accent-soft);
  }
  .kind {
    width: 14px;
    color: var(--accent);
    text-align: center;
  }
  .detail {
    margin-left: auto;
    color: var(--fg-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 260px;
  }
  .tooltip {
    position: absolute;
    z-index: 11;
    max-width: 560px;
    background: var(--panel-raised);
    border: 1px solid var(--border);
    border-radius: 6px;
    box-shadow: var(--shadow);
    padding: 6px 10px;
    font-family: var(--sans);
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--fg);
    pointer-events: none;
  }
  .tooltip.sig {
    font-family: var(--mono);
  }
  .tooltip :global(.tip-code) {
    margin: 2px 0;
    font-family: var(--mono);
    white-space: pre-wrap;
  }
  .tooltip :global(.tip-diag) {
    padding: 2px 0 4px;
    border-bottom: 1px solid var(--border-soft);
    margin-bottom: 4px;
  }
  .tooltip :global(.tip-diag.sev1) {
    color: var(--err);
  }
  .tooltip :global(.tip-diag.sev2) {
    color: var(--warn);
  }
  .tooltip :global(code) {
    font-family: var(--mono);
    background: var(--code-bg);
    padding: 0 3px;
    border-radius: 3px;
  }
  .status {
    display: flex;
    justify-content: space-between;
    gap: 16px;
    align-items: center;
    padding: 0 12px;
    height: 26px;
    flex-shrink: 0;
    background: var(--editor-bg);
    font-family: var(--sans);
    font-size: 11.5px;
    color: var(--fg-faint);
    border-top: 1px solid var(--border-soft);
    white-space: nowrap;
    overflow: hidden;
  }
  .status .pos {
    font-family: var(--mono);
  }
  :global(.t-keyword) {
    color: var(--syn-keyword);
  }
  :global(.t-type) {
    color: var(--syn-type);
  }
  :global(.t-typename) {
    color: var(--syn-typename);
  }
  :global(.t-number) {
    color: var(--syn-number);
  }
  :global(.t-string) {
    color: var(--syn-string);
  }
  :global(.t-comment) {
    color: var(--syn-comment);
    font-style: italic;
  }
  :global(.t-doc) {
    color: var(--syn-doc);
    font-style: italic;
  }
  :global(.t-operator) {
    color: var(--syn-operator);
  }
  :global(.t-bracket) {
    color: var(--syn-bracket);
  }
  :global(.t-builtin),
  :global(.t-function) {
    color: var(--syn-function);
  }
  :global(.t-constant) {
    color: var(--syn-constant);
  }
  :global(.t-port) {
    color: var(--syn-port);
  }
  :global(.t-register) {
    color: var(--syn-register);
  }
  :global(.t-attr) {
    color: var(--syn-attr);
  }
</style>
