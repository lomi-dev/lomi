import type { AndroidFrame } from "./frame";

export interface AndroidRenderer {
  readonly element: HTMLCanvasElement;
  draw(
    frame: AndroidFrame,
    source?: HTMLCanvasElement,
    target?: { width: number; height: number },
  ): void;
  dispose(): void;
}

/** Owns only a visible view's GPU resources; device/stream ownership lives elsewhere. */
export class AndroidCanvas implements AndroidRenderer {
  readonly element = document.createElement("canvas");
  private readonly gl: WebGLRenderingContext;
  private readonly program: WebGLProgram;
  private readonly texture: WebGLTexture;
  private readonly vertices: WebGLBuffer;
  private width = 0;
  private height = 0;
  private disposed = false;
  private readonly onLost: (event: Event) => void;

  constructor(onError: (message: string) => void) {
    this.element.className = "android-screen";
    this.element.setAttribute("aria-label", "Android phone screen");
    this.element.tabIndex = 0;
    const gl = this.element.getContext("webgl", {
      alpha: false,
      antialias: false,
      depth: false,
      stencil: false,
      preserveDrawingBuffer: false,
    });
    if (!gl || gl.isContextLost())
      throw new Error("Android graphics are unavailable. Reconnect the panel.");
    this.gl = gl;
    const program = gl.createProgram();
    if (!program)
      throw new Error("Cannot create the Android screen. Reconnect the panel.");
    try {
      for (const [kind, source] of [
        [
          gl.VERTEX_SHADER,
          "attribute vec2 p; varying vec2 uv; void main(){ gl_Position=vec4(p,0.,1.); uv=vec2((p.x+1.)*.5,(1.-p.y)*.5); }",
        ],
        [
          gl.FRAGMENT_SHADER,
          "precision mediump float; varying vec2 uv; uniform sampler2D frame; void main(){ gl_FragColor=texture2D(frame,uv); }",
        ],
      ] as const) {
        const shader = gl.createShader(kind);
        if (!shader) throw new Error("Cannot compile Android screen shaders.");
        gl.shaderSource(shader, source);
        gl.compileShader(shader);
        if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
          gl.deleteShader(shader);
          throw new Error(
            "Android screen shader compilation failed. Reconnect the panel.",
          );
        }
        gl.attachShader(program, shader);
        gl.deleteShader(shader);
      }
      gl.linkProgram(program);
      if (!gl.getProgramParameter(program, gl.LINK_STATUS))
        throw new Error("Android screen shader linking failed.");
      gl.useProgram(program);
      const vertices = gl.createBuffer();
      const texture = gl.createTexture();
      if (!vertices || !texture) {
        gl.deleteBuffer(vertices);
        gl.deleteTexture(texture);
        throw new Error("Cannot allocate Android screen resources.");
      }
      this.vertices = vertices;
      this.texture = texture;
      gl.bindBuffer(gl.ARRAY_BUFFER, vertices);
      gl.bufferData(
        gl.ARRAY_BUFFER,
        new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]),
        gl.STATIC_DRAW,
      );
      const position = gl.getAttribLocation(program, "p");
      gl.enableVertexAttribArray(position);
      gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.uniform1i(gl.getUniformLocation(program, "frame"), 0);
      this.program = program;
    } catch (error) {
      gl.deleteProgram(program);
      gl.getExtension("WEBGL_lose_context")?.loseContext();
      throw error;
    }
    this.onLost = (event) => {
      event.preventDefault();
      if (!this.disposed)
        onError("Android graphics context was lost. Reconnect the panel.");
    };
    this.element.addEventListener("webglcontextlost", this.onLost);
  }

  draw(
    frame: AndroidFrame,
    source?: HTMLCanvasElement,
    target = { width: frame.width, height: frame.height },
  ) {
    const { gl, element } = this;
    if (this.disposed || gl.isContextLost())
      throw new Error(
        "Android graphics context was lost. Reconnect the panel.",
      );
    if (!frame.width || !frame.height) {
      gl.clear(gl.COLOR_BUFFER_BIT);
      return;
    }
    if (element.width !== target.width || element.height !== target.height) {
      element.width = target.width;
      element.height = target.height;
      gl.viewport(0, 0, target.width, target.height);
    }
    if (this.width !== frame.width || this.height !== frame.height) {
      this.width = frame.width;
      this.height = frame.height;
      gl.texImage2D(
        gl.TEXTURE_2D,
        0,
        gl.RGBA,
        frame.width,
        frame.height,
        0,
        gl.RGBA,
        gl.UNSIGNED_BYTE,
        null,
      );
    }
    if (source)
      gl.texSubImage2D(
        gl.TEXTURE_2D,
        0,
        0,
        0,
        gl.RGBA,
        gl.UNSIGNED_BYTE,
        source,
      );
    else
      gl.texSubImage2D(
        gl.TEXTURE_2D,
        0,
        0,
        0,
        frame.width,
        frame.height,
        gl.RGBA,
        gl.UNSIGNED_BYTE,
        frame.pixels,
      );
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.element.removeEventListener("webglcontextlost", this.onLost);
    this.gl.deleteTexture(this.texture);
    this.gl.deleteBuffer(this.vertices);
    this.gl.deleteProgram(this.program);
    this.gl.getExtension("WEBGL_lose_context")?.loseContext();
    this.element.width = this.element.height = 0;
    this.element.remove();
  }
}
