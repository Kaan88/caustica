(async () => {
  const out = { errors: [] };
  const t = (n, fn) => { try { fn(); } catch (e) { out.errors.push(n + ': ' + e.message); } };
  const raf = () => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
  const mean = () => {
    const w = canvas.width, h = canvas.height;
    const c = document.createElement('canvas'); c.width = w; c.height = h;
    const x = c.getContext('2d', { willReadFrequently: true });
    x.drawImage(canvas, 0, 0, w, h);
    const d = x.getImageData(0, 0, w, h).data;
    let s = 0, n = 0;
    for (let y = 0; y < h; y += 2) for (let px = 0; px < w; px += 2) {
      const k = (y * w + px) * 4; s += 0.2126 * d[k] + 0.7152 * d[k + 1] + 0.0722 * d[k + 2]; n++;
    }
    return n ? +(s / n).toFixed(1) : 0;
  };
  const grad = () => {
    const w = canvas.width, h = canvas.height;
    const c = document.createElement('canvas'); c.width = w; c.height = h;
    const x = c.getContext('2d', { willReadFrequently: true });
    x.drawImage(canvas, 0, 0, w, h);
    const d = x.getImageData(0, 0, w, h).data;
    let s = 0, n = 0;
    for (let y = Math.floor(h * 0.25); y < Math.floor(h * 0.7); y++)
      for (let px = Math.floor(w * 0.3); px < Math.floor(w * 0.7) - 1; px++) {
        const k = (y * w + px) * 4, k2 = (y * w + px + 1) * 4;
        s += Math.abs(d[k] - d[k2]); n++;
      }
    return n ? +(s / n).toFixed(2) : 0;
  };

  S = structuredClone(DEFAULTS); paused = false; sceneDirty = true; needsReset = true; resDirty = true;
  S.resolution = 30; S.spp = 2; S.maxSamples = 400; syncUI(); refreshMaterialState();
  await new Promise(r => setTimeout(r, 5000));

  out.presets = {};
  for (const p of ['classic', 'reflective', 'mirrorglass', 'warm', 'dramatic', 'emissive']) {
    t('preset ' + p, () => doAction('applyPreset', p));
    await raf(); await raf();
    out.presets[p] = { shapes: S.objAShape + '/' + S.objBShape, cam: +S.camDistance.toFixed(2), mean: mean() };
  }

  S.viewMode = 'beauty'; await raf(); await raf();
  out.noiseOff = grad();
  t('denoise on', () => { S.denoise = true; });
  await new Promise(r => setTimeout(r, 1200)); await raf(); await raf();
  out.noiseOn = grad();
  S.denoise = false;

  out.views = {};
  for (const m of ['beauty', 'normal', 'albedo', 'depth', 'direct', 'ind', 'specular', 'convergence']) {
    t('view ' + m, () => { S.viewMode = m; });
    await raf(); await raf();
    out.views[m] = mean();
  }
  S.viewMode = 'beauty';

  out.actions = {};
  t('pause', () => doAction('togglePause')); out.actions.pausedTrue = paused;
  t('resume', () => doAction('togglePause')); out.actions.pausedFalse = paused;
  t('captureRef', () => doAction('captureRef'));
  t('resetAccum', () => doAction('resetAccum'));
  await raf(); await raf();
  out.actions.samplesAfterReset = samples;
  const before = JSON.stringify(S.lightIntensity) + S.wallLeftColor;
  t('randomize', () => doAction('randomize'));
  out.actions.randomizedChanged = before !== JSON.stringify(S.lightIntensity) + S.wallLeftColor;
  t('resetAll', () => doAction('resetAll'));
  out.actions.resetAllBack = S.lightIntensity === DEFAULTS.lightIntensity && S.preset === 'classic';

  let captured = null;
  const origClick = HTMLInputElement.prototype.click;
  HTMLInputElement.prototype.click = function () { if (this.type === 'file') { captured = this; return; } return origClick.call(this); };
  S.lightIntensity = 21; S.mats.A.rough = 0.33;
  const json = JSON.stringify(S, null, 2);
  S.lightIntensity = 5; S.mats.A.rough = 0.9;
  t('loadJSON', () => doAction('loadJSON'));
  if (captured) {
    const dt = new DataTransfer();
    dt.items.add(new File([json], 's.json', { type: 'application/json' }));
    captured.files = dt.files; captured.dispatchEvent(new Event('change'));
  }
  await new Promise(r => setTimeout(r, 500));
  HTMLInputElement.prototype.click = origClick;
  out.jsonRoundTrip = { intensity: S.lightIntensity, rough: S.mats.A.rough, ok: S.lightIntensity === 21 && S.mats.A.rough === 0.33 };

  for (const k of [' ', 'r', 'a', 'd', '3', '7']) {
    t('key ' + k, () => window.dispatchEvent(new KeyboardEvent('keydown', { key: k })));
  }
  await raf(); await raf();
  out.keys = { viewAfter7: S.viewMode, autorotate: S.autoRotate, denoise: S.denoise };
  out.final = { samples, status: typeof statusText !== 'undefined' ? statusText : (typeof S.statusText !== 'undefined' ? S.statusText : '?'), hud: [...document.querySelectorAll('#hud .stat .v')].map(e => e.textContent) };
  return JSON.stringify(out);
})()
