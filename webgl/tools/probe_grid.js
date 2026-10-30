(() => {
  displayPass(null, canvas.width, canvas.height, false);
  const w = canvas.width, h = canvas.height;
  const c2 = document.createElement('canvas'); c2.width = w; c2.height = h;
  const ctx = c2.getContext('2d', { willReadFrequently: true });
  ctx.drawImage(canvas, 0, 0, w, h);
  const d = ctx.getImageData(0, 0, w, h).data;
  const gx = 6, gy = 4, grid = [];
  for (let j = 0; j < gy; j++) {
    const row = [];
    for (let i = 0; i < gx; i++) {
      let sum = 0, n = 0;
      const x0 = Math.floor(i * w / gx), x1 = Math.floor((i + 1) * w / gx);
      const y0 = Math.floor(j * h / gy), y1 = Math.floor((j + 1) * h / gy);
      for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) {
        const k = (y * w + x) * 4;
        sum += 0.2126 * d[k] + 0.7152 * d[k + 1] + 0.0722 * d[k + 2]; n++;
      }
      row.push(n ? +(sum / n).toFixed(1) : 0);
    }
    grid.push(row);
  }
  return JSON.stringify({ grid, samples, view: S.viewMode });
})()
