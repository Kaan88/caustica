"""Extract Martin Newell's patch data from the vendored MIT three.js source."""
from pathlib import Path
import re
root = Path(__file__).resolve().parents[1]
source = (root / "assets/TeapotGeometry.source.js").read_text(encoding="utf8")
def array(name):
    body = re.search(r"const " + name + r" = \[([\s\S]*?)\];", source)[1]
    body = re.sub(r"/\*[\s\S]*?\*/", "", body)
    return [float(x.replace(" ", "")) for x in re.findall(r"-?\s*\d+(?:\.\d+)?", body)]
patches, points = array("teapotPatches"), array("teapotVertices")
out = "// Martin Newell teapot data from three.js r180 (MIT). See assets/THREE-LICENSE.txt.\n"
out += f"pub const PATCHES: [usize; {len(patches)}] = [" + ",".join(str(int(x)) for x in patches) + "];\n"
out += f"pub const POINTS: [[f32;3]; {len(points)//3}] = [" + ",".join("[" + ",".join(repr(v) for v in points[i:i+3]) + "]" for i in range(0,len(points),3)) + "];\n"
(root / "src/teapot_data.rs").write_text(out, encoding="utf8")
