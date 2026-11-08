"""Reference values for Wgpu's merge of a Photoshop document's layers, from
psd-tools 1.26.0: documents psd-tools writes, documents of fill layers this
script writes itself, as psd-tools' API writes no fill or shape layer, and
psd-tools' merge of each.

- BLEND_RGB, an RGB document with transparency: a background layer, then a
  layer in each of the 27 blend modes, one column of the canvas each, of
  colors and alphas that change down the column. psd-tools' own writer
  stores the layers' transparency as their transparency channels.
- BLEND_CMYK, the same in CMYK, its layers opaque at 160 of 255 opacity, as
  psd-tools writes a CMYK layer without a transparency channel.
- BLEND_GRAY, the same in gray with transparency.
- STRUCTURE_RGB: a group that passes through, holding a screen layer and a
  half-opaque normal one; a normal group at half opacity holding a multiply
  layer; a layer with two layers clipped to it; and a hidden layer.
- FILLS_RGB, which this script writes: a background layer, clear at its
  right and fading down the rest, then layers that store no pixels, drawn
  from their fills: a solid color through an
  ellipse's vector mask; a solid fill over the canvas in multiply at 160 of
  255 opacity; a pattern with transparency, from the document's Patt block,
  through a rectangle; a shape whose fill is off; an HSB color through a
  rectangle; and a solid color through an ellipse wider than its
  origination box, the box the fill is drawn in.
- FILLS_CMYK, which this script writes: a background layer with the same
  transparency, and a CMYK color through an ellipse.
- FILLS_GRAY, the same in gray.
- INVERTED_RGB, which this script writes: one solid color through an
  ellipse's vector mask whose flags invert it, over nothing.
- CROSS_RGB, CROSS_GRAY, CROSS_CMYK and CROSS_LAB, which this script
  writes: solid colors of other modes than the document's, each through a
  rectangle over its own column of the canvas, over nothing. These go into
  the output alone, as psd-tools converts a color of another mode its own
  way, and the module's tests take what its own conversion gives.

Each document's bytes go into the output beside psd-tools' merge of its
layers, psd_tools.composite.composite: its color, inks for CMYK with 0 for
none, and its alpha, each in 8 bits as the module's merge rounds its own,
so the tests read no file. psd-tools never applies a vector mask's invert
flag, so INVERTED_RGB's merge is the fill's color with 1 minus psd-tools'
coverage of the ellipse as its alpha, the coverage from
psd_tools.composite.vector.draw_vector_mask.

It writes the Rust test constants in src/psd_reference.rs. From the
repository's folder, with psd-tools 1.26.0 and its composite extras:
python -u -I tools/psd_reference.py src/psd_reference.rs
"""

import io
import struct
import sys

import numpy as np
from PIL import Image
from psd_tools import PSDImage
from psd_tools.api.layers import PixelLayer
from psd_tools.composite import composite
from psd_tools.composite.vector import draw_vector_mask
from psd_tools.constants import BlendMode, ColorMode
from psd_tools.psd.patterns import Pattern, Patterns, VirtualMemoryArray, VirtualMemoryArrayList
from psd_tools.psd.vector import ClosedKnotLinked, ClosedPath, Path, VectorMaskSetting

MODES = [
    BlendMode.NORMAL, BlendMode.DISSOLVE, BlendMode.DARKEN, BlendMode.MULTIPLY, BlendMode.COLOR_BURN,
    BlendMode.LINEAR_BURN, BlendMode.DARKER_COLOR, BlendMode.LIGHTEN, BlendMode.SCREEN, BlendMode.COLOR_DODGE,
    BlendMode.LINEAR_DODGE, BlendMode.LIGHTER_COLOR, BlendMode.OVERLAY, BlendMode.SOFT_LIGHT, BlendMode.HARD_LIGHT,
    BlendMode.VIVID_LIGHT, BlendMode.LINEAR_LIGHT, BlendMode.PIN_LIGHT, BlendMode.HARD_MIX, BlendMode.DIFFERENCE,
    BlendMode.EXCLUSION, BlendMode.SUBTRACT, BlendMode.DIVIDE, BlendMode.HUE, BlendMode.SATURATION,
    BlendMode.COLOR, BlendMode.LUMINOSITY,
]
HEIGHT = 16


def background(mode, width):
    """A background of every color the tests need under the layers."""
    pixels = []
    for y in range(HEIGHT):
        for x in range(width):
            rgb = (16 * y, 255 - 16 * y, (9 * x) % 256)
            if mode == "RGBA":
                pixels.append(rgb + (255,))
            elif mode == "LA":
                pixels.append(((16 * y + 7 * x) % 256, 255))
            else:
                pixels.append((16 * y, (9 * x) % 256, 255 - 16 * y, (5 * x + 3 * y) % 256))
    image = Image.new(mode, (width, HEIGHT))
    image.putdata(pixels)
    return image


def column(mode, index):
    """A column's layer: colors and alphas that change down it."""
    pixels = []
    for y in range(HEIGHT):
        rgb = (255 - 15 * y, (37 * y + 11 * index) % 256, (128 + 9 * index) % 256)
        alpha = 64 + 12 * y
        if mode == "RGBA":
            pixels.append(rgb + (alpha,))
        elif mode == "LA":
            pixels.append(((255 - 15 * y + 13 * index) % 256, alpha))
        else:
            pixels.append(((255 - 15 * y) % 256, (37 * y + 11 * index) % 256, (128 + 9 * index) % 256, (19 * y + 5 * index) % 256))
    image = Image.new(mode, (1, HEIGHT))
    image.putdata(pixels)
    return image


def blend_document(mode, psd_mode):
    width = len(MODES)
    psd = PSDImage.new(psd_mode, (width, HEIGHT))
    psd.append(PixelLayer.frompil(background(mode, width), psd, name="background"))
    for index, blend in enumerate(MODES):
        layer = PixelLayer.frompil(column(mode, index), psd, name=blend.name.lower(), left=index)
        layer.blend_mode = blend
        if psd_mode == "CMYK":
            layer.opacity = 160
        psd.append(layer)
    return psd


def solid(mode, size, rgb, alpha):
    return Image.new(mode, size, rgb + (alpha,))


def structure_document():
    psd = PSDImage.new("RGBA", (12, 8))
    psd.append(PixelLayer.frompil(background("RGBA", 12).crop((0, 0, 12, 8)), psd, name="background"))
    screen = PixelLayer.frompil(solid("RGBA", (6, 6), (40, 90, 200), 200), psd, name="screen", left=1, top=1)
    screen.blend_mode = BlendMode.SCREEN
    half = PixelLayer.frompil(solid("RGBA", (4, 4), (250, 20, 60), 255), psd, name="half", left=3, top=2)
    half.opacity = 128
    psd.create_group([screen, half], name="passing", blend_mode=BlendMode.PASS_THROUGH)
    multiply = PixelLayer.frompil(solid("RGBA", (5, 5), (200, 160, 30), 230), psd, name="multiply", left=6, top=2)
    multiply.blend_mode = BlendMode.MULTIPLY
    psd.create_group([multiply], name="isolated", opacity=128, blend_mode=BlendMode.NORMAL)
    base = PixelLayer.frompil(solid("RGBA", (5, 3), (10, 200, 120), 180), psd, name="base", left=0, top=5)
    psd.append(base)
    for name, rgb, left in (("clip one", (255, 255, 0), 1), ("clip two", (0, 0, 255), 3)):
        clip = PixelLayer.frompil(solid("RGBA", (3, 3), rgb, 255), psd, name=name, left=left, top=4)
        clip.clipping = True
        psd.append(clip)
    hidden = PixelLayer.frompil(solid("RGBA", (12, 8), (255, 0, 255), 255), psd, name="hidden")
    hidden.visible = False
    psd.append(hidden)
    return psd


FILL_WIDTH, FILL_HEIGHT = 24, 16
# How far along a quarter circle's tangent its handles are, as a fraction
# of the radius, for four cubics that follow a circle closely.
KAPPA = 0.5522847498


def descriptor_key(key):
    """A descriptor's key or class: length 0 and 4 bytes, or its length and bytes."""
    data = key.encode("ascii")
    return struct.pack(">I", 0) + data if len(data) == 4 else struct.pack(">I", len(data)) + data


def unicode_text(text):
    data = text.encode("utf-16-be")
    return struct.pack(">I", len(data) // 2) + data


def descriptor_body(cls, items):
    out = unicode_text("") + descriptor_key(cls) + struct.pack(">I", len(items))
    for key, value in items:
        out += descriptor_key(key) + value
    return out


def doub(v):
    return b"doub" + struct.pack(">d", v)


def unit_float(unit, v):
    return b"UntF" + unit.encode("ascii") + struct.pack(">d", v)


def long_value(v):
    return b"long" + struct.pack(">i", v)


def boolean(v):
    return b"bool" + bytes([1 if v else 0])


def text_value(text):
    return b"TEXT" + unicode_text(text)


def objc(cls, items):
    return b"Objc" + descriptor_body(cls, items)


def value_list(values):
    return b"VlLs" + struct.pack(">I", len(values)) + b"".join(values)


def descriptor_block(cls, items):
    """A tagged block of a descriptor: version 16, then the descriptor."""
    return struct.pack(">I", 16) + descriptor_body(cls, items)


def solid_block(color_class, components):
    return descriptor_block("null", [("Clr ", objc(color_class, components))])


def at(x, y):
    """A point in pixels as a path's (y, x) fractions of the canvas."""
    return (y / FILL_HEIGHT, x / FILL_WIDTH)


def rectangle_path(left, top, right, bottom):
    corners = [(left, top), (right, top), (right, bottom), (left, bottom)]
    return ClosedPath(items=[ClosedKnotLinked(at(x, y), at(x, y), at(x, y)) for x, y in corners], operation=1, index=0)


def ellipse_path(cx, cy, rx, ry):
    """An ellipse of four knots, top, right, bottom and left, clockwise."""
    kx, ky = KAPPA * rx, KAPPA * ry
    knots = [
        ClosedKnotLinked(at(cx - kx, cy - ry), at(cx, cy - ry), at(cx + kx, cy - ry)),
        ClosedKnotLinked(at(cx + rx, cy - ky), at(cx + rx, cy), at(cx + rx, cy + ky)),
        ClosedKnotLinked(at(cx + kx, cy + ry), at(cx, cy + ry), at(cx - kx, cy + ry)),
        ClosedKnotLinked(at(cx - rx, cy + ky), at(cx - rx, cy), at(cx - rx, cy - ky)),
    ]
    return ClosedPath(items=knots, operation=1, index=0)


def vector_mask_block(path, flags=0):
    """A vmsk block of one path; flags 1 inverts the mask."""
    buffer = io.BytesIO()
    VectorMaskSetting(version=3, flags=flags, path=Path([path])).write(buffer)
    return buffer.getvalue()


def origination_block(left, top, right, bottom):
    """A vogk block of one rectangle's origination: version 1, then a
    descriptor block of its list, the box in pixels."""
    box = objc("unitRect", [("unitValueQuadVersion", long_value(1)), ("Top ", unit_float("#Pxl", top)), ("Left", unit_float("#Pxl", left)), ("Btom", unit_float("#Pxl", bottom)), ("Rght", unit_float("#Pxl", right))])
    item = objc("null", [("keyOriginType", long_value(1)), ("keyOriginShapeBBox", box)])
    return struct.pack(">I", 1) + descriptor_block("null", [("keyDescriptorList", value_list([item]))])


def stroke_block(stroke, fill):
    return descriptor_block("strokeStyle", [("strokeStyleVersion", long_value(2)), ("strokeEnabled", boolean(stroke)), ("fillEnabled", boolean(fill))])


def pattern_fill_block(pattern_id):
    return descriptor_block("null", [("Ptrn", objc("Ptrn", [("Nm  ", text_value("Wgpu test pattern\x00")), ("Idnt", text_value(pattern_id + "\x00"))]))])


def patterns_block(pattern_id):
    """A Patt block of one RGB pattern 5 by 3 with transparency, its slots
    red, green, blue, one unwritten and the transparency last."""
    w, h = 5, 3

    def plane(f):
        return bytes(f(x, y) for y in range(h) for x in range(w))

    def written(data):
        array = VirtualMemoryArray()
        array.set_data((w, h), data, 8, 0)
        return array

    colors = [plane(lambda x, y: (40 * x + 30) % 256), plane(lambda x, y: (70 * y + 20) % 256), plane(lambda x, y: (25 * (x + y)) % 256)]
    transparency = plane(lambda x, y: 255 if (x + y) % 3 else 96)
    channels = [written(c) for c in colors] + [VirtualMemoryArray(is_written=0), written(transparency)]
    pattern = Pattern(version=1, image_mode=ColorMode.RGB, point=(h, w), name="Wgpu test pattern", pattern_id=pattern_id, data=VirtualMemoryArrayList(version=3, rectangle=(0, 0, h, w), channels=channels))
    buffer = io.BytesIO()
    Patterns([pattern]).write(buffer)
    return buffer.getvalue()


def pixel_layer(name, planes, alpha):
    """A layer over the whole canvas: its transparency, then its colors."""
    return {"name": name, "rect": (0, 0, FILL_HEIGHT, FILL_WIDTH), "channels": [(-1, alpha)] + list(enumerate(planes)), "blend": b"norm", "opacity": 255, "flags": 8, "blocks": []}


def fill_layer(name, colors, blocks, shape, blend=b"norm", opacity=255):
    """A layer that stores no pixels: an empty transparency and colors, its
    fill and shape in its blocks; a shape's flags say its pixels are
    irrelevant."""
    channels = [(-1, b"")] + [(i, b"") for i in range(colors)]
    return {"name": name, "rect": (0, 0, 0, 0), "channels": channels, "blend": blend, "opacity": opacity, "flags": 8 | (16 if shape else 0), "blocks": blocks}


def record_bytes(layer):
    top, left, bottom, right = layer["rect"]
    out = struct.pack(">4i", top, left, bottom, right) + struct.pack(">H", len(layer["channels"]))
    for channel_id, data in layer["channels"]:
        out += struct.pack(">hI", channel_id, 2 + len(data))
    out += b"8BIM" + layer["blend"] + bytes([layer["opacity"], 0, layer["flags"], 0])
    name = layer["name"].encode("ascii")
    pascal = bytes([len(name)]) + name
    pascal += b"\x00" * ((4 - len(pascal) % 4) % 4)
    extra = struct.pack(">I", 0) + struct.pack(">I", 0) + pascal
    for key, data in layer["blocks"]:
        extra += b"8BIM" + key + struct.pack(">I", len(data)) + data
    return out + struct.pack(">I", len(extra)) + extra


def fill_document(mode, colors, layers, global_blocks):
    """A document of the layers given, bottom first, its stored composite
    white, with the global tagged blocks given."""
    records = b"".join(record_bytes(layer) for layer in layers)
    channel_data = b"".join(struct.pack(">H", 0) + data for layer in layers for _, data in layer["channels"])
    info = struct.pack(">h", len(layers)) + records + channel_data
    info += b"\x00" * (len(info) % 2)
    section = struct.pack(">I", len(info)) + info + struct.pack(">I", 0)
    for key, data in global_blocks:
        section += b"8BIM" + key + struct.pack(">I", len(data)) + data + b"\x00" * ((4 - len(data) % 4) % 4)
    header = b"8BPS" + struct.pack(">H", 1) + b"\x00" * 6 + struct.pack(">HIIHH", colors, FILL_HEIGHT, FILL_WIDTH, 8, mode)
    composite_planes = b"\xff" * (colors * FILL_WIDTH * FILL_HEIGHT)
    return header + struct.pack(">I", 0) + struct.pack(">I", 0) + struct.pack(">I", len(section)) + section + struct.pack(">H", 0) + composite_planes


def canvas_plane(f):
    return bytes(f(x, y) % 256 for y in range(FILL_HEIGHT) for x in range(FILL_WIDTH))


def canvas_alpha():
    """A background's transparency: clear in the six columns at the right,
    and fading down the rest."""
    return canvas_plane(lambda x, y: 0 if x >= 18 else 255 - 8 * y)


def fills_rgb():
    """An RGB document: a background, a solid ellipse, a fill over the
    canvas in multiply, a pattern rectangle, a shape whose fill is off, an
    HSB rectangle, and a solid shape its origination box cuts short."""
    background = pixel_layer("background", [canvas_plane(lambda x, y: 9 * x + 40), canvas_plane(lambda x, y: 13 * y + 60), canvas_plane(lambda x, y: 5 * (x + y))], canvas_alpha())
    pattern_id = "wgpu-test-pattern"
    layers = [
        background,
        fill_layer("ellipse", 3, [(b"SoCo", solid_block("RGBC", [("Rd  ", doub(200.0)), ("Grn ", doub(60.0)), ("Bl  ", doub(30.0))])), (b"vmsk", vector_mask_block(ellipse_path(9.0, 7.0, 6.5, 4.75)))], shape=True),
        fill_layer("veil", 3, [(b"SoCo", solid_block("RGBC", [("Rd  ", doub(20.0)), ("Grn ", doub(90.0)), ("Bl  ", doub(220.0))]))], shape=False, blend=b"mul ", opacity=160),
        fill_layer("pattern", 3, [(b"PtFl", pattern_fill_block(pattern_id)), (b"vmsk", vector_mask_block(rectangle_path(2.5, 9.25, 20.75, 15.0)))], shape=True),
        fill_layer("fill off", 3, [(b"SoCo", solid_block("RGBC", [("Rd  ", doub(255.0)), ("Grn ", doub(255.0)), ("Bl  ", doub(0.0))])), (b"vmsk", vector_mask_block(rectangle_path(0.0, 0.0, 24.0, 16.0))), (b"vstk", stroke_block(False, False))], shape=True),
        fill_layer("hsb", 3, [(b"SoCo", solid_block("HSBC", [("H   ", unit_float("#Ang", 200.0)), ("Strt", doub(60.0)), ("Brgh", doub(80.0))])), (b"vmsk", vector_mask_block(rectangle_path(14.5, 1.25, 22.5, 6.75)))], shape=True),
        fill_layer("origin", 3, [(b"SoCo", solid_block("RGBC", [("Rd  ", doub(250.0)), ("Grn ", doub(230.0)), ("Bl  ", doub(90.0))])), (b"vmsk", vector_mask_block(ellipse_path(17.0, 11.0, 6.0, 4.0))), (b"vogk", origination_block(13.0, 8.0, 20.0, 15.0))], shape=True),
    ]
    return fill_document(3, 3, layers, [(b"Patt", patterns_block(pattern_id))])


def fills_cmyk():
    """A CMYK document: a background and a solid CMYK ellipse."""
    background = pixel_layer("background", [canvas_plane(lambda x, y: 255 - 7 * x), canvas_plane(lambda x, y: 255 - 11 * y), canvas_plane(lambda x, y: 200 - 3 * (x + y)), canvas_plane(lambda x, y: 240)], canvas_alpha())
    ink = [("Cyn ", doub(10.0)), ("Mgnt", doub(70.0)), ("Ylw ", doub(85.0)), ("Blck", doub(5.0))]
    layers = [background, fill_layer("ellipse", 4, [(b"SoCo", solid_block("CMYC", ink)), (b"vmsk", vector_mask_block(ellipse_path(12.0, 8.0, 8.5, 5.5)))], shape=True)]
    return fill_document(4, 4, layers, [])


def fills_gray():
    """A gray document: a background and a solid gray ellipse."""
    background = pixel_layer("background", [canvas_plane(lambda x, y: 10 * x + 4 * y)], canvas_alpha())
    layers = [background, fill_layer("ellipse", 1, [(b"SoCo", solid_block("Grsc", [("Gry ", doub(35.0))])), (b"vmsk", vector_mask_block(ellipse_path(12.0, 8.0, 8.5, 5.5)))], shape=True)]
    return fill_document(1, 1, layers, [])


# The fill colors the CROSS documents hold, by their class.
CROSS_COLORS = {
    "RGBC": [("Rd  ", doub(200.0)), ("Grn ", doub(60.0)), ("Bl  ", doub(30.0))],
    "HSBC": [("H   ", unit_float("#Ang", 200.0)), ("Strt", doub(60.0)), ("Brgh", doub(80.0))],
    "Grsc": [("Gry ", doub(35.0))],
    "CMYC": [("Cyn ", doub(10.0)), ("Mgnt", doub(70.0)), ("Ylw ", doub(85.0)), ("Blck", doub(5.0))],
    "LbCl": [("Lmnc", doub(60.0)), ("A   ", doub(40.0)), ("B   ", doub(30.0))],
}


def cross_document(mode, colors, classes):
    """A document of one solid fill of each class given, in another mode
    than the document's, each through a rectangle vector mask over its own
    column of the canvas, left to right, over nothing."""
    width = FILL_WIDTH / len(classes)
    layers = [
        fill_layer(cls, colors, [(b"SoCo", solid_block(cls, CROSS_COLORS[cls])), (b"vmsk", vector_mask_block(rectangle_path(i * width, 0.0, (i + 1) * width, float(FILL_HEIGHT))))], shape=True)
        for i, cls in enumerate(classes)
    ]
    return fill_document(mode, colors, layers, [])


# The color INVERTED_RGB's layer is filled with.
INVERTED_COLOR = (200, 60, 30)


def inverted_rgb():
    """An RGB document of one solid color through an ellipse's vector mask
    whose flags invert it, over nothing."""
    color = [("Rd  ", doub(float(INVERTED_COLOR[0]))), ("Grn ", doub(float(INVERTED_COLOR[1]))), ("Bl  ", doub(float(INVERTED_COLOR[2])))]
    layers = [fill_layer("inverted", 3, [(b"SoCo", solid_block("RGBC", color)), (b"vmsk", vector_mask_block(ellipse_path(12.0, 8.0, 8.5, 5.5), flags=1))], shape=True)]
    return fill_document(3, 3, layers, [])


def inverted_merged(psd):
    """INVERTED_RGB's merge as the module draws an inverted vector mask: the
    fill's color everywhere, and 1 minus psd-tools' coverage of the paths as
    alpha, in float32 as the merge works."""
    (layer,) = list(psd.descendants())
    if not layer.vector_mask.inverted:
        raise SystemExit("INVERTED_RGB: psd-tools reads its vector mask as not inverted")
    covered = np.asarray(draw_vector_mask(layer), dtype=np.float32)[:, :, 0]
    alpha = levels(np.float32(1.0) - covered)
    color = np.tile(np.array(INVERTED_COLOR, dtype=np.uint8), (FILL_HEIGHT, FILL_WIDTH, 1))
    return color.tobytes(), alpha.tobytes()


def levels(values):
    """Values from 0 to 1 in 8-bit levels, as the module's merge rounds its
    own: in float32, a value halfway between two levels to the one above."""
    scaled = np.clip(values, 0.0, 1.0) * 255.0
    whole = np.floor(scaled)
    return (whole + (scaled - whole >= 0.5)).astype(np.uint8)


def merged(psd, cmyk):
    color, _, alpha = composite(psd)
    if cmyk:
        color = 1.0 - color
    return levels(color).tobytes(), levels(alpha[:, :, 0]).tobytes()


def rust_bytes(name, data, doc):
    lines = [f"/// {doc}", f"pub(crate) static {name}: [u8; {len(data)}] = ["]
    for start in range(0, len(data), 24):
        lines.append("    " + ", ".join(str(b) for b in data[start:start + 24]) + ",")
    lines.append("];")
    return "\n".join(lines)


def main(out_path):
    parts = [
        "\n".join([
            "//! Reference values for psd_merge::tests, written by",
            "//! tools/psd_reference.py from psd-tools 1.26.0: documents psd-tools",
            "//! writes, documents of fill layers the script writes itself, and",
            "//! psd-tools' merge of each one's layers, its color, inks for CMYK",
            "//! with 0 for none, and its alpha, in 8 bits as the merge rounds its",
            "//! own; for INVERTED_RGB, whose vector mask psd-tools never inverts,",
            "//! the fill's color with 1 minus psd-tools' coverage as alpha; and",
            "//! without a merge, the RGB and gray fills documents, whose reference",
            "//! is Photoshop's own composite under tests/ps-render, and the CROSS",
            "//! documents of fills in other modes.",
        ]),
    ]
    documents = [
        ("BLEND_RGB", blend_document("RGBA", "RGBA"), False, "An RGB document with transparency, a layer in each blend mode"),
        ("BLEND_CMYK", blend_document("CMYK", "CMYK"), True, "A CMYK document, a layer in each blend mode at 160 of 255 opacity"),
        ("BLEND_GRAY", blend_document("LA", "LA"), False, "A gray document with transparency, a layer in each blend mode"),
        ("STRUCTURE_RGB", structure_document(), False, "An RGB document of groups, clipped layers and a hidden layer"),
    ]
    for name, psd, cmyk, what in documents:
        buffer = io.BytesIO()
        psd.save(buffer)
        data = buffer.getvalue()
        reread = PSDImage.open(io.BytesIO(data))
        color, alpha = merged(reread, cmyk)
        parts.append(rust_bytes(f"{name}_DOCUMENT", data, f"{what}, as psd-tools writes it."))
        parts.append(rust_bytes(f"{name}_COLOR", color, f"psd-tools' merge of {name}_DOCUMENT's layers: its color."))
        parts.append(rust_bytes(f"{name}_ALPHA", alpha, f"psd-tools' merge of {name}_DOCUMENT's layers: its alpha."))
        print(f"wrote {name}: {len(data)} bytes, {reread.width}x{reread.height}, {len(list(reread.descendants()))} layers", flush=True)
    # Each fills document, whether psd-tools' merge goes in beside it (the
    # RGB and gray documents are checked against Photoshop's composites).
    written = [
        ("FILLS_RGB", fills_rgb(), False, False, ["PixelLayer", "ShapeLayer", "SolidColorFill"] + ["ShapeLayer"] * 4,"An RGB document of layers drawn from their fills: solid colors, a pattern and an HSB color, through vector masks and an origination box"),
        ("FILLS_CMYK", fills_cmyk(), True, True, ["PixelLayer", "ShapeLayer"], "A CMYK document of a CMYK color through a vector mask"),
        ("FILLS_GRAY", fills_gray(), False, False, ["PixelLayer", "ShapeLayer"], "A gray document of a gray color through a vector mask"),
    ]
    for name, data, cmyk, with_merge, kinds, what in written:
        reread = PSDImage.open(io.BytesIO(data))
        read = [type(layer).__name__ for layer in reread.descendants()]
        if read != kinds:
            raise SystemExit(f"{name}: psd-tools reads its layers as {read}, where the document means {kinds}")
        parts.append(rust_bytes(f"{name}_DOCUMENT", data, f"{what}, as tools/psd_reference.py writes it."))
        if with_merge:
            color, alpha = merged(reread, cmyk)
            parts.append(rust_bytes(f"{name}_COLOR", color, f"psd-tools' merge of {name}_DOCUMENT's layers: its color."))
            parts.append(rust_bytes(f"{name}_ALPHA", alpha, f"psd-tools' merge of {name}_DOCUMENT's layers: its alpha."))
        print(f"wrote {name}: {len(data)} bytes, {reread.width}x{reread.height}, layers {read}", flush=True)
    data = inverted_rgb()
    reread = PSDImage.open(io.BytesIO(data))
    read = [type(layer).__name__ for layer in reread.descendants()]
    if read != ["ShapeLayer"]:
        raise SystemExit(f"INVERTED_RGB: psd-tools reads its layers as {read}, where the document means ['ShapeLayer']")
    color, alpha = inverted_merged(reread)
    parts.append(rust_bytes("INVERTED_RGB_DOCUMENT", data, "An RGB document of one solid color through an ellipse's inverted vector mask, as tools/psd_reference.py writes it."))
    parts.append(rust_bytes("INVERTED_RGB_COLOR", color, "INVERTED_RGB_DOCUMENT's merge with its vector mask inverted: its color, the fill's."))
    parts.append(rust_bytes("INVERTED_RGB_ALPHA", alpha, "INVERTED_RGB_DOCUMENT's merge with its vector mask inverted: its alpha, 1 minus psd-tools' coverage of the ellipse."))
    print(f"wrote INVERTED_RGB: {len(data)} bytes, {reread.width}x{reread.height}, alpha from {min(alpha)} to {max(alpha)}", flush=True)
    crossed = [
        ("CROSS_RGB", cross_document(3, 3, ["Grsc", "CMYC", "LbCl"]), "An RGB document of a gray, a CMYK and a Lab fill"),
        ("CROSS_GRAY", cross_document(1, 1, ["RGBC", "HSBC", "CMYC", "LbCl"]), "A gray document of an RGB, an HSB, a CMYK and a Lab fill"),
        ("CROSS_CMYK", cross_document(4, 4, ["RGBC", "HSBC", "Grsc", "LbCl"]), "A CMYK document of an RGB, an HSB, a gray and a Lab fill"),
        ("CROSS_LAB", cross_document(9, 3, ["RGBC", "Grsc", "CMYC"]), "A Lab document of an RGB, a gray and a CMYK fill"),
    ]
    for name, data, what in crossed:
        reread = PSDImage.open(io.BytesIO(data))
        read = [type(layer).__name__ for layer in reread.descendants()]
        if read != ["ShapeLayer"] * len(read) or not read:
            raise SystemExit(f"{name}: psd-tools reads its layers as {read}, where the document means shape layers")
        parts.append(rust_bytes(f"{name}_DOCUMENT", data, f"{what}, each in a column through a rectangle vector mask, as tools/psd_reference.py writes it."))
        print(f"wrote {name}: {len(data)} bytes, {reread.width}x{reread.height}, layers {read}", flush=True)
    with open(out_path, "w", newline="\n") as handle:
        handle.write("\n\n".join(parts) + "\n")
    print(f"WROTE {out_path}", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    main(sys.argv[1])
