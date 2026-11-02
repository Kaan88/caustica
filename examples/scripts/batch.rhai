// Batch render: each listed scene at 64 frames, plus a turned box in the classic scene.
// Run: cornell-rtx.exe --headless --width 640 --height 400 --script examples/scripts/batch.rhai
for name in ["hourglass", "snow-globe"] {
    load(name);
    frames(64);
    print(`${name}: ${samples()} samples`);
    capture(`renders/${name}.png`);
}
load("classic-cornell-box");
set("objects/0/rotation", "40");
frames(32);
capture("renders/classic-turned.png");
print(`${scenes().len()} scenes available`);
