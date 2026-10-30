(() => {
  try {
    const vs = gl.createShader(gl.VERTEX_SHADER);
    gl.shaderSource(vs, VS_QUAD); gl.compileShader(vs);
    const out = { vsLog: gl.getShaderInfoLog(vs) || '' };
    const fs = gl.createShader(gl.FRAGMENT_SHADER);
    gl.shaderSource(fs, FS_TRACE); gl.compileShader(fs);
    out.fsLog = gl.getShaderInfoLog(fs) || '(no log)';
    out.status = gl.getShaderParameter(fs, gl.COMPILE_STATUS);
    return JSON.stringify(out);
  } catch (e) { return 'ERR ' + e.message; }
})()
