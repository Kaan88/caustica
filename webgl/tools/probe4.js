(() => {
  const res = {};
  const shaders = { trace: FS_TRACE, denoise: FS_DENOISE, display: FS_DISPLAY, copy: FS_COPY };
  for (const [name, src] of Object.entries(shaders)) {
    try {
      const fs = gl.createShader(gl.FRAGMENT_SHADER);
      gl.shaderSource(fs, src);
      gl.compileShader(fs);
      const ok = gl.getShaderParameter(fs, gl.COMPILE_STATUS);
      res[name] = ok ? 'OK' : (gl.getShaderInfoLog(fs) || 'FAILED (no log)').slice(0, 900);
      gl.deleteShader(fs);
    } catch (e) { res[name] = 'ERR ' + e.message; }
  }
  return JSON.stringify(res);
})()
