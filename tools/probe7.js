(() => {
  S.resolution = 45; S.spp = 2; S.maxSamples = 220; S.denoise = false;
  resDirty = true; needsReset = true;
  return JSON.stringify({ res: S.resolution, spp: S.spp, max: S.maxSamples,
    camDistance: +S.camDistance.toFixed(2), fov: S.camFov });
})()
