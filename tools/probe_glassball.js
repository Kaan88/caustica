(() => {
  const sel = uiRefs.preset && uiRefs.preset.input;
  if (sel) { sel.value = 'mirrorglass'; sel.dispatchEvent(new Event('change')); } else applyPreset('mirrorglass');
  S.objBShape = 'sphere';
  S.mats.B.trans = 1.0; S.mats.B.rough = 0.02; S.mats.B.metal = 0; S.mats.B.refl = 0;
  settleObjects();
  S.resolution = 30; S.spp = 2; S.maxSamples = 400; S.denoise = false;
  sceneDirty = true; resDirty = true; needsReset = true;
  return JSON.stringify({ shapeB: S.objBShape, objBY: S.objBY, trans: S.mats.B.trans });
})()
