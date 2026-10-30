(() => {
  const out = {};
  const c = document.createElement('canvas');
  out.webgl2 = (() => { try { return !!c.getContext('webgl2'); } catch (e) { return 'THROW ' + e.message; } })();
  out.webgl1 = (() => { try { return !!c.getContext('webgl'); } catch (e) { return 'THROW ' + e.message; } })();
  try {
    const g2 = c.getContext('webgl2');
    if (g2) {
      out.ver = g2.getParameter(g2.VERSION);
      out.shading = g2.getParameter(g2.SHADING_LANGUAGE_VERSION);
      out.rbFloat = !!g2.getExtension('EXT_color_buffer_float');
      out.rbHalf = !!g2.getExtension('EXT_color_buffer_half_float');
      out.linFloat = !!g2.getExtension('OES_texture_float_linear');
      out.maxDraw = g2.getParameter(g2.MAX_DRAW_BUFFERS);
      out.maxTex = g2.getParameter(g2.MAX_TEXTURE_SIZE);
      const dbg = g2.getExtension('WEBGL_debug_renderer_info');
      out.renderer = dbg ? g2.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : g2.getParameter(g2.RENDERER);
    }
  } catch (e) { out.probeErr = e.message; }
  out.ua = navigator.userAgent.slice(0, 60);
  out.appGlErr = typeof glError !== 'undefined' ? String(glError).slice(0, 300) : 'no glError var';
  return JSON.stringify(out);
})()
