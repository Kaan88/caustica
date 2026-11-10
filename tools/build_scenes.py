"""Writes scenes/builtin/*.json: the 42 scenes buildable without engine changes
(docs/scene-research.md section 2). Each file is a scene wrapper around a sparse Settings
object: fields left out take Settings::default(), so only what a scene changes is written.

Run:  python tools/build_scenes.py [--overwrite]
It writes only missing scene files (--overwrite also rewrites ones that differ), never
deletes, and merges its entries into each pack.json, so hand-written scenes are safe.
Units: the floor is at y = -1; a shape of size s spans about 0.7*s (unit meshes are -1..1,
scaled by 0.35*size). Tori and rings lie flat; cylinders and cones stand up.
"""
import json
import math
import random
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "scenes" / "builtin"

# The scene folders also hold scenes written by hand, or edited by hand after this script
# wrote them, so the writers never delete anything. A scene file is written only when it is
# missing, or when it differs and --overwrite is given (files this script doesn't define are
# never touched), and the builder's entries are merged into an existing pack.json.
OVERWRITE = False  # True, or a set of file stems

# Dielectric contacts: two faces at exactly the same place give two hits at equal t, which the
# GPU (DXR traversal order) and the CPU reference (its BVH) resolve differently (engine-notes
# "Grounded dielectric gap"). Grounded transmissive objects get GAP from the engine itself
# (geometry::DIELECTRIC_GAP); the hand-placed ones here keep the same gap off the floor and off
# each other, and coincident side walls of nested liquids differ by GAP.
GAP = 0.004


def write_pack(out, entries, meta, ensure_ascii=False):
    """Writes `entries` ([(file name, data)]) into `out` and merges them into its pack.json."""
    out.mkdir(parents=True, exist_ok=True)
    written = kept = same = 0
    for name, data in entries:
        text = json.dumps(data, indent=1, ensure_ascii=ensure_ascii) + "\n"
        path = out / name
        if path.exists():
            if path.read_text(encoding="utf-8") == text:
                same += 1
                continue
            if not (OVERWRITE is True or (OVERWRITE and path.stem in OVERWRITE)):
                kept += 1
                continue
        path.write_text(text, encoding="utf-8")
        written += 1
    pack_path = out / "pack.json"
    if pack_path.exists():
        pack = json.loads(pack_path.read_text(encoding="utf-8"))
        listed = pack.get("scenes", [])
        pack["scenes"] = listed + [name for name, _ in entries if name not in listed]
    else:
        pack = dict(meta, scenes=[name for name, _ in entries])
    pack_text = json.dumps(pack, indent=1) + "\n"
    if not pack_path.exists() or pack_path.read_text(encoding="utf-8") != pack_text:
        pack_path.write_text(pack_text, encoding="utf-8")
    note = f", {kept} differ on disk and were kept (--overwrite rewrites them)" if kept else ""
    print(f"{out.name}: {written} written, {same} unchanged{note}")


# ---------------------------------------------------------------- materials
def M(color=(0.72, 0.70, 0.66), rough=0.55, metal=0.0, refl=0.05, trans=0.0, ior=1.5,
      emit=0.0, pattern="solid", second=(0.04, 0.04, 0.04), pscale=3.0, unlit=False):
    return {"color": [round(c, 4) for c in color], "roughness": rough, "metallic": metal,
            "reflectivity": refl, "transmission": trans, "ior": ior, "emission": emit,
            "pattern": pattern, "secondary_color": [round(c, 4) for c in second],
            "pattern_scale": pscale, "unlit": unlit}


PORCELAIN = M((0.88, 0.87, 0.83), 0.2)
CHROME = M((0.92, 0.94, 0.97), 0.035, metal=1)
GOLD = M((1.0, 0.71, 0.29), 0.18, metal=1)
COPPER = M((0.95, 0.47, 0.26), 0.25, metal=1)
SILVER = M((0.95, 0.95, 0.93), 0.12, metal=1)
GLASS = M((0.96, 0.99, 1.0), 0.0, trans=1, ior=1.52)
FROSTED = M((0.78, 0.92, 0.89), 0.19, trans=1)
JADE = M((0.09, 0.46, 0.25), 0.15, refl=0.15)
RUBBER = M((0.68, 0.025, 0.018), 0.9)
LACQUER = M((0.025, 0.17, 0.65), 0.12, refl=0.2)
MARBLE = M((0.85, 0.84, 0.79), 0.25, pattern="marble", second=(0.05, 0.07, 0.1), pscale=5)
WARM = M((1.0, 0.32, 0.08), 0.4, emit=8)
COOL = M((0.08, 0.5, 1.0), 0.4, emit=8)
LIBRARY = [PORCELAIN, CHROME, GOLD, COPPER, GLASS, FROSTED, JADE, RUBBER, LACQUER, MARBLE, WARM, COOL]


def lacquer(color, rough=0.12):
    return M(color, rough, refl=0.2)


def matte(color, rough=0.9):
    return M(color, rough, refl=0.0)


def emitter(color, strength, unlit=True):
    return M(color, 0.5, emit=strength, unlit=unlit)


def kelvin(k):
    """Linear RGB of a blackbody, brightest channel 1 (Tanner Helland's fit)."""
    t = k / 100
    r = 255 if t <= 66 else 329.698727446 * (t - 60) ** -0.1332047592
    g = (99.4708025861 * math.log(t) - 161.1195681661) if t <= 66 else 288.1221695283 * (t - 60) ** -0.0755148492
    b = 255 if t >= 66 else (0 if t <= 19 else 138.5177312231 * math.log(t - 10) - 305.0447927307)
    c = [(min(max(v, 0), 255) / 255) ** 2.2 for v in (r, g, b)]
    m = max(c)
    return [round(v / m, 4) for v in c]


# ---------------------------------------------------------------- objects and lights
def obj(name, shape, x, z, size, material, *, stretch=1.0, rotation=0.0, tilt=(0, 0),
        scale=(1, 1), elevation=0.0, grounded=True, visible=True, portal=-1):
    # validate() bounds stretch (0.05..10) and scale (0.01..20) separately from size (0.05..30);
    # move any excess into size, which keeps the world dimensions (0.35 * size * factor).
    sx, sz = scale
    grow = max(1.0, stretch / 10, sx / 20, sz / 20)
    shrink = min(1.0, stretch / 0.05, sx / 0.01, sz / 0.01)
    k = grow * shrink
    size, stretch, scale = size * k, stretch / k, (sx / k, sz / k)
    rotation = (rotation + 180) % 360 - 180
    return {"name": name, "shape": shape, "position": [round(x, 4), round(z, 4)],
            "size": round(size, 4), "stretch": round(stretch, 4), "rotation": round(rotation, 3),
            "tilt": [round(tilt[0], 3), round(tilt[1], 3)], "scale_xz": [round(scale[0], 4), round(scale[1], 4)],
            "elevation": round(elevation, 4), "grounded": grounded, "visible": visible,
            "material": material, "portal": portal}


def float_at(name, shape, x, y, z, size, material, **kw):
    """An object whose mesh center sits at world y (not resting on the floor)."""
    return obj(name, shape, x, z, size, material, elevation=y, grounded=False, **kw)


def aim(pos, target):
    """Pitch/yaw/roll that turn a downward-facing light quad toward target."""
    n = [t - p for t, p in zip(target, pos)]
    length = math.sqrt(sum(v * v for v in n))
    n = [v / length for v in n]
    pitch = math.degrees(math.acos(max(-1, min(1, -n[1]))))
    yaw = math.degrees(math.atan2(-n[0], -n[2])) if abs(n[1]) < 0.9999 else 0.0
    return [round(pitch, 3), round(yaw, 3), 0.0]


def light(name, pos, size, color=(1, 1, 1), intensity=12.0, target=None, visible=False, enabled=True):
    return {"name": name, "position": [round(v, 4) for v in pos],
            "rotation": aim(pos, target) if target else [0.0, 0.0, 0.0],
            "size": [round(size[0], 4), round(size[1], 4)], "color": [round(c, 4) for c in color],
            "intensity": round(intensity, 3), "enabled": enabled, "visible": visible}


def camera(yaw, pitch, distance, target, fov=36.0):
    return {"yaw": yaw, "pitch": pitch, "distance": distance, "target": list(target), "fov": fov}


def studio(**kw):
    """The open studio of presets.rs: a 15-unit floor, a soft key light and a cool rim."""
    s = {"cornell_room": False, "floor_extent": 15.0, "sky": 0.08, "base": [0.22, 0.25, 0.3],
         "floor": M((0.72, 0.70, 0.66), 0.3, refl=0.08),
         "light_on": True, "light_position": [-1.0, 4.0, 2.0], "light_size": 3.0,
         "light_intensity": 12.0, "light_visible": False, "light_temperature": 5600.0,
         "lights": [light("Cool rim", (2.5, 3, -2.5), (2, 2), (0.42, 0.65, 1.0), 10)],
         "bounces": 10, **camera(15, 14, 8, (0, -0.1, 0))}
    s.update(kw)
    return s


def dark_stage(**kw):
    """A black open stage: no main light, no sky."""
    s = {"light_on": False, "sky": 0.0, "lights": [], "base": [0.05, 0.05, 0.055], "background": [0.0, 0.0, 0.0]}
    s.update(kw)
    return studio(**s)


def cornell(**kw):
    """The Cornell box of Settings::default(); scenes replace its objects."""
    s = {"cornell_room": True, "objects": []}
    s.update(kw)
    return s


SCENES = []


def scene(slug, title, description, tags, settings):
    assert len(settings.get("objects", [])) <= 4096, slug
    assert len(settings.get("lights", [])) <= 32, slug
    SCENES.append((slug, {"format": 1, "title": title, "description": description, "tags": tags,
                          "credits": "Caustica built-in scenes", "settings": settings}))


# ================================================================ Cornell-box variants
scene("mirror-room", "Mirror room",
      "Every wall a mirror around one matte sphere: interreflections down to the bounce limit.",
      ["cornell", "mirror", "bounces"],
      cornell(left=[0.93, 0.86, 0.86], right=[0.86, 0.93, 0.88], base=[0.93, 0.93, 0.93],
              walls=M(rough=0.0, metal=1), floor=matte((0.4, 0.4, 0.4), 0.6), bounces=24,
              objects=[obj("Matte sphere", "sphere", 0, -0.2, 1.25, matte((0.75, 0.18, 0.08), 0.8)),
                       float_at("Front mirror", "box", 0, 0, 1.02, 1, M((0.93, 0.93, 0.93), 0.0, metal=1), stretch=2.86, scale=(2.86, 0.05))],
              **camera(0, 6, 0.9, (0, -0.15, -0.2), 72)))

scene("measured-cornell", "Measured Cornell box",
      "The original box's measured wall colours, light size and block layout (approximate positions).",
      ["cornell", "reference"],
      cornell(left=[0.63, 0.065, 0.05], right=[0.14, 0.45, 0.091], base=[0.725, 0.71, 0.68],
              light_size=0.47, light_aspect=0.81, light_temperature=4800.0, light_intensity=17.0,
              light_position=[0.0, 0.995, 0.0],
              objects=[obj("Tall block", "box", -0.33, -0.3, 0.85, matte((0.725, 0.71, 0.68), 0.85), stretch=1.98, rotation=17),
                       obj("Short block", "box", 0.33, 0.3, 0.85, matte((0.725, 0.71, 0.68), 0.85), stretch=0.99, rotation=-17)],
              **camera(0, 0, 3.9, (0, 0, 0), 39.3)))

scene("cornell-bunny", "Cornell bunny",
      "The classic box with a jade Stanford bunny in place of the tall block.",
      ["cornell", "mesh"],
      cornell(objects=[obj("Jade bunny", "bunny", -0.3, -0.1, 1.45, JADE, rotation=25),
                       obj("Short block", "box", 0.38, -0.38, 0.62, matte((0.72, 0.7, 0.66), 0.55), stretch=0.62, rotation=-14)]))

scene("cornell-caustic", "Cornell caustic",
      "A small, very bright light over a glass sphere: the path tracer finds the caustic only by BSDF paths, "
      "so it stays noisy; Render → Integrator → Photon mapping traces it from the light and resolves it in seconds.",
      ["cornell", "glass", "caustics", "stress"],
      cornell(light_size=0.18, light_intensity=200.0, bounces=12,
              objects=[obj("Glass sphere", "sphere", 0.0, 0.0, 1.3, GLASS),
                       obj("Short block", "box", 0.55, -0.55, 0.5, matte((0.72, 0.7, 0.66), 0.6), stretch=0.6, rotation=-20)]))

scene("cornell-dark-room", "Cornell dark room",
      "The ceiling light is off; one glowing torus lights the whole box.",
      ["cornell", "emissive"],
      cornell(light_on=False, light_visible=False,
              objects=[float_at("Glowing torus", "torus", 0, 0.05, 0, 1.6, emitter((1.0, 0.62, 0.3), 3), tilt=(65, 0), rotation=20),
                       obj("Tall block", "box", -0.45, -0.35, 0.8, matte((0.72, 0.7, 0.66), 0.6), stretch=2.0, rotation=18),
                       obj("Short block", "box", 0.45, 0.25, 0.7, matte((0.72, 0.7, 0.66), 0.6), stretch=0.7, rotation=-14)]))

scene("cornell-water-tank", "Cornell water tank",
      "A tank of blue-tinted water fills the lower half of the box: Beer–Lambert absorption over long paths.",
      ["cornell", "glass", "absorption"],
      cornell(bounces=16,
              objects=[obj("Water", "box", 0, 0, 2.78, M((0.55, 0.82, 0.95), 0.0, trans=1, ior=1.33), stretch=0.42),
                       obj("Gold sphere", "sphere", -0.35, 0.1, 0.9, GOLD),
                       obj("Marble pawn", "pawn", 0.4, -0.25, 1.3, MARBLE, stretch=1.3)]))

# ================================================================ validation and lighting studies
scene("white-furnace", "White furnace",
      "Uniform white surroundings, white floor and spheres from mirror to matte. Anything darker than the background lost energy.",
      ["validation", "energy"],
      studio(light_on=False, lights=[], sky=0.0, background=[1.0, 1.0, 1.0], base=[1.0, 1.0, 1.0],
             floor=M((1, 1, 1), 1.0, refl=0.0), floor_extent=40.0, tonemap=0, exposure=0.7, bounces=32,
             objects=[obj(f"Metal roughness {r}", "sphere", -3 + i * 1.2, 0, 1.4, M((1, 1, 1), r, metal=1)) for i, r in enumerate([0.0, 0.25, 0.5, 0.75, 1.0])]
             + [obj("White diffuse", "sphere", -1.2, -1.6, 1.4, M((1, 1, 1), 1.0, refl=0.0)),
                obj("Clear glass", "sphere", 1.2, -1.6, 1.4, M((1, 1, 1), 0.0, trans=1, ior=1.5))],
             **camera(0, 18, 9, (0, -0.4, -0.5))))


def veach():
    eye = (0.0, -0.0 + 8.5 * math.sin(math.radians(12)), -0.8 + 8.5 * math.cos(math.radians(12)))
    lamp = (0.0, 1.6, -3.2)
    plates = []
    for i, (rough, z, y) in enumerate([(0.005, 1.4, -0.85), (0.02, 0.6, -0.6), (0.05, -0.2, -0.3), (0.12, -1.0, 0.05)]):
        # The plate normal halves the angle between the camera and the lights, so each plate
        # shows the row of lights. Pitch tilts the up-facing normal toward +z.
        to_eye = [e - q for e, q in zip(eye, (0, y, z))]
        to_lamp = [l - q for l, q in zip(lamp, (0, y, z))]
        norm = lambda v: [c / math.sqrt(sum(d * d for d in v)) for c in v]
        n = [a + b for a, b in zip(norm(to_eye), norm(to_lamp))]
        pitch = math.degrees(math.atan2(n[2], n[1]))
        plates.append(float_at(f"Plate roughness {rough}", "box", 0, y, z, 1.0, M((0.6, 0.6, 0.6), rough, metal=1),
                               stretch=0.05, scale=(5.6, 1.05), tilt=(pitch, 0)))
    spheres = []
    for i, (r, color) in enumerate([(0.04, (1, 0.85, 0.5)), (0.11, (0.5, 1, 0.55)), (0.28, (0.5, 0.7, 1)), (0.6, (1, 0.5, 0.45))]):
        emit = min(20.0, 20.0 * (0.04 / r) ** 2)
        spheres.append(float_at(f"Light radius {r}", "sphere", -2.4 + i * 1.6, 1.6, -3.2, r / 0.35, emitter(color, emit)))
    return plates + spheres


scene("veach-mis", "Veach MIS plates",
      "Four plates from sharp to rough reflect four lights from tiny to large at equal power: NEE wins on rough plates and small lights, BSDF sampling on sharp plates and large lights; MIS combines both.",
      ["validation", "mis", "veach"],
      dark_stage(base=[0.04, 0.04, 0.045], floor_extent=8.0, exposure=2.5, objects=veach(),
                 **camera(0, 12, 8.5, (0, 0.0, -0.8), 34)))


def ajar_room():
    wall = matte((0.75, 0.73, 0.7), 0.85)
    o = []
    # Interior 6 x 6, height 3 (floor at -1, ceiling at 2). Walls 0.2 thick.
    o.append(float_at("Ceiling", "box", 0, 2.1, 0, 1, wall, stretch=0.29, scale=(9.2, 9.2)))
    o.append(float_at("Left wall", "box", -3.1, 0.5, 0, 1, wall, stretch=4.3, scale=(0.29, 9.2)))
    o.append(float_at("Right wall", "box", 3.1, 0.5, 0, 1, wall, stretch=4.3, scale=(0.29, 9.2)))
    o.append(float_at("Front wall", "box", 0, 0.5, 3.1, 1, wall, stretch=4.3, scale=(9.2, 0.29)))
    # Back wall with a door opening from x = 0.6 to 1.6, up to y = 1.2.
    o.append(float_at("Back wall left", "box", -1.25, 0.5, -3.1, 1, wall, stretch=4.3, scale=(5.36, 0.29)))
    o.append(float_at("Back wall right", "box", 2.35, 0.5, -3.1, 1, wall, stretch=4.3, scale=(2.14, 0.29)))
    o.append(float_at("Lintel", "box", 1.1, 1.6, -3.1, 1, wall, stretch=1.15, scale=(1.43, 0.29)))
    # The door, open by a few degrees, hinged at x = 0.6.
    angle = 12
    w = 1.0
    cx = 0.6 + math.cos(math.radians(angle)) * w / 2
    cz = -3.1 + math.sin(math.radians(angle)) * w / 2
    o.append(float_at("Door", "box", cx, 0.1, cz, 1, matte((0.45, 0.3, 0.2), 0.6), stretch=3.14, scale=(1.43, 0.11), rotation=-angle))
    o.append(obj("Table", "box", -0.8, -0.6, 1, matte((0.5, 0.35, 0.22), 0.5), stretch=1.1, scale=(3.4, 2.0)))
    o.append(float_at("Teapot", "teapot", -0.8, -0.05, -0.6, 0.9, PORCELAIN))
    o.append(obj("Glass", "cylinder", -0.1, -0.3, 0.35, GLASS, stretch=1.5, elevation=0.77))
    return o


def penumbra():
    o, l = [], []
    for i, size in enumerate([0.1, 0.3, 0.6, 1.2, 2.0]):
        x = -3.2 + i * 1.6
        o.append(obj(f"Post {i + 1}", "cylinder", x, 0, 3.0, matte((0.8, 0.78, 0.74), 0.6), scale=(0.07, 0.07)))
        l.append(light(f"Light {size}", (x, 3.2, -1.4), (size, size), (1, 0.97, 0.92), min(2000.0, 20.0 * (0.6 / size) ** 2), target=(x, -1, 0.8)))
    return o, l


_o, _l = penumbra()
scene("penumbra-ladder", "Penumbra ladder",
      "Five posts, each under its own light from 0.1 to 2 units wide at equal power: shadow softness grows with light size.",
      ["validation", "shadows"],
      studio(light_on=False, sky=0.02, lights=_l, objects=_o, base=[0.75, 0.74, 0.72],
             floor=M((0.7, 0.7, 0.7), 0.9, refl=0.0), **camera(0, 38, 9, (0, -0.6, 0.6))))


def temperature_row():
    o, l = [], []
    temps = [2000, 2700, 3500, 4500, 5600, 7500, 10000]
    for i, k in enumerate(temps):
        x = -4.2 + i * 1.4
        o.append(obj(f"Pawn {k} K", "pawn", x, 0, 1.3, matte((0.85, 0.85, 0.85), 0.5), stretch=1.2))
        l.append(light(f"{k} K", (x, 2.2, 1.0), (0.6, 0.6), kelvin(k), 25, target=(x, -0.6, 0)))
        if i < len(temps) - 1:
            o.append(obj(f"Divider {i + 1}", "box", x + 0.7, -0.4, 1, matte((0.03, 0.03, 0.03), 0.9), stretch=2.2, scale=(0.05, 3.0)))
    return o, l


_o, _l = temperature_row()
scene("light-temperature-row", "Light temperature row",
      "Seven white pawns, each lit by one light from 2000 K to 10000 K. Kelvin colours side by side.",
      ["lighting", "color"],
      studio(light_on=False, sky=0.0, lights=_l, objects=_o, base=[0.6, 0.6, 0.6], **camera(0, 14, 9.5, (0, -0.4, 0))))

random.seed(12)
scene("overcast-still-life", "Overcast still life",
      "No lights at all: a bright overcast sky only. Shows the cost of sampling an environment without importance sampling.",
      ["lighting", "sky"],
      studio(light_on=False, lights=[], sky=4.0, background=[0.0, 0.0, 0.0], exposure=0.6,
             objects=[obj("Teapot", "teapot", 0, 0, 1.6, PORCELAIN, rotation=30),
                      obj("Copper sphere", "sphere", -1.6, 0.5, 1.2, COPPER),
                      obj("Jade knot", "knot", 1.7, 0.4, 1.4, JADE, elevation=0.15),
                      obj("Rubber cylinder", "cylinder", -0.6, -1.4, 0.8, RUBBER, stretch=1.6),
                      obj("Marble cube", "box", 1.0, -1.5, 0.9, MARBLE, rotation=25)],
             **camera(20, 20, 7, (0, -0.4, 0))))


def tiles():
    o = []
    for i in range(9):
        r = i / 8
        o.append(obj(f"Tile roughness {r:.2f}", "box", -3.2 + i * 0.8, 1.0, 1, M((0.1, 0.1, 0.11), r, refl=0.5),
                     stretch=0.03, scale=(1.1, 5.0)))
    o.append(float_at("Glow bar", "box", 0, 0.2, -2.2, 1, emitter((1.0, 0.75, 0.45), 6), stretch=0.12, scale=(11.0, 0.12)))
    o += [obj("Red sphere", "sphere", -2.0, -1.4, 1.1, lacquer((0.7, 0.05, 0.03))),
          obj("Teal cone", "cone", 0.0, -1.5, 1.1, lacquer((0.05, 0.5, 0.5))),
          obj("Gold torus", "torus", 2.0, -1.4, 1.3, GOLD, tilt=(90, 0))]
    return o


# ================================================================ material showcases
SHAPES = ["box", "sphere", "cylinder", "cone", "torus", "knot", "teapot", "bunny", "octahedron", "pawn", "ring"]


def matrix():
    o = []
    for row, m in enumerate(LIBRARY):
        for col, shape in enumerate(SHAPES):
            o.append(obj(f"{shape} {row + 1}", shape, -4.5 + col * 0.9, -5.0 + row * 0.9, 0.95, m,
                         tilt=(70, 0) if shape in ("torus", "ring") else (0, 0), elevation=0.05 if shape in ("torus", "ring", "knot") else 0))
    return o


scene("shape-material-matrix", "Shape × material matrix",
      "All 11 shapes in all 12 library materials: 132 objects, the whole catalogue at a glance.",
      ["material", "catalogue"],
      studio(light_size=8.0, light_position=[0.0, 7.0, 3.0], light_intensity=6.0, objects=matrix(),
             **camera(0, 48, 14, (0, -1, 0), 40)))


def gems():
    o, l = [], []
    gems = [("Diamond", (0.98, 0.99, 1.0), 2.42), ("Sapphire", (0.25, 0.4, 0.95), 1.77),
            ("Emerald", (0.2, 0.85, 0.4), 1.57), ("Ruby", (0.95, 0.15, 0.2), 1.77), ("Topaz", (1.0, 0.7, 0.25), 1.62)]
    for i, (name, color, ior) in enumerate(gems):
        x = -2.4 + i * 1.2
        o.append(obj(f"{name} plinth", "cylinder", x, 0, 1.0, matte((0.02, 0.02, 0.025), 0.4), stretch=1.2, scale=(0.9, 0.9)))
        o.append(float_at(name, "octahedron", x, 0.2, 0, 1.0, M(color, 0.0, trans=1, ior=ior), stretch=1.25, rotation=20 + i * 15))
        l.append(light(f"Spot {name}", (x + 0.6, 3.0, 1.5), (0.3, 0.3), (1, 1, 1), 500, target=(x, 0.2, 0)))
    l.append(light("Soft fill", (0, 4.0, 3.0), (5, 3), kelvin(6500), 1.2, target=(0, -0.5, 0)))
    return o, l


_o, _l = gems()
scene("gem-display", "Gem display",
      "Diamond, sapphire, emerald, ruby and topaz cut as octahedra on black plinths under tiny spots: strong internal reflection and sparkle.",
      ["material", "glass", "caustics"],
      dark_stage(lights=_l, objects=_o, bounces=24, base=[0.02, 0.02, 0.025], exposure=1.4, floor=M((0.02, 0.02, 0.025), 0.15, refl=0.3),
                 **camera(0, 16, 6.2, (0, -0.1, 0))))


def jewellery():
    velvet = matte((0.1, 0.008, 0.02), 1.0)
    o = [obj("Velvet tray", "box", 0, 0, 1, velvet, stretch=0.2, scale=(8.0, 5.5))]
    top = -1 + 0.07
    o.append(float_at("Gold ring", "ring", -1.0, top + 0.31, 0.2, 0.9, GOLD, tilt=(90, 0), rotation=30))
    o.append(float_at("Silver ring", "ring", -0.3, top + 0.26, -0.4, 0.75, SILVER, tilt=(90, 0), rotation=-20))
    o.append(float_at("Gold band", "torus", 0.6, top + 0.06, 0.3, 0.55, GOLD, stretch=0.6))
    o.append(float_at("Copper bangle", "torus", -1.2, top + 0.06, -0.8, 0.9, COPPER, stretch=0.5))
    pearl = M((0.95, 0.92, 0.88), 0.08, refl=0.2)
    for i in range(9):
        a = i / 9 * math.tau
        o.append(obj(f"Pearl {i + 1}", "sphere", 1.0 + 0.45 * math.cos(a), -0.5 + 0.3 * math.sin(a), 0.32, pearl, elevation=0.14))
    o.append(float_at("Diamond", "octahedron", 0.1, top + 0.18, 0.75, 0.45, M((1, 1, 1), 0.0, trans=1, ior=2.42), stretch=1.2))
    return o


random.seed(7)
scene("ice-cubes", "Ice cubes",
      "Frosted glass cubes tumbled on dark slate: rough transmission.",
      ["material", "glass", "roughness"],
      studio(base=[0.06, 0.065, 0.07], floor=M((0.06, 0.06, 0.07), 0.35, refl=0.1), sky=0.02, bounces=16,
             light_position=[-2.0, 3.0, 1.0], light_size=1.5, light_intensity=24.0, light_temperature=6500.0,
             lights=[light("Warm back", (1.5, 1.2, -2.5), (1.2, 1.2), kelvin(2800), 30, target=(0, -0.8, 0))],
             objects=[obj(f"Ice {i + 1}", "box", random.uniform(-1.4, 1.4), random.uniform(-1.0, 1.0), random.uniform(0.7, 1.0),
                          M((0.85, 0.95, 1.0), random.uniform(0.12, 0.3), trans=1, ior=1.31),
                          rotation=random.uniform(-180, 180), tilt=(random.uniform(-25, 25), random.uniform(-25, 25)))
                      for i in range(9)],
             **camera(10, 30, 5.0, (0, -0.7, 0))))


def glass_stack():
    o = []
    y = -1.0 + GAP
    tints = [(0.85, 0.95, 1.0), (1.0, 0.9, 0.8), (0.9, 1.0, 0.9), (1.0, 0.92, 0.97)]
    for i in range(6):
        h = 0.35 * 0.9 * 0.5
        o.append(float_at(f"Glass slab {i + 1}", "box", 0.1 * math.sin(i), y + h, 0.1 * math.cos(i), 0.9 - i * 0.07,
                          M(tints[i % 4], 0.0, trans=1, ior=1.5), stretch=0.5, rotation=i * 17))
        y += 2 * h * (0.9 - i * 0.07) / 0.9 + GAP
    o.append(float_at("Glass cone", "cone", 0, y + 0.3, 0, 0.9, GLASS))
    o.append(obj("Glass torus", "torus", -1.5, 0.4, 1.4, GLASS, tilt=(80, 10)))
    o.append(obj("Glass knot", "knot", 1.6, -0.2, 1.3, M((0.9, 0.95, 1.0), 0.0, trans=1, ior=1.5), elevation=0.1))
    o.append(obj("Glass sphere", "sphere", 0.9, 1.1, 0.9, GLASS))
    return o


scene("glass-stack", "Glass stack",
      "Stacked glass slabs, a cone, a torus and a knot at 32 bounces: long chains of refraction.",
      ["material", "glass", "bounces"],
      studio(objects=glass_stack(), bounces=32, sky=0.15, base=[0.75, 0.72, 0.68], light_size=2.0, light_intensity=18.0,
             **camera(15, 18, 6.5, (0, -0.2, 0))))


def bunny_museum():
    o, l = [], []
    mats = [("Glass", GLASS), ("Gold", GOLD), ("Jade", JADE), ("Marble", MARBLE), ("Chrome", CHROME), ("Red rubber", RUBBER)]
    for i, (name, m) in enumerate(mats):
        a = math.radians(-50 + i * 20)
        x, z = 4.0 * math.sin(a), -4.0 * math.cos(a) + 2.5
        o.append(obj(f"Plinth {i + 1}", "box", x, z, 1.1, matte((0.82, 0.8, 0.76), 0.5), stretch=2.4, rotation=-math.degrees(a)))
        o.append(obj(f"{name} bunny", "bunny", x, z, 1.0, m, elevation=0.35 * 1.1 * 2.4 * 2, rotation=-math.degrees(a) + 20))
        l.append(light(f"Spot {name}", (x * 0.85, 3.6, z * 0.85 + 0.6), (0.7, 0.7), kelvin(4200), 36, target=(x, 1.0, z)))
    return o, l


def tea_set():
    o = [obj("Tray", "box", 0, 0, 1, lacquer((0.05, 0.018, 0.012), 0.15), stretch=0.12, scale=(9.0, 6.0))]
    top = -1 + 0.084
    china = M((0.93, 0.92, 0.88), 0.15, refl=0.1)
    o.append(obj("Teapot", "teapot", -0.7, -0.2, 1.4, china, rotation=-25, elevation=0.084))
    for i, (x, z) in enumerate([(0.8, 0.2), (1.1, -0.9), (-0.4, 1.0)]):
        o.append(float_at(f"Saucer {i + 1}", "cylinder", x, top + 0.02, z, 1.0, china, stretch=0.06, scale=(0.75, 0.75)))
        o.append(float_at(f"Cup {i + 1}", "cylinder", x, top + 0.04 + 0.21, z, 0.6, china, stretch=1.0))
        o.append(float_at(f"Tea {i + 1}", "cylinder", x, top + 0.04 + 0.37, z, 0.55, M((0.35, 0.12, 0.03), 0.05, refl=0.3), stretch=0.02))
        o.append(float_at(f"Handle {i + 1}", "ring", x + 0.24, top + 0.27, z, 0.38, china, tilt=(90, 0), stretch=3.0))
    o.append(float_at("Sugar bowl", "sphere", 0.1, top + 0.15, -0.9, 0.9, china, stretch=0.6))
    return o


scene("tea-set", "Tea set",
      "Teapot, cups and saucers on a lacquered tray: a domestic still life.",
      ["still life", "mesh"],
      studio(objects=tea_set(), base=[0.32, 0.22, 0.14], floor=M((0.3, 0.2, 0.13), 0.45, refl=0.06),
             light_position=[-2.0, 3.5, 2.0], light_size=2.0, light_intensity=14.0, light_temperature=4200.0,
             **camera(25, 30, 4.4, (0, -0.7, 0), 40)))

# ================================================================ architecture and environments
def stonehenge():
    stone = M((0.48, 0.46, 0.42), 0.9, refl=0.02)
    o = []
    n = 15
    for i in range(n):
        a = i / n * math.tau
        x, z = 4.0 * math.cos(a), 4.0 * math.sin(a)
        o.append(obj(f"Upright {i + 1}", "box", x, z, 1, stone, stretch=3.2, scale=(1.2, 0.65), rotation=-math.degrees(a) + 90 + random.uniform(-4, 4)))
    for i in range(0, n, 2):
        a = (i + 0.5) / n * math.tau
        x, z = 4.0 * math.cos(a), 4.0 * math.sin(a)
        o.append(float_at(f"Lintel {i // 2 + 1}", "box", x, -1 + 2 * 0.35 * 3.2 + 0.18, z, 1, stone, stretch=0.5, scale=(3.0, 0.65),
                          rotation=-math.degrees(a) + 90))
    for i in range(5):
        a = math.radians(200 + i * 30)
        o.append(obj(f"Trilithon {i + 1}", "box", 1.8 * math.cos(a), 1.8 * math.sin(a), 1, stone, stretch=4.5, scale=(1.4, 0.8),
                     rotation=-math.degrees(a) + 90))
    o.append(obj("Altar stone", "box", 0.3, -0.2, 1, stone, stretch=0.35, scale=(3.0, 1.0), rotation=10))
    return o


random.seed(3)
scene("stonehenge-dusk", "Stonehenge at dusk",
      "A ring of stones and lintels under a 2800 K sun on the horizon: long shadows and warm–cool contrast.",
      ["architecture", "sunset", "open"],
      studio(light_on=False, sky=0.2, background=[0.16, 0.09, 0.08], base=[0.22, 0.26, 0.14],
             floor=M((0.3, 0.32, 0.18), 0.95, refl=0.0), floor_extent=40.0,
             lights=[light("Low sun", (-38, 3.0, -12), (6, 6), kelvin(2800), 900, target=(0, -1, 0)),
                     light("Blue sky fill", (6, 20, 10), (14, 14), (0.4, 0.55, 1.0), 0.6, target=(0, -1, 0))],
             objects=stonehenge(), **camera(72, 8, 13, (0, 0.2, 0), 45)))


def torii():
    red = lacquer((0.75, 0.08, 0.03), 0.35)
    black = matte((0.03, 0.03, 0.03), 0.6)
    o = []
    for i in range(12):
        z = 2 - i * 2.2
        for side in (-1, 1):
            o.append(obj(f"Gate {i + 1} pillar", "cylinder", side * 1.3, z, 3.4, red, scale=(0.065, 0.065)))
        o.append(float_at(f"Gate {i + 1} beam", "box", 0, 1.45, z, 1, black, stretch=0.35, scale=(9.0, 0.45)))
        o.append(float_at(f"Gate {i + 1} tie", "box", 0, 0.95, z, 1, red, stretch=0.22, scale=(7.6, 0.3)))
    return o


def staircase():
    o = [obj("Column", "cylinder", 0, 0, 6.0, MARBLE, scale=(0.1, 0.1), stretch=1.0)]
    steps = 48
    for i in range(steps):
        a = i / steps * 1.5 * math.tau
        y = -1 + 0.09 * i + 0.04
        r = 1.05
        o.append(float_at(f"Step {i + 1}", "box", r * math.cos(a), y, r * math.sin(a), 1, lacquer((0.5, 0.33, 0.2), 0.4),
                          stretch=0.11, scale=(2.6, 0.95), rotation=-math.degrees(a)))
    return o


scene("spiral-staircase", "Spiral staircase",
      "48 wooden steps winding around a marble column, lit from the top: rotation and self-shadowing.",
      ["architecture", "shadows"],
      studio(light_on=False, sky=0.04, objects=staircase(), base=[0.3, 0.3, 0.32],
             lights=[light("Skylight", (0, 6.5, 0), (2.5, 2.5), kelvin(6000), 40)],
             **camera(30, 18, 9, (0, 1.5, 0), 45)))


def colonnade():
    stone = M((0.7, 0.66, 0.6), 0.7, refl=0.02)
    o = []
    for i in range(7):
        z = 3 - i * 2.0
        for side in (-1, 1):
            o.append(obj(f"Column {i + 1}{'LR'[side > 0]}", "cylinder", side * 2.2, z, 4.0, stone, scale=(0.1, 0.1)))
            o.append(float_at(f"Capital {i + 1}{'LR'[side > 0]}", "box", side * 2.2, 1.85, z, 1, stone, stretch=0.3, scale=(1.4, 1.4)))
        if i % 2 == 0:
            for side in (-1, 1):
                o.append(obj(f"Brazier stand {i + 1}{'LR'[side > 0]}", "cylinder", side * 1.3, z - 1.0, 1.0, M((0.15, 0.12, 0.1), 0.4, metal=1), stretch=1.2, scale=(0.4, 0.4)))
                o.append(float_at(f"Fire {i + 1}{'LR'[side > 0]}", "sphere", side * 1.3, -0.05, z - 1.0, 0.5, emitter((1.0, 0.45, 0.12), 14)))
    o.append(float_at("Architrave L", "box", -2.2, 2.15, -3, 1, stone, stretch=0.4, scale=(1.4, 40)))
    o.append(float_at("Architrave R", "box", 2.2, 2.15, -3, 1, stone, stretch=0.4, scale=(1.4, 40)))
    return o


scene("colonnade-night", "Colonnade at night",
      "Two rows of columns lit only by glowing braziers: many emissive meshes driving light sampling.",
      ["architecture", "night", "emissive"],
      studio(light_on=False, lights=[], sky=0.015, background=[0.005, 0.008, 0.02], base=[0.3, 0.28, 0.25],
             floor=M((0.4, 0.37, 0.33), 0.6, refl=0.05), objects=colonnade(), **camera(0, 6, 7, (0, 0.1, -1.5), 55)))


def skyline():
    random.seed(42)
    o = []
    for gx in range(-6, 7):
        for gz in range(-6, 6):
            if len(o) >= 150 or random.random() < 0.08:
                continue
            h = random.uniform(1.5, 9.0) * (1.4 if abs(gx) + abs(gz) < 4 else 1.0)
            w = random.uniform(0.7, 1.05)
            lit = random.random() < 0.45
            warm = random.random() < 0.7
            window = (1.0, 0.72, 0.42) if warm else (0.6, 0.8, 1.0)
            m = M(window if lit else (0.06, 0.065, 0.08), 0.4, refl=0.05, emit=random.uniform(0.04, 0.2) if lit else 0.0, pattern="stripes",
                  second=(0.01, 0.011, 0.015), pscale=random.choice([16.0, 20.0, 24.0]))
            o.append(obj(f"Tower {len(o) + 1}", "box", gx * 1.25 + random.uniform(-0.1, 0.1), gz * 1.25 + random.uniform(-0.1, 0.1), 1, m,
                         stretch=h, scale=(w * 1.4, w * 1.4)))
    return o


def zen():
    random.seed(9)
    rock = M((0.32, 0.31, 0.3), 0.85, refl=0.02)
    o = []
    for i, (x, z, s) in enumerate([(-1.6, -0.6, 1.4), (-1.0, -0.2, 0.8), (1.4, 0.8, 1.1), (0.4, -1.6, 0.9), (1.9, -0.9, 0.6)]):
        o.append(obj(f"Rock {i + 1}", "octahedron", x, z, s, rock, stretch=0.65, rotation=random.uniform(0, 90), tilt=(random.uniform(-15, 15), 0)))
    o.append(obj("Moss", "sphere", -1.3, 0.3, 1.2, matte((0.18, 0.3, 0.08), 1.0), stretch=0.3))
    return o


def mirror_maze():
    random.seed(21)
    o = []
    for gx in range(-4, 5):
        for gz in range(-4, 5):
            if abs(gx) <= 1 and abs(gz) <= 1:
                continue
            if random.random() < 0.55:
                horizontal = random.random() < 0.5
                o.append(obj(f"Mirror {len(o) + 1}", "box", gx * 1.2 + (0 if horizontal else 0.6), gz * 1.2 + (0.6 if horizontal else 0), 1, CHROME,
                             stretch=3.0, scale=(3.4 if horizontal else 0.06, 0.06 if horizontal else 3.4)))
    o.append(float_at("Glowing sphere", "sphere", 0, 0.0, 0, 1.4, emitter((0.6, 0.85, 1.0), 6)))
    return o


def stained_glass():
    random.seed(11)
    stone = M((0.62, 0.58, 0.52), 0.8, refl=0.02)
    colors = [(0.85, 0.04, 0.03), (0.05, 0.15, 0.9), (0.9, 0.7, 0.04), (0.04, 0.6, 0.12), (0.45, 0.05, 0.7), (0.9, 0.35, 0.02)]
    o = []
    for i in range(5):
        x = -4 + i * 2
        o.append(obj(f"Pier {i + 1}", "box", x, -2, 1, stone, stretch=6, scale=(0.9, 0.9)))
    for i in range(4):
        cx = -3 + i * 2
        for r in range(4):
            for c in range(3):
                o.append(float_at(f"Pane {i + 1}.{r + 1}.{c + 1}", "box", cx - 0.47 + c * 0.47, -0.15 + r * 0.62, -2 + (c - 1) * 0.006, 1,
                                  M(random.choice(colors), 0.08, trans=1, ior=1.5), stretch=0.86, scale=(1.3, 0.45)))
        o.append(float_at(f"Sill {i + 1}", "box", cx, -0.5, -2, 1, stone, stretch=1.0, scale=(4.6, 0.6)))
        o.append(float_at(f"Head {i + 1}", "box", cx, 2.55, -2, 1, stone, stretch=1.2, scale=(4.6, 0.6)))
    o.append(float_at("Low wall", "box", 0, -0.85, -2, 1, stone, stretch=0.6, scale=(26, 0.6)))
    for i in range(3):
        o.append(obj(f"Bench {i + 1}", "box", -2.5 + i * 2.5, 1.5, 1, matte((0.35, 0.22, 0.13), 0.6), stretch=0.6, scale=(4.0, 0.9)))
    return o


scene("stained-glass-hall", "Stained-glass hall",
      "A wall of coloured glass panes with the sun behind. Shadow rays pass glass untinted here, so the colours show in the glass, not on the floor: an engine limit made visible.",
      ["architecture", "glass", "limits"],
      studio(light_on=False, sky=0.05, glass_shadow=False, base=[0.5, 0.47, 0.42], floor=M((0.5, 0.47, 0.42), 0.5, refl=0.06),
             lights=[light("Sun", (0, 1.5, -12), (14, 7), kelvin(5600), 3, target=(0, 1.0, 0), visible=True)],
             objects=stained_glass(), bounces=12, **camera(0, 10, 8, (0, 0.6, -0.5), 50)))

# ================================================================ playful still lifes
def newtons_cradle():
    frame = M((0.15, 0.15, 0.17), 0.3, metal=1)
    wood = lacquer((0.25, 0.12, 0.06), 0.25)
    o = [obj("Base", "box", 0, 0, 1, wood, stretch=0.25, scale=(6.0, 3.0))]
    top = 1.0
    for side in (-1, 1):
        for end in (-1, 1):
            o.append(float_at("Post", "cylinder", end * 1.0, 0.08, side * 0.45, 2.6, frame, scale=(0.03, 0.03), stretch=1.0))
        o.append(float_at("Top bar", "cylinder", 0, top, side * 0.45, 1.0, frame, scale=(0.1, 0.1), stretch=5.8, tilt=(0, 90)))
    r = 0.17
    for i in range(5):
        x = -0.68 + i * 0.34
        swing = -35 if i == 0 else 0
        length = 1.2
        ax = x - length * math.sin(math.radians(-swing)) if swing else x
        ay = top - length * math.cos(math.radians(swing))
        o.append(float_at(f"Ball {i + 1}", "sphere", ax, ay, 0, r / 0.35, CHROME))
        for side in (-1, 1):
            mx, my, mz = (x + ax) / 2, (top + ay) / 2, side * 0.225
            dz = side * 0.45
            tilt_x = math.degrees(math.atan2(dz, top - ay))
            o.append(float_at(f"String {i + 1}", "cylinder", mx, my, mz, 1.0, matte((0.9, 0.9, 0.9)), scale=(0.008, 0.008),
                              stretch=math.hypot(top - ay, 0.45) / 0.7, tilt=(tilt_x, swing)))
    return o


scene("newtons-cradle", "Newton's cradle",
      "Five chrome balls on strings, one pulled back: mirror spheres reflecting each other.",
      ["still life", "mirror"],
      studio(objects=newtons_cradle(), base=[0.55, 0.55, 0.57], light_size=2.0, light_intensity=16.0,
             **camera(20, 14, 4.5, (0, -0.2, 0), 40)))


def dominoes():
    o = []
    theta, a, b = 2.5, 0.3, 0.17
    colors = [(0.92, 0.9, 0.85), (0.08, 0.08, 0.09)]
    while len(o) < 200:
        r = a + b * theta
        x, z = r * math.cos(theta), r * math.sin(theta)
        tangent = math.degrees(math.atan2(math.cos(theta) * r + b * math.sin(theta), -math.sin(theta) * r + b * math.cos(theta)))
        o.append(obj(f"Domino {len(o) + 1}", "box", x, z, 0.6, lacquer(colors[len(o) % 2], 0.25), stretch=1.6, scale=(0.22, 0.9),
                     rotation=-tangent))
        theta += 0.32 / r
    return o


scene("domino-spiral", "Domino spiral",
      "200 dominoes along an Archimedean spiral: many copies of a simple shape near the object limit.",
      ["still life", "pattern", "many objects"],
      studio(objects=dominoes(), base=[0.5, 0.42, 0.34], floor=M((0.5, 0.42, 0.34), 0.35, refl=0.08),
             light_size=4.0, light_intensity=10.0, **camera(10, 50, 11, (0, -1, 0), 45)))


def orrery():
    o = [float_at("Sun", "sphere", 0, 0.3, 0, 1.8, emitter((1.0, 0.8, 0.45), 8))]
    planets = [(1.0, 0.12, (0.6, 0.55, 0.5)), (1.5, 0.17, (0.9, 0.7, 0.35)), (2.1, 0.18, (0.15, 0.35, 0.8)),
               (2.7, 0.14, (0.75, 0.3, 0.15)), (3.6, 0.38, (0.8, 0.65, 0.45)), (4.6, 0.32, (0.85, 0.75, 0.5)),
               (5.4, 0.24, (0.45, 0.75, 0.85)), (6.1, 0.23, (0.25, 0.4, 0.9))]
    for i, (dist, r, color) in enumerate(planets):
        a = i * 2.4 + 0.5
        x, z = dist * math.cos(a), dist * math.sin(a)
        o.append(float_at(f"Planet {i + 1}", "sphere", x, 0.3, z, r / 0.35, lacquer(color, 0.3)))
        o.append(float_at(f"Orbit {i + 1}", "ring", 0, 0.3, 0, dist / (0.35 * 0.92), M((0.6, 0.5, 0.3), 0.3, metal=1), stretch=0.15))
        if i == 5:
            o.append(float_at("Saturn's rings", "ring", x, 0.3, z, 0.65 / 0.35, M((0.85, 0.78, 0.6), 0.5), stretch=0.5, tilt=(25, 0), scale=(1, 1)))
    o.append(obj("Stand", "cylinder", 0, 0, 0.5, M((0.6, 0.5, 0.3), 0.3, metal=1), stretch=1.0, scale=(0.3, 0.3)))
    return o


def ring_point(radius, angle, pitch, roll, center):
    # Ring rotation is YXZ with yaw 0: Rx(pitch) * Rz(roll) applied to a point of the flat ring.
    x, y, z = radius * math.cos(angle), 0.0, radius * math.sin(angle)
    c, s = math.cos(math.radians(roll)), math.sin(math.radians(roll))
    x, y = x * c - y * s, x * s + y * c
    c, s = math.cos(math.radians(pitch)), math.sin(math.radians(pitch))
    y, z = y * c - z * s, y * s + z * c
    return center[0] + x, center[1] + y, center[2] + z


def atom():
    o = []
    center = (0.0, 1.5, 0.0)
    random.seed(1)
    for i in range(14):
        v = [random.gauss(0, 1) for _ in range(3)]
        n = math.sqrt(sum(c * c for c in v)) or 1
        p = [c / n * 0.28 * random.random() ** 0.3 for c in v]
        o.append(float_at(f"Nucleon {i + 1}", "sphere", p[0], center[1] + p[1], p[2], 0.45,
                          lacquer((0.85, 0.15, 0.1) if i % 2 else (0.15, 0.3, 0.9), 0.2)))
    for i, (tx, tz) in enumerate([(70, 0), (70, 60), (70, -60)]):
        o.append(float_at(f"Orbit {i + 1}", "ring", 0, center[1], 0, 1.2 / (0.35 * 0.92), emitter((0.15, 0.45, 1.0), 0.7), stretch=0.6, tilt=(tx, tz)))
        x, y, z = ring_point(1.2, 0.8 + i * 2.1, tx, tz, center)
        o.append(float_at(f"Electron {i + 1}", "sphere", x, y, z, 0.3, emitter((0.8, 0.95, 1.0), 4)))
    return o


scene("atom", "Atom",
      "A nucleus of red and blue spheres inside three tilted glowing orbits.",
      ["still life", "emissive"],
      dark_stage(objects=atom(), base=[0.05, 0.05, 0.06], floor=M((0.05, 0.05, 0.06), 0.2, refl=0.2),
                 **camera(10, 10, 6.5, (0, 1.3, 0), 40)))


def armillary():
    o = [obj("Base", "cylinder", 0, 0, 1.6, GOLD, stretch=0.15),
         obj("Stem", "cylinder", 0, 0, 1.4, GOLD, scale=(0.08, 0.08), elevation=0.08),
         float_at("Glass globe", "sphere", 0, 1.1, 0, 1.6, GLASS)]
    for i, (tx, tz) in enumerate([(90, 0), (90, 60), (90, 120), (0, 0), (66, 0)]):
        o.append(float_at(f"Ring {i + 1}", "ring", 0, 1.1, 0, 1.3 / (0.35 * 0.92), GOLD, stretch=1.0, tilt=(tx, tz)))
    return o


scene("armillary-sphere", "Armillary sphere",
      "Five gold rings at different tilts around a glass globe.",
      ["still life", "metal", "glass"],
      studio(objects=armillary(), sky=0.05, light_size=2.0, light_intensity=16.0, bounces=16,
             **camera(20, 15, 6.0, (0, 0.3, 0), 40)))


def snowmen():
    snow = M((0.92, 0.94, 0.97), 0.7, refl=0.02)
    o = []
    for k, (x, z, s) in enumerate([(-1.0, 0.0, 1.0), (1.2, -0.6, 0.8)]):
        y = -1.0
        for i, r in enumerate([0.55, 0.4, 0.28]):
            r *= s
            o.append(float_at(f"Snowman {k + 1} ball {i + 1}", "sphere", x, y + r * 0.92, z, r / 0.35, snow))
            y += r * 1.75
        head = y - 0.28 * s * 0.83
        o.append(float_at(f"Snowman {k + 1} nose", "cone", x, head, z + 0.3 * s, 0.15 * s, matte((0.95, 0.4, 0.05), 0.6), stretch=2.4, tilt=(90, 0)))
        for e in (-1, 1):
            o.append(float_at(f"Snowman {k + 1} eye", "sphere", x + e * 0.1 * s, head + 0.08 * s, z + 0.24 * s, 0.1 * s, matte((0.02, 0.02, 0.02), 0.3)))
        o.append(float_at(f"Snowman {k + 1} hat", "cylinder", x, head + 0.4 * s, z, 0.55 * s, matte((0.03, 0.03, 0.03), 0.5), stretch=1.0))
        o.append(float_at(f"Snowman {k + 1} brim", "cylinder", x, head + 0.2 * s, z, 0.85 * s, matte((0.03, 0.03, 0.03), 0.5), stretch=0.08))
    return o


scene("snowmen", "Snowmen",
      "Two snowmen on snow under a cool sky and a warm low sun: subtle white-on-white shading.",
      ["still life", "winter"],
      studio(light_on=False, sky=0.55, background=[0.45, 0.55, 0.75], base=[0.9, 0.92, 0.96], floor=M((0.9, 0.92, 0.96), 0.8, refl=0.02),
             lights=[light("Low sun", (-12, 5, 10), (2.5, 2.5), kelvin(3500), 260, target=(0, -1, 0))],
             objects=snowmen(), exposure=0.75, **camera(15, 10, 6.5, (0, 0.0, 0), 40)))


def pool_table():
    felt = matte((0.03, 0.32, 0.12), 0.95)
    wood = lacquer((0.25, 0.1, 0.04), 0.3)
    top = -1 + 2 * 0.35 * 0.5 * 2.2
    o = [obj("Table", "box", 0, 0, 1, felt, stretch=2.2 * 0.5, scale=(8.0, 4.4))]
    for side in (-1, 1):
        o.append(float_at("Long rail", "box", 0, top + 0.05, side * 1.62, 1, wood, stretch=0.35, scale=(8.6, 0.4)))
        o.append(float_at("Short rail", "box", side * 2.9, top + 0.05, 0, 1, wood, stretch=0.35, scale=(0.4, 9.5)))
    colors = [(0.9, 0.75, 0.1), (0.1, 0.2, 0.75), (0.85, 0.1, 0.08), (0.35, 0.1, 0.55), (0.95, 0.45, 0.05), (0.05, 0.45, 0.2),
              (0.5, 0.08, 0.06), (0.03, 0.03, 0.03)]
    r = 0.09
    k = 0
    for row in range(5):
        for j in range(row + 1):
            c = colors[k % 8] if k != 4 else (0.03, 0.03, 0.03)
            o.append(float_at(f"Ball {k + 1}", "sphere", 1.0 + row * r * 1.75, top + r, (j - row / 2) * r * 2.02, r / 0.35, lacquer(c, 0.06)))
            k += 1
    o.append(float_at("Cue ball", "sphere", -1.3, top + r, 0.2, r / 0.35, lacquer((0.95, 0.94, 0.9), 0.05)))
    o.append(float_at("Cue", "cylinder", -2.3, top + 0.18, 0.5, 1, lacquer((0.6, 0.4, 0.2), 0.3), scale=(0.035, 0.035), stretch=6.0,
                      tilt=(0, 84), rotation=-15))
    o.append(float_at("Lamp shade", "box", 0, 2.3, 0, 1, lacquer((0.05, 0.25, 0.1), 0.3), stretch=0.4, scale=(6.0, 1.8)))
    return o


def track(path, keys):
    """An animation track (track 8): keys are (time, value, interp), interp one of smooth,
    linear, ease, hold (how the segment from that key runs)."""
    return {"path": path, "keys": [{"time": t, "value": round(v, 4), "interp": i} for t, v, i in keys]}


def pool_break():
    """Cook, Porter and Carpenter's 1984 pool balls: the cue strikes, the cue ball runs into the
    rack and the front balls scatter. Indices follow pool_table(): table 0, rails 1-4, rack balls
    5-19 (apex 5, then 6 and 7), cue ball 20, cue 21."""
    objects = pool_table()
    x0 = [o["position"] for o in objects]
    tracks = []

    def move(i, keys):
        for axis in (0, 1):
            tracks.append(track(f"objects/{i}/position/{axis}", [(t, p[axis], interp) for t, p, interp in keys]))

    cue_ball, cue = 20, 21
    move(cue, [(0.0, x0[cue], "ease"), (0.12, (x0[cue][0] + 0.45, x0[cue][1] - 0.12), "linear"),
               (0.16, (x0[cue][0] + 0.5, x0[cue][1] - 0.13), "hold")])
    contact = (0.82, 0.0)
    move(cue_ball, [(0.0, x0[cue_ball], "hold"), (0.12, x0[cue_ball], "linear"), (0.45, contact, "ease"),
                    (1.0, (1.05, -0.12), "hold")])
    move(5, [(0.0, x0[5], "hold"), (0.45, x0[5], "ease"), (1.0, (2.45, 0.55), "hold")])
    move(6, [(0.0, x0[6], "hold"), (0.48, x0[6], "ease"), (1.0, (2.2, -0.85), "hold")])
    move(7, [(0.0, x0[7], "hold"), (0.48, x0[7], "ease"), (1.0, (2.0, 1.2), "hold")])
    return objects, {"length": 1.0, "fps": 24, "shutter": 0.5, "tracks": tracks}


_break_objects, _break_animation = pool_break()
scene("pool-break-1984", "1984 pool break",
      "After Cook, Porter and Carpenter's distributed ray tracing: a break shot keyed on the timeline. "
      "K plays it; a timeline video export renders it with a 180° motion-blur shutter.",
      ["still life", "glossy", "animation", "motion blur"],
      studio(light_on=False, sky=0.02, base=[0.2, 0.16, 0.13], objects=_break_objects,
             lights=[light("Lamp", (0, 2.1, 0), (2.0, 0.55), kelvin(3800), 60)],
             animation=_break_animation, **camera(-12, 38, 5.2, (0.6, -0.2, 0), 45)))


def candles():
    random.seed(4)
    wax = M((0.93, 0.88, 0.76), 0.6, refl=0.02)
    wood = lacquer((0.22, 0.12, 0.06), 0.4)
    o = []
    shelves = [-0.3, 0.9]
    for y in shelves:
        o.append(float_at("Shelf", "box", 0, y, 0, 1, wood, stretch=0.12, scale=(14.0, 1.6)))
    o.append(float_at("Back wall", "box", 0, 1.0, -0.6, 1, matte((0.35, 0.3, 0.26), 0.9), stretch=6.0, scale=(16, 0.2)))
    for k, y in enumerate([-1.0] + [s + 0.042 for s in shelves]):
        n = 7
        for i in range(n):
            x = -2.0 + i * 0.66 + random.uniform(-0.1, 0.1)
            h = random.uniform(0.09, 0.25)  # half height
            size = 0.12 / 0.35 * random.uniform(0.7, 1.0)  # radius about 0.1
            z = random.uniform(-0.25, 0.15)
            o.append(float_at(f"Candle {k + 1}.{i + 1}", "cylinder", x, y + h, z, size, wax, stretch=h / (0.35 * size)))
            o.append(float_at(f"Flame {k + 1}.{i + 1}", "sphere", x, y + 2 * h + 0.06, z, 0.1, emitter((1.0, 0.55, 0.18), 18), stretch=1.8))
    return o


scene("candle-shelf", "Candle shelf",
      "Twenty-one candles on shelves in a dark room: many small lights, a test for light sampling.",
      ["emissive", "night", "many lights"],
      dark_stage(objects=candles(), base=[0.18, 0.14, 0.11], floor=M((0.18, 0.14, 0.11), 0.6, refl=0.04),
                 **camera(0, 8, 5.2, (0, 0.0, 0), 45)))


def bowling():
    lane = M((0.75, 0.52, 0.3), 0.04, refl=0.35)
    pin = M((0.95, 0.94, 0.9), 0.15, refl=0.15)
    o = [obj("Lane", "box", 0, -4, 1, lane, stretch=0.1, scale=(4.0, 30.0))]
    for side in (-1, 1):
        o.append(obj("Gutter", "box", side * 0.95, -4, 1, M((0.2, 0.2, 0.22), 0.3, metal=1), stretch=0.08, scale=(1.3, 30.0)))
    rows = [[0], [-1, 1], [-2, 0, 2], [-3, -1, 1, 3]]
    k = 0
    for r, xs in enumerate(rows):
        for x in xs:
            o.append(obj(f"Pin {k + 1}", "pawn", x * 0.15, -12 - r * 0.26, 0.5, pin, stretch=1.9, elevation=0.07))
            k += 1
    o.append(obj("Ball", "sphere", 0.12, -5.0, 0.62, lacquer((0.1, 0.12, 0.6), 0.08), elevation=0.07))
    return o


def forest():
    random.seed(17)
    o = []
    for i in range(120):
        for _ in range(30):
            x, z = random.uniform(-12, 12), random.uniform(-12, 8)
            if abs(x) < 1.2 and z > -4:
                continue  # leave a path
            if all((x - t[0]) ** 2 + (z - t[1]) ** 2 > 0.7 for t in [(p["position"][0], p["position"][1]) for p in o[::2]]):
                break
        h = random.uniform(0.7, 1.4)
        green = (0.04 + random.uniform(0, 0.04), 0.22 + random.uniform(0, 0.12), 0.06)
        o.append(obj(f"Trunk {i + 1}", "cylinder", x, z, 0.9 * h, matte((0.25, 0.15, 0.08), 0.9), scale=(0.15, 0.15), stretch=0.9))
        o.append(obj(f"Crown {i + 1}", "cone", x, z, 1.4 * h, matte(green, 0.8), stretch=1.9, elevation=0.35 * 0.9 * h * 0.9 * 1.6))
    return o


scene("stylised-forest", "Stylised forest",
      "120 cone-and-cylinder trees around a path, low sun: shadow complexity near the object limit.",
      ["open", "nature", "many objects"],
      studio(light_on=False, sky=0.5, background=[0.5, 0.6, 0.75], base=[0.25, 0.3, 0.12], floor=M((0.25, 0.3, 0.12), 0.95, refl=0.0),
             floor_extent=40.0, objects=forest(),
             lights=[light("Sun", (-30, 18, -20), (3, 3), kelvin(4500), 900, target=(0, -1, 0))],
             **camera(0, 10, 8, (0, 0.2, -2), 55)))


def abacus():
    wood = lacquer((0.3, 0.15, 0.06), 0.35)
    rod = M((0.75, 0.72, 0.65), 0.2, metal=1)
    colors = [(0.8, 0.1, 0.06), (0.95, 0.75, 0.1), (0.1, 0.45, 0.8), (0.1, 0.6, 0.25), (0.95, 0.45, 0.05)]
    o = []
    for side in (-1, 1):
        o.append(obj("Side", "box", side * 1.6, 0, 1, wood, stretch=2.6, scale=(0.35, 0.7)))
    o.append(float_at("Top", "box", 0, 0.82, 0, 1, wood, stretch=0.3, scale=(9.5, 0.7)))
    o.append(float_at("Bottom", "box", 0, -0.9, 0, 1, wood, stretch=0.3, scale=(9.5, 0.7)))
    random.seed(8)
    for r in range(5):
        y = -0.6 + r * 0.32
        o.append(float_at(f"Rod {r + 1}", "cylinder", 0, y, 0, 1, rod, scale=(0.03, 0.03), stretch=4.4, tilt=(0, 90)))
        moved = random.randint(1, 9)
        for b in range(10):
            x = (-1.35 + b * 0.16) if b < moved else (0.1 + (b - moved) * 0.16 + (9 - moved) * 0.0)
            o.append(float_at(f"Bead {r + 1}.{b + 1}", "torus", x, y, 0, 0.32, lacquer(colors[r], 0.18), stretch=1.6, tilt=(0, 90)))
    return o


def pipe_organ():
    o = [float_at("Wall", "box", 0, 2.0, -1.0, 1, matte((0.45, 0.2, 0.08), 0.8), stretch=9.0, scale=(24.0, 0.3)),
         obj("Console", "box", 0, 1.3, 1, lacquer((0.25, 0.12, 0.05), 0.3), stretch=1.6, scale=(4.0, 1.5))]
    n = 40
    for i in range(n):
        t = (i - (n - 1) / 2) / ((n - 1) / 2)
        h = 1.2 + 4.0 * (1 - abs(t)) ** 1.5 + 0.3 * math.cos(i * 1.7) ** 2
        x = t * 3.6
        o.append(obj(f"Pipe {i + 1}", "cylinder", x, -0.5 - 0.15 * (i % 2), 1, M((0.85, 0.82, 0.78), 0.12, metal=1), scale=(0.22, 0.22),
                     stretch=h / 0.7, elevation=0.4))
        o.append(obj(f"Foot {i + 1}", "cone", x, -0.5 - 0.15 * (i % 2), 0.45, M((0.85, 0.82, 0.78), 0.12, metal=1), tilt=(180, 0),
                     elevation=0.0))
    return o


scene("pipe-organ", "Pipe organ",
      "Forty chrome pipes of rising height in front of a warm wall: long streaky reflections on cylinders.",
      ["architecture", "metal"],
      studio(objects=pipe_organ(), sky=0.03, base=[0.3, 0.2, 0.12], light_position=[-2.0, 5.0, 4.0], light_size=3.0, light_intensity=12.0,
             light_temperature=3800.0, lights=[light("Cool fill", (3.0, 2.0, 4.0), (2, 2), (0.5, 0.65, 1.0), 6, target=(0, 1, 0))],
             **camera(0, 4, 10, (0, 1.4, 0), 45)))


# ================================================================ roadmap 5e: art rooms and portals
def room_shell(width, depth, height, material, floor_y=-1.0, ceiling=True, front=True):
    """Walls (and ceiling, front) as thin boxes around the open stage floor."""
    o = []
    t = 0.2
    cy = floor_y + height / 2
    o.append(float_at("Back wall", "box", 0, cy, -depth / 2 - t / 2, 1, material, stretch=height / 0.7, scale=((width + 2 * t) / 0.7, t / 0.7)))
    if front:
        o.append(float_at("Front wall", "box", 0, cy, depth / 2 + t / 2, 1, material, stretch=height / 0.7, scale=((width + 2 * t) / 0.7, t / 0.7)))
    for side in (-1, 1):
        o.append(float_at("Side wall", "box", side * (width / 2 + t / 2), cy, 0, 1, material, stretch=height / 0.7, scale=(t / 0.7, depth / 0.7)))
    if ceiling:
        o.append(float_at("Ceiling", "box", 0, floor_y + height + t / 2, 0, 1, material, stretch=t / 0.7, scale=((width + 2 * t) / 0.7, (depth + 2 * t) / 0.7)))
    return o


def kusama():
    random.seed(1965)
    mirror = M((0.9, 0.9, 0.92), 0.0, metal=1)
    o = room_shell(4.0, 4.0, 3.0, mirror)
    colors = [(1.0, 0.2, 0.6), (1.0, 0.7, 0.1), (0.2, 0.6, 1.0), (0.85, 0.9, 1.0)]
    while len(o) < 250:
        x, z = random.uniform(-1.8, 1.8), random.uniform(-1.8, 1.8)
        if abs(x) < 0.45 and z > 0.6:
            continue  # keep the viewpoint clear
        y = random.uniform(-0.2, 1.9)
        o.append(float_at(f"Lantern {len(o)}", "sphere", x, y, z, random.uniform(0.07, 0.12), emitter(random.choice(colors), 1.6)))
    return o


scene("kusama-infinity-room", "Infinity mirror room",
      "After Yayoi Kusama: a mirrored room full of small glowing lanterns that repeat in every direction.",
      ["art", "mirror", "emissive", "many lights"],
      studio(light_on=False, lights=[], sky=0.0, background=[0.0, 0.0, 0.0], base=[0.02, 0.02, 0.025],
             floor=M((0.02, 0.02, 0.025), 0.02, refl=1.0), floor_extent=3.0, bounces=24, objects=kusama(), exposure=0.8,
             **camera(0, 4, 0.8, (0, 0.2, 0.6), 75)))


def flavin():
    wall = matte((0.86, 0.86, 0.84), 0.9)
    o = room_shell(8.0, 6.0, 3.2, wall, ceiling=True, front=False)
    tubes = [(1.0, 0.08, 0.45), (0.15, 1.0, 0.3), (1.0, 0.6, 0.05), (0.15, 0.35, 1.0)]
    # A corner piece: vertical tubes in the back-left corner, a horizontal barrier across the back.
    for i, c in enumerate(tubes):
        o.append(obj(f"Corner tube {i + 1}", "cylinder", -3.85 + i * 0.09, -2.85, 1, emitter(c, 2.5), scale=(0.035, 0.035), stretch=3.4))
    for k in range(6):
        c = tubes[k % 4]
        o.append(float_at(f"Barrier tube {k + 1}", "cylinder", 0.5, 0.2 + k * 0.12, -2.9, 1, emitter(c, 2.5), scale=(0.035, 0.035), stretch=7.0, tilt=(0, 90)))
    return o


scene("flavin-fluorescent", "Fluorescent light",
      "After Dan Flavin: coloured fluorescent tubes in a white room; the colours bleed across the walls and mix where they meet.",
      ["art", "emissive", "color"],
      studio(light_on=False, lights=[], sky=0.0, background=[0.0, 0.0, 0.0], base=[0.8, 0.8, 0.78],
             floor=M((0.8, 0.8, 0.78), 0.6, refl=0.05), floor_extent=6.0, bounces=12, objects=flavin(), exposure=1.0,
             **camera(-20, 4, 4.5, (-0.6, 0.4, -1.2), 60)))


def turrell():
    white = matte((0.95, 0.95, 0.95), 1.0)
    o = room_shell(6.0, 6.0, 3.6, white)
    # Rounded seams: cylinders fill every vertical corner, and along the floor and ceiling edges.
    for sx in (-1, 1):
        for sz in (-1, 1):
            o.append(obj("Corner cove", "cylinder", sx * 3.0, sz * 3.0, 1, white, scale=(1.6, 1.6), stretch=3.6 / 0.7))
    for y in (-1.0, 2.6):
        for side in (-1, 1):
            o.append(float_at("Edge cove", "cylinder", side * 3.0, y, 0, 1, white, scale=(1.6, 1.6), stretch=6.0 / 0.7, tilt=(90, 0)))
            o.append(float_at("Edge cove", "cylinder", 0, y, side * 3.0, 1, white, scale=(1.6, 1.6), stretch=6.0 / 0.7, tilt=(0, 90)))
    return o


scene("turrell-ganzfeld", "Ganzfeld",
      "After James Turrell: a room with rounded seams washed in one colour of hidden light, so depth and edges dissolve.",
      ["art", "lighting", "color"],
      studio(light_on=False, sky=0.0, background=[0.0, 0.0, 0.0], base=[0.95, 0.95, 0.95], floor=M((0.95, 0.95, 0.95), 1.0, refl=0.0),
             floor_extent=4.0, bounces=16, objects=turrell(), exposure=1.0,
             lights=[light("Hidden cove light", (0, 2.45, 1.8), (4.5, 0.5), (0.55, 0.3, 1.0), 4, target=(0, 1.0, -3.0)),
                     light("Floor glow", (0, -0.85, -2.6), (4.5, 0.3), (0.85, 0.35, 0.9), 2.5, target=(0, 2.0, 0.0))],
             **camera(0, 4, 1.0, (0, 0.6, 0.8), 70)))


def corridor():
    wall = matte((0.75, 0.72, 0.68), 0.8)
    o = []
    length = 8.0
    for side in (-1, 1):
        o.append(float_at("Wall", "box", side * 1.1, 0.4, 0, 1, wall, stretch=2.8 / 0.7, scale=(0.2 / 0.7, length / 0.7)))
    o.append(float_at("Ceiling", "box", 0, 1.9, 0, 1, wall, stretch=0.2 / 0.7, scale=(2.4 / 0.7, length / 0.7)))
    for k in range(4):
        z = -length / 2 + 1 + k * 2
        o.append(float_at(f"Ceiling lamp {k + 1}", "box", 0, 1.78, z, 1, emitter((1.0, 0.85, 0.65), 6), stretch=0.03 / 0.7, scale=(1.2 / 0.7, 0.3 / 0.7)))
        o.append(obj(f"Column {k + 1}L", "cylinder", -0.85, z, 1, MARBLE, scale=(0.18, 0.18), stretch=2.8 / 0.7))
        o.append(obj(f"Column {k + 1}R", "cylinder", 0.85, z, 1, MARBLE, scale=(0.18, 0.18), stretch=2.8 / 0.7))
    o.append(obj("Red sphere", "sphere", 0.3, -1.5, 0.9, lacquer((0.8, 0.08, 0.05))))
    # Portals at both ends, paired: walking out of one end walks back in at the other.
    n = len(o)
    o.append(float_at("Portal far", "box", 0, 0.4, -length / 2, 1, matte((1, 1, 1)), stretch=2.8 / 0.7, scale=(2.0 / 0.7, 0.05 / 0.7), portal=n + 1))
    o.append(float_at("Portal near", "box", 0, 0.4, length / 2, 1, matte((1, 1, 1)), stretch=2.8 / 0.7, scale=(2.0 / 0.7, 0.05 / 0.7), rotation=180, portal=n))
    return o


scene("infinite-corridor", "Infinite corridor",
      "A short corridor whose two ends are paired portals: it repeats forever in both directions.",
      ["portal", "architecture", "impossible"],
      studio(light_on=False, lights=[], sky=0.0, background=[0.0, 0.0, 0.0], base=[0.35, 0.3, 0.26], floor=M((0.35, 0.3, 0.26), 0.25, refl=0.1),
             floor_extent=5.0, bounces=12, objects=corridor(), **camera(0, 2, 0.6, (0, 0.2, 2.4), 70)))


def bigger_inside():
    o = []
    blue = lacquer((0.05, 0.12, 0.45), 0.35)
    # The small box, out on the studio floor.
    o.append(obj("Box", "box", 0, 0, 1, blue, stretch=2.4 / 0.7, scale=(1.2 / 0.7, 1.2 / 0.7)))
    o.append(float_at("Lamp", "sphere", 0, 1.55, 0, 0.12 / 0.35, emitter((1.0, 1.0, 0.9), 12)))
    # The big hall, far away behind the studio's horizon.
    hall = matte((0.82, 0.8, 0.76), 0.7)
    cx = 30.0
    for side in (-1, 1):
        o.append(float_at("Hall wall", "box", cx + side * 6.0, 2.0, 0, 1, hall, stretch=6.0 / 0.7, scale=(0.3 / 0.7, 14.0 / 0.7)))
    o.append(float_at("Hall back", "box", cx, 2.0, -7.0, 1, hall, stretch=6.0 / 0.7, scale=(12.0 / 0.7, 0.3 / 0.7)))
    o.append(float_at("Hall ceiling", "box", cx, 5.0, 0, 1, hall, stretch=0.3 / 0.7, scale=(12.0 / 0.7, 14.0 / 0.7)))
    for i in range(5):
        o.append(obj(f"Hall column {i + 1}", "cylinder", cx - 4 + i * 2, -3.0, 1, MARBLE, scale=(0.3, 0.3), stretch=6.0 / 0.7))
        o.append(float_at(f"Hall lamp {i + 1}", "box", cx - 4 + i * 2, 4.8, 0.0, 1, emitter((1.0, 0.85, 0.6), 8), stretch=0.05, scale=(0.8 / 0.7, 6.0 / 0.7)))
    o.append(obj("Console", "cylinder", cx, 1.0, 1, M((0.6, 0.6, 0.65), 0.2, metal=1), scale=(1.4, 1.4), stretch=1.4))
    o.append(float_at("Rotor", "cylinder", cx, 1.4, 1.0, 1, M((0.4, 0.9, 1.0), 0.0, trans=1, ior=1.4), scale=(0.3, 0.3), stretch=1.6))
    # The door on the box's front face and its pair, the doorway at the hall's open front.
    n = len(o)
    o.append(float_at("Door", "box", 0, 0.15, 0.62, 1, matte((1, 1, 1)), stretch=2.1 / 0.7, scale=(0.8 / 0.7, 0.02 / 0.7), portal=n + 1))
    # Turned around: stepping through the box's door walks into the hall, not out of it.
    o.append(float_at("Hall doorway", "box", cx, 0.15, 6.5, 1, matte((1, 1, 1)), stretch=2.1 / 0.7, scale=(0.8 / 0.7, 0.02 / 0.7), rotation=180, portal=n))
    return o


scene("bigger-on-the-inside", "Bigger on the inside",
      "A small blue box on the studio floor; its door is a portal into a hall far larger than the box.",
      ["portal", "impossible"],
      studio(objects=bigger_inside(), floor_extent=40.0, sky=0.25, base=[0.3, 0.32, 0.35], bounces=12,
             **camera(10, 6, 5.0, (0, 0.2, 0), 45)))


# ---------------------------------------------------------------- physical sky (track 9)
def sky(time, **kw):
    """A physical sky (src/sky.rs); omitted fields take PhysicalSky::default()."""
    return {"on": True, "time": time, **kw}


def sunlit_room():
    """A plaster room with a window in its east wall (+x): the sun's patch crosses the floor."""
    plaster = matte((0.8, 0.77, 0.72), 0.85)
    w, d, h, t = 6.0, 6.0, 3.0, 0.2
    y0 = -1.0
    o = []

    def slab(name, x, y, z, sx, sy, sz, m=plaster):
        o.append(float_at(name, "box", x, y, z, 1, m, stretch=sy / 0.7, scale=(sx / 0.7, sz / 0.7)))

    slab("Back wall", 0, y0 + h / 2, -d / 2 - t / 2, w + 2 * t, h, t)
    slab("Front wall", 0, y0 + h / 2, d / 2 + t / 2, w + 2 * t, h, t)
    slab("West wall", -w / 2 - t / 2, y0 + h / 2, 0, t, h, d)
    slab("Ceiling", 0, y0 + h + t / 2, 0, w + 2 * t, t, d + 2 * t)
    # East wall around a window 2.4 wide, 1.5 high, its sill 0.9 above the floor.
    ex = w / 2 + t / 2
    sill, top, half = y0 + 0.9, y0 + 2.4, 1.2
    slab("East wall (below)", ex, (y0 + sill) / 2, 0, t, sill - y0, d)
    slab("East wall (above)", ex, (top + y0 + h) / 2, 0, t, y0 + h - top, d)
    for side in (-1, 1):
        zc = side * (half + (d / 2 - half) / 2)
        slab("East wall (side)", ex, (sill + top) / 2, zc, t, top - sill, d / 2 - half)
    slab("Mullion", ex, (sill + top) / 2, 0, t * 0.6, top - sill, 0.06, matte((0.2, 0.18, 0.16), 0.6))
    slab("Transom", ex, y0 + 1.75, 0, t * 0.6, 0.06, 2 * half, matte((0.2, 0.18, 0.16), 0.6))
    wood = M((0.42, 0.26, 0.15), 0.45, refl=0.06)
    slab("Table top", 0.4, y0 + 0.75, -0.6, 1.6, 0.06, 0.9, wood)
    for dx in (-0.7, 0.7):
        for dz in (-0.38, 0.38):
            slab("Table leg", 0.4 + dx, y0 + 0.36, -0.6 + dz, 0.06, 0.72, 0.06, wood)
    o.append(float_at("Vase", "sphere", 0.1, y0 + 0.93, -0.75, 0.33, M((0.85, 0.9, 1.0), 0.02, trans=1, ior=1.5)))
    o.append(float_at("Orange", "sphere", 0.7, y0 + 0.84, -0.45, 0.22, matte((0.9, 0.38, 0.05), 0.6)))
    o.append(obj("Rug", "box", -0.6, 0.6, 1, matte((0.55, 0.12, 0.1), 0.95), stretch=0.02 / 0.7, scale=(2.4 / 0.7, 1.6 / 0.7)))
    return o


# Half past six to half past seven in the evening over 12 s; the window faces east, so the
# morning sun comes in low across the floor, and the room glows from the sky after noon.
scene("sunlit-room-day-cycle", "Sunlit room · day cycle",
      "A room with an east window under the physical sky; its time of day is keyed from 06:30 to "
      "19:30, so playing the timeline (K) is a time-lapse of the sun crossing the floor.",
      ["physical sky", "interior", "animation", "daylight"],
      studio(light_on=False, lights=[], sky=0.0, floor_extent=20.0, base=[0.5, 0.48, 0.45],
             objects=sunlit_room(), bounces=12, exposure=2.0,
             physical_sky=sky(8.0, day=80.0, latitude=40.0),
             animation={"length": 12.0, "fps": 24, "shutter": 0.0,
                        "tracks": [track("physical_sky/time", [(0.0, 6.5, "linear"), (12.0, 19.5, "linear")])]},
             **camera(40, 12, 2.4, (-0.6, -0.2, 0), 62)))

scene("sky-garden-dusk", "Sky garden at dusk",
      "Glass, chrome and clay on an open floor under the physical sky, a little before sunset: the low "
      "sun is orange and the shadows are blue from the sky.",
      ["physical sky", "outdoor", "glass", "daylight"],
      studio(light_on=False, lights=[], sky=0.0, floor_extent=40.0, base=[0.6, 0.58, 0.55],
             floor=M((0.6, 0.58, 0.55), 0.7, refl=0.04),
             objects=[obj("Glass sphere", "sphere", -1.2, 0.3, 1.1, M((0.95, 0.97, 1.0), 0.0, trans=1, ior=1.5)),
                      obj("Chrome torus", "torus", 0.4, -0.6, 1.0, M((0.95, 0.95, 0.95), 0.05, metal=1), tilt=(70, 0)),
                      obj("Clay block", "box", 1.4, 0.8, 1.0, matte((0.75, 0.4, 0.28), 0.8), stretch=1.6, rotation=20),
                      obj("Clay cone", "cone", -0.2, 1.4, 0.8, matte((0.85, 0.82, 0.75), 0.8))],
             physical_sky=sky(18.9, day=172.0, latitude=45.0, turbidity=3.0, north=40.0),
             exposure=1.6, **camera(30, 10, 6.5, (0, -0.1, 0), 40)))


def ranked(m, priority):
    """A nested-dielectric material (track 10): where solids overlap, the highest priority wins."""
    return {**m, "priority": priority}


def clear(color, ior, priority, rough=0.0):
    """A clear dielectric; its colour is the transmittance per unit of distance."""
    return ranked(M(color, rough, trans=1, ior=ior), priority)


def cyl(name, x, z, y0, y1, radius, material, **kw):
    """An upright cylinder from y0 to y1."""
    return float_at(name, "cylinder", x, (y0 + y1) / 2, z, 1.0, material,
                    stretch=(y1 - y0) / 0.7, scale=(radius / 0.35, radius / 0.35), **kw)


def block(name, x, z, y0, y1, width, depth, material, **kw):
    """A box from y0 to y1, width along x and depth along z (before rotation)."""
    return float_at(name, "box", x, (y0 + y1) / 2, z, 1.0, material,
                    stretch=(y1 - y0) / 0.7, scale=(width / 0.7, depth / 0.7), **kw)


def ball(name, x, y, z, radius, material, stretch=1.0, **kw):
    # validate() keeps size >= 0.05: a smaller sphere is a size 0.1 one scaled down.
    size = max(radius / 0.35, 0.1)
    f = radius / 0.35 / size
    return float_at(name, "sphere", x, y, z, size, material, stretch=stretch * f, scale=(f, f), **kw)


def glass_of_water():
    """Track 10: a tumbler of water (gen:glass/2, two parts sharing one transform) with ice, a
    straw and air bubbles. Priorities: bubbles 4 > ice 3 > glass 2 > water 1, so the water may
    reach into the glass wall and the ice may poke out of the water."""
    glass = ranked(M((0.88, 0.97, 0.94), 0.0, trans=1, ior=1.52), 2)
    water = ranked(M((0.9, 0.97, 0.99), 0.0, trans=1, ior=1.33), 1)
    ice = ranked(M((0.95, 0.98, 1.0), 0.03, trans=1, ior=1.31), 3)
    air = ranked(M((1, 1, 1), 0.0, trans=1, ior=1.0), 4)
    # Size 2: 1.4 tall from the floor (y = -1); the water's surface is near y = 0.03 and the
    # glass's inner floor at y = -0.86; the cavity is at least 0.35 across in radius.
    out = []
    for part, name, m in [(0, "Glass", glass), (1, "Water", water)]:
        o = obj(name, "box", 0, 0, 2.0, m)
        o["mesh"] = f"gen:glass/2#{part}"
        out.append(o)
    out.append(float_at("Straw", "cylinder", 0.05, 0.03, 0.06, 1.0, lacquer((0.75, 0.05, 0.04), 0.3),
                        stretch=2.7, scale=(0.075, 0.075), tilt=(6, 20)))
    for i, (x, y, z, rot, tilt) in enumerate([(0.12, -0.06, 0.1, 20, (10, 15)),
                                               (-0.14, -0.07, -0.06, 50, (-12, 8)),
                                               (-0.02, -0.08, -0.2, 75, (6, -14))]):
        out.append(float_at(f"Ice {i + 1}", "box", x, y, z, 0.34, ice, rotation=rot, tilt=tilt))
    r = random.Random(10)
    for i in range(28):
        a, d = r.uniform(0, 2 * math.pi), 0.3 * math.sqrt(r.random())
        k = r.uniform(0.25, 0.6)  # size 0.1 at scale k: validate() keeps size >= 0.05
        out.append(float_at(f"Bubble {i + 1}", "sphere", d * math.cos(a), r.uniform(-0.82, -0.1),
                            d * math.sin(a), 0.1, air, stretch=k, scale=(k, k)))
    # Vertical bars behind the glass (a checker varies along x only on a board facing +z).
    out.append(obj("Bars", "box", 0, -1.4, 1.0, matte((0.85, 0.83, 0.78), 0.8),
                   stretch=8.0, scale=(14.0, 0.2)) | {"material": M((0.85, 0.83, 0.78), 0.8, refl=0.0,
                   pattern="checker", second=(0.05, 0.05, 0.06), pscale=6.0)})
    return out


scene("glass-of-water", "Glass of water",
      "A tumbler of water with ice, a straw and air bubbles: nested dielectrics, so light crosses "
      "water–glass, ice–water and water–air interfaces with the right ratio of indices; the straw "
      "breaks at the surface and the bars behind flip through the water.",
      ["glass", "nested dielectrics", "caustics", "mesh"],
      studio(light_position=[-2.5, 3.0, -1.0], light_size=1.2, light_intensity=30.0,
             lights=[light("Fill", (2.5, 2.0, 3.0), (2.5, 2.5), (1.0, 0.95, 0.9), 3.0, target=(0, -0.5, 0))],
             floor=M((0.62, 0.6, 0.57), 0.35, refl=0.06), base=[0.62, 0.6, 0.57], sky=0.15,
             objects=glass_of_water(), bounces=24, **camera(0, 12, 3.2, (0, -0.35, 0), 34)))


def coat(m, amount=1.0, rough=0.02):
    return {**m, "clearcoat": amount, "clearcoat_roughness": rough}


def sheen(m, color, amount=1.0, rough=0.4):
    return {**m, "sheen": amount, "sheen_color": [round(c, 4) for c in color], "sheen_roughness": rough}


def brushed(m, amount=0.8, rotation=0.0, spun=False):
    out = {**m, "anisotropy": amount, "anisotropy_rotation": rotation}
    return out | {"anisotropy_spun": True} if spun else out


def film(m, thickness, ior=1.33, amount=1.0):
    """A thin film over the surface (track 11, KHR_materials_iridescence): colours from its thickness."""
    return {**m, "iridescence": amount, "iridescence_thickness": thickness, "iridescence_ior": ior}


def grating(m, amount=0.8, spacing=1.6, spun=True, rotation=0.0):
    """A diffraction grating (track 11): grooves along the brush direction, circles when spun."""
    out = {**m, "grating": amount, "grating_spacing": spacing, "anisotropy_rotation": rotation}
    return out | {"anisotropy_spun": True} if spun else out


def layered():
    """Track 11: clear coat and sheen on shapes, with the same red paint bare and coated."""
    paint = M((0.55, 0.02, 0.03), 0.38, metal=0.6, refl=0.1)
    return [
        obj("Paint, bare", "sphere", -2.7, -0.5, 1.25, paint),
        obj("Paint, clear coat", "sphere", -1.5, -0.5, 1.25, coat(paint)),
        obj("Velvet cushion", "sphere", -0.05, -0.6, 1.5, sheen(matte((0.07, 0.006, 0.035), 1.0), (0.85, 0.3, 0.55), rough=0.35),
            stretch=0.42, scale=(1.25, 1.0), rotation=18),
        obj("Glazed teapot", "teapot", 1.55, -0.55, 1.35, coat(M((0.86, 0.85, 0.8), 0.7, refl=0.02), rough=0.05), rotation=-30),
        obj("Dusty bronze", "torus", 2.85, -0.6, 1.0, sheen(M((0.8, 0.5, 0.25), 0.35, metal=1), (0.6, 0.58, 0.55), 0.5, rough=1.0),
            tilt=(80, 0), rotation=-15),
        obj("Lacquered pawn", "pawn", -1.6, 0.75, 1.1, coat(matte((0.02, 0.09, 0.3), 0.9), rough=0.08)),
        obj("Peach", "sphere", 0.0, 0.75, 0.85, sheen(M((0.8, 0.28, 0.08), 0.75, refl=0.02), (1.0, 0.8, 0.7), 0.7, rough=0.9)),
        obj("Felt cone", "cone", 1.6, 0.75, 0.85, sheen(matte((0.08, 0.25, 0.08), 1.0), (0.5, 0.9, 0.5), 0.8, rough=0.6)),
    ]


scene("layered-materials", "Layered materials",
      "Clear coat and sheen (track 11): red paint bare and under a clear coat, a velvet cushion, a peach, a "
      "glazed teapot, dusty bronze, a lacquered pawn and a felt cone, rim-lit so the sheen glows at the edges.",
      ["material", "clearcoat", "sheen", "validation"],
      studio(light_position=[-2.0, 4.0, 3.0], light_size=2.5, light_intensity=10.0,
             lights=[light("Rim", (1.0, 2.0, -4.0), (3.0, 1.5), (0.9, 0.95, 1.0), 14.0, target=(0, -0.6, 0)),
                     light("Fill", (4.0, 1.5, 3.0), (2.0, 2.0), (1.0, 0.9, 0.8), 2.5, target=(0, -0.6, 0))],
             floor=M((0.16, 0.15, 0.14), 0.5, refl=0.05), base=[0.16, 0.15, 0.14], sky=0.04,
             objects=layered(), bounces=12, **camera(0, 13, 6.4, (0.1, -0.5, 0.1), 40)))


# Nested dielectrics as constructive solid geometry: a solid glass cylinder hollowed by a liquid
# or an air cylinder of higher priority reads as a filled glass, with no inner wall to model.
AIR = clear((1, 1, 1), 1.0, 6)
CRYSTAL = clear((0.95, 0.985, 0.975), 1.52, 1)


def backdrop_bars(z=-1.6):
    """A board of vertical stripes behind glassware, to show what the liquids bend."""
    return obj("Bars", "box", 0, z, 1.0, M((0.85, 0.83, 0.78), 0.8, refl=0.0, pattern="checker",
               second=(0.05, 0.05, 0.06), pscale=6.0), stretch=8.0, scale=(14.0, 0.2))


def cocktail():
    """A highball of three liquids (grenadine 4 > juice 3 > curaçao 2, each overlapping the next)
    under an air headspace (6), with ice (7) above everything; and a whisky tumbler beside it."""
    x0 = -0.45
    out = [cyl("Highball glass", x0, 0, -1.0 + GAP, 0.45, 0.42, CRYSTAL),
           cyl("Headspace", x0, 0, 0.14, 0.55, 0.38 - GAP, AIR),
           cyl("Grenadine", x0, 0, -0.9, -0.52, 0.38, clear((0.7, 0.02, 0.05), 1.45, 4)),
           cyl("Orange juice", x0, 0, -0.53, -0.12, 0.38 - GAP, clear((0.97, 0.5, 0.08), 1.35, 3)),
           cyl("Blue curaçao", x0, 0, -0.13, 0.15, 0.38, clear((0.1, 0.4, 0.95), 1.36, 2))]
    ice = clear((0.96, 0.98, 1.0), 1.31, 7, rough=0.04)
    for i, (dx, dz, rot) in enumerate([(-0.12, 0.06, 20), (0.13, -0.05, 55)]):
        out.append(float_at(f"Ice {i + 1}", "box", x0 + dx, 0.12, dz, 0.31, ice, rotation=rot, tilt=(8, -6)))
    out.append(ball("Cherry", x0 - 0.1, 0.3, 0.08, 0.07, lacquer((0.55, 0.01, 0.03), 0.15)))
    out.append(float_at("Straw", "cylinder", x0 + 0.12, -0.1, -0.08, 1.0, lacquer((0.95, 0.9, 0.85), 0.3),
                        stretch=1.5 / 0.7, scale=(0.075, 0.075), tilt=(-9, -8)))
    x1, z1 = 0.6, 0.35
    out += [cyl("Tumbler", x1, z1, -1.0 + GAP, -0.3, 0.33, CRYSTAL),
            cyl("Tumbler headspace", x1, z1, -0.56, -0.2, 0.29 - GAP, AIR),
            cyl("Whisky", x1, z1, -0.92, -0.55, 0.29, clear((0.8, 0.42, 0.1), 1.36, 2)),
            ball("Ice sphere", x1 + 0.03, -0.58, z1 - 0.02, 0.15, ice)]
    return out + [backdrop_bars()]


scene("layered-cocktail", "Layered cocktail",
      "Grenadine, orange juice and curaçao in a highball, and whisky over an ice ball: nested dielectrics "
      "carve the liquids and the air above them out of solid glass cylinders by priority, so every "
      "liquid–liquid, liquid–glass and ice–liquid interface bends light by its own ratio of indices.",
      ["glass", "nested dielectrics", "caustics", "absorption"],
      studio(light_position=[-1.5, 3.0, -2.0], light_size=0.8, light_intensity=45.0,
             lights=[light("Fill", (2.5, 1.5, 3.0), (2.5, 2.5), (1.0, 0.95, 0.9), 3.0, target=(0, -0.5, 0))],
             floor=M((0.75, 0.73, 0.7), 0.4, refl=0.05), base=[0.62, 0.6, 0.57], sky=0.12,
             objects=cocktail(), bounces=24, **camera(0, 10, 3.8, (0.05, -0.35, 0), 36)))


def paperweights():
    """Glass paperweights (1) holding coloured glass of a higher index (2, 3) and air bubbles (6)."""
    def colored(c, ior=1.7, priority=2):
        return clear(c, ior, priority)
    r = random.Random(11)
    out = []
    # A sphere around a cobalt knot, with a shell of bubbles clear of the knot.
    cx, cy, cz, radius = -1.1, -1.0 + 0.525, 0.1, 0.525
    out.append(ball("Sphere paperweight", cx, cy, cz, radius, CRYSTAL))
    out.append(float_at("Cobalt knot", "knot", cx, cy, cz, 0.75, colored((0.02, 0.12, 0.75)), tilt=(20, 10)))
    for i in range(18):
        a, b, d = r.uniform(0, 2 * math.pi), r.uniform(-0.8, 0.8), r.uniform(0.33, 0.44)
        out.append(ball(f"Bubble {i + 1}", cx + d * math.cos(a) * math.sqrt(1 - b * b), cy + d * b,
                        cz + d * math.sin(a) * math.sqrt(1 - b * b), r.uniform(0.012, 0.03), AIR))
    # A block around a ruby torus with an emerald octahedron in its hole.
    bx, bz = 0.2, -0.45
    out.append(block("Block paperweight", bx, bz, -1.0, -0.45, 0.9, 0.9, CRYSTAL, rotation=20))
    out.append(float_at("Ruby torus", "torus", bx, -0.725, bz, 0.7, colored((0.85, 0.02, 0.05)), tilt=(35, 0), rotation=20))
    out.append(float_at("Emerald", "octahedron", bx, -0.725, bz, 0.3, colored((0.04, 0.7, 0.15), 1.6, 3), rotation=30))
    # A low dome of millefiori: canes of coloured glass around a gold bead. (A wide flat-bottomed
    # solid shows a large patch of floor under it, lit straight through the glass by shadow rays.)
    dx, dz = 1.25, 0.5
    out.append(float_at("Dome paperweight", "sphere", dx, -0.81, dz, 1.0, CRYSTAL, stretch=0.54, scale=(1.43, 1.43)))
    out.append(ball("Gold bead", dx, -0.81, dz, 0.05, GOLD))
    hues = [(0.9, 0.1, 0.05), (0.95, 0.6, 0.02), (0.1, 0.75, 0.1), (0.05, 0.4, 0.95), (0.55, 0.05, 0.9), (0.95, 0.2, 0.6)]
    for i, c in enumerate(hues):
        a = i / len(hues) * 2 * math.pi
        out.append(float_at(f"Cane {i + 1}", "torus", dx + 0.25 * math.cos(a), -0.81, dz + 0.25 * math.sin(a), 0.3, colored(c)))
    out.append(float_at("Centre cane", "torus", dx, -0.81, dz, 0.4, colored((0.95, 0.95, 0.3), 1.65)))
    return out


scene("paperweights", "Glass paperweights",
      "Three paperweights: a cobalt knot and a shell of bubbles in a sphere, a ruby torus around an emerald "
      "in a block, and millefiori canes in a low dome. Coloured glass of a higher index nests in clear glass; a "
      "small hard light throws their coloured caustics across the linen.",
      ["glass", "nested dielectrics", "caustics", "absorption"],
      studio(light_position=[-2.0, 3.5, 1.0], light_size=0.45, light_intensity=90.0,
             lights=[light("Rim", (2.0, 2.0, -3.0), (2.0, 1.5), (0.8, 0.9, 1.0), 6.0, target=(0, -0.8, 0))],
             floor=M((0.8, 0.77, 0.7), 0.6, refl=0.03), base=[0.5, 0.48, 0.45], sky=0.08,
             objects=paperweights(), bounces=24, **camera(0, 24, 4.2, (0.1, -0.7, 0.05), 34)))


def aquarium():
    """A glass tank (1) hollowed by water (2) and an air headspace (3) that reaches past its rim,
    with marbles (4) and bubbles (5) in the water, and gravel, plants, fish and a rock that are
    opaque and sit inside the water medium."""
    out = [block("Tank", 0, 0, -1.0 + GAP, 0.6, 3.0, 1.2, clear((0.9, 0.97, 0.95), 1.5, 1)),
           block("Water", 0, 0, -0.92, 0.35, 2.9, 1.1, clear((0.75, 0.92, 0.95), 1.33, 2)),
           block("Air above the water", 0, 0, 0.34, 0.7, 2.9 - GAP, 1.1 - GAP, clear((1, 1, 1), 1.0, 3))]
    r = random.Random(12)
    for i in range(220):
        rad = r.uniform(0.03, 0.06)
        g = r.uniform(0.08, 0.3)
        tone = (g, g * r.uniform(0.75, 0.95), g * r.uniform(0.5, 0.85))
        out.append(ball(f"Pebble {i + 1}", r.uniform(-1.4, 1.4), -0.92 + rad * 0.3, r.uniform(-0.5, 0.5), rad,
                        M(tone, 0.6, refl=0.04), stretch=r.uniform(0.6, 0.9), rotation=r.uniform(0, 360)))
    for i, (x, z, h, tilt) in enumerate([(-1.25, -0.4, 1.0, (4, 6)), (-1.1, -0.3, 0.8, (-6, 3)), (-1.3, -0.15, 0.6, (3, -8)),
                                         (-0.95, -0.42, 0.7, (5, -4)), (1.0, -0.4, 0.9, (-5, 5)), (1.2, -0.3, 1.1, (3, -3)),
                                         (1.3, -0.1, 0.65, (-4, -7)), (0.2, -0.45, 0.55, (6, 2))]):
        green = lacquer((0.04 + 0.03 * (i % 3), 0.35 + 0.05 * (i % 2), 0.08), 0.35)
        out.append(obj(f"Plant {i + 1}", "cone", x, z, 0.3, green, stretch=h / 0.21, tilt=tilt, elevation=0.06))
    out.append(obj("Rock", "octahedron", 0.6, -0.2, 0.9, matte((0.12, 0.11, 0.1), 0.8), stretch=0.7, rotation=25,
                   tilt=(18, -12), elevation=0.06))
    out.append(obj("Jade bunny", "bunny", -0.45, 0.1, 0.7, JADE, rotation=35, elevation=0.06))
    for i in range(5):
        out.append(cyl(f"Coin {i + 1}", -0.05 + 0.09 * i + r.uniform(-0.03, 0.03), 0.25 + r.uniform(-0.1, 0.1),
                       -0.91 + 0.012 * (i % 2), -0.9 + 0.012 * (i % 2), 0.045, GOLD, tilt=(r.uniform(-8, 8), r.uniform(-8, 8))))
    for i, (x, z, c) in enumerate([(-0.15, 0.35, (0.3, 0.6, 0.95)), (0.95, 0.3, (0.95, 0.4, 0.2)), (0.3, 0.05, (0.3, 0.9, 0.4))]):
        out.append(ball(f"Marble {i + 1}", x, -0.85, z, 0.07, clear(c, 1.5, 4)))
    for i in range(16):
        y = -0.85 + i * 0.075
        out.append(ball(f"Bubble {i + 1}", 1.1 + 0.03 * math.sin(i * 1.7), y, 0.3 + 0.02 * math.cos(i * 2.3),
                        0.018 + 0.0015 * i, clear((1, 1, 1), 1.0, 5)))
    for i, (x, y, z, side, c) in enumerate([(-0.3, -0.1, 0.0, 1, (0.95, 0.35, 0.03)), (0.55, 0.1, 0.2, -1, (0.95, 0.75, 0.05)),
                                            (0.1, -0.45, -0.25, 1, (0.1, 0.35, 0.9))]):
        fish = lacquer(c, 0.25)
        out.append(float_at(f"Fish {i + 1}", "sphere", x, y, z, 0.35, fish, stretch=0.55, scale=(1.7, 0.45),
                            rotation=0 if side > 0 else 180))
        out.append(float_at(f"Fish {i + 1} tail", "cone", x - side * 0.24, y, z, 0.25, fish, stretch=0.8,
                            scale=(1.0, 0.25), tilt=(0, -90 * side)))
    out.append(block("Backdrop", 0, -0.78, -1.0, 1.4, 4.0, 0.04, matte((0.04, 0.16, 0.3), 0.9)))
    return out


scene("aquarium", "Aquarium",
      "A glass tank of water under its lamp: the water and the air above it are carved out of a solid glass "
      "box by priority, so the walls are one water–glass interface. Fish, plants, coins, marbles, bubbles "
      "and a jade bunny sit in the water, and its surface mirrors them seen from below.",
      ["glass", "nested dielectrics", "water"],
      studio(light_position=[-2.0, 3.0, 3.0], light_size=2.0, light_intensity=2.5, sky=0.05,
             lights=[light("Tank lamp", (0, 1.3, 0), (2.6, 0.8), (1.0, 0.97, 0.9), 30.0)],
             floor=M((0.3, 0.27, 0.24), 0.5, refl=0.04), base=[0.2, 0.22, 0.25],
             objects=aquarium(), bounces=24, **camera(0, 6, 4.6, (0, -0.2, 0), 40)))


def layer_sweeps():
    """Four rows of spheres: coat roughness, coat amount, sheen roughness, sheen amount."""
    steps = [0.0, 0.2, 0.4, 0.6, 0.8, 1.0]
    red = M((0.6, 0.03, 0.03), 0.6, refl=0.04)
    navy = matte((0.03, 0.04, 0.12), 1.0)
    rows = [("Coat roughness", lambda v: coat(red, 1.0, v)),
            ("Coat", lambda v: coat(red, v, 0.03)),
            ("Sheen roughness", lambda v: sheen(navy, (1, 1, 1), 1.0, v)),
            ("Sheen", lambda v: sheen(navy, (1, 1, 1), v, 0.5))]
    return [obj(f"{label} {v}", "sphere", -2.75 + 1.1 * j, -1.65 + 1.1 * i, 1.4, make(v))
            for i, (label, make) in enumerate(rows) for j, v in enumerate(steps)]


scene("layer-sweeps", "Layer sweeps",
      "Rows of spheres sweeping one layer parameter each, 0 to 1 from left to right: clear-coat roughness and "
      "amount over rough red paint, then sheen roughness and amount over navy cloth.",
      ["material", "clearcoat", "sheen", "validation"],
      studio(light_position=[0.0, 4.5, 1.5], light_size=4.0, light_aspect=1.0, light_intensity=6.0,
             lights=[light("Rim", (0, 1.2, -5.0), (6.0, 1.0), (0.9, 0.95, 1.0), 12.0, target=(0, -0.6, 0))],
             floor=M((0.2, 0.2, 0.2), 0.6, refl=0.04), base=[0.2, 0.2, 0.2], sky=0.5,
             objects=layer_sweeps(), bounces=12, **camera(0, 30, 8.6, (0, -0.6, 0.15), 40)))

scene("layered-furnace", "Layered white furnace",
      "The white furnace for track 11: white spheres under clear coats and sheens of several roughnesses, and "
      "brushed white metal, in a uniform white world. A layer that gains energy shows brighter than the background; a small darkening "
      "is the energy the GGX lobes lose to single scattering.",
      ["validation", "energy", "clearcoat", "sheen"],
      studio(light_on=False, lights=[], sky=0.0, background=[1.0, 1.0, 1.0], base=[1.0, 1.0, 1.0],
             floor=M((1, 1, 1), 1.0, refl=0.0), floor_extent=40.0, tonemap=0, exposure=0.7, bounces=32,
             objects=[obj(f"Coat {c}", "sphere", -1.8 + 1.2 * i, 0, 1.4, coat(M((1, 1, 1), 1.0, refl=0.0), 1.0, c))
                      for i, c in enumerate([0.0, 0.3, 0.6, 1.0])]
             + [obj(f"Sheen {s}", "sphere", -1.8 + 1.2 * i, -1.6, 1.4, sheen(M((1, 1, 1), 1.0, refl=0.0), (1, 1, 1), 1.0, s))
                for i, s in enumerate([0.1, 0.4, 0.7, 1.0])]
             + [obj("Coat and sheen", "sphere", -0.6, 1.6, 1.4, coat(sheen(M((1, 1, 1), 1.0, refl=0.0), (1, 1, 1), 1.0, 0.5), 1.0, 0.1)),
                obj("Coated metal", "sphere", 0.6, 1.6, 1.4, coat(M((1, 1, 1), 0.5, metal=1), 1.0, 0.1)),
                obj("Brushed metal", "sphere", -1.8, 1.6, 1.4, brushed(M((1, 1, 1), 0.3, metal=1), 1.0)),
                obj("Spun metal", "sphere", 1.8, 1.6, 1.4, brushed(M((1, 1, 1), 0.3, metal=1), 1.0, spun=True))],
             **camera(0, 18, 9, (0, -0.4, 0))))


# ================================================================ volumes (track 12)
def volume(m, density, scatter=(1, 1, 1), phase=0.0):
    """A transmissive material filled with a scattering medium."""
    return {**m, "density": density, "scatter_color": [round(c, 4) for c in scatter], "phase": phase}


def fog(density, color=(1, 1, 1), phase=0.0, height=3.0, extent=6.0):
    return {"density": density, "color": [round(c, 4) for c in color], "phase": phase, "height": height, "extent": extent}


def spot(name, pos, target, color, intensity, spread=24.0, blend=0.25, size=0.3):
    return {**light(name, pos, (size, size), color, intensity, target=target), "kind": "spot", "spread": spread, "blend": blend}


scene("cornell-fog", "Cornell fog",
      "The box filled with a thin forward-scattering haze: the ceiling light glows in the air and the blocks cast "
      "shadow shafts through it.",
      ["cornell", "volume", "fog"],
      cornell(light_size=0.35, light_intensity=40.0, bounces=12, fog=fog(0.35, (0.95, 0.95, 0.95), 0.5, 1.0, 1.0),
              objects=[obj("Tall block", "box", -0.33, -0.3, 0.85, matte((0.72, 0.7, 0.66), 0.85), stretch=1.98, rotation=17),
                       obj("Short block", "box", 0.33, 0.3, 0.85, matte((0.72, 0.7, 0.66), 0.85), stretch=0.99, rotation=-17)]))


scene("underwater", "Underwater",
      "The whole stage under blue-green water: a forward-scattering medium that swallows the distance, with sun "
      "shafts around the pillars.",
      ["volume", "fog", "underwater", "sun"],
      dark_stage(fog=fog(0.07, (0.35, 0.75, 0.8), 0.8, 8.0, 30.0), floor=M((0.6, 0.55, 0.42), 0.8, refl=0.02),
                 floor_extent=30.0, bounces=10,
                 lights=[{**light("Sun", (0, 10, 0), (1, 1), (1.0, 0.97, 0.9), 6.0, target=(0.5, 0, 0.3)), "kind": "sun", "angle": 2.0}],
                 objects=[obj(f"Pillar {i}", "cylinder", -3 + 2 * i, -1.5 + (i % 2) * 1.5, 1.6, matte((0.5, 0.48, 0.42), 0.8), stretch=6.0)
                          for i in range(4)]
                 + [obj("Amphora", "pawn", 0.3, 1.2, 1.2, M((0.6, 0.3, 0.15), 0.6, refl=0.04))],
                 **camera(10, 4, 8, (0, 0.8, 0), 55)))


# ================================================================ procedural surfaces (track 13)
def sdf(o, kind):
    """Render an object as an exact procedural surface (its shape stays the proxy for picking)."""
    return {**o, "sdf": kind}


def lounge():
    """A velvet sofa with cushions and pillows, a lacquered table, a glazed vase, peaches, a felt rug."""
    velvet = sheen(matte((0.015, 0.06, 0.04), 1.0), (0.35, 0.85, 0.6), 1.0, 0.3)
    wood = M((0.2, 0.1, 0.05), 0.5, refl=0.04)
    chrome = M((0.92, 0.94, 0.97), 0.05, metal=1)
    out = [block("Back wall", 0, -1.7, -1.0, 2.4, 13.0, 0.1, matte((0.38, 0.33, 0.29), 0.9)),
           block("Rug", 0, 0.05, -1.0, -0.99, 3.4, 2.2, sheen(matte((0.45, 0.1, 0.06), 1.0), (1.0, 0.6, 0.5), 0.8, 0.7)),
           block("Sofa base", 0, -0.9, -0.88, -0.6, 2.4, 0.9, velvet),
           block("Sofa back", 0, -1.25, -0.6, 0.05, 2.4, 0.2, velvet)]
    for side in (-1, 1):
        out.append(block("Sofa arm", side * 1.3, -0.9, -0.88, -0.3, 0.2, 0.9, velvet))
        out.append(block("Seat cushion", side * 0.58, -0.82, -0.6, -0.42, 1.12, 0.74, velvet))
        out.append(block("Back cushion", side * 0.58, -1.08, -0.42, 0.0, 1.1, 0.16, velvet, tilt=(-8, 0)))
        for dz in (-1.28, -0.52):
            out.append(cyl("Sofa leg", side * 1.3, dz, -1.0, -0.88, 0.035, wood))
    out.append(float_at("Mustard pillow", "sphere", -0.85, -0.2, -0.98, 1.0,
                        sheen(matte((0.35, 0.2, 0.02), 1.0), (1.0, 0.85, 0.4), 1.0, 0.35),
                        stretch=0.85, scale=(1.0, 0.4), tilt=(-15, 0), rotation=12))
    out.append(float_at("Rose pillow", "sphere", 0.85, -0.2, -0.98, 1.0,
                        sheen(matte((0.3, 0.02, 0.08), 1.0), (1.0, 0.45, 0.6), 1.0, 0.35),
                        stretch=0.85, scale=(1.0, 0.4), tilt=(-15, 0), rotation=-10))
    # The table: a black lacquer top on chrome legs.
    out.append(block("Table top", 0, 0.35, -0.62, -0.58, 1.4, 0.7, coat(matte((0.02, 0.015, 0.012), 0.6), 1.0, 0.02)))
    for sx in (-1, 1):
        for sz in (-1, 1):
            out.append(cyl("Table leg", sx * 0.64, 0.35 + sz * 0.29, -1.0, -0.62, 0.02, chrome))
    out.append(cyl("Glazed vase", -0.38, 0.3, -0.58, -0.18, 0.1, coat(matte((0.1, 0.25, 0.55), 0.7), 1.0, 0.04)))
    out.append(obj("Brass bowl", "torus", 0.25, 0.4, 0.75, coat(M((0.9, 0.65, 0.3), 0.35, metal=1), 1.0, 0.05),
                   elevation=0.42))
    peach = sheen(M((0.8, 0.28, 0.08), 0.75, refl=0.02), (1.0, 0.8, 0.7), 0.7, 0.9)
    for i, (dx, dz) in enumerate([(-0.05, 0.0), (0.05, 0.02), (0.0, -0.05)]):
        out.append(ball(f"Peach {i + 1}", 0.25 + dx, -0.47, 0.4 + dz, 0.06, peach))
    # A floor lamp: a chrome pole and a glowing globe.
    out.append(cyl("Lamp pole", 1.85, -1.2, -1.0, 0.75, 0.02, chrome))
    out.append(ball("Lamp globe", 1.85, 0.9, -1.2, 0.17, emitter((1.0, 0.75, 0.45), 6.0, unlit=False)))
    return out


scene("velvet-lounge", "Velvet lounge",
      "A green velvet sofa with mustard and rose pillows, a black lacquer table, a glazed vase, a brass bowl of "
      "peaches and a felt rug on a clear-coated floor: sheen gives the cloth its bright edges, the coats their "
      "sharp reflections over a soft base.",
      ["interior", "clearcoat", "sheen", "material"],
      studio(light_position=[-1.5, 3.2, 1.5], light_size=2.5, light_intensity=8.0, light_temperature=3400.0,
             lights=[light("Window", (3.2, 1.0, -0.4), (1.5, 2.5), (0.7, 0.8, 1.0), 5.0, target=(0, -0.5, -0.9))],
             floor=coat(M((0.3, 0.16, 0.08), 0.5, refl=0.04), 1.0, 0.1), base=[0.35, 0.3, 0.26], sky=0.04,
             objects=lounge(), bounces=12, **camera(-12, 8, 4.6, (0, -0.45, -0.4), 45)))


def vanity():
    """Perfume bottles (glass 1, liquids 3, sealed air 5) on a mirror tray, on velvet (sheen) in
    front of a velvet curtain, with a lacquered box, a gold ring and coated pearls."""
    chrome = M((0.92, 0.94, 0.97), 0.02, metal=1)
    sealed = clear((1, 1, 1), 1.0, 5)
    out = [block("Mirror tray", 0, 0, -1.0, -0.97, 1.8, 0.9, chrome)]
    # A square amber bottle with a gold cap.
    x, z, rot = -0.5, 0.0, 15
    out += [block("Square bottle", x, z, -0.97, -0.27, 0.42, 0.26, CRYSTAL, rotation=rot),
            block("Amber perfume", x, z, -0.91, -0.5, 0.34, 0.18, clear((0.85, 0.4, 0.05), 1.36, 3), rotation=rot),
            block("Square bottle air", x, z, -0.51, -0.33, 0.34, 0.18, sealed, rotation=rot),
            cyl("Square bottle neck", x, z, -0.29, -0.2, 0.05, CRYSTAL),
            cyl("Gold cap", x, z, -0.2, -0.08, 0.08, coat(GOLD, 1.0, 0.03))]
    # A round rose bottle with an atomiser: a chrome tube to a velvet bulb.
    x, z = 0.15, -0.15
    out += [cyl("Round bottle", x, z, -0.97, -0.45, 0.2, CRYSTAL),
            cyl("Rose perfume", x, z, -0.92, -0.6, 0.16, clear((0.9, 0.12, 0.4), 1.36, 3)),
            cyl("Round bottle air", x, z, -0.61, -0.5, 0.16, sealed),
            cyl("Atomiser collar", x, z, -0.46, -0.38, 0.045, chrome),
            float_at("Atomiser tube", "cylinder", x + 0.14, -0.41, z, 1.0, chrome, stretch=0.28 / 0.7, scale=(0.04, 0.04), tilt=(0, 90)),
            ball("Atomiser bulb", x + 0.33, -0.41, z, 0.09, sheen(matte((0.3, 0.02, 0.1), 1.0), (1.0, 0.5, 0.7), 1.0, 0.3))]
    # A slim blue bottle with a faceted stopper.
    x, z = 0.6, 0.15
    out += [cyl("Slim bottle", x, z, -0.97, -0.1, 0.1, CRYSTAL),
            cyl("Blue perfume", x, z, -0.92, -0.35, 0.075, clear((0.2, 0.45, 0.95), 1.36, 3)),
            cyl("Slim bottle air", x, z, -0.36, -0.15, 0.075, sealed),
            float_at("Stopper", "octahedron", x, -0.03, z, 0.28, CRYSTAL, stretch=1.3, rotation=20)]
    out.append(block("Lacquer box", 1.3, -0.35, -1.0, -0.75, 0.6, 0.4, coat(matte((0.45, 0.02, 0.02), 0.7), 1.0, 0.03), rotation=-20))
    out.append(obj("Gold ring", "ring", 1.0, 0.55, 0.35, GOLD))
    pearl = coat(M((0.92, 0.88, 0.84), 0.35, refl=0.1), 1.0, 0.02)
    for i in range(26):
        a = math.pi * (0.1 + 0.8 * i / 25)
        out.append(ball(f"Pearl {i + 1}", -0.75 + 0.45 * math.cos(a), -1.0 + 0.035, 0.8 - 0.3 * math.sin(a), 0.035, pearl))
    curtain = sheen(matte((0.1, 0.008, 0.02), 1.0), (0.9, 0.3, 0.4), 1.0, 0.35)
    for i in range(30):
        out.append(cyl(f"Curtain fold {i + 1}", -3.0 + 0.2 * i, -1.5 + 0.04 * math.sin(i * 1.3), -1.0, 2.5, 0.12, curtain))
    return out


scene("perfume-vanity", "Perfume vanity",
      "Perfume bottles on a mirror tray in front of a velvet curtain: the liquids and the sealed air above them "
      "nest inside the glass by priority, the atomiser bulb and the curtain folds are sheen, the jewellery box "
      "and the pearls are clear-coated.",
      ["glass", "nested dielectrics", "clearcoat", "sheen"],
      studio(light_position=[-1.2, 2.5, 1.5], light_size=1.2, light_intensity=18.0, light_temperature=4200.0,
             lights=[light("Rim", (1.6, 1.0, -1.0), (1.0, 1.0), (1.0, 0.92, 0.85), 10.0, target=(0, -0.7, 0))],
             floor=sheen(matte((0.12, 0.01, 0.03), 1.0), (0.9, 0.35, 0.45), 1.0, 0.35), base=[0.12, 0.05, 0.06],
             sky=0.03, objects=vanity(), bounces=24, **camera(8, 14, 2.9, (0.15, -0.65, 0.1), 38)))


def brushed_metal():
    """Anisotropic GGX (track 11): spun discs and a pan, a brushed plate and a brushed sphere."""
    steel = M((0.9, 0.9, 0.92), 0.3, metal=1)
    alu = M((0.91, 0.92, 0.93), 0.25, metal=1)
    copper = M((0.95, 0.6, 0.45), 0.3, metal=1)
    out = [cyl("Spun disc", -1.4, 0.3, -1.0, -0.96, 0.75, brushed(steel, 0.9, spun=True)),
           block("Brushed plate", 0.0, -1.3, -1.0, 1.0, 3.2, 0.06, brushed(alu, 0.9)),
           ball("Spun sphere", 0.05, -0.45, 0.2, 0.55, brushed(steel, 0.8, spun=True)),
           ball("Brushed sphere", 1.35, -0.6, -0.35, 0.4, brushed(alu, 0.8, rotation=90)),
           cyl("Copper pan", 1.3, 0.75, -1.0, -0.78, 0.45, brushed(copper, 0.85, spun=True)),
           block("Pan handle", 2.05, 0.75, -0.85, -0.81, 0.7, 0.08, brushed(steel, 0.9)),
           obj("Brushed ring", "torus", -1.4, 0.3, 0.8, brushed(GOLD, 0.8, spun=True), elevation=0.04),
           obj("Polished sphere", "sphere", -0.6, 1.1, 0.7, M((0.9, 0.9, 0.92), 0.3, metal=1))]
    return out


scene("brushed-metal", "Brushed metal",
      "Anisotropic metals (track 11): a spun steel disc with a gold ring on it, a spun sphere and a copper pan "
      "whose highlights fan out radially, a brushed aluminium plate and sphere streaked across the brushing, and "
      "a polished sphere of the same roughness for comparison.",
      ["material", "anisotropy", "metal"],
      dark_stage(light_on=True, light_position=[-1.0, 3.0, 1.5], light_size=0.5, light_intensity=60.0,
                 lights=[light("Strip", (2.0, 1.8, 1.5), (0.15, 2.0), (1.0, 0.95, 0.9), 40.0, target=(0, -0.6, 0)),
                         light("Fill", (-3.0, 1.5, 2.5), (2.0, 2.0), (0.8, 0.85, 1.0), 2.0, target=(0, -0.6, 0))],
                 floor=M((0.05, 0.05, 0.055), 0.6, refl=0.04), base=[0.05, 0.05, 0.055],
                 objects=brushed_metal(), bounces=12, **camera(0, 30, 5.2, (0, -0.6, 0), 40)))


def thin_film():
    """Thin-film interference (track 11): soap bubbles, an oil slick, anodized titanium and a beetle-green pawn."""
    bubble = clear((1, 1, 1), 1.0, 0)
    titanium = M((0.56, 0.53, 0.5), 0.12, metal=1)
    out = [ball(f"Soap bubble {t} nm", x, y, z, r, film(bubble, t))
           for (x, y, z, r, t) in [(-1.6, 0.35, 0.3, 0.55, 380), (-0.35, 0.9, -0.4, 0.4, 520), (0.55, 0.15, 0.55, 0.3, 700)]]
    out += [ball(f"Anodized titanium {t} nm", -1.65 + i * 0.8, -0.72, 1.35, 0.28, film(titanium, t, 2.4))
            for i, t in enumerate([60, 110, 160, 210, 260])]
    out += [cyl("Oil slick", 1.6, -0.5, -1.0, -0.99, 0.9, film(M((0.015, 0.015, 0.02), 0.04, refl=0.04), 420, 1.47)),
            obj("Beetle-green pawn", "pawn", 1.65, -0.5, 0.9, film(M((0.05, 0.25, 0.08), 0.25, metal=1), 520, 1.6))]
    return out


scene("thin-film", "Thin film",
      "Thin-film interference (track 11): three soap bubbles of different wall thickness, a row of anodized "
      "titanium spheres whose oxide grows from 60 to 260 nm, an oil slick on a dark puddle and a beetle-green "
      "pawn, under a broad softbox so the colours shift across each surface.",
      ["material", "iridescence", "thin film"],
      studio(light_position=[-1.5, 4.0, 2.5], light_size=3.0, light_intensity=9.0,
             lights=[light("Softbox", (3.5, 2.0, 1.5), (3.0, 3.0), (1.0, 0.97, 0.93), 6.0, target=(0, -0.3, 0)),
                     light("Back", (0.0, 2.5, -4.0), (5.0, 2.0), (0.9, 0.95, 1.0), 5.0, target=(0, 0, 0))],
             floor=M((0.5, 0.5, 0.5), 0.6, refl=0.04), base=[0.5, 0.5, 0.5], sky=0.15,
             objects=thin_film(), bounces=12, **camera(0, 10, 6.0, (0, -0.2, 0.2), 40)))


def optical_discs():
    """Diffraction gratings (track 11): a CD, a DVD and a Blu-ray, and a CD standing up."""
    alu = M((0.92, 0.92, 0.93), 0.08, metal=1)
    return [cyl("CD", -1.3, 0.2, -1.0, -0.985, 0.6, grating(alu, 0.85, 1.6)),
            cyl("DVD", 0.15, 0.55, -1.0, -0.985, 0.6, grating(M((0.8, 0.72, 0.55), 0.08, metal=1), 0.85, 0.74)),
            cyl("Blu-ray", 1.55, 0.15, -1.0, -0.985, 0.6, grating(alu, 0.85, 0.32)),
            float_at("Standing CD", "cylinder", -0.1, -0.4, -0.7, 1.0, grating(alu, 0.85, 1.6),
                     stretch=0.015 / 0.7, scale=(0.6 / 0.35, 0.6 / 0.35), tilt=(90, 0), rotation=10)]


scene("optical-discs", "Optical discs",
      "Diffraction gratings (track 11): a CD (1.6 µm grooves), a gold DVD (0.74 µm) and a Blu-ray (0.32 µm, whose "
      "orders only reach grazing views) lie on a dark stage and a CD stands up behind them, "
      "under small lamps so each reflection splits into rainbows across the grooves.",
      ["material", "diffraction", "grating"],
      dark_stage(light_on=True, light_position=[0.5, 3.0, -2.5], light_size=0.25, light_intensity=300.0,
                 lights=[light("Lamp", (-3.0, 1.2, 1.8), (0.2, 0.2), (1.0, 0.97, 0.92), 300.0, target=(0, -0.9, 0)),
                         light("Fill", (2.5, 2.5, 3.0), (2.5, 2.5), (0.8, 0.85, 1.0), 0.6, target=(0, -0.9, 0))],
                 floor=M((0.03, 0.03, 0.035), 0.6, refl=0.04), base=[0.03, 0.03, 0.035],
                 objects=optical_discs(), bounces=8, **camera(0, 35, 5.0, (0.1, -0.85, 0.0), 40)))


# ================================================================ scene overhaul (2026-11-05)
# Reworked scenes (their earlier scene() calls are gone; the builder functions above stay in use)
# and new scenes for what no scene showed before: depth of field and bokeh, a real lens, the
# camera models (orthographic, 360°, little planet, tilt-shift), sun and sphere lights, the
# flashlight, fog noise / extinction / equiangular sampling, blackbody emission, dispersion, the
# film look with a LUT, cloth in wind, a chain, a camera path and a per-frame script.
def sphere_light(name, pos, diameter, color, intensity, visible=True, temperature=0.0):
    out = {**light(name, pos, (diameter, diameter), color, intensity, visible=visible), "kind": "sphere"}
    return out | {"temperature": temperature} if temperature else out


def sun_light(name, toward_sun, color, intensity, angle=0.53):
    """A distant sun in the direction of `toward_sun` from the stage (its light travels back)."""
    return {**light(name, toward_sun, (1, 1), color, intensity, target=(0, -1, 0)), "kind": "sun", "angle": angle}


def disk(name, pos, target, diameter, color, intensity, ies=""):
    out = {**light(name, pos, (diameter, diameter), color, intensity, target=target), "kind": "disk"}
    return out | {"ies": ies} if ies else out


def hot(m, kelvin_, strength):
    """Emission as a blackbody (Material::temperature): the colour follows Planck's law."""
    return {**m, "emission": strength, "temperature": kelvin_}


def fog_ex(density, color=(1, 1, 1), phase=0.0, height=3.0, extent=6.0, noise=0.0, noise_size=1.0,
           extinction=None, equiangular=False):
    f = {**fog(density, color, phase, height, extent), "noise": noise, "noise_size": noise_size}
    if extinction:
        f["extinction"] = [round(c, 4) for c in extinction]
    return f | {"equiangular": True} if equiangular else f


def roof(name, x, z, y, width, depth, pitch, material, rotation=0.0):
    """A gable roof as two tilted slabs meeting along the ridge (along z before rotation)."""
    out = []
    half = width / 2 / math.cos(math.radians(pitch))
    rise = width / 4 * math.tan(math.radians(pitch))
    c, s = math.cos(math.radians(rotation)), math.sin(math.radians(rotation))
    for side in (-1, 1):
        dx = side * width / 4
        out.append(float_at(f"{name} {'LR'[side > 0]}", "box", x + dx * c, y + rise, z - dx * s, 1, material,
                            stretch=0.04 / 0.7, scale=(half * 1.04 / 0.7, depth * 1.08 / 0.7),
                            tilt=(0, -side * pitch), rotation=rotation))
    return out


# ---------------------------------------------------------------- reworked
def abacus_desk():
    wall = matte((0.55, 0.5, 0.44), 0.9)
    o = abacus()
    o += [block("Wall", 0, -2.2, -1.0, 3.0, 10.0, 0.1, wall),
          block("Notebook", 2.15, 0.9, -1.0, -0.955, 0.85, 1.15, matte((0.62, 0.1, 0.08), 0.75), rotation=-14),
          block("Notebook pages", 2.15, 0.9, -0.995, -0.96, 0.8, 1.1, matte((0.9, 0.88, 0.82), 0.9), rotation=-14),
          float_at("Pencil", "cylinder", 1.7, -0.935, 1.55, 1, lacquer((0.95, 0.7, 0.08), 0.3), stretch=0.75 / 0.7,
                   scale=(0.045, 0.045), tilt=(0, 90), rotation=30),
          ball("Brass weight", -2.1, -0.84, 0.8, 0.16, M((0.9, 0.7, 0.35), 0.25, metal=1))]
    return o


scene("abacus", "Abacus",
      "Fifty lacquered beads on five rods, on a wooden desk by a window. A thin lens at a wide aperture with "
      "six blades and cat-eye vignetting: the beads in focus, the notebook and the wall soft, highlights as "
      "hexagons that turn into cat's eyes toward the edges.",
      ["still life", "many objects", "depth of field", "bokeh"],
      studio(objects=abacus_desk(), floor=lacquer((0.3, 0.16, 0.07), 0.3), base=[0.3, 0.16, 0.07], floor_extent=5.0,
             light_position=[-3.0, 2.5, 1.5], light_size=2.5, light_intensity=12.0, light_temperature=5200.0,
             lights=[light("Warm lamp", (2.5, 1.2, 2.0), (0.6, 0.6), kelvin(2900), 30, target=(0, -0.5, 0))],
             sky=0.05, aperture=0.08, focus_distance=4.4, aperture_blades=6, blade_rotation=15.0, cat_eye=0.5,
             **camera(14, 10, 4.4, (0, -0.15, 0), 38)))


def bowling_hall():
    o = bowling()
    ball_index = next(i for i, x in enumerate(o) if x["name"] == "Ball")
    dark = lacquer((0.04, 0.04, 0.06), 0.3)
    for side in (-1, 1):
        o.append(block("Kickback", side * 1.75, -12.3, -1.0, 0.2, 0.15, 3.0, dark))
    o.append(block("Masking unit", 0, -14.0, 0.2, 1.4, 3.6, 0.2, dark))
    for i, c in enumerate([(1.0, 0.1, 0.5), (0.1, 0.6, 1.0), (1.0, 0.7, 0.05)]):
        o.append(float_at(f"Neon {i + 1}", "cylinder", 0, 0.45 + i * 0.35, -13.88, 1, emitter(c, 8), stretch=3.2 / 0.7,
                          scale=(0.05, 0.05), tilt=(0, 90)))
    for k, side in enumerate((-1, 1)):
        o.append(obj(f"Approach dot {k + 1}", "sphere", side * 0.3, -1.0, 0.05, emitter((1.0, 0.9, 0.7), 3), stretch=0.3, elevation=0.07))
    # The roll: K plays it; the export renders it with motion blur.
    z0 = o[ball_index]["position"][1]
    tracks = [track(f"objects/{ball_index}/position/1", [(0.0, z0, "linear"), (1.6, -11.75, "ease"), (2.4, -12.45, "hold")]),
              track(f"objects/{ball_index}/position/0", [(0.0, 0.12, "linear"), (1.6, 0.02, "hold")])]
    for idx, (dx, dz), tz in [(3, (0.35, -13.1), 80), (4, (-0.55, -12.9), -75), (5, (0.6, -12.7), 70), (7, (-0.2, -13.3), -85)]:
        x0, zz0 = o[idx]["position"]
        tracks += [track(f"objects/{idx}/position/0", [(0.0, x0, "hold"), (1.6, x0, "ease"), (2.2, dx, "hold")]),
                   track(f"objects/{idx}/position/1", [(0.0, zz0, "hold"), (1.6, zz0, "ease"), (2.2, dz, "hold")]),
                   track(f"objects/{idx}/tilt/1", [(0.0, 0.0, "hold"), (1.6, 0.0, "ease"), (2.2, tz, "hold")])]
    return o, {"length": 2.4, "fps": 24, "shutter": 0.5, "tracks": tracks}


_o, _a = bowling_hall()
scene("bowling-lane", "Bowling lane",
      "A long glossy lane under a row of lights, neon on the masking unit: stretched reflections down the "
      "boards. The timeline (K) rolls the ball into the pins; a timeline export renders the strike with a "
      "180° motion-blur shutter.",
      ["glossy", "interior", "animation", "motion blur"],
      studio(light_on=False, sky=0.0, base=[0.06, 0.06, 0.08], objects=_o, animation=_a, exposure=1.3,
             lights=[light(f"Lane light {i + 1}", (0, 2.2, 1 - i * 3.0), (2.0, 0.4), kelvin(4500), 18) for i in range(5)]
             + [light("Pin deck light", (0, 1.2, -11.0), (2.6, 0.3), kelvin(5200), 30, target=(0, -1, -12.4))],
             **camera(0, 6, 6.0, (0, -0.6, -6.5), 40)))


def dunes():
    random.seed(23)
    sand = M((0.78, 0.47, 0.24), 0.95, refl=0.0)
    o = [obj("Monolith", "box", 0, 0, 1, matte((0.012, 0.012, 0.014), 0.25), stretch=6.5, scale=(1.3, 0.3), rotation=15)]
    for i in range(9):
        a = random.uniform(0, math.tau)
        d = random.uniform(9, 30)
        o.append(obj(f"Dune {i + 1}", "sphere", d * math.cos(a), d * math.sin(a) - 6, random.uniform(10, 20), sand,
                     stretch=random.uniform(0.08, 0.16), scale=(random.uniform(1.2, 2.0), 1.0), rotation=random.uniform(0, 180),
                     elevation=-0.3))
    return o


scene("desert-monolith", "Desert monolith",
      "A black slab among dunes under the physical sky, an hour before sunset: one hard shadow stretching "
      "across the sand, the sky's blue in the shade and the sun's orange on the lit faces.",
      ["open", "shadows", "physical sky"],
      studio(light_on=False, lights=[], sky=0.0, base=[0.78, 0.47, 0.24], floor=M((0.78, 0.47, 0.24), 0.95, refl=0.0),
             floor_extent=40.0, objects=dunes(), exposure=1.3,
             physical_sky=sky(18.2, day=200.0, latitude=28.0, turbidity=4.0, north=60.0),
             **camera(-30, 6, 11, (0, 0.8, 0), 40)))

random.seed(5)
scene("disco-floor", "Disco floor",
      "A dark glass dance floor with glowing purple squares, a mirror ball and four coloured spots in a light "
      "haze: the beams sample the haze toward each spot (equiangular sampling), the ball scatters them.",
      ["emissive", "party", "fog", "spot"],
      dark_stage(floor=M((0.55, 0.12, 0.85), 0.2, refl=0.2, emit=0.5, pattern="checker", second=(0.005, 0.005, 0.01), pscale=1.2),
                 floor_extent=5.0, exposure=1.4, bounces=10,
                 fog=fog_ex(0.05, (0.95, 0.95, 1.0), 0.5, 4.5, 6.0, equiangular=True),
                 lights=[spot(f"Spot {i + 1}", (4.0 * math.cos(a), 4.2, 4.0 * math.sin(a)), (0, 2.2, 0), c, 160.0, spread=14.0)
                         for i, (a, c) in enumerate([(0.4, (1.0, 0.1, 0.4)), (2.0, (0.1, 0.5, 1.0)), (3.6, (1.0, 0.8, 0.1)),
                                                     (5.1, (0.2, 1.0, 0.4))])],
                 objects=[float_at("Mirror ball", "sphere", 0, 2.2, 0, 1.6, M((0.9, 0.9, 0.95), 0.02, metal=1))]
                 + [obj(f"Dancer {i + 1}", "pawn", 2.2 * math.cos(i / 7 * math.tau + 0.3), 2.2 * math.sin(i / 7 * math.tau + 0.3), 1.4,
                        lacquer(random.choice([(0.05, 0.6, 0.9), (0.9, 0.85, 0.1), (0.1, 0.8, 0.3), (0.95, 0.95, 0.95)])), stretch=1.4)
                    for i in range(7)],
                 **camera(15, 10, 8, (0, 0.6, 0), 45)))


def jewellery_velvet():
    o = jewellery()
    o[0]["material"] = sheen(matte((0.1, 0.008, 0.02), 1.0), (0.9, 0.3, 0.4), 1.0, 0.35)
    o.append(block("Box", 0.1, -2.1, -1.0, -0.2, 3.4, 1.4, coat(matte((0.06, 0.02, 0.012), 0.6), 1.0, 0.03)))
    o.append(block("Box lining", 0.1, -1.38, -0.2, 1.0, 3.3, 0.06, sheen(matte((0.1, 0.008, 0.02), 1.0), (0.9, 0.3, 0.4), 1.0, 0.35),
                   tilt=(-20, 0)))
    return o


scene("jewellery-box", "Jewellery box",
      "Gold, silver and copper rings, pearls and a diamond on velvet in front of an open lacquered box: "
      "metals reflecting each other at close range, the velvet's sheen at grazing angles, a shallow depth of field.",
      ["material", "metal", "sheen", "depth of field"],
      studio(objects=jewellery_velvet(), light_position=[-1.5, 3.0, 1.5], light_size=1.5, light_intensity=20.0, sky=0.03,
             base=[0.05, 0.02, 0.02], floor=matte((0.05, 0.02, 0.02), 0.6),
             lights=[light("Warm rim", (2.0, 1.5, -2.0), (1.5, 1.5), kelvin(3200), 14, target=(0, -1, 0))],
             aperture=0.04, focus_distance=3.4, **camera(25, 42, 3.4, (0, -0.85, 0), 40)))


def zen_garden():
    stone = M((0.55, 0.53, 0.5), 0.85, refl=0.02)
    o = zen()
    o[-1]["material"] = sheen(matte((0.14, 0.26, 0.06), 1.0), (0.6, 0.85, 0.4), 0.8, 0.6)
    # A stone lantern (tōrō) and the edge of a wooden veranda in the foreground.
    x, z = 2.4, -2.0
    o += [cyl("Lantern base", x, z, -1.0, -0.85, 0.28, stone), cyl("Lantern post", x, z, -0.85, -0.25, 0.09, stone),
          block("Lantern firebox", x, z, -0.25, 0.05, 0.36, 0.36, stone),
          float_at("Lantern light", "sphere", x, -0.1, z, 0.35, emitter((1.0, 0.6, 0.25), 3, unlit=False)),
          obj("Lantern cap", "cone", x, z, 1.2, stone, stretch=0.3, elevation=1.05),
          block("Veranda", -0.5, 3.4, -1.0, -0.6, 8.0, 1.2, M((0.36, 0.22, 0.12), 0.45, refl=0.05))]
    return o


scene("zen-garden", "Zen garden",
      "Raked sand (a fine stripe pattern), rough rocks, moss and a stone lantern seen from a wooden veranda, "
      "in early morning light from the physical sky: patterns at grazing light.",
      ["open", "pattern", "physical sky"],
      studio(light_on=False, lights=[], sky=0.0, base=[0.82, 0.78, 0.7],
             floor=M((0.82, 0.78, 0.7), 0.9, refl=0.0, pattern="stripes", second=(0.62, 0.58, 0.5), pscale=18.0), floor_extent=6.0,
             physical_sky=sky(7.2, day=120.0, latitude=35.0, turbidity=3.0, north=20.0), exposure=1.4,
             objects=zen_garden(), **camera(20, 24, 7.5, (0, -0.8, 0), 45)))


def lit_maze():
    o = mirror_maze()
    o[-1]["material"] = emitter((0.6, 0.85, 1.0), 14)
    random.seed(30)
    for i, c in enumerate([(1.0, 0.25, 0.5), (1.0, 0.7, 0.1), (0.3, 1.0, 0.5), (0.7, 0.3, 1.0)]):
        a = i / 4 * math.tau + 0.4
        o.append(float_at(f"Orb {i + 1}", "sphere", 3.0 * math.cos(a), -0.6, 3.0 * math.sin(a), 0.5, emitter(c, 12)))
    return o


scene("mirror-maze", "Mirror maze",
      "Chrome walls in a grid around a glowing sphere and four coloured orbs: very long specular paths, "
      "each light repeated down every corridor.",
      ["mirror", "bounces", "emissive"],
      dark_stage(objects=lit_maze(), bounces=24, base=[0.08, 0.08, 0.09], floor=M((0.06, 0.06, 0.07), 0.15, refl=0.3),
                 exposure=1.6, **camera(20, 30, 10, (0, -0.2, 0), 45)))


def lit_tiles():
    o = tiles()
    bar = next(x for x in o if x["name"] == "Glow bar")
    bar["material"] = emitter((1.0, 0.75, 0.45), 16)
    return o


scene("glossy-floor-tiles", "Glossy floor tiles",
      "A strip of black tiles from mirror to matte under a glowing bar: the reflection blurs and spreads as the "
      "roughness rises from left to right.",
      ["material", "roughness"],
      studio(light_on=False, sky=0.02, base=[0.08, 0.08, 0.09], objects=lit_tiles(), exposure=1.3,
             lights=[light("Fill", (0, 3.0, 4.0), (4.0, 2.0), (0.8, 0.85, 1.0), 1.2, target=(0, -1, 0))],
             **camera(0, 12, 6.5, (0, -0.7, 0), 45)))

scene("scattering-materials", "Scattering materials",
      "Objects filled with participating media against a dark backdrop, backlit to show light passing through "
      "them: milk, jade, smoke in an invisible box (IOR 1), candle wax, a ruby syrup, and opal glass whose "
      "extinction removes blue first, so it glows orange where the light comes through and blue where it scatters.",
      ["volume", "subsurface", "material"],
      studio(light_position=[0.0, 2.5, -4.0], light_size=3.0, light_intensity=24.0, sky=0.02,
             lights=[light("Front fill", (2.0, 2.5, 4.0), (2, 2), (1.0, 0.95, 0.9), 2.0, target=(0, -0.5, 0)),
                     light("Backlight", (0, 0.2, -3.0), (8.0, 1.2), (1.0, 0.97, 0.92), 6.0, target=(0, -0.4, 0))],
             floor=M((0.06, 0.06, 0.07), 0.4, refl=0.06), base=[0.03, 0.03, 0.035], bounces=32,
             objects=[obj("Milk", "sphere", -2.9, 0, 1.2, volume(M((1, 1, 1), 0.0, trans=1, ior=1.35), 40.0, (0.999, 0.998, 0.995))),
                      obj("Jade", "bunny", -1.65, 0, 1.5, volume(M((0.55, 0.85, 0.6), 0.15, trans=1, ior=1.62), 12.0, (0.7, 0.95, 0.75), 0.2), rotation=25),
                      obj("Smoke", "box", -0.4, 0, 1.2, volume(M((1, 1, 1), 0.0, trans=1, ior=1.0), 3.0, (0.8, 0.8, 0.8), 0.4)),
                      obj("Wax", "cylinder", 0.75, 0, 1.0, volume(M((1.0, 0.9, 0.75), 0.3, trans=1, ior=1.45), 25.0, (0.98, 0.85, 0.6)), stretch=1.6),
                      obj("Syrup", "sphere", 1.9, 0, 1.0, volume(M((0.8, 0.12, 0.08), 0.0, trans=1, ior=1.45), 6.0, (0.95, 0.3, 0.2), 0.3)),
                      obj("Opal glass", "sphere", 3.0, 0, 1.1, volume(M((1, 1, 1), 0.0, trans=1, ior=1.47), 18.0, (0.995, 0.995, 0.995))
                          | {"extinction": [0.3, 0.55, 1.0]})],
             **camera(0, 8, 7.8, (0, -0.4, 0), 42)))


def skyline_night():
    o = skyline()
    tall = sorted(range(len(o)), key=lambda i: -o[i]["stretch"])[:10]
    for k, i in enumerate(tall):
        x, z = o[i]["position"]
        top = -1 + 0.7 * o[i]["stretch"] * o[i]["size"]
        o.append(float_at(f"Beacon {k + 1}", "sphere", x, top + 0.06, z, 0.25, emitter((1.0, 0.08, 0.05), 20)))
    o.append(float_at("Moon", "sphere", -30, 16, -30, 8.0, emitter((0.85, 0.9, 1.0), 2.5)))
    return o


scene("night-skyline", "Night skyline",
      "About 150 towers with glowing window bands, red beacons on the tallest and a low moon, over a wet dark "
      "plaza: hundreds of emissive triangles, with a touch of lens glare around the brightest lights.",
      ["city", "night", "emissive", "glare"],
      studio(light_on=False, lights=[], sky=0.03, background=[0.01, 0.015, 0.04], base=[0.03, 0.03, 0.04],
             floor=M((0.03, 0.03, 0.04), 0.15, refl=0.35), floor_extent=40.0, objects=skyline_night(), exposure=1.5,
             glare=0.25, glare_threshold=3.0, **camera(30, 22, 22, (0, 1.0, 0), 45)))

# A slow walk down the gates: the eye and the look-at point follow a spline through the keys.
scene("torii-corridor", "Torii corridor",
      "Twelve red gates receding toward a setting sun: repetition, perspective and colour bleeding. The "
      "timeline (K) walks through the gates on a camera path (a spline through the keyed views).",
      ["architecture", "sunset", "animation", "camera path"],
      studio(light_on=False, sky=0.2, background=[0.2, 0.1, 0.08], base=[0.35, 0.3, 0.26],
             floor=M((0.4, 0.35, 0.3), 0.85, refl=0.02), floor_extent=40.0, objects=torii(),
             lights=[light("Sunset", (0, 1.8, -36), (8, 4), kelvin(2300), 300, target=(0, 0, 0), visible=True)],
             animation={"length": 12.0, "fps": 24, "shutter": 0.0, "camera_path": True,
                        "tracks": [track("target/2", [(0, 0.0, "smooth"), (4, -6.0, "smooth"), (8, -12.0, "smooth"), (12, -18.0, "smooth")]),
                                   track("target/0", [(0, 0.0, "smooth"), (4, 0.35, "smooth"), (8, -0.35, "smooth"), (12, 0.0, "smooth")]),
                                   track("yaw", [(0, 0.0, "smooth"), (4, 6.0, "smooth"), (8, -6.0, "smooth"), (12, 0.0, "smooth")])]},
             **camera(0, 3, 6, (0, 0.3, 0), 50)))


def orrery_script():
    """Rhai run every frame (Animation::script): each planet circles the sun at Kepler's rate."""
    o = orrery()
    rows = []
    for i, x in enumerate(o):
        if x["name"].startswith("Planet "):
            px, pz = x["position"]
            dist, a0 = math.hypot(px, pz), math.atan2(pz, px)
            rows.append(f"[{i}, {dist:.4f}, {a0:.4f}, {1.6 / dist ** 1.5:.4f}]")
            if x["name"] == "Planet 6":
                ring = next(j for j, y in enumerate(o) if y["name"] == "Saturn's rings")
                rows.append(f"[{ring}, {dist:.4f}, {a0:.4f}, {1.6 / dist ** 1.5:.4f}]")
    script = ("let planets = [" + ", ".join(rows) + "];\n"
              "for p in planets {\n"
              "    let a = p[2] + p[3] * t;\n"
              "    set(`objects/${p[0]}/position/0`, p[1] * a.cos());\n"
              "    set(`objects/${p[0]}/position/1`, p[1] * a.sin());\n"
              "}\n")
    return o, {"length": 30.0, "fps": 24, "shutter": 0.0, "tracks": [], "script": script}


_o, _a = orrery_script()
scene("orrery", "Orrery",
      "A glowing sun lights its planets and their brass orbits: one central emitter for the whole scene. The "
      "timeline (K) runs a per-frame script that moves each planet round its orbit at Kepler's rate.",
      ["still life", "emissive", "animation", "script"],
      dark_stage(objects=_o, animation=_a, base=[0.12, 0.1, 0.09], floor=M((0.12, 0.1, 0.09), 0.3, refl=0.1),
                 **camera(25, 32, 12, (0, -0.2, 0), 45)))


def museum_ies():
    o, l = bunny_museum()
    # The gallery's track spots as IES-shaped disks (a narrow beam with a soft field).
    l = [disk(x["name"], x["position"], None, 0.16, kelvin(4200), 400, ies="ies/narrow-spot.ies") | {"rotation": x["rotation"]}
         for x in l]
    return o, l


_o, _l = museum_ies()
scene("bunny-museum", "Bunny museum",
      "Six Stanford bunnies on plinths in glass, gold, jade, marble, chrome and rubber, each under its own "
      "IES-profiled track spot in a dark gallery: measured beam shapes on the plinths and the floor.",
      ["material", "mesh", "gallery", "ies"],
      dark_stage(lights=_l, objects=_o, base=[0.12, 0.11, 0.1], floor=M((0.12, 0.11, 0.1), 0.25, refl=0.15), bounces=16,
                 **camera(0, 12, 8.5, (0, 0.6, -0.8), 45)))

scene("sdf-gallery", "Procedural surfaces",
      "Shapes intersected in the shader, with no triangles: an exact glass sphere, gold torus, copper cylinder "
      "and jade cone (no facets in the refraction or the highlights), a Mandelbulb and a level-5 Menger sponge, "
      "both sphere traced.",
      ["procedural", "sdf", "fractal"],
      studio(objects=[sdf(obj("Exact glass sphere", "sphere", -2.4, 0.3, 1.3, GLASS), "sphere"),
                      sdf(obj("Exact gold torus", "torus", -0.8, -0.4, 1.4, GOLD, tilt=(70, 0), elevation=0.2), "torus"),
                      sdf(obj("Mandelbulb", "sphere", 0.9, 0.2, 1.6, M((0.8, 0.45, 0.25), 0.35, refl=0.08), rotation=20), "mandelbulb"),
                      sdf(obj("Menger sponge", "box", 2.6, -0.3, 1.2, M((0.82, 0.8, 0.76), 0.6), rotation=30), "menger"),
                      sdf(obj("Exact copper cylinder", "cylinder", -1.6, 1.4, 0.9, COPPER, stretch=1.4), "cylinder"),
                      sdf(obj("Exact jade cone", "cone", 1.7, 1.3, 1.0, M((0.2, 0.6, 0.35), 0.0, trans=1, ior=1.6)), "cone")],
             bounces=12, **camera(0, 18, 8.6, (0.1, -0.3, 0.4), 40)))

scene("stage-beams", "Stage beams",
      "Three coloured spotlights cut through drifting smoke on a black stage onto a satin metal sphere and a "
      "microphone: the smoke's density varies (fBm noise, sampled by delta tracking), so the beams break up "
      "into drifting wisps. Set Fog → Noise to 0 for a clean haze.",
      ["volume", "fog", "spot", "lights", "noise"],
      dark_stage(fog=fog_ex(0.14, (0.9, 0.9, 0.9), 0.6, 6.0, 10.0, noise=0.75, noise_size=0.9),
                 floor=M((0.08, 0.08, 0.08), 0.3, refl=0.06), bounces=8, exposure=2.0,
                 lights=[spot("Red", (-2.5, 4.5, -1.0), (-0.4, -1, 0), (1.0, 0.15, 0.08), 220.0),
                         spot("Green", (0.0, 4.8, -2.2), (0, -1, 0.1), (0.15, 1.0, 0.25), 220.0),
                         spot("Blue", (2.5, 4.5, -1.0), (0.4, -1, 0), (0.12, 0.3, 1.0), 220.0)],
                 objects=[obj("Satin metal sphere", "sphere", 0, 0, 1.3, M((0.92, 0.94, 0.97), 0.3, metal=1)),
                          cyl("Mic stand", 1.1, 0.6, -1.0, 0.4, 0.015, M((0.1, 0.1, 0.1), 0.3, metal=1)),
                          cyl("Mic stand foot", 1.1, 0.6, -1.0, -0.97, 0.2, M((0.1, 0.1, 0.1), 0.3, metal=1)),
                          ball("Microphone", 1.1, 0.48, 0.6, 0.07, M((0.6, 0.6, 0.62), 0.35, metal=1), stretch=1.4)],
                 **camera(0, 4, 9, (0, 1.2, 0), 45)))

scene("veach-ajar", "Veach ajar room",
      "All light comes through a door left slightly open: almost every path is indirect. Bidirectional path "
      "tracing connects light paths through the gap and converges quickly; switch Render → Integrator to path "
      "tracing to see the noise it removes, and what the denoisers do with it.",
      ["validation", "indirect", "veach", "interior", "bdpt"],
      studio(light_on=False, sky=0.0, floor_extent=6.0, base=[0.55, 0.5, 0.45], bounces=16, exposure=1.6, integrator=1,
             lights=[light("Outside", (1.6, 0.6, -5.0), (3.0, 2.5), kelvin(5000), 60, target=(0.9, 0.0, -3.0))],
             objects=ajar_room(), **camera(20, 6, 3.0, (0.4, 0.1, -1.0), 65)))


# ---------------------------------------------------------------- new: lenses and camera models
def street_bokeh():
    random.seed(77)
    stone = coat(M((0.12, 0.12, 0.13), 0.5, refl=0.05), 1.0, 0.05)
    brass = M((0.9, 0.68, 0.35), 0.25, metal=1)
    o = [cyl("Table top", 0, 0, -0.3, -0.26, 0.5, stone), cyl("Table stem", 0, 0, -1.0, -0.3, 0.04, brass),
         cyl("Table foot", 0, 0, -1.0, -0.97, 0.28, brass),
         cyl("Wine glass bowl", -0.12, 0.05, -0.12, 0.06, 0.08, CRYSTAL), cyl("Wine glass stem", -0.12, 0.05, -0.26, -0.12, 0.012, CRYSTAL),
         cyl("Wine", -0.12, 0.05, -0.1, -0.02, 0.07, clear((0.55, 0.03, 0.08), 1.34, 2)),
         cyl("Candle jar", 0.16, -0.08, -0.26, -0.12, 0.06, CRYSTAL),
         ball("Candle flame", 0.16, -0.15, -0.08, 0.018, emitter((1.0, 0.55, 0.2), 20), stretch=1.8)]
    colors = [(1.0, 0.75, 0.4), (1.0, 0.55, 0.25), (0.4, 0.7, 1.0), (1.0, 0.3, 0.55), (0.5, 1.0, 0.7)]
    for i in range(40):
        z = -random.uniform(5, 32)
        x = random.uniform(-0.5, 0.5) * (4 - z * 0.5)
        o.append(ball(f"City light {i + 1}", x, random.uniform(-0.2, 3.5), z, random.uniform(0.06, 0.16),
                      emitter(random.choice(colors), random.uniform(10, 20))))
    for i in range(6):
        o.append(float_at(f"Street lamp {i + 1}", "sphere", -3.0 + (i % 2) * 6.0, 2.6, -4 - i * 4.5, 0.5,
                          emitter((1.0, 0.7, 0.35), 18)))
    return o


scene("street-bokeh", "Street bokeh",
      "A café table at night in front of a street of lights, through a real Double Gauss 50 mm wide open: "
      "the lights far behind melt into discs whose edges carry the glass's dispersion, and glare blooms around "
      "the brightest. Lens systems trace the actual elements (Look → Lens).",
      ["lens", "bokeh", "depth of field", "night", "glare"],
      dark_stage(objects=street_bokeh(), floor=M((0.02, 0.02, 0.025), 0.1, refl=0.5), base=[0.02, 0.02, 0.025],
                 floor_extent=40.0, bounces=12, exposure=1.6, lens_system=1, f_number=1.4, lens_scale=1.0, dispersion=1.0,
                 focus_distance=1.9, glare=0.35, glare_threshold=2.0,
                 lights=[light("Café window", (2.5, 0.8, 1.5), (1.2, 1.5), kelvin(3000), 4.0, target=(0, -0.3, 0))],
                 **camera(-8, 6, 1.9, (0, -0.2, 0), 40)))


def village():
    random.seed(31)
    walls = [(0.92, 0.88, 0.8), (0.95, 0.8, 0.65), (0.85, 0.9, 0.95), (0.98, 0.93, 0.7), (0.9, 0.75, 0.7)]
    roofs = [(0.55, 0.18, 0.1), (0.25, 0.27, 0.3), (0.45, 0.25, 0.12)]
    o = [block("River", 0.9, 0, -1.0, -0.955, 1.1, 15.0, clear((0.55, 0.75, 0.8), 1.33, 0), rotation=8),
         block("Bridge", 0.9, 0.4, -0.95, -0.85, 1.6, 0.5, M((0.5, 0.45, 0.4), 0.8), rotation=8)]
    for gx in range(-4, 5):
        for gz in range(-3, 4):
            x, z = gx * 1.3 + random.uniform(-0.2, 0.2), gz * 1.4 + random.uniform(-0.2, 0.2)
            if abs(x - 0.9 - z * 0.14) < 1.0 or random.random() < 0.15:
                continue
            if random.random() < 0.3:
                h = random.uniform(0.5, 0.9)
                o.append(cyl("Trunk", x, z, -1.0, -1.0 + h * 0.4, 0.04, matte((0.3, 0.2, 0.1))))
                o.append(obj("Crown", "cone", x, z, h * 0.9, matte((0.12, 0.35 + random.uniform(0, 0.15), 0.1), 0.8), stretch=1.6,
                             elevation=h * 0.3))
                continue
            w, d, h = random.uniform(0.5, 0.75), random.uniform(0.6, 0.9), random.uniform(0.4, 0.7)
            rot = random.choice([0, 90]) + random.uniform(-6, 6)
            o.append(block("House", x, z, -1.0, -1.0 + h, w, d, matte(random.choice(walls), 0.8), rotation=rot))
            o += roof("Roof", x, z, -1.0 + h, w, d, 38, matte(random.choice(roofs), 0.7), rotation=rot)
    o.append(cyl("Church tower", -2.6, -1.4, -1.0, 0.6, 0.22, matte((0.9, 0.88, 0.82), 0.8)))
    o.append(obj("Spire", "cone", -2.6, -1.4, 0.8, matte((0.3, 0.3, 0.33), 0.6), stretch=1.8, elevation=1.6))
    return o


scene("tilt-shift-village", "Tilt-shift village",
      "A village by a river seen from high above through a tilt-shift lens: the focal plane is tilted, so only "
      "a band across the middle is sharp and the town reads as a miniature (Look → Camera → Tilt-shift).",
      ["camera", "tilt-shift", "depth of field", "physical sky"],
      studio(light_on=False, lights=[], sky=0.0, base=[0.3, 0.42, 0.18], floor=matte((0.3, 0.42, 0.18), 0.95), floor_extent=40.0,
             objects=village(), physical_sky=sky(16.0, day=150.0, latitude=47.0, turbidity=3.0, north=30.0), exposure=1.3,
             projection=4, tilt=14.0, aperture=0.35, focus_distance=13.0, **camera(20, 40, 13, (0, -0.8, 0), 32)))


def clearing():
    random.seed(52)
    o = []
    for i in range(70):
        a = random.uniform(0, math.tau)
        d = random.uniform(3.0, 9.0)
        x, z = d * math.cos(a), d * math.sin(a)
        h = random.uniform(0.8, 1.6)
        o.append(obj(f"Trunk {i + 1}", "cylinder", x, z, 0.9 * h, matte((0.28, 0.17, 0.09), 0.9), scale=(0.15, 0.15), stretch=0.9))
        o.append(obj(f"Crown {i + 1}", "cone", x, z, 1.4 * h, matte((0.05, 0.26 + random.uniform(0, 0.12), 0.07), 0.8), stretch=1.9,
                     elevation=0.35 * 0.9 * h * 0.9 * 1.6))
    o += [block("Cabin", 0.0, -1.9, -1.0, -0.2, 1.4, 1.0, matte((0.45, 0.28, 0.15), 0.8))]
    o += roof("Cabin roof", 0.0, -1.9, -0.2, 1.4, 1.0, 35, matte((0.3, 0.1, 0.06), 0.7))
    o += [obj("Pond", "cylinder", 1.6, 1.0, 3.0, clear((0.6, 0.8, 0.85), 1.33, 0), stretch=0.006, elevation=0.0),
          ball("Camp fire", -1.4, -0.92, 1.2, 0.1, emitter((1.0, 0.45, 0.12), 14))]
    return o


scene("little-planet", "Little planet",
      "A forest clearing with a cabin, a pond and a camp fire, rendered as a little planet: a stereographic "
      "view straight down from just above the ground, so the ground curls into a globe with the sky around it "
      "(Look → Camera → Little planet).",
      ["camera", "projection", "nature", "physical sky"],
      studio(light_on=False, lights=[], sky=0.0, base=[0.25, 0.32, 0.12], floor=matte((0.25, 0.32, 0.12), 0.95), floor_extent=40.0,
             objects=clearing(), physical_sky=sky(9.0, day=160.0, latitude=50.0, turbidity=2.5), exposure=1.3, projection=3,
             **camera(0, 0, 0.5, (0, 0.6, 0), 90)))


def gallery():
    wall = matte((0.9, 0.89, 0.86), 0.9)
    frame = M((0.85, 0.65, 0.3), 0.3, metal=1)
    o = room_shell(10.0, 8.0, 4.0, wall)
    paintings = [(-5.0, 0, 90, (0.7, 0.2, 0.1)), (5.0, 0, -90, (0.1, 0.3, 0.6)), (0, -4.0, 0, (0.85, 0.7, 0.1)),
                 (-2.5, -4.0, 0, (0.1, 0.45, 0.25)), (2.5, -4.0, 0, (0.55, 0.1, 0.4)), (0, 4.0, 180, (0.15, 0.15, 0.2))]
    for i, (x, z, rot, c) in enumerate(paintings):
        nx, nz = -math.sin(math.radians(rot)), -math.cos(math.radians(rot))
        o.append(float_at(f"Frame {i + 1}", "box", x - nx * 0.03, 0.9, z - nz * 0.03, 1, frame, stretch=1.4 / 0.7,
                          scale=(1.9 / 0.7, 0.04 / 0.7), rotation=rot))
        o.append(float_at(f"Painting {i + 1}", "box", x - nx * 0.06, 0.9, z - nz * 0.06, 1,
                          M(c, 0.6, refl=0.02, pattern="marble", second=tuple(min(1, v + 0.35) for v in c), pscale=2.0),
                          stretch=1.2 / 0.7, scale=(1.7 / 0.7, 0.03 / 0.7), rotation=rot))
    for i, (x, z, shape, m) in enumerate([(-2.0, -1.2, "knot", GOLD), (2.0, -1.2, "bunny", JADE),
                                          (-2.0, 1.6, "teapot", PORCELAIN), (2.0, 1.6, "torus", GLASS)]):
        o.append(block(f"Plinth {i + 1}", x, z, -1.0, 0.0, 0.7, 0.7, matte((0.95, 0.95, 0.94), 0.7)))
        o.append(obj(f"Sculpture {i + 1}", shape, x, z, 1.0, m, elevation=1.0 + (0.15 if shape in ("knot", "torus") else 0),
                     tilt=(70, 0) if shape == "torus" else (0, 0)))
    return o


scene("panorama-gallery", "360° gallery",
      "A white gallery with paintings and four sculptures on plinths, under two skylights, rendered as a 360° "
      "equirectangular panorama from the middle of the room (Look → Camera → 360°): the whole room in one frame, "
      "ready for a VR viewer.",
      ["camera", "projection", "interior", "360"],
      studio(light_on=False, sky=0.0, base=[0.6, 0.58, 0.55], floor=M((0.6, 0.58, 0.55), 0.3, refl=0.08), floor_extent=5.0,
             objects=gallery(), bounces=10, exposure=0.35, projection=2,
             lights=[light(f"Skylight {i + 1}", (x, 2.85, 0), (2.5, 2.5), kelvin(5800), 9, visible=True) for i, x in enumerate((-2.5, 2.5))],
             **camera(0, 0, 0.5, (0, 0.5, 0), 90)))


def workshop():
    wall = matte((0.82, 0.78, 0.7), 0.9)
    wood = M((0.45, 0.28, 0.15), 0.5, refl=0.05)
    o = [block("Back wall", 0, -2.05, -1.0, 2.0, 4.2, 0.1, wall), block("Left wall", -2.05, 0, -1.0, 2.0, 0.1, 4.2, wall),
         block("Rug", 0.3, 0.4, -1.0, -0.99, 2.0, 1.4, matte((0.6, 0.25, 0.15), 0.95)),
         block("Desk top", -0.6, -1.5, -0.25, -0.2, 1.8, 0.8, wood)]
    for dx in (-0.85, 0.85):
        for dz in (-0.35, 0.35):
            o.append(block("Desk leg", -0.6 + dx, -1.5 + dz, -1.0, -0.25, 0.06, 0.06, wood))
    o += [block("Monitor", -0.6, -1.75, -0.2, 0.3, 0.7, 0.05, M((0.03, 0.03, 0.03), 0.3)),
          block("Screen", -0.6, -1.72, -0.15, 0.25, 0.62, 0.01, emitter((0.4, 0.65, 1.0), 2.5)),
          block("Chair seat", -0.5, -0.7, -0.55, -0.5, 0.5, 0.5, lacquer((0.1, 0.3, 0.55), 0.4)),
          block("Chair back", -0.5, -0.47, -0.5, 0.0, 0.5, 0.05, lacquer((0.1, 0.3, 0.55), 0.4)),
          cyl("Chair post", -0.5, -0.7, -1.0, -0.55, 0.03, M((0.2, 0.2, 0.2), 0.3, metal=1)),
          block("Shelf", -1.85, 0.5, 0.6, 0.65, 0.35, 2.0, wood)]
    random.seed(4)
    z = -0.4
    while z < 1.4:
        w = random.uniform(0.05, 0.1)
        h = random.uniform(0.22, 0.32)
        o.append(block("Book", -1.85, z, 0.65, 0.65 + h, 0.25, w, lacquer(random.choice([(0.6, 0.1, 0.1), (0.1, 0.3, 0.6), (0.1, 0.45, 0.2),
                                                                                          (0.8, 0.6, 0.1), (0.2, 0.2, 0.22)]), 0.5)))
        z += w + 0.01
    o += [cyl("Plant pot", 1.4, -1.6, -1.0, -0.65, 0.18, matte((0.7, 0.35, 0.2), 0.8)),
          ball("Plant", 1.4, -0.4, -1.6, 0.35, matte((0.15, 0.45, 0.12), 0.85), stretch=1.3),
          cyl("Lamp stand", 0.1, -1.6, -0.2, 0.25, 0.012, M((0.2, 0.2, 0.2), 0.3, metal=1))]
    return o


scene("isometric-workshop", "Isometric workshop",
      "A cut-away room (desk, chair, bookshelf, plant, rug, a glowing monitor and a desk lamp) seen through an "
      "orthographic camera from the classic isometric angle: no perspective, parallel edges stay parallel "
      "(Look → Camera → Orthographic).",
      ["camera", "orthographic", "interior", "sphere light"],
      studio(light_on=False, sky=0.25, background=[0.85, 0.82, 0.78], base=[0.7, 0.66, 0.6], floor=M((0.5, 0.36, 0.22), 0.45, refl=0.06),
             floor_extent=2.1, objects=workshop(), bounces=10, projection=1,
             lights=[sphere_light("Desk lamp", (0.1, 0.32, -1.6), 0.12, (1, 1, 1), 6.0, temperature=2900.0),
                     light("Window", (2.5, 1.5, 0.5), (2.0, 2.0), kelvin(6500), 6.0, target=(-1, -0.5, -1))],
             **camera(45, 35, 9, (0, -0.2, 0), 30)))


# ---------------------------------------------------------------- new: lights
def greenhouse():
    glass = M((0.95, 0.98, 0.96), 0.0, trans=1, ior=1.5)
    iron = M((0.08, 0.2, 0.12), 0.4, metal=0.6, refl=0.1)
    w, d, h = 4.0, 3.0, 2.2
    o = []
    for i in range(5):
        x = -w / 2 + i * w / 4
        for side in (-1, 1):
            o.append(block("Post", x, side * d / 2, -1.0, -1.0 + h, 0.05, 0.05, iron))
        o.append(block("Rafter", x, 0, -1.0 + h, -1.0 + h + 0.05, 0.05, d, iron))
        if i < 4:
            for side in (-1, 1):
                o.append(block("Side pane", x + w / 8, side * d / 2, -1.0, -1.0 + h, w / 4 - 0.06, 0.01, glass))
            o.append(block("Roof pane", x + w / 8, 0, -1.0 + h + 0.05, -1.0 + h + 0.06, w / 4 - 0.06, d, glass))
    for side in (-1, 1):
        for j in range(3):
            z = -d / 2 + (j + 0.5) * d / 3
            o.append(block("End pane", side * w / 2, z, -1.0, -1.0 + h, 0.01, d / 3 - 0.06, glass))
    wood = M((0.5, 0.36, 0.22), 0.7, refl=0.03)
    o.append(block("Bench", 0, -0.9, -0.3, -0.25, 3.4, 0.7, wood))
    for dx in (-1.6, 1.6):
        o.append(block("Bench leg", dx, -0.9, -1.0, -0.3, 0.06, 0.6, wood))
    random.seed(8)
    for i in range(7):
        x = -1.4 + i * 0.47
        o.append(cyl(f"Pot {i + 1}", x, -0.9, -0.25, -0.05, 0.12, matte((0.72, 0.36, 0.2), 0.85)))
        if i % 2:
            o.append(obj(f"Plant {i + 1}", "cone", x, -0.9, random.uniform(0.6, 0.9), matte((0.1, 0.4, 0.12), 0.8), stretch=1.6, elevation=0.95))
        else:
            o.append(ball(f"Plant {i + 1}", x, 0.15, -0.9, random.uniform(0.17, 0.24), matte((0.12, 0.42 + random.uniform(0, 0.1), 0.1), 0.8)))
    o += [cyl("Watering can", 0.9, 0.6, -1.0, -0.7, 0.14, M((0.6, 0.62, 0.6), 0.3, metal=1)),
          float_at("Spout", "cylinder", 1.12, -0.78, 0.6, 1, M((0.6, 0.62, 0.6), 0.3, metal=1), stretch=0.4 / 0.7,
                   scale=(0.05, 0.05), tilt=(0, -55))]
    return o


scene("sun-greenhouse", "Greenhouse in the sun",
      "A glass greenhouse with a potting bench, lit by an analytic sun light (a distant disc 0.53° across, so "
      "shadows have a real penumbra) and a blue sky: the iron frame draws crisp shadows through the panes.",
      ["lights", "sun", "glass", "shadows"],
      studio(light_on=False, sky=0.6, background=[0.45, 0.62, 0.9], base=[0.5, 0.46, 0.38], floor=M((0.45, 0.4, 0.32), 0.9),
             floor_extent=40.0, objects=greenhouse(), bounces=12, exposure=0.45,
             lights=[sun_light("Sun", (-18, 22, 12), kelvin(5600), 4.0)],
             **camera(30, 16, 7.5, (0, 0.0, 0), 42)))


def bulbs():
    brick = M((0.42, 0.18, 0.12), 0.9, pattern="checker", second=(0.35, 0.15, 0.1), pscale=8.0)
    wood = M((0.42, 0.28, 0.16), 0.6, refl=0.04)
    steel = M((0.7, 0.7, 0.72), 0.25, metal=1)
    o = [block("Back wall", 0, -1.6, -1.0, 2.6, 6.0, 0.1, brick),
         block("Bench top", 0, -0.9, -0.15, -0.08, 3.6, 1.0, wood)]
    for dx in (-1.6, 1.6):
        o.append(block("Bench leg", dx, -0.9, -1.0, -0.15, 0.08, 0.9, wood))
    o += [block("Crate", -1.2, -1.0, -0.08, 0.3, 0.6, 0.5, M((0.55, 0.4, 0.22), 0.8)),
          cyl("Hammer handle", 0.3, -0.75, -0.06, -0.02, 0.02, wood, tilt=(0, 90)),
          block("Hammer head", 0.6, -0.75, -0.08, 0.0, 0.08, 0.2, steel),
          float_at("Wrench", "box", -0.3, -0.07, -0.6, 1, steel, stretch=0.02 / 0.7, scale=(0.5 / 0.7, 0.06 / 0.7), rotation=25),
          obj("Spool", "torus", 1.0, -1.0, 0.5, lacquer((0.7, 0.1, 0.05), 0.4), elevation=0.95),
          cyl("Oil can", 1.4, -0.7, -0.08, 0.15, 0.08, lacquer((0.8, 0.55, 0.05), 0.35))]
    for i in range(5):
        x = -1.6 + i * 0.8
        y = 1.0 + 0.25 * math.cos(i * 1.3)
        o.append(cyl(f"Cord {i + 1}", x, -0.6, y + 0.1, 2.6, 0.006, matte((0.05, 0.05, 0.05))))
        o.append(cyl(f"Socket {i + 1}", x, -0.6, y + 0.04, y + 0.12, 0.025, M((0.6, 0.5, 0.3), 0.3, metal=1)))
    return o


scene("bulb-workshop", "Bulb workshop",
      "A workbench under five bare bulbs on cords, each a sphere light at its own blackbody temperature from "
      "2200 K to 3200 K: small round sources, soft-edged shadows of the tools, and the warm pools overlapping on "
      "the brick.",
      ["lights", "sphere light", "kelvin", "interior"],
      dark_stage(objects=bulbs(), base=[0.12, 0.1, 0.08], floor=M((0.12, 0.1, 0.08), 0.6, refl=0.04), bounces=10, exposure=1.3,
                 lights=[sphere_light(f"Bulb {i + 1}", (-1.6 + i * 0.8, 1.0 + 0.25 * math.cos(i * 1.3), -0.6), 0.11, (1, 1, 1), 2.5,
                                      temperature=2200.0 + i * 250.0) for i in range(5)],
                 **camera(10, 8, 4.2, (0, 0.2, -0.6), 45)))


def crypt():
    stone = M((0.42, 0.4, 0.37), 0.85, refl=0.02)
    o = room_shell(6.0, 14.0, 4.0, stone)
    for i in range(6):
        z = -5.5 + i * 2.2
        for side in (-1, 1):
            o.append(cyl(f"Column {i + 1}{'LR'[side > 0]}", side * 1.6, z, -1.0, 3.0, 0.22, stone))
        if i % 2 == 0:
            o.append(block(f"Sarcophagus {i // 2 + 1}", -2.3, z + 1.1, -1.0, -0.2, 0.9, 1.9, MARBLE))
            o.append(block(f"Lid {i // 2 + 1}", -2.3, z + 1.1, -0.2, -0.05, 1.0, 2.0, MARBLE))
        else:
            o.append(obj(f"Urn {i // 2 + 1}", "pawn", 2.3, z + 1.1, 1.0, M((0.6, 0.35, 0.2), 0.4, metal=1), stretch=1.4))
    o += [block("Altar", 0, -6.4, -1.0, 0.0, 2.0, 0.8, MARBLE),
          obj("Chalice", "pawn", 0, -6.4, 0.6, GOLD, elevation=1.0),
          block("Moon slit", 0, -6.95, 1.8, 2.8, 0.25, 0.12, emitter((0.6, 0.7, 1.0), 4))]
    return o


scene("flashlight-crypt", "Flashlight crypt",
      "A vaulted crypt in the dark: columns, marble sarcophagi, copper urns and an altar under a thin slit of "
      "moonlight. The flashlight is on (Render → Lights): a spot rides with the camera, so orbiting sweeps its "
      "beam across the stone.",
      ["lights", "flashlight", "interior", "night"],
      studio(light_on=False, lights=[], sky=0.0, background=[0.0, 0.0, 0.0], base=[0.35, 0.33, 0.3],
             floor=M((0.35, 0.33, 0.3), 0.75, refl=0.03), floor_extent=7.0, objects=crypt(), bounces=8, flashlight=True,
             exposure=1.4, **camera(0, 4, 3.0, (0, 0.2, 2.0), 55)))


# ---------------------------------------------------------------- new: volumes and emission
def lighthouse():
    random.seed(19)
    red, white = lacquer((0.7, 0.06, 0.04), 0.45), lacquer((0.92, 0.9, 0.86), 0.45)
    rock = M((0.2, 0.2, 0.22), 0.8, refl=0.03)
    o = []
    for i in range(6):
        r = 0.75 - i * 0.06
        o.append(cyl(f"Tower {i + 1}", 0, 0, -1.0 + i, i, r, red if i % 2 else white))
    o += [cyl("Gallery", 0, 0, 6.0 - 1.0, 5.1, 0.7, M((0.1, 0.1, 0.1), 0.4, metal=1)),
          cyl("Lantern room", 0, 0, 5.1, 5.9, 0.42, M((0.95, 0.97, 1.0), 0.0, trans=1, ior=1.5)),
          obj("Cap", "cone", 0, 0, 1.4, M((0.1, 0.1, 0.1), 0.4, metal=1), stretch=0.7, elevation=6.9),
          # Below the spot, not around it: an emitter enclosing the spot would shadow its beam.
          ball("Lamp", 0, 5.28, 0, 0.08, emitter((1.0, 0.9, 0.7), 20))]
    for i in range(14):
        a = random.uniform(0, math.tau)
        d = random.uniform(1.0, 3.0)
        o.append(obj(f"Rock {i + 1}", "octahedron", d * math.cos(a), d * math.sin(a), random.uniform(0.8, 2.2), rock,
                     stretch=random.uniform(0.4, 0.8), rotation=random.uniform(0, 90), tilt=(random.uniform(-20, 20), random.uniform(-20, 20))))
    return o


scene("lighthouse-fog", "Lighthouse in fog",
      "A lighthouse on its rocks on a foggy night, its beam turning (play the timeline, K): the fog is "
      "heterogeneous (fBm noise, delta tracked) and removes red faster than blue, so the beam glows warm "
      "near the lamp and the distance turns blue-grey.",
      ["volume", "fog", "noise", "spot", "animation", "night"],
      dark_stage(objects=lighthouse(), base=[0.01, 0.015, 0.025], floor=M((0.01, 0.015, 0.025), 0.05, refl=0.6),
                 floor_extent=40.0, sky=0.01, background=[0.004, 0.006, 0.012], bounces=8, exposure=2.6,
                 fog=fog_ex(0.07, (0.9, 0.93, 1.0), 0.7, 14.0, 30.0, noise=0.8, noise_size=3.0,
                            extinction=(1.0, 0.85, 0.7)),
                 lights=[spot("Beam", (0.0, 5.5, 0.0), (10.0, 5.2, 0.0), (1.0, 0.92, 0.75), 2000.0, spread=9.0, blend=0.4, size=0.2),
                         sphere_light("Lamp glow", (0.0, 5.28, 0.0), 0.08, (1.0, 0.9, 0.7), 30.0, visible=False)],
                 # The beam leaves through the lantern-room glass.
                 glass_shadow=False,
                 animation={"length": 8.0, "fps": 24, "shutter": 0.0,
                            "tracks": [track("lights/0/rotation/1", [(0.0, -90.0, "linear"), (8.0, 270.0, "linear")])]},
                 **camera(-25, 4, 17, (0, 2.6, 0), 45)))


def forge():
    random.seed(41)
    iron = M((0.08, 0.08, 0.09), 0.45, metal=1)
    brick = M((0.3, 0.12, 0.08), 0.9)
    o = [block("Hearth", -1.6, -0.8, -1.0, -0.2, 1.4, 1.0, brick),
         block("Hood", -1.6, -1.0, 1.1, 1.7, 1.4, 0.8, iron),
         block("Back wall", 0, -1.6, -1.0, 2.5, 6.0, 0.1, matte((0.16, 0.13, 0.11), 0.9))]
    for i in range(40):
        x, z = -1.6 + random.uniform(-0.5, 0.5), -0.8 + random.uniform(-0.35, 0.35)
        k = random.uniform(1000, 1350)
        o.append(ball(f"Coal {i + 1}", x, -0.16, z, random.uniform(0.05, 0.09), hot(matte((0.05, 0.05, 0.05), 0.9), k, (k - 950) / 60)))
    # The anvil: a block on a stump, a bar at forging heat on it.
    o += [cyl("Stump", 0.4, 0.0, -1.0, -0.4, 0.3, M((0.35, 0.22, 0.12), 0.8)),
          block("Anvil waist", 0.4, 0.0, -0.4, -0.15, 0.25, 0.2, iron),
          block("Anvil face", 0.4, 0.0, -0.15, 0.0, 0.7, 0.26, iron),
          obj("Anvil horn", "cone", 0.92, 0.0, 0.4, iron, stretch=1.0, tilt=(0, -90), elevation=0.92),
          block("Hot bar", 0.4, 0.02, 0.0, 0.04, 0.6, 0.05, hot(iron, 1500, 14), rotation=10),
          cyl("Tongs", 0.95, 0.15, -0.02, 0.0, 0.012, iron, tilt=(0, 80), rotation=20)]
    # A rack of bars cooling from yellow heat to a dull red.
    for i, (k, e) in enumerate([(1800, 20), (1500, 12), (1250, 6), (1100, 2.5), (1000, 0.9)]):
        o.append(block(f"Bar {k} K", 1.8, -0.9 + i * 0.22, -0.4, -0.36, 0.7, 0.05, hot(iron, k, e)))
    o.append(block("Rack", 1.8, -0.45, -1.0, -0.4, 0.8, 1.1, iron))
    for i in range(18):
        o.append(ball(f"Spark {i + 1}", 0.4 + random.uniform(-0.6, 0.6), random.uniform(0.05, 0.9), random.uniform(-0.4, 0.4),
                      0.006, hot(matte((0, 0, 0)), random.uniform(1700, 2100), 20)))
    return o


scene("blackbody-forge", "Blackbody forge",
      "A smithy lit by what is hot: coals, a bar on the anvil and a rack of bars cooling from 1800 K to 1000 K, "
      "each emitting as a blackbody at its temperature (Planck's law; the spectral tracer, on here, emits the "
      "exact spectrum), from yellow-white to a dull red.",
      ["emissive", "blackbody", "spectral", "interior"],
      dark_stage(objects=forge(), base=[0.1, 0.09, 0.08], floor=M((0.1, 0.09, 0.08), 0.7, refl=0.03), bounces=10, spectral=True,
                 exposure=1.2, lights=[light("Window", (3.0, 2.0, 1.0), (1.0, 1.5), kelvin(9000), 0.8, target=(0, -0.5, 0))],
                 **camera(15, 12, 4.6, (0.2, -0.2, -0.3), 45)))


def diamonds():
    velvet = sheen(matte((0.01, 0.01, 0.012), 1.0), (0.35, 0.35, 0.45), 1.0, 0.3)
    diamond = {**M((1, 1, 1), 0.0, trans=1, ior=2.42), "dispersion": 55.0}
    flint = {**M((0.98, 0.99, 1.0), 0.0, trans=1, ior=1.62), "dispersion": 36.0}
    o = [block("Velvet", 0, 0, -1.0, -0.97, 4.0, 3.0, velvet)]
    for i, (x, z, s, rot) in enumerate([(-0.5, 0.1, 0.9, 20), (0.35, -0.2, 0.6, 50), (0.75, 0.45, 0.45, 5), (-0.1, 0.65, 0.35, 70)]):
        o.append(float_at(f"Diamond {i + 1}", "octahedron", x, -0.97 + 0.35 * s * 1.2, z, s, diamond, stretch=1.2, rotation=rot))
    o.append(obj("Flint prism", "prism", -1.2, -0.6, 0.8, flint, rotation=30, elevation=0.03))
    return o


scene("diamond-fire", "Diamond fire",
      "Diamonds (IOR 2.42, Abbe number 55) and a flint-glass prism on black velvet under tiny hard spots, "
      "rendered spectrally: each wavelength bends by its own index, so the facets throw coloured fire that RGB "
      "rendering cannot show.",
      ["glass", "dispersion", "spectral", "caustics"],
      dark_stage(objects=diamonds(), base=[0.01, 0.01, 0.012], floor=M((0.01, 0.01, 0.012), 0.6), bounces=24, spectral=True,
                 exposure=1.3,
                 lights=[disk(f"Spot {i + 1}", p, (0, -0.8, 0), 0.06, kelvin(6500), 900.0) for i, p in
                         enumerate([(-1.5, 2.5, 1.5), (1.8, 2.2, 0.8), (0.2, 2.8, -1.8)])],
                 **camera(10, 32, 3.0, (0, -0.8, 0.1), 32)))


# ---------------------------------------------------------------- new: film look
def diner():
    chrome = M((0.92, 0.94, 0.97), 0.05, metal=1)
    red = coat(lacquer((0.65, 0.04, 0.05), 0.4), 1.0, 0.05)
    o = [block("Back wall", 0, -2.0, -1.0, 2.2, 7.0, 0.1, matte((0.75, 0.7, 0.6), 0.85)),
         block("Counter", 0, -0.6, -1.0, -0.05, 4.4, 0.6, red),
         block("Counter top", 0, -0.6, -0.05, 0.0, 4.6, 0.75, coat(matte((0.9, 0.9, 0.88), 0.4), 1.0, 0.02)),
         block("Counter trim", 0, -0.29, -0.15, -0.08, 4.4, 0.02, chrome),
         block("Back counter", 0, -1.75, -1.0, -0.1, 6.0, 0.4, M((0.5, 0.5, 0.52), 0.25, metal=1))]
    for i in range(5):
        x = -1.8 + i * 0.9
        o += [cyl(f"Stool post {i + 1}", x, 0.2, -1.0, -0.42, 0.03, chrome),
              cyl(f"Stool seat {i + 1}", x, 0.2, -0.42, -0.34, 0.19, red),
              cyl(f"Stool ring {i + 1}", x, 0.2, -0.36, -0.34, 0.2, chrome)]
    o += [cyl("Milkshake glass", -0.6, -0.6, 0.0, 0.3, 0.07, CRYSTAL),
          cyl("Milkshake", -0.6, -0.6, 0.02, 0.24, 0.06, volume(M((1.0, 0.75, 0.8), 0.3, trans=1, ior=1.35), 30.0, (1.0, 0.85, 0.9))),
          ball("Cherry", -0.6, 0.27, -0.6, 0.03, lacquer((0.7, 0.02, 0.03), 0.15)),
          cyl("Napkin box", 0.6, -0.6, 0.0, 0.14, 0.08, chrome)]
    for i, (y, c) in enumerate([(1.7, (1.0, 0.15, 0.5)), (1.5, (0.1, 0.9, 1.0))]):
        o.append(float_at(f"Neon bar {i + 1}", "cylinder", 0, y, -1.93, 1, emitter(c, 10), stretch=4.0 / 0.7, scale=(0.05, 0.05), tilt=(0, 90)))
    o.append(float_at("Neon ring", "ring", 2.2, 1.1, -1.93, 1.2, emitter((1.0, 0.55, 0.1), 10), tilt=(90, 0)))
    return o


scene("neon-diner", "Neon diner",
      "A late-night diner counter with chrome stools, a milkshake and neon on the wall, finished like a film "
      "print: halation (red glow around the neon), grain, a vignette, a warm grade and a .cube LUT "
      "(scenes/luts/warm-film.cube) at 70% (Look → Film).",
      ["interior", "night", "emissive", "film look", "lut"],
      dark_stage(objects=diner(), base=[0.8, 0.8, 0.78],
                 floor=M((0.85, 0.84, 0.8), 0.2, refl=0.15, pattern="checker", second=(0.03, 0.03, 0.03), pscale=2.5), floor_extent=3.5,
                 bounces=10, exposure=1.2, halation=0.5, grain=0.3, grain_size=1, vignette=0.35,
                 lift=[0.01, 0.006, 0.0], gain=[1.04, 1.0, 0.93], lut_path="luts/warm-film.cube", lut_strength=0.7,
                 lights=[sphere_light(f"Pendant {i + 1}", (-1.5 + i * 1.5, 1.4, -0.6), 0.14, (1, 1, 1), 5.0, temperature=2700.0)
                         for i in range(3)],
                 **camera(-18, 8, 4.6, (0, 0.0, -0.6), 45)))


# ---------------------------------------------------------------- new: physics
def banners():
    o = []
    wood = M((0.4, 0.27, 0.15), 0.7)
    for side in (-1, 1):
        o.append(cyl(f"Pole {'LR'[side > 0]}", side * 3.2, 0, -1.0, 2.2, 0.06, wood) | {"body": "fixed"})
    o.append(float_at("Rope", "cylinder", 0, 2.0, 0, 1, matte((0.85, 0.8, 0.7)), stretch=6.4 / 0.7, scale=(0.02, 0.02), tilt=(0, 90))
             | {"body": "none"})
    colors = [(0.85, 0.1, 0.08), (0.95, 0.65, 0.05), (0.1, 0.55, 0.2), (0.1, 0.3, 0.8), (0.6, 0.1, 0.6), (0.95, 0.95, 0.92)]
    for i, c in enumerate(colors):
        x = -2.5 + i * 1.0
        banner = float_at(f"Banner {i + 1}", "box", x, 1.6, 0, 1.15, sheen(matte(c, 0.9), (1, 1, 1), 0.4, 0.5), tilt=(90, 0))
        banner["cloth"] = {"resolution": 20, "pin": "edge", "stiffness": 0.15, "mass": 0.3, "self_collision": False}
        o.append(banner)
    return o


scene("festival-banners", "Festival banners",
      "Six cloth banners pinned along a rope between two poles, blowing in a gusting wind: cloth simulation "
      "(each a particle grid with bending stiffness) under the physical sky. Pause physics to freeze a gust.",
      ["physics", "cloth", "wind", "physical sky"],
      studio(light_on=False, lights=[], sky=0.0, base=[0.3, 0.42, 0.18], floor=matte((0.3, 0.42, 0.18), 0.95), floor_extent=40.0,
             objects=banners(), physical_sky=sky(14.0, day=172.0, latitude=45.0, turbidity=2.5, north=-30.0), exposure=1.3,
             physics={"enabled": True, "wind": [1.6, 0.0, 0.7], "gusts": 0.7},
             **camera(-15, 8, 7.5, (0, 0.9, 0), 45)))


def lantern_chain():
    dark = M((0.42, 0.36, 0.3), 0.85)
    iron = M((0.12, 0.12, 0.13), 0.4, metal=1)
    o = [block("Left wall", -1.4, 0, -1.0, 3.5, 0.3, 8.0, dark) | {"body": "fixed"},
         block("Right wall", 1.6, 0, -1.0, 3.5, 0.3, 8.0, dark) | {"body": "fixed"},
         block("Bracket", -1.0, 0, 2.55, 2.65, 0.5, 0.06, iron) | {"body": "fixed"}]
    anchor = (-0.8, 2.55)
    n, length, angle = 8, 0.12, math.radians(40)
    d = (math.sin(angle), -math.cos(angle))
    for i in range(n):
        cx, cy = anchor[0] + d[0] * (i + 0.5) * length, anchor[1] + d[1] * (i + 0.5) * length
        link = float_at(f"Link {i + 1}", "cylinder", cx, cy, 0, 1, iron, stretch=length * 0.9 / 0.7, scale=(0.05, 0.05),
                        tilt=(0, math.degrees(angle)))
        link["mass"] = 20.0
        link["links"] = [{"object": len(o) - 1,
                          "at": [round(anchor[0] + d[0] * i * length, 4), round(anchor[1] + d[1] * i * length, 4), 0.0]}]
        o.append(link)
    end = (anchor[0] + d[0] * n * length, anchor[1] + d[1] * n * length)
    lantern = cyl("Paper lantern", end[0], 0, end[1] - 0.4, end[1], 0.15,
                  sheen(emitter((1.0, 0.55, 0.2), 20, unlit=False), (1.0, 0.8, 0.6), 0.5, 0.6))
    lantern["mass"] = 2.0
    lantern["links"] = [{"object": len(o) - 1, "at": [round(end[0], 4), round(end[1], 4), 0.0]}]
    o.append(lantern)
    return o


scene("swinging-lantern", "Swinging lantern",
      "A paper lantern on an iron chain, released sideways in a narrow alley: the chain is eight ball-jointed "
      "links, and the lantern is the only light, so the shadows of the chain and the walls swing with it.",
      ["physics", "chain", "emissive", "night"],
      dark_stage(objects=lantern_chain(), base=[0.25, 0.22, 0.19], floor=M((0.25, 0.22, 0.19), 0.5, refl=0.08), bounces=10,
                 exposure=1.8, physics={"enabled": True, "bounce": 0.05, "friction": 0.8},
                 **camera(-10, 6, 5.5, (0, 1.0, 0), 50)))


# ---------------------------------------------------------------- checks
def textured(folder, scale=0.5, **kw):
    """A material with a texture set from scenes/textures/<folder> (core track 5). Colour,
    roughness and metal are the maps' averages (as the app sets them on Load…), so the maps show
    unscaled; keyword arguments override the other material fields."""
    from PIL import Image
    root = Path(__file__).resolve().parents[1] / "scenes" / "textures" / folder
    files = sorted(root.glob("*.jpg")) + sorted(root.glob("*.png"))
    def mean(words, channel=None, srgb=False):
        for f in files:
            tokens = f.stem.lower().replace("-", "_").split("_")
            if any(w in tokens for w in words):
                im = Image.open(f).convert("RGB").reduce(16)
                px = list(im.getdata())
                dec = (lambda v: (v / 255 / 12.92) if v <= 10 else ((v / 255 + 0.055) / 1.055) ** 2.4) if srgb else (lambda v: v / 255)
                avg = [sum(dec(p[k]) for p in px) / len(px) for k in range(3)]
                return avg if channel is None else avg[channel]
        return None
    color = mean(["color", "diff", "albedo", "basecolor"], srgb=True)
    if color is None:
        raise SystemExit(f"Texture set {folder} missing: run tools/fetch_assets.ps1")
    arm = mean(["arm"])
    rough = arm[1] if arm else mean(["roughness", "rough"], 0)
    metal = arm[2] if arm else (mean(["metalness", "metallic"], 0) or 0.0)
    m = M(tuple(min(1.0, c) for c in color), round(rough if rough is not None else 0.5, 4), metal=round(metal, 4), **kw)
    stem = next((f for f in files if any(w in f.stem.lower().replace("-", "_").split("_") for w in ["color", "diff"])))
    m.update(texture=f"textures/{folder}/{stem.name}", texture_scale=scale)
    return m


def check(slug, s):
    """The ranges of Settings::validate, so a bad scene fails here with a readable message."""
    bad = []
    def rng(label, v, lo, hi):
        if not lo <= v <= hi:
            bad.append(f"{label} = {v} not in {lo}..{hi}")
    def material(label, m):
        for c in m["color"] + m["secondary_color"]:
            rng(label + " color", c, 0, 1)
        for k in ("roughness", "metallic", "reflectivity", "transmission"):
            rng(label + " " + k, m[k], 0, 1)
        rng(label + " ior", m["ior"], 1, 2.5)
        rng(label + " emission", m["emission"], 0, 20)
        rng(label + " pattern_scale", m["pattern_scale"], 0.1, 100)
        rng(label + " texture_scale", m.get("texture_scale", 0.5), 0.01, 100)
        rng(label + " priority", m.get("priority", 0), 0, 7)
        for k in ("clearcoat", "clearcoat_roughness", "sheen", "sheen_roughness"):
            rng(label + " " + k, m.get(k, 0), 0, 1)
        for c in m.get("sheen_color", []):
            rng(label + " sheen_color", c, 0, 1)
        rng(label + " anisotropy", m.get("anisotropy", 0), 0, 1)
        rng(label + " anisotropy_rotation", m.get("anisotropy_rotation", 0), -180, 180)
        rng(label + " iridescence", m.get("iridescence", 0), 0, 1)
        rng(label + " iridescence_thickness", m.get("iridescence_thickness", 400), 0, 2000)
        rng(label + " iridescence_ior", m.get("iridescence_ior", 1.3), 1, 2.5)
        rng(label + " grating", m.get("grating", 0), 0, 1)
        rng(label + " grating_spacing", m.get("grating_spacing", 1.6), 0.3, 10)
    for o in s.get("objects", []):
        n = o["name"]
        rng(n + " size", o["size"], 0.05, 30)
        rng(n + " stretch", o["stretch"], 0.05, 10)
        rng(n + " rotation", abs(o["rotation"]), 0, 360)
        for v in o["position"]:
            rng(n + " position", abs(v), 0, 40)
        rng(n + " elevation", abs(o["elevation"]), 0, 40)
        for v in o["tilt"]:
            rng(n + " tilt", abs(v), 0, 360)
        for v in o["scale_xz"]:
            rng(n + " scale", v, 0.01, 20)
        material(n, o["material"])
    for l in s.get("lights", []):
        for v in l["position"]:
            rng(l["name"] + " position", abs(v), 0, 50)
        for v in l["rotation"]:
            rng(l["name"] + " rotation", abs(v), 0, 360)
        for v in l["size"]:
            rng(l["name"] + " size", v, 0.05, 40)
        rng(l["name"] + " intensity", l["intensity"], 0, 2000)
        for c in l["color"]:
            rng(l["name"] + " color", c, 0, 1)
    for k, lo, hi in [("floor_extent", 0.5, 40), ("sky", 0, 4), ("fov", 10, 100), ("distance", 0.5, 200),
                      ("light_size", 0.05, 40), ("light_aspect", 0.1, 10), ("light_intensity", 0, 2000),
                      ("exposure", 0, 10), ("bounces", 1, 32), ("light_temperature", 1000, 12000)]:
        if k in s:
            rng(k, s[k], lo, hi)
    for v in s.get("target", []):
        rng("target", abs(v), 0, 20)
    for v in s.get("light_position", []):
        rng("light_position", abs(v), 0, 50)
    if "pitch" in s:
        rng("pitch", abs(s["pitch"]), 0, 89)
    for k in ("left", "right", "base", "background", "light_tint"):
        for c in s.get(k, []):
            rng(k, c, 0, 1)
    for k in ("walls", "floor"):
        if k in s:
            material(k, s[k])
    sky = s.get("physical_sky", {})
    for k, lo, hi in [("time", 0, 24), ("day", 1, 366), ("latitude", -90, 90), ("north", -360, 360),
                      ("turbidity", 1, 10), ("intensity", 0, 1000), ("sun_size", 0.05, 20), ("ground", 0, 1)]:
        if k in sky:
            rng("physical_sky " + k, sky[k], lo, hi)
    return [f"{slug}: {b}" for b in bad]


# ---------------------------------------------------------------- generated pack
# Curated seeds of the Rust generators (src/generators.rs); each file is just family + seed.
GENERATED = {
    "still-life": ("Studio still life", [3, 24, 77]),
    "material-grid": ("Material grid", [1, 3, 5]),
    "city": ("City blocks", [4, 12, 21]),
    "temple": ("Temple", [9, 40]),
    "stairs": ("Stairs", [1, 6, 10]),
    "stacking": ("Stacking", [3, 5, 22]),
    "menger": ("Menger sponge", [16, 25]),
    "sierpinski": ("Sierpinski tetrahedron", [11, 18]),
    "sphereflake": ("Sphereflake", [1, 44]),
    "kaleidoscope": ("Kaleidoscope", [15, 61]),
    "truchet": ("Truchet maze", [14, 26]),
    "mandelbulb": ("Mandelbulb", [2, 5]),
    "terrain": ("Terrain island", [1, 3, 4]),
    "deep-menger": ("Deep Menger sponge", [3, 7]),
    "tree": ("Trees", [3, 4, 7]),
    "forest": ("Forest", [1, 2]),
    "crowd": ("Crowd", [2]),
}


def write_generated():
    files = []
    for family, (label, seeds) in GENERATED.items():
        for seed in seeds:
            name = f"{family}-{seed}.json"
            data = {"format": 1, "title": f"{label} {seed}", "description": f"Generated scene: {label.lower()}, seed {seed}. Regenerated from the seed on every load.",
                    "tags": ["generated", family.replace("still-life", "still life")], "credits": "Caustica generators", "generator": {"family": family, "seed": seed}}
            files.append((name, data))
    write_pack(OUT.parent / "generated", files,
               {"name": "Generated scenes", "version": 1, "license": "Own work", "commercial_use": True, "attribution": "Caustica",
                "description": "Curated seeds of the procedural generators."}, ensure_ascii=True)


# ---------------------------------------------------------------- mesh pack
# Scenes built on the models of tools/fetch_models.ps1 (scenes/models, gitignored). Part names and
# file materials come from tools/mesh_parts.json, written by `caustica.exe --mesh-info`.
# Capture it as raw bytes (cmd /c "caustica.exe --mesh-info ... > tools\mesh_parts.json", or
# Start-Process -RedirectStandardOutput): PowerShell 5.1's `>` re-decodes it and turns "·" into "Â·".
PARTS = json.loads((Path(__file__).resolve().parent / "mesh_parts.json").read_text(encoding="utf-8"))
MESH_SCENES = []


def mesh(file, x, z, size, material=None, *, part=None, **kw):
    """An imported mesh object; material None takes the file's (per part)."""
    info = PARTS[file][part or 0]
    o = obj(info["name"], "box", x, z, size, material or info["material"], **kw)
    o["mesh"] = f"models/{file}" + (f"#{part}" if part is not None else "")
    return o


def whole(file, x, z, size, overrides=None, **kw):
    """Every part of a multi-material file, with the same transform so they line up."""
    overrides = overrides or {}
    return [mesh(file, x, z, size, overrides.get(PARTS[file][i]["name"].split(" · ")[-1]), part=i, **kw)
            for i in range(len(PARTS[file]))]


def mesh_scene(slug, title, description, tags, credits, settings):
    MESH_SCENES.append((slug, {"format": 1, "title": title, "description": description, "tags": ["mesh", *tags],
                               "credits": credits, "settings": settings}))


STANFORD = "Stanford 3D Scanning Repository (free for research use; credit Stanford Computer Graphics Laboratory), via common-3d-test-models"
KHRONOS = "Khronos glTF-Sample-Assets (licence in the model's README)"

mesh_scene("stanford-dragon-gold", "Gold dragon",
           "The XYZ RGB Asian dragon scan (7.2M triangles in the original; 250k here), in gold in the Cornell box.",
           ["cornell", "metal", "scan"], "XYZ RGB dragon: " + STANFORD,
           cornell(objects=[mesh("xyzrgb_dragon.obj", 0, 0.05, 2.2, M((1.0, 0.77, 0.34), 0.18, metal=1.0), rotation=-30)]))
mesh_scene("happy-buddha-jade", "Jade Buddha",
           "The Happy Buddha scan in green glass on a dark stage, lit from behind so the thin parts glow.",
           ["glass", "scan"], "Happy Buddha: " + STANFORD,
           dark_stage(lights=[light("Back", (0, 3.2, -3), (2.4, 2.4), kelvin(5200), 14, target=(0, 0, 0)),
                              light("Fill", (2.6, 2, 2.6), (1.2, 1.2), kelvin(4000), 3, target=(0, 0, 0))],
                      bounces=20, objects=[mesh("happy.obj", 0, 0, 3.4, M((0.35, 0.8, 0.5), 0.08, trans=1.0, ior=1.55))],
                      **camera(0, 10, 6.5, (0, 0.1, 0))))
mesh_scene("scan-gallery", "Scan gallery",
           "The Stanford armadillo and Lucy on plinths, in plaster and bronze.",
           ["museum", "scan"], "Armadillo and Lucy: " + STANFORD,
           studio(objects=[obj("Plinth", "box", -1.3, 0, 1.6, M((0.85, 0.84, 0.8), 0.6), stretch=0.6),
                           obj("Plinth", "box", 1.3, 0, 1.6, M((0.85, 0.84, 0.8), 0.6), stretch=0.6),
                           mesh("armadillo.obj", -1.3, 0, 2.0, M((0.9, 0.88, 0.84), 0.75), rotation=180, elevation=0.56),
                           mesh("lucy.obj", 1.3, 0, 2.4, M((0.8, 0.5, 0.28), 0.32, metal=1.0), tilt=(-90, 0), rotation=180, elevation=0.56)],
                  **camera(0, 8, 8.5, (0, 0.2, 0))))
mesh_scene("spot-and-suzanne", "Spot and Suzanne",
           "Keenan Crane's cow Spot in red lacquer meets Blender's monkey in chrome.",
           ["material"], "Spot: Keenan Crane (CC0). Suzanne: the Blender Foundation's test monkey. Both via common-3d-test-models",
           studio(objects=[mesh("spot.obj", -0.9, 0, 2.0, lacquer((0.75, 0.06, 0.05)), rotation=210),
                           mesh("suzanne.obj", 1.0, -0.2, 1.5, M((0.95, 0.95, 0.95), 0.05, metal=1.0), rotation=-20)],
                  **camera(0, 10, 6.5, (0, -0.3, 0))))
mesh_scene("beautiful-game", "A beautiful game",
           "The ASWF chess set mid-game: wood, glass pawn tops and the board, one object per material.",
           ["chess", "glass", "gltf"], "A Beautiful Game: " + KHRONOS,
           studio(light_position=[-2.0, 5.0, 3.0], light_size=2.0, light_intensity=14.0, light_temperature=4200.0, bounces=16,
                  exposure=1.5, objects=whole("ABeautifulGame.glb", 0, 0, 5),
                  **camera(25, 30, 5.5, (0, -0.8, 0))))
mesh_scene("glass-dragon", "Glass dragon",
           "The Stanford dragon in clear glass on a cloth backdrop, from the Khronos attenuation sample.",
           ["glass", "caustics", "gltf"], "Dragon Attenuation: " + KHRONOS,
           studio(bounces=20, objects=whole("DragonAttenuation.glb", 0, 0, 4,
                                            {"Dragon with Attenuation": M((0.95, 0.75, 0.45), 0.0, trans=1.0, ior=1.5)}),
                  **camera(0, 6, 4.6, (0, -0.4, 0))))
mesh_scene("toy-car", "Toy car",
           "The Khronos toy car on its cloth: clearcoat paint, fabric and green glass.",
           ["vehicle", "gltf"], "Toy Car: " + KHRONOS,
           studio(bounces=16, objects=whole("ToyCar.glb", 0, 0, 4), **camera(-30, 16, 4.2, (0, -0.4, 0))))


# ---------------------------------------------------------------- environments pack
# HDRI-lit scenes (core track 3); the HDRIs come from tools/fetch_assets.ps1 (scenes/hdri).
ENV_SCENES = []
POLYHAVEN = "Poly Haven (CC0)"


def env_scene(slug, title, description, tags, credits, hdri, settings, strength=1.0, rotation=0.0, visible=True):
    settings = {**settings, "hdri": f"hdri/{hdri}.hdr", "hdri_strength": strength, "hdri_rotation": rotation,
                "hdri_visible": visible, "light_on": False, "lights": [], "sky": 0.0}
    ENV_SCENES.append((slug, {"format": 1, "title": title, "description": description, "tags": ["hdri", *tags],
                              "credits": credits, "settings": settings}))


env_scene("hdri-noon-spheres", "Spheres at noon",
          "Diffuse, gold, glass and rough blue spheres under a partly cloudy noon sky. The sun is a few texels of the map: importance sampling finds it.",
          ["material", "outdoor"], "Kloofendal 48d partly cloudy (pure sky): " + POLYHAVEN, "kloofendal_48d_partly_cloudy_puresky_8k",
          studio(objects=[obj("Diffuse", "sphere", -2.4, 0, 1.4, M((0.8, 0.8, 0.8), 0.9)),
                          obj("Gold", "sphere", -0.8, 0, 1.4, M((1.0, 0.77, 0.34), 0.2, metal=1.0)),
                          obj("Glass", "sphere", 0.8, 0, 1.4, M((1, 1, 1), 0.0, trans=1.0)),
                          obj("Rough", "sphere", 2.4, 0, 1.4, M((0.3, 0.5, 0.9), 0.5, refl=0.3))],
                 floor=M((0.5, 0.5, 0.5), 0.6), bounces=16, **camera(0, 10, 8, (0, -0.4, 0))), rotation=40)
env_scene("hdri-sunset-dragon", "Dragon at sunset",
          "The Khronos glass dragon on a dark mirror floor, against a Venetian sunset.",
          ["glass", "outdoor", "gltf"], "Dragon Attenuation: " + KHRONOS + ". Venice sunset: " + POLYHAVEN, "venice_sunset_8k",
          studio(objects=[mesh("DragonAttenuation.glb", 0, 0, 4, M((0.95, 0.75, 0.45), 0.0, trans=1.0), part=0)],
                 floor=M((0.02, 0.02, 0.025), 0.08, refl=0.6), bounces=20, **camera(150, 6, 5.5, (0, -0.3, 0))), rotation=0)
env_scene("hdri-studio-chess", "Studio chess",
          "The ASWF chess set lit only by a photo studio HDRI, which the camera does not see.",
          ["chess", "studio", "gltf"], "A Beautiful Game: " + KHRONOS + ". Brown photo studio 02: " + POLYHAVEN, "brown_photostudio_02_4k",
          studio(objects=whole("ABeautifulGame.glb", 0, 0, 5), base=[0.05, 0.05, 0.05], background=[0.03, 0.03, 0.035],
                 floor=M((0.05, 0.05, 0.05), 0.4, refl=0.2), bounces=16, exposure=1.3, **camera(25, 30, 5.5, (0, -0.8, 0))),
          visible=False)
env_scene("hdri-toy-car-road", "Toy car on the road",
          "The Khronos toy car on its cloth, by a country road at midday.",
          ["vehicle", "outdoor", "gltf"], "Toy Car: " + KHRONOS + ". Rural asphalt road: " + POLYHAVEN, "rural_asphalt_road_4k",
          studio(objects=whole("ToyCar.glb", 0, 0, 4), floor=M((0.12, 0.12, 0.12), 0.8), floor_extent=40.0, bounces=12,
                 **camera(-35, 10, 4.8, (0, -0.4, 0))), rotation=90)
env_scene("hdri-product-shot", "Product shot",
          "Spot, Suzanne and a teapot on a white sweep, lit by a small studio HDRI that the camera does not see.",
          ["studio", "material"], "Spot, Suzanne: common-3d-test-models. Studio small 09: " + POLYHAVEN, "studio_small_09_4k",
          studio(objects=[mesh("spot.obj", -1.4, 0, 1.6, lacquer((0.9, 0.85, 0.1)), rotation=210),
                          mesh("suzanne.obj", 1.3, -0.2, 1.3, M((0.95, 0.95, 0.95), 0.2, metal=1.0), rotation=-20),
                          obj("Teapot", "teapot", 0, 0.8, 1.2, M((0.1, 0.3, 0.75), 0.15, refl=0.4))],
                 base=[0.9, 0.9, 0.9], background=[0.92, 0.92, 0.93], floor=M((0.9, 0.9, 0.9), 0.5),
                 bounces=12, **camera(0, 14, 4.4, (0, -0.5, 0))), visible=False)

env_scene("hdri-night-bund", "Night on the Bund",
          "Chrome and glass spheres on a wet-looking dark floor, lit only by Shanghai's skyline at night.",
          ["night", "urban", "material"], "Shanghai Bund: " + POLYHAVEN, "shanghai_bund_4k",
          studio(objects=[obj("Chrome", "sphere", -1.2, 0, 1.6, M((0.95, 0.95, 0.95), 0.03, metal=1.0)),
                          obj("Glass", "sphere", 1.2, 0, 1.6, M((1, 1, 1), 0.0, trans=1.0))],
                 floor=M((0.03, 0.03, 0.035), 0.25, refl=0.3), bounces=16, exposure=0.7, **camera(0, 6, 6, (0, -0.3, 0))))
env_scene("hdri-living-room-teapot", "Teapot in the hallway",
          "A glazed teapot and a bowl of fruit on a plinth, lit by a hallway's window and wall lamps.",
          ["interior", "material"], "Lebombo: " + POLYHAVEN, "lebombo_4k",
          studio(objects=[obj("Plinth", "box", 0, 0, 1.8, M((0.85, 0.84, 0.8), 0.6), stretch=0.55),
                          obj("Teapot", "teapot", -0.3, 0, 1.1, M((0.1, 0.35, 0.3), 0.12, refl=0.5), elevation=0.693),
                          obj("Orange", "sphere", 0.55, 0.2, 0.45, M((0.9, 0.35, 0.05), 0.5), elevation=0.693),
                          obj("Lime", "sphere", 0.45, -0.25, 0.4, M((0.3, 0.6, 0.08), 0.5), elevation=0.693)],
                 floor=M((0.35, 0.25, 0.18), 0.45), floor_extent=1.4, bounces=12, **camera(35, 14, 3.6, (0, -0.1, 0))), rotation=120)
env_scene("hdri-snow-spheres", "Snow day",
          "Pearl, copper and ice spheres in a snowy park under a flat winter sky.",
          ["outdoor", "overcast", "material"], "Snowy park 01: " + POLYHAVEN, "snowy_park_01_4k",
          studio(objects=[obj("Pearl", "sphere", -1.4, 0, 1.2, M((0.92, 0.9, 0.86), 0.25, refl=0.6)),
                          obj("Copper", "sphere", 0, 0, 1.2, M((0.95, 0.64, 0.54), 0.2, metal=1.0)),
                          obj("Ice", "sphere", 1.4, 0, 1.2, M((0.85, 0.95, 1.0), 0.1, trans=1.0, ior=1.31))],
                 floor=M((0.9, 0.92, 0.95), 0.8), bounces=16, **camera(0, 8, 6, (0, -0.4, 0))))
env_scene("hdri-fire-sky-dragon", "Gold dragon at dusk",
          "The XYZ RGB dragon in gold on a mirror floor under a burning sunset sky (8k).",
          ["outdoor", "sunset", "metal", "scan"], "XYZ RGB dragon: " + STANFORD + ". The sky is on fire: " + POLYHAVEN, "the_sky_is_on_fire_8k",
          studio(objects=[mesh("xyzrgb_dragon.obj", 0, 0, 3, M((1.0, 0.77, 0.34), 0.15, metal=1.0), rotation=-40)],
                 floor=M((0.05, 0.05, 0.06), 0.05, refl=0.7), bounces=16, **camera(160, 6, 5, (0, -0.4, 0))))


def fireside():
    wood = coat(M((0.3, 0.16, 0.08), 0.45, refl=0.05), 1.0, 0.04)
    brandy = clear((0.85, 0.45, 0.12), 1.36, 2)
    o = [block("Side table", 0, 0, -1.0, -0.3, 1.6, 1.0, wood),
         cyl("Decanter body", -0.35, -0.1, -0.3, 0.05, 0.2, CRYSTAL), ball("Decanter shoulder", -0.35, 0.05, -0.1, 0.2, CRYSTAL, stretch=0.6),
         cyl("Decanter neck", -0.35, -0.1, 0.05, 0.3, 0.045, CRYSTAL), ball("Stopper", -0.35, 0.34, -0.1, 0.07, CRYSTAL),
         cyl("Decanter brandy", -0.35, -0.1, -0.29, -0.05, 0.19, brandy)]
    for i, (x, z) in enumerate([(0.2, 0.15), (0.48, -0.15)]):
        o += [ball(f"Snifter {i + 1}", x, -0.14, z, 0.12, CRYSTAL, stretch=1.1),
              cyl(f"Snifter {i + 1} stem", x, z, -0.3, -0.25, 0.012, CRYSTAL),
              ball(f"Snifter {i + 1} brandy", x, -0.2, z, 0.08, brandy, stretch=0.45)]
    o.append(block("Book", 0.45, 0.3, -0.3, -0.25, 0.42, 0.3, lacquer((0.35, 0.05, 0.05), 0.5), rotation=-12))
    return o


env_scene("hdri-fireside-glassware", "Fireside glassware",
          "A crystal decanter and two snifters of brandy on a lacquered side table, lit only by the room's fireplace "
          "and lamps: warm caustics through the glass and the brandy's colour deepening with its depth.",
          ["interior", "glass", "caustics"], "Fireplace: " + POLYHAVEN, "fireplace_4k",
          studio(objects=fireside(), floor=M((0.3, 0.2, 0.14), 0.6), floor_extent=3.0, bounces=24, exposure=1.2,
                 aperture=0.02, focus_distance=2.6, **camera(-20, 12, 2.6, (0.05, -0.15, 0), 40)), rotation=200)


def loft_ceramics():
    glazes = [coat(M((0.08, 0.3, 0.35), 0.4), 1.0, 0.03), sheen(matte((0.85, 0.8, 0.72), 0.8), (1, 1, 1), 0.3, 0.6),
              coat(M((0.6, 0.15, 0.06), 0.3), 1.0, 0.08), M((0.1, 0.1, 0.1), 0.05, refl=0.5),
              brushed(M((0.8, 0.6, 0.35), 0.3, metal=1), 0.6, spun=True)]
    o = [block("Plinth", 0, 0, -1.0, -0.4, 3.2, 1.0, matte((0.92, 0.91, 0.88), 0.7))]
    for i, (shape, x, size, stretch, m) in enumerate([("pawn", -1.2, 0.8, 1.6, glazes[0]), ("cylinder", -0.55, 0.55, 1.9, glazes[1]),
                                                       ("sphere", 0.1, 0.75, 0.9, glazes[2]), ("cone", 0.7, 0.6, 1.4, glazes[3]),
                                                       ("torus", 1.25, 0.6, 1.6, glazes[4])]):
        o.append(obj(f"Vessel {i + 1}", shape, x, 0, size, m, stretch=stretch, elevation=0.6, rotation=i * 25))
    return o


env_scene("hdri-loft-ceramics", "Loft ceramics",
          "Five glazed vessels on a white plinth in a daylit loft photo studio: a glossy celadon glaze, a matte "
          "stoneware with sheen, a red coat over a rough body, black gloss and spun brass. One soft window light.",
          ["studio", "material", "coat", "sheen"], "Photo studio loft hall: " + POLYHAVEN, "photo_studio_loft_hall_4k",
          studio(objects=loft_ceramics(), floor=M((0.6, 0.58, 0.55), 0.5, refl=0.05), floor_extent=6.0, bounces=12,
                 **camera(10, 8, 4.6, (0, -0.1, 0), 40)), rotation=70)

env_scene("hdri-potsdamer-chrome", "Chrome in the city",
          "A chrome trefoil knot and a mirror sphere on a granite plinth in a city square: nothing but the city "
          "reflected, curved through the knot's turns.",
          ["urban", "mirror", "metal"], "Potsdamer Platz: " + POLYHAVEN, "potsdamer_platz_4k",
          studio(objects=[block("Plinth", 0, 0, -1.0, -0.2, 1.6, 1.6, M((0.3, 0.3, 0.32), 0.35, refl=0.1, pattern="marble",
                                                                         second=(0.15, 0.15, 0.16), pscale=4.0)),
                          obj("Chrome knot", "knot", 0, 0, 1.4, CHROME, elevation=0.8 + 0.05),
                          ball("Mirror sphere", 1.6, -0.65, 0.6, 0.35, CHROME)],
                 floor=M((0.25, 0.25, 0.26), 0.6), floor_extent=40.0, bounces=16, **camera(25, 10, 5.0, (0.3, 0.0, 0), 45)),
          rotation=-30)

env_scene("hdri-sunrise-glass", "Glass at sunrise",
          "Frosted and clear glass bunnies and a jade sphere on a mirror-dark floor, backlit by a low sunrise: "
          "the sun shines through the glass toward the camera, and the frosted glass glows.",
          ["glass", "outdoor", "sunrise", "backlight"], "Spruit sunrise: " + POLYHAVEN, "spruit_sunrise_4k",
          studio(objects=[obj("Clear bunny", "bunny", -0.9, 0, 1.5, GLASS, rotation=30),
                          obj("Frosted bunny", "bunny", 0.8, 0.2, 1.5, FROSTED, rotation=-20),
                          obj("Jade sphere", "sphere", 0, -1.0, 0.9, volume(M((0.55, 0.85, 0.6), 0.1, trans=1, ior=1.62), 12.0, (0.7, 0.95, 0.75)))],
                 floor=M((0.04, 0.04, 0.045), 0.1, refl=0.5), floor_extent=40.0, bounces=24, exposure=0.9,
                 **camera(180, 5, 5.0, (0, -0.4, 0), 40)), rotation=0)


# ---------------------------------------------------------------- materials pack
# Texture sets (core track 5) from tools/fetch_assets.ps1 (scenes/textures): ambientCG and
# Poly Haven, all CC0.
MATERIAL_SCENES = []
AMBIENTCG = "ambientCG (CC0)"


def material_scene(slug, title, description, tags, credits, settings):
    MATERIAL_SCENES.append((slug, {"format": 1, "title": title, "description": description, "tags": ["texture", *tags],
                                   "credits": credits, "settings": settings}))


def material_scenes():
    wood, marble, metal = textured("WoodFloor041", 0.6), textured("Marble016", 0.6), textured("Metal032", 0.8)
    bricks, tiles, fabric = textured("Bricks097", 0.7), textured("Tiles074", 0.8), textured("Fabric062", 0.9)
    laminate, rock = textured("laminate_floor_02", 0.4), textured("rock_wall_08", 0.5)
    material_scene("texture-showroom", "Showroom",
                   "Marble, brushed metal and fabric on a wood floor: ambientCG texture sets with colour, normal, roughness and metalness maps.",
                   ["studio", "material"], "WoodFloor041, Marble016, Metal032, Fabric062: " + AMBIENTCG,
                   studio(objects=[obj("Marble sphere", "sphere", -1.5, 0.2, 1.5, marble),
                                   obj("Metal cube", "box", 0.1, -0.3, 1.3, metal, rotation=25),
                                   obj("Fabric cushion", "box", 1.6, 0.4, 1.5, fabric, stretch=0.35, rotation=-15)],
                          floor=wood, base=[0.6, 0.5, 0.4], bounces=12, **camera(10, 16, 6, (0, -0.5, 0))))
    material_scene("texture-brick-box", "Brick box",
                   "The Cornell box rebuilt in brick with a tiled floor, holding a marble sphere and a metal block.",
                   ["cornell", "material"], "Bricks097, Tiles074, Marble016, Metal032: " + AMBIENTCG,
                   cornell(walls=bricks, floor=tiles,
                           objects=[obj("Marble", "sphere", -0.35, 0.25, 1.4, marble),
                                    obj("Block", "box", 0.4, -0.2, 1.1, metal, stretch=1.6, rotation=-20)]))
    material_scene("texture-rock-garden", "Rock garden",
                   "Rough stone plinths on a laminate floor (Poly Haven sets, with roughness and metalness packed in one ARM map).",
                   ["studio", "material"], "Laminate floor 02, Rock wall 08: Poly Haven (CC0)",
                   studio(objects=[obj("Plinth", "box", -1.0, 0, 1.6, rock, stretch=0.7, rotation=12),
                                   obj("Boulder", "sphere", 1.0, 0.3, 1.5, rock),
                                   obj("Gold", "sphere", 0.1, -1.0, 0.8, M((1.0, 0.77, 0.34), 0.2, metal=1.0))],
                          floor=laminate, base=[0.5, 0.45, 0.4], bounces=12, **camera(-20, 18, 6.5, (0, -0.5, 0))))
    material_scene("texture-duck-on-wood", "Duck on wood",
                   "The Khronos duck (its own UV-mapped texture) beside a marble sphere on a wood floor, lit by a studio HDRI.",
                   ["studio", "gltf", "material"], "Duck: " + KHRONOS + ". WoodFloor041, Marble016: " + AMBIENTCG + ". Studio small 09: " + POLYHAVEN,
                   {**studio(objects=[mesh("Duck.glb", -0.4, 0, 1.6, rotation=-30),
                                      obj("Marble", "sphere", 1.1, -0.2, 1.0, marble)],
                             floor=wood, bounces=12, **camera(15, 12, 4.5, (0, -0.4, 0))),
                    "hdri": "hdri/studio_small_09_4k.hdr", "hdri_visible": False, "hdri_strength": 1.0,
                    "light_on": False, "lights": [], "sky": 0.0})
    sofa = fabric
    plaster = M((0.78, 0.76, 0.72), 0.85, refl=0.0, pattern="marble", second=(0.7, 0.68, 0.64), pscale=1.2)
    o = [block("Brick wall", 0, -2.0, -1.0, 2.0, 6.0, 0.15, bricks), block("Plaster wall", -2.9, 0.5, -1.0, 2.0, 0.15, 5.0, plaster),
         block("Rug", 0.3, 0.6, -1.0, -0.99, 3.0, 2.0, tiles),
         block("Sofa base", -0.2, -1.35, -1.0, -0.55, 2.6, 0.9, sofa), block("Sofa back", -0.2, -1.72, -0.55, 0.05, 2.6, 0.2, sofa)]
    for side in (-1, 1):
        o.append(block("Sofa arm", -0.2 + side * 1.2, -1.35, -0.55, -0.3, 0.2, 0.9, sofa))
    for i, x in enumerate((-0.85, 0.45)):
        o.append(block(f"Cushion {i + 1}", x, -1.3, -0.55, -0.4, 1.25, 0.75, sofa))
    o += [block("Coffee table", 0.3, 0.5, -0.62, -0.56, 1.4, 0.7, marble), block("Table base", 0.3, 0.5, -1.0, -0.62, 1.0, 0.3, metal),
          cyl("Mug", 0.6, 0.45, -0.56, -0.44, 0.05, PORCELAIN), obj("Bowl", "sphere", 0.0, 0.55, 0.45, rock, stretch=0.5, elevation=0.38),
          cyl("Lamp pole", 2.0, -1.4, -1.0, 0.9, 0.02, metal), obj("Lamp shade", "cone", 2.0, -1.4, 0.5, sheen(matte((0.95, 0.9, 0.8), 0.8), (1, 1, 1), 0.3),
                                                                  stretch=0.9, elevation=1.75),
          cyl("Planter", -2.4, -1.5, -1.0, -0.5, 0.25, rock), ball("Plant", -2.4, -0.15, -1.5, 0.4, matte((0.12, 0.4, 0.1), 0.85), stretch=1.4)]
    material_scene("texture-loft-corner", "Loft corner",
                   "The texture sets in one room: a sofa in woven fabric against a brick wall, a marble table on a metal base "
                   "on a tiled floor patch, a stone bowl and planter, on a wood floor in window light and a warm lamp. Normal "
                   "maps carry the detail at grazing light.",
                   ["interior", "material", "texture"],
                   "Bricks097, Fabric062, Marble016, Metal032, Tiles074, WoodFloor041: " + AMBIENTCG + ". Rock wall 08: Poly Haven (CC0)",
                   studio(objects=o, floor=wood, floor_extent=3.0, base=[0.5, 0.4, 0.3], light_on=False, sky=0.15,
                          background=[0.7, 0.75, 0.8], bounces=12, exposure=1.2,
                          lights=[light("Window", (3.2, 1.3, 1.0), (1.6, 2.2), kelvin(6200), 14, target=(0, -0.7, -0.8)),
                                  {**light("Lamp", (2.0, 0.65, -1.4), (0.18, 0.18), (1, 1, 1), 6.0), "kind": "sphere", "temperature": 2800.0}],
                          **camera(25, 14, 5.2, (0, -0.3, -0.5), 48)))


def write_materials():
    problems = [p for slug, data in MATERIAL_SCENES for p in check(slug, data["settings"])]
    if problems:
        raise SystemExit("; ".join(problems))
    write_pack(OUT.parent / "materials", [(f"{slug}.json", data) for slug, data in MATERIAL_SCENES],
               {"name": "Textured materials", "version": 1, "license": "CC0 textures (ambientCG, Poly Haven)", "commercial_use": False,
                "attribution": "See credits", "source": "tools/fetch_assets.ps1",
                "description": "Texture sets on shapes, floors and walls. Run tools/fetch_assets.ps1 first; without the maps the surfaces show their average colours."})


def write_environments():
    problems = [p for slug, data in ENV_SCENES for p in check(slug, data["settings"])]
    if problems:
        raise SystemExit("; ".join(problems))
    write_pack(OUT.parent / "environments", [(f"{slug}.json", data) for slug, data in ENV_SCENES],
               {"name": "HDRI environments", "version": 1, "license": "Mixed: see each scene's credits", "commercial_use": False,
                "attribution": "See credits", "source": "tools/fetch_assets.ps1",
                "description": "Scenes lit by Poly Haven HDRIs. Run tools/fetch_assets.ps1 first; without the HDRIs they fall back to the sky light."})


def write_meshes():
    problems = [p for slug, data in MESH_SCENES for p in check(slug, data["settings"])]
    if problems:
        raise SystemExit("; ".join(problems))
    write_pack(OUT.parent / "meshes", [(f"{slug}.json", data) for slug, data in MESH_SCENES],
               {"name": "Mesh scenes", "version": 1, "license": "Mixed: see each scene's credits", "commercial_use": False,
                "attribution": "See credits", "source": "tools/fetch_models.ps1",
                "description": "Scanned and glTF models. Run tools/fetch_assets.ps1 first; without the models each object falls back to a box."})


# ---------------------------------------------------------------- write
def main():
    problems = [p for slug, data in SCENES for p in check(slug, data["settings"])]
    if problems:
        raise SystemExit("; ".join(problems))
    write_pack(OUT, [(f"{slug}.json", data) for slug, data in SCENES],
               {"name": "Built-in scenes", "version": 1, "license": "Own work", "commercial_use": True,
                "attribution": "Caustica", "source": "",
                "description": "Built-in scenes, written by tools/build_scenes.py and by hand."})
    write_generated()
    write_meshes()
    write_environments()
    material_scenes()
    write_materials()


if __name__ == "__main__":
    import sys
    # --overwrite rewrites every file that differs; --overwrite=a,b only those scenes (by file stem).
    for arg in sys.argv[1:]:
        if arg == "--overwrite":
            OVERWRITE = True
        elif arg.startswith("--overwrite="):
            OVERWRITE = set(arg.split("=", 1)[1].split(","))
    main()
