"""Writes scenes/builtin/*.json: the 42 scenes buildable without engine changes
(docs/scene-research.md section 2). Each file is a scene wrapper around a sparse Settings
object: fields left out take Settings::default(), so only what a scene changes is written.

Run from native/:  python tools/build_scenes.py
Units: the floor is at y = -1; a shape of size s spans about 0.7*s (unit meshes are -1..1,
scaled by 0.35*size). Tori and rings lie flat; cylinders and cones stand up.
"""
import json
import math
import random
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "scenes" / "builtin"


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
    assert len(settings.get("objects", [])) <= 256, slug
    assert len(settings.get("lights", [])) <= 32, slug
    SCENES.append((slug, {"format": 1, "title": title, "description": description, "tags": tags,
                          "credits": "Cornell RTX built-in scenes", "settings": settings}))


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
      "A small, very bright light over a glass sphere: the caustic is found only by BSDF paths.",
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


scene("veach-ajar", "Veach ajar room",
      "All light comes through a door left slightly open: almost every path is indirect. Shows the noise floor and what the denoisers do with it.",
      ["validation", "indirect", "veach", "interior"],
      studio(light_on=False, sky=0.0, floor_extent=6.0, base=[0.55, 0.5, 0.45], bounces=16, exposure=1.6,
             lights=[light("Outside", (1.6, 0.6, -5.0), (3.0, 2.5), kelvin(5000), 60, target=(0.9, 0.0, -3.0))],
             objects=ajar_room(), **camera(20, 6, 3.0, (0.4, 0.1, -1.0), 65)))


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


scene("glossy-floor-tiles", "Glossy floor tiles",
      "A strip of black tiles from mirror to matte under a glowing bar: reflections blur as roughness rises.",
      ["material", "roughness"],
      studio(light_on=False, lights=[], sky=0.02, base=[0.08, 0.08, 0.09], objects=tiles(),
             **camera(0, 12, 6.5, (0, -0.7, 0), 45)))

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


scene("jewellery-box", "Jewellery box",
      "Gold, silver and copper rings, pearls and a diamond on velvet: metals reflecting each other at close range.",
      ["material", "metal"],
      studio(objects=jewellery(), light_position=[-1.5, 3.0, 1.5], light_size=1.5, light_intensity=20.0, sky=0.03,
             lights=[light("Warm rim", (2.0, 1.5, -2.0), (1.5, 1.5), kelvin(3200), 14, target=(0, -1, 0))],
             **camera(25, 42, 3.4, (0, -0.85, 0), 40)))

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
    y = -1.0
    tints = [(0.85, 0.95, 1.0), (1.0, 0.9, 0.8), (0.9, 1.0, 0.9), (1.0, 0.92, 0.97)]
    for i in range(6):
        h = 0.35 * 0.9 * 0.5
        o.append(float_at(f"Glass slab {i + 1}", "box", 0.1 * math.sin(i), y + h, 0.1 * math.cos(i), 0.9 - i * 0.07,
                          M(tints[i % 4], 0.0, trans=1, ior=1.5), stretch=0.5, rotation=i * 17))
        y += 2 * h * (0.9 - i * 0.07) / 0.9
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


_o, _l = bunny_museum()
scene("bunny-museum", "Bunny museum",
      "Six Stanford bunnies on plinths in glass, gold, jade, marble, chrome and rubber, each under its own light in a dark gallery.",
      ["material", "mesh", "gallery"],
      dark_stage(lights=_l, objects=_o, base=[0.12, 0.11, 0.1], floor=M((0.12, 0.11, 0.1), 0.25, refl=0.15), bounces=16,
                 **camera(0, 12, 8.5, (0, 0.6, -0.8), 45)))


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


scene("torii-corridor", "Torii corridor",
      "Twelve red gates receding toward a setting sun: repetition, perspective and colour bleeding.",
      ["architecture", "sunset"],
      studio(light_on=False, sky=0.2, background=[0.2, 0.1, 0.08], base=[0.35, 0.3, 0.26],
             floor=M((0.4, 0.35, 0.3), 0.85, refl=0.02), floor_extent=40.0, objects=torii(),
             lights=[light("Sunset", (0, 1.8, -36), (8, 4), kelvin(2300), 300, target=(0, 0, 0), visible=True)],
             **camera(0, 3, 6, (0, 0.3, 0), 50)))


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


scene("night-skyline", "Night skyline",
      "About 150 towers with glowing window bands under a faint night sky: hundreds of emissive triangles.",
      ["city", "night", "emissive"],
      studio(light_on=False, lights=[], sky=0.03, background=[0.01, 0.015, 0.04], base=[0.04, 0.04, 0.05],
             floor=M((0.05, 0.05, 0.06), 0.4, refl=0.1), floor_extent=40.0, objects=skyline(),
             **camera(30, 22, 22, (0, 1.0, 0), 45)))

scene("desert-monolith", "Desert monolith",
      "A black slab on orange sand under a hard sun and a bright sky: minimalism and a sharp shadow.",
      ["open", "shadows"],
      studio(light_on=False, sky=1.4, background=[0.45, 0.6, 0.85], base=[0.8, 0.42, 0.18],
             floor=M((0.8, 0.42, 0.18), 0.95, refl=0.0), floor_extent=40.0,
             lights=[light("Sun", (25, 30, 20), (2.5, 2.5), kelvin(5600), 2000, target=(0, -1, 0))],
             objects=[obj("Monolith", "box", 0, 0, 1, matte((0.012, 0.012, 0.014), 0.25), stretch=6.5, scale=(1.3, 0.3), rotation=15)],
             **camera(-30, 6, 11, (0, 0.8, 0), 40)))


def zen():
    random.seed(9)
    rock = M((0.32, 0.31, 0.3), 0.85, refl=0.02)
    o = []
    for i, (x, z, s) in enumerate([(-1.6, -0.6, 1.4), (-1.0, -0.2, 0.8), (1.4, 0.8, 1.1), (0.4, -1.6, 0.9), (1.9, -0.9, 0.6)]):
        o.append(obj(f"Rock {i + 1}", "octahedron", x, z, s, rock, stretch=0.65, rotation=random.uniform(0, 90), tilt=(random.uniform(-15, 15), 0)))
    o.append(obj("Moss", "sphere", -1.3, 0.3, 1.2, matte((0.18, 0.3, 0.08), 1.0), stretch=0.3))
    return o


scene("zen-garden", "Zen garden",
      "Raked sand (a fine stripe pattern) and rough rocks under one low sun: patterns at grazing light.",
      ["open", "pattern"],
      studio(light_on=False, sky=0.25, background=[0.3, 0.32, 0.35], base=[0.82, 0.78, 0.7],
             floor=M((0.82, 0.78, 0.7), 0.9, refl=0.0, pattern="stripes", second=(0.62, 0.58, 0.5), pscale=18.0), floor_extent=6.0,
             lights=[light("Low sun", (-20, 6, -8), (3, 3), kelvin(3800), 400, target=(0, -1, 0))],
             objects=zen(), **camera(30, 30, 7, (0, -0.8, 0), 45)))

random.seed(5)
scene("disco-floor", "Disco floor",
      "A glowing checkerboard floor lights a mirror ball and dancers from below.",
      ["emissive", "party"],
      studio(light_on=False, lights=[], sky=0.0, background=[0.0, 0.0, 0.0], base=[1.0, 0.25, 0.75],
             floor=M((1.0, 0.25, 0.75), 0.3, refl=0.1, emit=1.6, pattern="checker", second=(0.0, 0.0, 0.0), pscale=1.2), floor_extent=5.0,
             objects=[float_at("Mirror ball", "sphere", 0, 2.2, 0, 2.2, M((0.9, 0.9, 0.95), 0.05, metal=1))]
             + [obj(f"Dancer {i + 1}", "pawn", 2.2 * math.cos(i / 7 * math.tau + 0.3), 2.2 * math.sin(i / 7 * math.tau + 0.3), 1.4,
                    lacquer(random.choice([(0.05, 0.6, 0.9), (0.9, 0.85, 0.1), (0.1, 0.8, 0.3), (0.95, 0.95, 0.95)])), stretch=1.4)
                for i in range(7)],
             **camera(15, 12, 8, (0, 0.4, 0), 45)))


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


scene("mirror-maze", "Mirror maze",
      "Chrome walls in a grid around one glowing sphere: very long specular paths.",
      ["mirror", "bounces", "emissive"],
      dark_stage(objects=mirror_maze(), bounces=24, base=[0.08, 0.08, 0.09], floor=M((0.08, 0.08, 0.09), 0.3, refl=0.1),
                 **camera(20, 30, 10, (0, -0.2, 0), 45)))


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
                o.append(float_at(f"Pane {i + 1}.{r + 1}.{c + 1}", "box", cx - 0.47 + c * 0.47, -0.15 + r * 0.62, -2, 1,
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


scene("orrery", "Orrery",
      "A glowing sun lights its planets and their brass orbits: one central emitter for the whole scene.",
      ["still life", "emissive"],
      dark_stage(objects=orrery(), base=[0.12, 0.1, 0.09], floor=M((0.12, 0.1, 0.09), 0.3, refl=0.1),
                 **camera(25, 32, 12, (0, -0.2, 0), 45)))


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


scene("pool-table", "Pool table",
      "Sixteen glossy balls on green felt under a hanging lamp.",
      ["still life", "glossy"],
      studio(light_on=False, sky=0.02, base=[0.2, 0.16, 0.13], objects=pool_table(),
             lights=[light("Lamp", (0, 2.1, 0), (2.0, 0.55), kelvin(3800), 60)],
             **camera(-20, 32, 6.0, (0.3, -0.2, 0), 45)))


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


scene("bowling-lane", "Bowling lane",
      "A long glossy lane with ten pins and a ball: long, stretched reflections.",
      ["glossy", "interior"],
      studio(light_on=False, sky=0.0, base=[0.1, 0.1, 0.12], objects=bowling(),
             lights=[light(f"Lane light {i + 1}", (0, 2.2, 1 - i * 3.0), (2.0, 0.4), kelvin(4500), 18) for i in range(6)],
             **camera(0, 7, 6.0, (0, -0.6, -6.5), 40)))


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


scene("abacus", "Abacus",
      "Fifty lacquered beads on five rods: many small curved objects.",
      ["still life", "many objects"],
      studio(objects=abacus(), light_size=2.0, light_intensity=14.0, **camera(10, 12, 4.8, (0, -0.1, 0), 40)))


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


# ---------------------------------------------------------------- checks
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
    return [f"{slug}: {b}" for b in bad]


# ---------------------------------------------------------------- write
def main():
    problems = [p for slug, data in SCENES for p in check(slug, data["settings"])]
    if problems:
        raise SystemExit("; ".join(problems))
    OUT.mkdir(parents=True, exist_ok=True)
    for old in OUT.glob("*.json"):
        old.unlink()
    for slug, data in SCENES:
        (OUT / f"{slug}.json").write_text(json.dumps(data, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    pack = {"name": "Built-in scenes", "version": 1, "license": "Own work", "commercial_use": True,
            "attribution": "Cornell RTX", "source": "",
            "description": "The 42 scenes of docs/scene-research.md section 2, generated by tools/build_scenes.py.",
            "scenes": [f"{slug}.json" for slug, _ in SCENES]}
    (OUT / "pack.json").write_text(json.dumps(pack, indent=1) + "\n", encoding="utf-8")
    print(f"wrote {len(SCENES)} scenes to {OUT}")


if __name__ == "__main__":
    main()
