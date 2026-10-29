(() => {
  const preset = __PRESET__ || 'classic';
  const sel = uiRefs.preset && uiRefs.preset.input;
  if (sel) { sel.value = preset; sel.dispatchEvent(new Event('change')); } else applyPreset(preset);
  S.resolution = 45; S.spp = 2; S.maxSamples = 220; S.denoise = false;
  resDirty = true;
  return JSON.stringify({ preset: S.preset, objAY: S.objAY, objBY: S.objBY,
    camDistance: +S.camDistance.toFixed(2), dist: S.mats.A.dist.toFixed(2), trans: S.mats.A.trans.toFixed(2),
    samples: samples });
})()
