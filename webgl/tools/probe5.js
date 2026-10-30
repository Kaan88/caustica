(() => {
  try {
    S.resolution = 26; S.spp = 1; S.maxSamples = 48; S.denoise = false;
    resDirty = true; needsReset = true;
    return JSON.stringify({ started: true, res: S.resolution, spp: S.spp });
  } catch (e) { return 'ERR ' + e.message; }
})()
