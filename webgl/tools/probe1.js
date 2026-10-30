(() => {
  try {
    const g = k => (typeof k !== 'undefined' ? k : 'MISSING');
    return JSON.stringify({
      glOK: g(glOK),
      overlay: document.getElementById('overlay').textContent.slice(0, 600),
      samples: g(samples),
      canvasW: g(canvas && canvas.width),
      canvasH: g(canvas && canvas.height),
      groups: document.querySelectorAll('#panels .group').length,
      ranges: document.querySelectorAll('#panels input[type=range]').length,
      colors: document.querySelectorAll('#panels input[type=color]').length,
      buttons: document.querySelectorAll('#panels .btn').length,
      selects: document.querySelectorAll('#panels select').length,
      checks: document.querySelectorAll('#panels input[type=checkbox]').length,
      nTargets: g(nTargets),
      gpu: document.getElementById('gpu').textContent.slice(0, 140),
      hud: [...document.querySelectorAll('#hud .stat .v')].map(x => x.textContent)
    });
  } catch (e) { return 'ERR ' + e.message + ' | ' + e.stack; }
})()
