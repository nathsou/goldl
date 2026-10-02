// A small LSP client talking JSON-RPC to the language server inside the WASM worker.
import { engine } from './engine';

export interface Pos {
  line: number;
  character: number;
}
export interface Range {
  start: Pos;
  end: Pos;
}
export interface LspDiag {
  range: Range;
  severity: number;
  message: string;
}
export interface CompletionItem {
  label: string;
  kind?: number;
  detail?: string;
  documentation?: string;
  insertText?: string;
  insertTextFormat?: number;
}
export interface InlayHint {
  position: Pos;
  label: string;
}

const URI = 'file:///main.goldl';

export class LspClient {
  private id = 1;
  private version = 0;
  private ready: Promise<void>;
  tokenTypes: string[] = [];
  onDiagnostics: ((d: LspDiag[], version: number) => void) | null = null;

  constructor() {
    this.ready = this.init();
  }

  private async init() {
    const r = await this.request('initialize', { processId: null, rootUri: null, capabilities: {} }, true);
    this.tokenTypes = r?.capabilities?.semanticTokensProvider?.legend?.tokenTypes ?? [];
    await this.notify('initialized', {}, true);
  }

  /** Re-establish the session after the worker was restarted. */
  reset(text: string) {
    this.ready = this.init().then(() => this.open(text));
  }

  private handle(msgs: any[]): any {
    let result: any = null;
    for (const m of msgs) {
      if (m.method === 'textDocument/publishDiagnostics') {
        this.onDiagnostics?.(m.params.diagnostics, m.params.version ?? 0);
      } else if ('result' in m || 'error' in m) {
        result = m.result ?? null;
      }
    }
    return result;
  }

  private async request(method: string, params: unknown, internal = false): Promise<any> {
    if (!internal) await this.ready;
    const msgs = await engine.lsp({ jsonrpc: '2.0', id: this.id++, method, params });
    return this.handle(msgs);
  }

  private async notify(method: string, params: unknown, internal = false) {
    if (!internal) await this.ready;
    const msgs = await engine.lsp({ jsonrpc: '2.0', method, params });
    this.handle(msgs);
  }

  async open(text: string) {
    this.version++;
    await this.notify('textDocument/didOpen', { textDocument: { uri: URI, languageId: 'goldl', version: this.version, text } });
  }

  async change(text: string): Promise<number> {
    this.version++;
    const v = this.version;
    await this.notify('textDocument/didChange', { textDocument: { uri: URI, version: v }, contentChanges: [{ text }] });
    return v;
  }

  get docVersion() {
    return this.version;
  }

  private td(pos: Pos) {
    return { textDocument: { uri: URI }, position: pos };
  }

  hover(pos: Pos): Promise<{ contents: { value: string }; range?: Range } | null> {
    return this.request('textDocument/hover', this.td(pos));
  }
  async completion(pos: Pos): Promise<CompletionItem[]> {
    const r = await this.request('textDocument/completion', this.td(pos));
    return r?.items ?? [];
  }
  definition(pos: Pos): Promise<{ range: Range } | null> {
    return this.request('textDocument/definition', this.td(pos));
  }
  async highlights(pos: Pos): Promise<Range[]> {
    const r = await this.request('textDocument/documentHighlight', this.td(pos));
    return (r ?? []).map((x: any) => x.range);
  }
  async formatting(tabSize: number): Promise<{ range: Range; newText: string }[]> {
    return (await this.request('textDocument/formatting', { textDocument: { uri: URI }, options: { tabSize, insertSpaces: true } })) ?? [];
  }
  async rename(pos: Pos, newName: string): Promise<{ range: Range; newText: string }[]> {
    const r = await this.request('textDocument/rename', { ...this.td(pos), newName });
    return r?.changes?.[URI] ?? [];
  }
  async inlayHints(): Promise<InlayHint[]> {
    return (await this.request('textDocument/inlayHint', { textDocument: { uri: URI } })) ?? [];
  }
  async signatureHelp(pos: Pos): Promise<{ signatures: { label: string; documentation?: string }[]; activeParameter: number } | null> {
    return this.request('textDocument/signatureHelp', this.td(pos));
  }
  /** Semantic tokens decoded to absolute [line, char, length, type, modifiers]. */
  async semanticTokens(): Promise<{ version: number; tokens: [number, number, number, string, number][] }> {
    const v = this.version;
    const r = await this.request('textDocument/semanticTokens/full', { textDocument: { uri: URI } });
    const data: number[] = r?.data ?? [];
    const out: [number, number, number, string, number][] = [];
    let line = 0;
    let ch = 0;
    for (let i = 0; i + 4 < data.length; i += 5) {
      const dl = data[i];
      line += dl;
      ch = dl === 0 ? ch + data[i + 1] : data[i + 1];
      out.push([line, ch, data[i + 2], this.tokenTypes[data[i + 3]] ?? 'variable', data[i + 4]]);
    }
    return { version: v, tokens: out };
  }
}

export const lsp = new LspClient();
