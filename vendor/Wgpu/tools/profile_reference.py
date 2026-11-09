"""Reference values for Wgpu's conversion of an image's color profile to
sRGB, from littleCMS through Pillow's ImageCms with littleCMS's
optimizations off, NOOPTIMIZE, so it computes each color from the profile
as written rather than from a table it first resamples the profile into.
Each profile converts with the perceptual, relative colorimetric,
saturation and absolute colorimetric intents.

Matrix profiles: a color test image from Display P3 (version 4 and version
2), Adobe RGB (1998), ProPhoto, Rec. 2020 and sRGB, and a gray one from
sGrey, Clinton Ingram's Compact ICC Profiles, CC0 1.0
(github.com/saucecontrol/Compact-ICC-Profiles), read from the folder given:
DisplayP3-v4.icc, DisplayP3-v2-micro.icc, AdobeCompat-v4.icc,
ProPhoto-v4.icc, Rec2020-v4.icc, sRGB-v4.icc and sGrey-v4.icc.

Table profiles, which this script writes:
- CMYK, an ICC version 2 output profile whose lut16 tables give Lab on a
  grid of 6 points per ink, 0, 0.2, ... 1, behind curves that bend each
  ink: a perceptual table (A2B0) whose full inks lift black off 0, a
  colorimetric one (A2B1) whose do not, no saturation table, and a paper
  white. The inks tested are the grid's points.
- RGB through a lutAtoB table to XYZ, a version 4 display profile with
  every element: A curves, an 8-bit grid of 5 points, M curves, a matrix
  with an offset and B curves. The color test image.
- RGB through a lut8 table to Lab, a version 2 input profile on a grid of 9
  points whose black lifts off 0. The color test image.
- Gray through a lutAtoB table to XYZ, a version 4 display profile with a
  16-bit grid of 17 points. The gray test image.
- RGB through D2B tables, floating point multiProcessElementType ones, to
  XYZ: a version 4 display profile with a D2B table for every intent
  beside an A2B0 table and a matrix-shaper's colorants and curves. The
  color test image. Also written with one of D2B0's elements of a type
  littleCMS refuses; the script checks littleCMS refuses that table and
  converts by the others as before.
- CMYK through D2B tables to Lab, a version 4 output profile with D2B0 and
  D2B1 and the CMYK profile's perceptual table as A2B0. The CMYK profile's
  inks.
- Gray through a D2B table to XYZ, a version 4 display profile with D2B0
  and a gray curve. The gray test image.
- Matrix-shapers moxcms converts otherwise than littleCMS: RGB whose
  curves leave black above 0, version 4; RGB beside an A2B1 table alone,
  version 4; RGB whose media white is not D50, a version 2 input profile;
  and gray whose curve gives L* through a Lab PCS, a version 2 output
  profile. The color test image, and the gray one for gray.

CIE Lab: the color test image's bytes read as 8-bit Lab, L then a and b
each plus 128, as a Photoshop Lab document stores them, converted from the
Lab profile littleCMS builds, ImageCms.createProfile("LAB"). Pillow's LAB
mode takes a and b as signed bytes, so each reaches it with its top bit
flipped. The script checks that profile is abstract with a media white of
D50 and that every intent gives the same values, and writes them once.

The test images are made of integer arithmetic, so the bytes here and
those image_decode::tests builds are the same. The profiles go into the
output beside the values, so the tests read no file.

It writes the Rust test constants in src/profile_reference.rs. From the
repository's folder, with Pillow installed:
python -u -I tools/profile_reference.py <Compact-ICC-Profiles folder> src/profile_reference.rs
"""

import io
import os
import struct
import sys

from PIL import Image, ImageCms

WIDTH, HEIGHT = 16, 8
RGB_PROFILES = ["DisplayP3-v4", "DisplayP3-v2-micro", "AdobeCompat-v4", "ProPhoto-v4", "Rec2020-v4", "sRGB-v4"]
GRAY_PROFILE = "sGrey-v4"
GRID = 6
INTENTS = [
    (ImageCms.Intent.PERCEPTUAL, "PERCEPTUAL"),
    (ImageCms.Intent.RELATIVE_COLORIMETRIC, "RELATIVE"),
    (ImageCms.Intent.SATURATION, "SATURATION"),
    (ImageCms.Intent.ABSOLUTE_COLORIMETRIC, "ABSOLUTE"),
]
D50 = (0.9642, 1.0, 0.8249)
# The largest XYZ a profile's 16-bit PCS encodes, as littleCMS takes it.
MAX_ENCODEABLE_XYZ = 1.0 + 32767.0 / 32768.0
# Linear sRGB to XYZ adapted to D50 by Bradford: the ICC sRGB profile's colorants.
SRGB_TO_XYZ = [(0.4360747, 0.3850649, 0.1430804), (0.2225045, 0.7168786, 0.0606169), (0.0139322, 0.0971045, 0.7141733)]


def rgb_bytes():
    """The color test image's bytes, HEIGHT by WIDTH by 3."""
    out = bytearray()
    for y in range(HEIGHT):
        for x in range(WIDTH):
            out += bytes([(x * 37 + y * 91) % 256, (x * x * 3 + y * 5) % 256, (x * y * 7 + 13) % 256])
    return bytes(out)


def gray_bytes():
    """The gray test image's bytes, HEIGHT by WIDTH."""
    return bytes((x * 17 + y * 29) % 256 for y in range(HEIGHT) for x in range(WIDTH))


def ink_bytes():
    """Every ink of the CMYK profile's grid, C slowest, four bytes each."""
    steps = [round(255 * i / (GRID - 1)) for i in range(GRID)]
    return bytes(v for c in steps for m in steps for y in steps for k in steps for v in (c, m, y, k))


def s15(value):
    return struct.pack(">i", round(value * 65536))


def u16s(values):
    return struct.pack(">%dH" % len(values), *values)


def pad4(data):
    return data + b"\0" * (-len(data) % 4)


def unit16(value):
    """A value from 0 to 1 as a 16-bit table entry."""
    return max(0, min(65535, round(value * 65535)))


def xyz_of_linear(rgb):
    return [sum(row[i] * rgb[i] for i in range(3)) for row in SRGB_TO_XYZ]


def linear_of_srgb(v):
    return v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4


def lab_of_xyz(xyz):
    def f(t):
        return t ** (1 / 3) if t > (6 / 29) ** 3 else t / (3 * (6 / 29) ** 2) + 4 / 29

    fx, fy, fz = (f(xyz[i] / D50[i]) for i in range(3))
    return 116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz)


def desc(text):
    """A version 2 textDescriptionType."""
    ascii = text.encode("ascii")
    return b"desc" + b"\0" * 4 + struct.pack(">I", len(ascii) + 1) + ascii + b"\0" + b"\0" * 8 + b"\0" * 3 + b"\0" * 67


def text(content):
    return b"text" + b"\0" * 4 + content.encode("ascii") + b"\0"


def xyz_tag(x, y, z):
    return b"XYZ " + b"\0" * 4 + s15(x) + s15(y) + s15(z)


def curv(entries):
    return b"curv" + b"\0" * 4 + struct.pack(">I", len(entries)) + u16s(entries)


def para(function, parameters):
    return b"para" + b"\0" * 4 + struct.pack(">HH", function, 0) + b"".join(s15(p) for p in parameters)


IDENTITY = (1, 0, 0, 0, 1, 0, 0, 0, 1)


def lut16(inputs, outputs, grid, input_curves, values, output_curves):
    data = b"mft2" + b"\0" * 4 + bytes([inputs, outputs, grid, 0]) + b"".join(s15(v) for v in IDENTITY)
    data += struct.pack(">HH", len(input_curves[0]), len(output_curves[0]))
    return data + b"".join(u16s(c) for c in input_curves) + u16s(values) + b"".join(u16s(c) for c in output_curves)


def lut8(inputs, outputs, grid, input_curves, values, output_curves):
    data = b"mft1" + b"\0" * 4 + bytes([inputs, outputs, grid, 0]) + b"".join(s15(v) for v in IDENTITY)
    return data + b"".join(bytes(c) for c in input_curves) + bytes(values) + b"".join(bytes(c) for c in output_curves)


def mab(inputs, outputs, a_curves, grid, precision, values, m_curves, matrix, b_curves):
    """A lutAtoB tag of the elements given; an element given as None is
    left out, its offset 0. `matrix` is nine numbers and three offsets."""
    body = b""
    offsets = {}
    for name, blob in [
        ("B", b"".join(pad4(c) for c in b_curves)),
        ("matrix", matrix and b"".join(s15(v) for v in matrix)),
        ("M", m_curves and b"".join(pad4(c) for c in m_curves)),
        ("CLUT", values and bytes(grid + [0] * (16 - len(grid))) + bytes([precision, 0, 0, 0]) + (bytes(values) if precision == 1 else u16s(values))),
        ("A", a_curves and b"".join(pad4(c) for c in a_curves)),
    ]:
        if blob:
            offsets[name] = 32 + len(body)
            body += pad4(blob)
    head = b"mAB " + b"\0" * 4 + bytes([inputs, outputs, 0, 0])
    head += struct.pack(">IIIII", *(offsets.get(n, 0) for n in ("B", "matrix", "M", "CLUT", "A")))
    return head + body


def profile(version, device_class, space, pcs, tags):
    """An ICC profile of `tags`, (signature, data) pairs, behind a header."""
    table = struct.pack(">I", len(tags))
    body = b""
    first = 128 + 4 + 12 * len(tags)
    for signature, data in tags:
        table += signature + struct.pack(">II", first + len(body), len(data))
        body += pad4(data)
    size = 128 + len(table) + len(body)
    header = struct.pack(">I", size) + b"lcms" + bytes(version) + device_class + space + pcs + b"\0" * 12 + b"acsp" + b"\0" * 24
    header += struct.pack(">I", 0) + s15(D50[0]) + s15(D50[1]) + s15(D50[2]) + b"\0" * 48
    assert len(header) == 128, len(header)
    return header + table + body


def lab16(lab):
    """Lab in an ICC version 2 lut16's encoding: L from 0 to 0xFF00, a and b
    with 0 at 0x8000, 256 to a unit."""
    l, a, b = lab

    def clamp(v):
        return max(0, min(65535, round(v)))

    return clamp(l * 652.8), clamp((a + 128) * 256), clamp((b + 128) * 256)


def lab8(lab):
    """Lab in a lut8's encoding: L from 0 to 255 for 0 to 100, a and b with 0
    at 128."""
    l, a, b = lab

    def clamp(v):
        return max(0, min(255, round(v)))

    return clamp(l * 2.55), clamp(a + 128), clamp(b + 128)


def ink_lab(inks, lift):
    """The Lab of CMYK inks, each from 0 to 1, as the CMYK profiles model
    them: each of cyan, magenta and yellow takes its share of linear sRGB's
    red, green or blue, black takes its share of all three, and `lift` is
    the share of white full inks keep."""
    c, m, y, k = inks
    rgb = [lift + (1 - lift) * (1 - ink) * (1 - k) for ink in (c, m, y)]
    return lab_of_xyz(xyz_of_linear(rgb))


def cmyk_lut16(lift):
    """A lut16 table to Lab on a grid of GRID points per ink, of inks whose
    black lifts by `lift`, behind curves of 9 entries that take each ink to
    its power 1.25."""
    bend = [unit16((i / 8) ** 1.25) for i in range(9)]
    points = [i / (GRID - 1) for i in range(GRID)]
    values = [v for c in points for m in points for y in points for k in points for v in lab16(ink_lab((c, m, y, k), lift))]
    return lut16(4, 3, GRID, [bend] * 4, values, [[0, 65535]] * 3)


def cmyk_profile():
    """CMYK, version 2: lut16 tables to Lab; the perceptual table lifts
    black to 5 percent of white, the colorimetric one does not; a paper
    white; no saturation table."""
    return profile([2, 0x10, 0, 0], b"prtr", b"CMYK", b"Lab ", [
        (b"desc", desc("Wgpu test CMYK, lut16 tables")),
        (b"cprt", text("No copyright, use freely")),
        (b"wtpt", xyz_tag(0.9100, 0.9450, 0.7800)),
        (b"A2B0", cmyk_lut16(0.05)),
        (b"A2B1", cmyk_lut16(0.0)),
    ])


def mab_rgb_profile():
    """RGB, version 4: a lutAtoB table to XYZ with sRGB's curve as its A
    curves, an 8-bit grid of 5 points of the XYZ of linear sRGB, M curves
    of 3 entries, a matrix near the identity with an offset, and identity
    B curves."""
    points = [i / 4 for i in range(5)]
    values = []
    for r in points:
        for g in points:
            for b in points:
                values += [max(0, min(255, round(v / MAX_ENCODEABLE_XYZ * 255))) for v in xyz_of_linear([r, g, b])]
    srgb_curve = para(3, [2.4, 1 / 1.055, 0.055 / 1.055, 1 / 12.92, 0.04045])
    matrix = [0.99, 0.005, 0.0, 0.0, 0.995, 0.005, 0.005, 0.0, 0.99, 0.001, 0.0005, 0.001]
    table = mab(3, 3, [srgb_curve] * 3, [5, 5, 5], 1, values, [curv([0, 33000, 65535])] * 3, matrix, [curv([])] * 3)
    return profile([4, 0x30, 0, 0], b"mntr", b"RGB ", b"XYZ ", [
        (b"desc", desc("Wgpu test RGB, lutAtoB table")),
        (b"cprt", text("No copyright, use freely")),
        (b"wtpt", xyz_tag(*D50)),
        (b"A2B0", table),
    ])


def lut8_rgb_profile():
    """RGB, version 2: a lut8 table to Lab on a grid of 9 points behind
    curves that take each channel to its power 0.9, its colors sRGB's with
    black lifted to 2 percent of white."""
    bend = [max(0, min(255, round(255 * (i / 255) ** 0.9))) for i in range(256)]
    points = [i / 8 for i in range(9)]
    values = []
    for r in points:
        for g in points:
            for b in points:
                linear = [0.02 + 0.98 * linear_of_srgb(v) for v in (r, g, b)]
                values += lab8(lab_of_xyz(xyz_of_linear(linear)))
    table = lut8(3, 3, 9, [bend] * 3, values, [list(range(256))] * 3)
    return profile([2, 0x10, 0, 0], b"scnr", b"RGB ", b"Lab ", [
        (b"desc", desc("Wgpu test RGB, lut8 table")),
        (b"cprt", text("No copyright, use freely")),
        (b"wtpt", xyz_tag(*D50)),
        (b"A2B0", table),
    ])


def mab_gray_profile():
    """Gray, version 4: a lutAtoB table to XYZ with a gamma 1.8 A curve and
    a 16-bit grid of 17 points of D50's white scaled from 1 percent to
    all of it."""
    values = []
    for i in range(17):
        y = 0.01 + 0.99 * i / 16
        values += [unit16(w * y / MAX_ENCODEABLE_XYZ) for w in D50]
    table = mab(1, 3, [para(0, [1.8])], [17], 2, values, None, None, [curv([])] * 3)
    return profile([4, 0x30, 0, 0], b"mntr", b"GRAY", b"XYZ ", [
        (b"desc", desc("Wgpu test gray, lutAtoB table")),
        (b"cprt", text("No copyright, use freely")),
        (b"wtpt", xyz_tag(*D50)),
        (b"A2B0", table),
    ])


def f32s(values):
    return struct.pack(">%df" % len(values), *values)


def positioned(head, items):
    """`head`, a position table of `items` after it, each offset counted from
    where `head` begins, and the items, each padded to 4 bytes."""
    first = len(head) + 8 * len(items)
    table, body = b"", b""
    for item in items:
        table += struct.pack(">II", first + len(body), len(item))
        body += pad4(item)
    return head + table + body


def formula(function, parameters):
    """A segmented curve's piece of one of the ICC's formulas, 0 to 2."""
    return ("formula", function, parameters)


def sampled(values):
    """A segmented curve's piece of values sampled across it."""
    return ("sampled", values)


IDENTITY_PIECE = formula(0, [1.0, 1.0, 0.0, 0.0])


def segmented(breakpoints, pieces):
    """A segmentedCurve of `pieces`, one more than the breakpoints between
    them."""
    assert len(pieces) == len(breakpoints) + 1
    data = b"curf" + b"\0" * 4 + struct.pack(">HH", len(pieces), 0) + f32s(breakpoints)
    for piece in pieces:
        if piece[0] == "formula":
            data += b"parf" + b"\0" * 4 + struct.pack(">HH", piece[1], 0) + f32s(piece[2])
        else:
            data += b"samf" + b"\0" * 4 + struct.pack(">I", len(piece[1])) + f32s(piece[1])
    return data


def power_curve(gamma):
    """A segmented curve of one piece: the input to the power `gamma`, and 0
    below 0."""
    return segmented([], [formula(0, [gamma, 1.0, 0.0, 0.0])])


def sampled_curve(function, count):
    """A segmented curve of `function` sampled at `count` points across 0 to
    1, and the input itself outside that span."""
    return segmented([0.0, 1.0], [IDENTITY_PIECE, sampled([function((i + 1) / count) for i in range(count)]), IDENTITY_PIECE])


def curve_set(curves):
    return positioned(b"cvst" + b"\0" * 4 + struct.pack(">HH", len(curves), len(curves)), curves)


def matrix_element(rows, offsets):
    """A matrix element: a row of a weight for each input for each output,
    and an offset for each output."""
    return b"matf" + b"\0" * 4 + struct.pack(">HH", len(rows[0]), len(rows)) + f32s([v for row in rows for v in row]) + f32s(offsets)


def clut_element(points, outputs, values, kind=b"clut"):
    """A grid element of `points` along each input, the first input's
    farthest apart in `values`; `kind` is its type."""
    return kind + b"\0" * 4 + struct.pack(">HH", len(points), outputs) + bytes(points + [0] * (16 - len(points))) + f32s(values)


def acs(kind):
    """A bACS or eACS element, which littleCMS passes over."""
    return kind + b"\0" * 4 + struct.pack(">HH", 3, 3) + b"\0" * 4


def mpet(inputs, outputs, elements):
    """A multiProcessElementType, a D2B tag, of `elements`."""
    return positioned(b"mpet" + b"\0" * 4 + struct.pack(">HHI", inputs, outputs, len(elements)), elements)


def srgb_grid(points, adjust):
    """The XYZ, in its own units, of linear sRGB at a grid of `points` per
    channel, red slowest, each color through `adjust` first."""
    steps = [i / (points - 1) for i in range(points)]
    return [v for r in steps for g in steps for b in steps for v in xyz_of_linear(adjust([r, g, b]))]


def saturated(rgb):
    """`rgb` moved 20 percent farther from its gray, within 0 to 1."""
    gray = sum(rgb) / 3
    return [max(0.0, min(1.0, gray + 1.2 * (v - gray))) for v in rgb]


def float_rgb_profile(broken=False):
    """RGB, version 4: a D2B table for each intent, beside an A2B0 table and
    the colorants and curves of a matrix-shaper, which those take the
    place of. D2B0 holds every element: bACS, curves of a sampled piece, of
    sRGB's two formula pieces and of a log and an exponential piece, a grid
    of 5 points, a matrix with an offset, identity curves and eACS. D2B1's
    matrix lifts black, the black the perceptual and saturation intents
    compensate, as the profile is a version 4 matrix-shaper. D2B2 is power
    curves and a grid of 3 points of colors more saturated, D2B3 a matrix,
    sampled curves and another matrix. The white is not D50. With `broken`,
    D2B0's grid has the type clux, which littleCMS refuses."""
    srgb_rows = [list(row) for row in SRGB_TO_XYZ]
    srgb_formula = segmented([0.04045], [formula(0, [1.0, 1 / 12.92, 0.0, 0.0]), formula(0, [2.4, 1 / 1.055, 0.055 / 1.055, 0.0])])
    log_exponential = segmented([0.0, 0.5], [IDENTITY_PIECE, formula(1, [1.0, 0.25, 9.0, 1.0, 0.0]), formula(2, [0.05, 2.0, 4.0, 0.0, -0.015])])
    perceptual = mpet(3, 3, [
        acs(b"bACS"),
        curve_set([sampled_curve(lambda x: x ** 2.2, 16), srgb_formula, log_exponential]),
        clut_element([5, 5, 5], 3, srgb_grid(5, lambda rgb: [rgb[0] ** 1.1, rgb[1], rgb[2] ** 0.9]), b"clux" if broken else b"clut"),
        matrix_element([[0.98, 0.01, 0.0], [0.0, 0.99, 0.01], [0.01, 0.0, 0.97]], [0.002, 0.001, 0.0015]),
        curve_set([power_curve(1.0)] * 3),
        acs(b"eACS"),
    ])
    relative = mpet(3, 3, [curve_set([power_curve(2.2)] * 3), matrix_element([[0.97 * v for v in row] for row in srgb_rows], [0.0045, 0.0047, 0.0039])])
    saturation = mpet(3, 3, [curve_set([power_curve(2.0)] * 3), clut_element([3, 3, 3], 3, srgb_grid(3, saturated))])
    absolute = mpet(3, 3, [
        matrix_element([[0.95, 0.05, 0.0], [0.02, 0.96, 0.02], [0.0, 0.05, 0.95]], [0.0, 0.0, 0.0]),
        curve_set([sampled_curve(linear_of_srgb, 32)] * 3),
        matrix_element(srgb_rows, [0.0, 0.0, 0.0]),
    ])
    a2b0 = mab(3, 3, None, [], 1, None, [para(0, [2.2])] * 3, [0.9 * v / MAX_ENCODEABLE_XYZ for row in srgb_rows for v in row] + [0.0, 0.0, 0.0], [curv([])] * 3)
    return profile([4, 0x30, 0, 0], b"mntr", b"RGB ", b"XYZ ", [
        (b"desc", desc("Wgpu test RGB, D2B tables")),
        (b"cprt", text("No copyright, use freely")),
        (b"wtpt", xyz_tag(0.9500, 0.9800, 0.8000)),
        (b"rXYZ", xyz_tag(*[row[0] for row in SRGB_TO_XYZ])),
        (b"gXYZ", xyz_tag(*[row[1] for row in SRGB_TO_XYZ])),
        (b"bXYZ", xyz_tag(*[row[2] for row in SRGB_TO_XYZ])),
        (b"rTRC", para(0, [2.2])),
        (b"gTRC", para(0, [2.2])),
        (b"bTRC", para(0, [2.2])),
        (b"A2B0", a2b0),
        (b"D2B0", perceptual),
        (b"D2B1", relative),
        (b"D2B2", saturation),
        (b"D2B3", absolute),
    ])


def float_cmyk_profile():
    """CMYK, version 4: D2B tables giving Lab in its own units, D2B0 of
    sampled curves that take each ink to its power 1.25 and a grid of 5
    points per ink of inks whose black lifts by 4 percent, D2B1 of
    identity curves and a grid of 3 points per ink; and the CMYK profile's
    perceptual lut16 table as A2B0, which the saturation and absolute
    intents convert by, as the profile has no D2B2 or D2B3."""
    def grid(points, lift):
        steps = [i / (points - 1) for i in range(points)]
        return [v for c in steps for m in steps for y in steps for k in steps for v in ink_lab((c, m, y, k), lift)]

    perceptual = mpet(4, 3, [curve_set([sampled_curve(lambda x: x ** 1.25, 16)] * 4), clut_element([5] * 4, 3, grid(5, 0.04))])
    relative = mpet(4, 3, [curve_set([power_curve(1.0)] * 4), clut_element([3] * 4, 3, grid(3, 0.0))])
    return profile([4, 0x30, 0, 0], b"prtr", b"CMYK", b"Lab ", [
        (b"desc", desc("Wgpu test CMYK, D2B tables")),
        (b"cprt", text("No copyright, use freely")),
        (b"wtpt", xyz_tag(0.9100, 0.9450, 0.7800)),
        (b"A2B0", cmyk_lut16(0.05)),
        (b"D2B0", perceptual),
        (b"D2B1", relative),
    ])


def matrix_shaper_profile(version, device_class, curve, white=D50, extra=()):
    """RGB of sRGB's colorants with `curve` for each channel, the media white
    `white`, and the `extra` tags after."""
    return profile(version, device_class, b"RGB ", b"XYZ ", [
        (b"desc", desc("Wgpu test RGB, matrix-shaper")),
        (b"cprt", text("No copyright, use freely")),
        (b"wtpt", xyz_tag(*white)),
        (b"rXYZ", xyz_tag(*[row[0] for row in SRGB_TO_XYZ])),
        (b"gXYZ", xyz_tag(*[row[1] for row in SRGB_TO_XYZ])),
        (b"bXYZ", xyz_tag(*[row[2] for row in SRGB_TO_XYZ])),
        (b"rTRC", curve),
        (b"gTRC", curve),
        (b"bTRC", curve),
    ] + list(extra))


def lifted_rgb_profile():
    """RGB, version 4: a matrix-shaper whose curves, x^2.2 + 0.01 by the
    ICC's function 4, leave black at 1 percent of white, which littleCMS
    compensates for the perceptual and saturation intents."""
    return matrix_shaper_profile([4, 0x30, 0, 0], b"mntr", para(4, [2.2, 1.0, 0.0, 0.0, 0.0, 0.01, 0.01]))


def a2b1_only_rgb_profile():
    """RGB, version 4: a matrix-shaper of x^2.2 curves beside an A2B1 table
    alone, a lutAtoB of those curves and a matrix whose offset lifts black.
    The colorimetric intents convert by the table, the others by the matrix
    and curves with the table's black compensated."""
    rows = [list(row) for row in SRGB_TO_XYZ]
    matrix = [v / MAX_ENCODEABLE_XYZ for row in rows for v in row] + [0.004, 0.004, 0.0035]
    a2b1 = mab(3, 3, None, [], 1, None, [para(0, [2.2])] * 3, matrix, [curv([])] * 3)
    return matrix_shaper_profile([4, 0x30, 0, 0], b"mntr", para(0, [2.2]), extra=[(b"A2B1", a2b1)])


def white_rgb_profile():
    """RGB, version 2: an input profile's matrix-shaper of x^2.2 curves whose
    media white is not D50, which the absolute intent scales by."""
    return matrix_shaper_profile([2, 0x10, 0, 0], b"scnr", para(0, [2.2]), white=(0.9500, 0.9800, 0.8000))


def lab_gray_profile():
    """Gray, version 2: an output profile whose gray curve, of 256 entries
    that take each value to its power 0.8, gives L* through a Lab PCS, with
    a paper white."""
    curve = curv([unit16((i / 255) ** 0.8) for i in range(256)])
    return profile([2, 0x10, 0, 0], b"prtr", b"GRAY", b"Lab ", [
        (b"desc", desc("Wgpu test gray, Lab")),
        (b"cprt", text("No copyright, use freely")),
        (b"wtpt", xyz_tag(0.9100, 0.9450, 0.7800)),
        (b"kTRC", curve),
    ])


def float_gray_profile():
    """Gray, version 4: a D2B0 table of an exponential curve and a matrix of
    one input that takes the gray to D50's white, beside a gray curve, by
    which the other intents convert."""
    curve = segmented([0.0], [IDENTITY_PIECE, formula(2, [1 / 15, 16.0, 1.0, 0.0, -1 / 15])])
    perceptual = mpet(1, 3, [curve_set([curve]), matrix_element([[D50[0]], [D50[1]], [D50[2]]], [0.0, 0.0, 0.0])])
    return profile([4, 0x30, 0, 0], b"mntr", b"GRAY", b"XYZ ", [
        (b"desc", desc("Wgpu test gray, D2B table")),
        (b"cprt", text("No copyright, use freely")),
        (b"wtpt", xyz_tag(*D50)),
        (b"kTRC", para(0, [2.2])),
        (b"D2B0", perceptual),
    ])


def converted(data, mode, size, profile_bytes, intent, out_mode):
    image = Image.frombytes(mode, size, data)
    srgb = ImageCms.createProfile("sRGB")
    source = ImageCms.ImageCmsProfile(io.BytesIO(profile_bytes))
    result = ImageCms.profileToProfile(image, source, srgb, renderingIntent=intent, outputMode=out_mode, flags=ImageCms.Flags.NOOPTIMIZE)
    return result.tobytes()


def rust_bytes(name, data, doc):
    """A static of `data`, a static as clippy asks of an array as large as
    a profile."""
    lines = [f"/// {doc}", f"pub(crate) static {name}: [u8; {len(data)}] = ["]
    for start in range(0, len(data), 24):
        lines.append("    " + ", ".join(str(b) for b in data[start:start + 24]) + ",")
    lines.append("];")
    return "\n".join(lines)


def main(folder, out_path):
    def read(name):
        with open(os.path.join(folder, name + ".icc"), "rb") as handle:
            return handle.read()

    parts = [
        "\n".join([
            "//! Reference values for image_decode::tests, written by",
            "//! tools/profile_reference.py from littleCMS through Pillow's ImageCms:",
            "//! the test images' colors converted to sRGB from each profile, with",
            "//! littleCMS's optimizations off, as the profile is written. The matrix",
            "//! profiles are Compact ICC Profiles, CC0 1.0",
            "//! (github.com/saucecontrol/Compact-ICC-Profiles); the table profiles",
            "//! the script writes.",
        ]),
        "\n".join([
            f"pub(crate) const WIDTH: u32 = {WIDTH};",
            f"pub(crate) const HEIGHT: u32 = {HEIGHT};",
            f"pub(crate) const GRID: u32 = {GRID};",
        ]),
    ]
    rgb, gray, inks = rgb_bytes(), gray_bytes(), ink_bytes()

    def every_intent(constant, profile_bytes, data, mode, size, what, keep_red=False):
        for intent, label in INTENTS:
            values = converted(data, mode, size, profile_bytes, intent, "RGB")
            if keep_red:
                values = bytes(values[0::3])
            parts.append(rust_bytes(f"{constant}_{label}", values, f"{what} to sRGB, {label.lower()}."))

    for name in RGB_PROFILES:
        profile_bytes = read(name)
        constant = name.split("-")[0].upper().replace("COMPAT", "") + ("_V2" if "-v2" in name else "")
        parts.append(rust_bytes(f"{constant}_PROFILE", profile_bytes, f"{name}.icc, as Compact ICC Profiles has it."))
        every_intent(constant, profile_bytes, rgb, "RGB", (WIDTH, HEIGHT), f"The color test image from {name}")
        print(f"wrote {name}", flush=True)
    gray_profile = read(GRAY_PROFILE)
    parts.append(rust_bytes("SGREY_PROFILE", gray_profile, f"{GRAY_PROFILE}.icc, as Compact ICC Profiles has it."))
    every_intent("SGREY", gray_profile, gray, "L", (WIDTH, HEIGHT), f"The gray test image from {GRAY_PROFILE}, the red of the RGB littleCMS gives,", keep_red=True)
    print(f"wrote {GRAY_PROFILE}", flush=True)
    tables = [
        ("CMYK", cmyk_profile(), inks, "CMYK", (GRID ** 4, 1), "Every ink of the CMYK profile's grid", False),
        ("MAB_RGB", mab_rgb_profile(), rgb, "RGB", (WIDTH, HEIGHT), "The color test image from the lutAtoB RGB profile", False),
        ("LUT8_RGB", lut8_rgb_profile(), rgb, "RGB", (WIDTH, HEIGHT), "The color test image from the lut8 RGB profile", False),
        ("MAB_GRAY", mab_gray_profile(), gray, "L", (WIDTH, HEIGHT), "The gray test image from the lutAtoB gray profile, the red of the RGB littleCMS gives,", True),
        ("FLOAT_RGB", float_rgb_profile(), rgb, "RGB", (WIDTH, HEIGHT), "The color test image from the D2B RGB profile", False),
        ("FLOAT_CMYK", float_cmyk_profile(), inks, "CMYK", (GRID ** 4, 1), "Every ink of the CMYK profile's grid from the D2B CMYK profile", False),
        ("FLOAT_GRAY", float_gray_profile(), gray, "L", (WIDTH, HEIGHT), "The gray test image from the D2B gray profile, the red of the RGB littleCMS gives,", True),
        ("LIFTED_RGB", lifted_rgb_profile(), rgb, "RGB", (WIDTH, HEIGHT), "The color test image from the lifted black RGB profile", False),
        ("A2B1_ONLY_RGB", a2b1_only_rgb_profile(), rgb, "RGB", (WIDTH, HEIGHT), "The color test image from the RGB profile of an A2B1 table alone", False),
        ("WHITE_RGB", white_rgb_profile(), rgb, "RGB", (WIDTH, HEIGHT), "The color test image from the RGB profile whose white is not D50", False),
        ("LAB_GRAY", lab_gray_profile(), gray, "L", (WIDTH, HEIGHT), "The gray test image from the gray profile of a Lab PCS, the red of the RGB littleCMS gives,", True),
    ]
    for constant, profile_bytes, data, mode, size, what, keep_red in tables:
        parts.append(rust_bytes(f"{constant}_PROFILE", profile_bytes, f"The {constant.lower().replace('_', ' ')} table profile this script writes."))
        every_intent(constant, profile_bytes, data, mode, size, what, keep_red)
        print(f"wrote the {constant} table profile, {len(profile_bytes)} bytes", flush=True)
    broken = float_rgb_profile(broken=True)
    try:
        converted(rgb, "RGB", (WIDTH, HEIGHT), broken, ImageCms.Intent.PERCEPTUAL, "RGB")
    except ImageCms.PyCMSError as e:
        print(f"littleCMS refuses the broken D2B0 table: {e}", flush=True)
    else:
        raise SystemExit("littleCMS converted by the broken D2B0 table")
    relative = converted(rgb, "RGB", (WIDTH, HEIGHT), broken, ImageCms.Intent.RELATIVE_COLORIMETRIC, "RGB")
    if relative != converted(rgb, "RGB", (WIDTH, HEIGHT), float_rgb_profile(), ImageCms.Intent.RELATIVE_COLORIMETRIC, "RGB"):
        raise SystemExit("the broken D2B0 table changed littleCMS's relative colorimetric conversion")
    parts.append(rust_bytes("FLOAT_RGB_BROKEN_PROFILE", broken, "The float rgb table profile with its D2B0 table's grid of the type clux, which littleCMS refuses, and its other tables as they are."))
    print("wrote the broken FLOAT_RGB profile", flush=True)
    lab_profile = ImageCms.createProfile("LAB")
    white = lab_profile.media_white_point[0]
    if lab_profile.device_class != "abst" or any(abs(w - d) > 1e-4 for w, d in zip(white, D50)):
        raise SystemExit(f"littleCMS's Lab profile is {lab_profile.device_class!r} with a media white of {white}")
    signed = bytes(v ^ 0x80 if i % 3 else v for i, v in enumerate(rgb))
    lab_values = []
    for intent, _ in INTENTS:
        image = Image.frombytes("LAB", (WIDTH, HEIGHT), signed)
        result = ImageCms.profileToProfile(image, lab_profile, ImageCms.createProfile("sRGB"), renderingIntent=intent, outputMode="RGB", flags=ImageCms.Flags.NOOPTIMIZE)
        lab_values.append(result.tobytes())
    if any(values != lab_values[0] for values in lab_values):
        raise SystemExit("littleCMS converts Lab differently with different intents")
    parts.append(rust_bytes("LAB", lab_values[0], "The color test image's bytes read as CIE Lab, converted to sRGB, the same with every intent."))
    print(f"wrote LAB: littleCMS's Lab profile is {lab_profile.device_class!r}, white {white}, the four intents alike", flush=True)
    with open(out_path, "w", newline="\n") as handle:
        handle.write("\n\n".join(parts) + "\n")
    print(f"WROTE {out_path}", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    main(sys.argv[1], sys.argv[2])
