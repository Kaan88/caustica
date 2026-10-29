(() => {
  const captured = [];
  const origCreate = URL.createObjectURL;
  const origClick = HTMLAnchorElement.prototype.click;
  const origDownload = Object.getOwnPropertyDescriptor(HTMLAnchorElement.prototype, 'download');
  let lastDownload = null;
  URL.createObjectURL = function (b) { captured.push({ kind: b.type || 'blob', size: b.size }); return origCreate.call(URL, b); };
  Object.defineProperty(HTMLAnchorElement.prototype, 'download', {
    set(v) { lastDownload = v; }, get() { return null; }
  });
  HTMLAnchorElement.prototype.click = function () {
    captured.push({ href: (this.href || '').slice(0, 40), len: (this.href || '').length, name: lastDownload });
  };
  const out = { errors: [] };
  const run = fn => { try { fn(); } catch (e) { out.errors.push(String(e && e.message || e)); } };

  run(() => exportPNG());
  run(() => screenshot());
  run(() => saveJSON());
  out.captured = captured;
  Object.defineProperty(HTMLAnchorElement.prototype, 'download', origDownload || {});
  HTMLAnchorElement.prototype.click = origClick;
  URL.createObjectURL = origCreate;
  out.res = S.resolution; out.camDistance = +S.camDistance.toFixed(3);
  out.objAY = S.objAY; out.objBY = S.objBY;
  return JSON.stringify(out);
})()
