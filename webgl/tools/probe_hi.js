(() => {
  S.resolution = 20; S.spp = 2; S.maxSamples = 500; S.denoise = false;
  resDirty = true; needsReset = true;
  return JSON.stringify({ res: S.resolution, max: S.maxSamples, rough: S.mats.B.rough, metal: S.mats.B.metal,
    shapeB: S.objBShape });
})()
