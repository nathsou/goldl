// WebGL2 renderer for the Life universe: instanced rectangles (cells, components) and soft
// glow sprites (gliders, reacting components). World coordinates are converted to
// camera-relative float32 on the CPU so huge patterns keep full precision.

export interface Camera {
  /** World coordinates (cells) at the screen centre. */
  cx: number;
  cy: number;
  /** Pixels per cell. */
  zoom: number;
}

const VS = `#version 300 es
layout(location=0) in vec2 corner;
layout(location=1) in vec4 rect;   // x0 y0 x1 y1, camera-relative world units
layout(location=2) in vec4 color;
uniform vec2 uScale;               // 2*zoom/viewport
uniform float uMinPx;              // minimum size in pixels
uniform vec2 uPx;                  // world units per pixel
out vec4 vColor;
out vec2 vUv;
void main() {
  vec2 lo = rect.xy;
  vec2 hi = rect.zw;
  vec2 c = (lo + hi) * 0.5;
  vec2 half_ = max((hi - lo) * 0.5, uPx * uMinPx * 0.5);
  vec2 p = c + (corner * 2.0 - 1.0) * half_;
  gl_Position = vec4(p.x * uScale.x, -p.y * uScale.y, 0.0, 1.0);
  vColor = color;
  vUv = corner * 2.0 - 1.0;
}`;

const FS = `#version 300 es
precision mediump float;
in vec4 vColor;
in vec2 vUv;
uniform int uShape; // 0 solid, 1 glow disc, 2 outline
uniform float uEdge;
out vec4 o;
void main() {
  if (uShape == 1) {
    float d = length(vUv);
    float a = exp(-d * d * 3.5) * smoothstep(1.0, 0.6, d);
    o = vec4(vColor.rgb, vColor.a * a);
  } else if (uShape == 2) {
    vec2 e = step(vec2(1.0 - uEdge), abs(vUv));
    float a = max(e.x, e.y);
    o = vec4(vColor.rgb, vColor.a * max(a, 0.18));
  } else {
    o = vColor;
  }
  o.rgb *= o.a;
}`;

const SEG_VS = `#version 300 es
layout(location=0) in vec2 corner;
layout(location=1) in vec4 seg;    // x0 y0 x1 y1, camera-relative
layout(location=2) in vec4 color;
uniform vec2 uScale;
uniform vec2 uPx;
uniform float uWidth;              // pixels
out vec4 vColor;
out float vAcross;
void main() {
  vec2 a = seg.xy;
  vec2 b = seg.zw;
  vec2 d = b - a;
  float len = max(length(d), 1e-6);
  vec2 n = vec2(-d.y, d.x) / len;
  vec2 p = mix(a, b, corner.x) + n * (corner.y * 2.0 - 1.0) * uWidth * 0.5 * uPx;
  gl_Position = vec4(p.x * uScale.x, -p.y * uScale.y, 0.0, 1.0);
  vColor = color;
  vAcross = corner.y * 2.0 - 1.0;
}`;

const SEG_FS = `#version 300 es
precision mediump float;
in vec4 vColor;
in float vAcross;
out vec4 o;
void main() {
  float a = vColor.a * (1.0 - smoothstep(0.35, 1.0, abs(vAcross)));
  o = vec4(vColor.rgb * a, a);
}`;

export class Batch {
  rects: Float32Array;
  colors: Float32Array;
  n = 0;
  constructor(cap = 1024) {
    this.rects = new Float32Array(cap * 4);
    this.colors = new Float32Array(cap * 4);
  }
  reset() {
    this.n = 0;
  }
  push(x0: number, y0: number, x1: number, y1: number, r: number, g: number, b: number, a: number) {
    if (this.n * 4 >= this.rects.length) {
      const nr = new Float32Array(this.rects.length * 2);
      nr.set(this.rects);
      this.rects = nr;
      const nc = new Float32Array(this.colors.length * 2);
      nc.set(this.colors);
      this.colors = nc;
    }
    const k = this.n * 4;
    this.rects[k] = x0;
    this.rects[k + 1] = y0;
    this.rects[k + 2] = x1;
    this.rects[k + 3] = y1;
    this.colors[k] = r;
    this.colors[k + 1] = g;
    this.colors[k + 2] = b;
    this.colors[k + 3] = a;
    this.n++;
  }
}

export class Renderer {
  gl: WebGL2RenderingContext;
  private prog: WebGLProgram;
  private vao: WebGLVertexArrayObject;
  private rectBuf: WebGLBuffer;
  private colorBuf: WebGLBuffer;
  private uScale: WebGLUniformLocation;
  private uMinPx: WebGLUniformLocation;
  private uPx: WebGLUniformLocation;
  private uShape: WebGLUniformLocation;
  private uEdge: WebGLUniformLocation;
  width = 1;
  height = 1;
  private segProg: WebGLProgram;
  private segU: { scale: WebGLUniformLocation; px: WebGLUniformLocation; width: WebGLUniformLocation };

  constructor(public canvas: HTMLCanvasElement) {
    const gl = canvas.getContext('webgl2', { antialias: false, premultipliedAlpha: true, alpha: false });
    if (!gl) throw new Error('WebGL2 is not available');
    this.gl = gl;
    const sh = (type: number, src: string) => {
      const s = gl.createShader(type)!;
      gl.shaderSource(s, src);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s) ?? 'shader');
      return s;
    };
    const link = (vs: string, fs: string) => {
      const p = gl.createProgram()!;
      gl.attachShader(p, sh(gl.VERTEX_SHADER, vs));
      gl.attachShader(p, sh(gl.FRAGMENT_SHADER, fs));
      gl.linkProgram(p);
      if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p) ?? 'link');
      return p;
    };
    const p = link(VS, FS);
    this.segProg = link(SEG_VS, SEG_FS);
    this.segU = {
      scale: gl.getUniformLocation(this.segProg, 'uScale')!,
      px: gl.getUniformLocation(this.segProg, 'uPx')!,
      width: gl.getUniformLocation(this.segProg, 'uWidth')!,
    };
    this.prog = p;
    this.uScale = gl.getUniformLocation(p, 'uScale')!;
    this.uMinPx = gl.getUniformLocation(p, 'uMinPx')!;
    this.uPx = gl.getUniformLocation(p, 'uPx')!;
    this.uShape = gl.getUniformLocation(p, 'uShape')!;
    this.uEdge = gl.getUniformLocation(p, 'uEdge')!;
    this.vao = gl.createVertexArray()!;
    gl.bindVertexArray(this.vao);
    const corners = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, corners);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    this.rectBuf = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.rectBuf);
    gl.enableVertexAttribArray(1);
    gl.vertexAttribPointer(1, 4, gl.FLOAT, false, 0, 0);
    gl.vertexAttribDivisor(1, 1);
    this.colorBuf = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.colorBuf);
    gl.enableVertexAttribArray(2);
    gl.vertexAttribPointer(2, 4, gl.FLOAT, false, 0, 0);
    gl.vertexAttribDivisor(2, 1);
    gl.bindVertexArray(null);
  }

  resize() {
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = Math.max(1, Math.round(this.canvas.clientWidth * dpr));
    const h = Math.max(1, Math.round(this.canvas.clientHeight * dpr));
    if (this.canvas.width !== w || this.canvas.height !== h) {
      this.canvas.width = w;
      this.canvas.height = h;
    }
    this.width = w;
    this.height = h;
  }

  begin(bg: [number, number, number]) {
    const gl = this.gl;
    gl.viewport(0, 0, this.width, this.height);
    gl.clearColor(bg[0], bg[1], bg[2], 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.enable(gl.BLEND);
  }

  /** Draw line segments (batch rects hold x0 y0 x1 y1) with a width in pixels. */
  drawSegments(b: Batch, cam: Camera, widthPx: number, additive = true) {
    if (b.n === 0) return;
    const gl = this.gl;
    const dpr = this.width / Math.max(1, this.canvas.clientWidth);
    const z = cam.zoom * dpr;
    gl.useProgram(this.segProg);
    gl.bindVertexArray(this.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.rectBuf);
    gl.bufferData(gl.ARRAY_BUFFER, b.rects.subarray(0, b.n * 4), gl.STREAM_DRAW);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.colorBuf);
    gl.bufferData(gl.ARRAY_BUFFER, b.colors.subarray(0, b.n * 4), gl.STREAM_DRAW);
    gl.uniform2f(this.segU.scale, (2 * z) / this.width, (2 * z) / this.height);
    gl.uniform2f(this.segU.px, 1 / z, 1 / z);
    gl.uniform1f(this.segU.width, widthPx * dpr);
    if (additive) gl.blendFunc(gl.ONE, gl.ONE);
    else gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, b.n);
    gl.bindVertexArray(null);
  }

  /** Draw a batch. `minPx`: minimum on-screen size; `additive` for glows. */
  draw(b: Batch, cam: Camera, shape: 0 | 1 | 2, minPx = 0, additive = false, edge = 0.1) {
    if (b.n === 0) return;
    const gl = this.gl;
    const dpr = this.width / Math.max(1, this.canvas.clientWidth);
    const z = cam.zoom * dpr;
    gl.useProgram(this.prog);
    gl.bindVertexArray(this.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.rectBuf);
    gl.bufferData(gl.ARRAY_BUFFER, b.rects.subarray(0, b.n * 4), gl.STREAM_DRAW);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.colorBuf);
    gl.bufferData(gl.ARRAY_BUFFER, b.colors.subarray(0, b.n * 4), gl.STREAM_DRAW);
    gl.uniform2f(this.uScale, (2 * z) / this.width, (2 * z) / this.height);
    gl.uniform1f(this.uMinPx, minPx * dpr);
    gl.uniform2f(this.uPx, 1 / z, 1 / z);
    gl.uniform1i(this.uShape, shape);
    gl.uniform1f(this.uEdge, edge);
    if (additive) gl.blendFunc(gl.ONE, gl.ONE);
    else gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, b.n);
    gl.bindVertexArray(null);
  }
}
