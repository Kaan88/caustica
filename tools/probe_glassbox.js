(() => {
  const sel = uiRefs.preset && uiRefs.preset.input;
  if (sel) { sel.value = 'mirrorglass'; sel.dispatchEvent(new Event('change')); } else applyPreset('mirrorglass');
  S.glassShadow = false;
  S.resolution = 30; S.spp = 2; S.maxSamples = 400; S.denoise = false;
  sceneDirty = true; resDirty = true; needsReset = true;
  return JSON.stringify({ glassShadow: S.glassShadow, shapeB: S.objBShape, transB: S.mats.B.trans });
})()
