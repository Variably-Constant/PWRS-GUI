"""Reference values for Wgpu's cube maps and lighting maps, in doubles.

Written apart from the module's shaders, from the formulas they follow:
cube face directions (Direct3D, Vulkan and wgpu's layout), the panorama,
angular map and octahedral map projections, resampling by a grid of
samples weighed by solid angle, texel solid angles, the irradiance sums,
spherical harmonics from the general real-harmonic formula, the GGX
specular prefilter with filtered importance sampling, and the BRDF table.

It writes the Rust test constants in src/cube_reference.rs. From the
repository's folder: python -u -I tools/cube_reference.py src/cube_reference.rs
"""

import math
import sys

import numpy as np

PI = math.pi


# ---------------------------------------------------------------- inputs


def image_value(x, y, w, h, c):
    """Texel (x, y), channel c, of the test image of w by h texels."""
    return 0.5 + 0.3 * math.sin(2.0 * PI * x / w + 0.7 * c) * math.sin(PI * (y + 0.5) / h) + 0.15 * math.cos(4.0 * PI * y / h + c)


def test_image(w, h):
    """The test image as float32 values, h by w by 4, alpha 1."""
    out = np.ones((h, w, 4), dtype=np.float64)
    for y in range(h):
        for x in range(w):
            for c in range(3):
                out[y, x, c] = np.float32(image_value(x, y, w, h, c))
    return out


SUN = np.array([1.0, 0.6, -0.4]) / math.sqrt(1.0 + 0.36 + 0.16)
SUN_COLOR = [1.0, 0.9, 0.7]


def env_value(d, c):
    """The test environment's light from the unit direction d, channel c."""
    sun = 3.0 * SUN_COLOR[c] * math.exp(12.0 * (d[0] * SUN[0] + d[1] * SUN[1] + d[2] * SUN[2] - 1.0))
    return 0.4 + 0.2 * d[c] + 0.1 * d[(c + 1) % 3] ** 2 + sun


def face_direction(face, s, t):
    """The direction of the point (s, t) of a face, s right and t down, not normalized."""
    face = np.asarray(face)
    s = np.asarray(s, dtype=np.float64)
    t = np.asarray(t, dtype=np.float64)
    one = np.ones(np.broadcast(face, s, t).shape)
    s = s * one
    t = t * one
    conditions = [face == 0, face == 1, face == 2, face == 3, face == 4]
    x = np.select(conditions, [one, -one, s, s, s], -s)
    y = np.select(conditions, [-t, -t, one, -one, -t], -t)
    z = np.select(conditions, [-s, s, t, -t, one], -one)
    return np.stack([x, y, z], -1)


def normalize(d):
    return d / np.linalg.norm(d, axis=-1, keepdims=True)


def texel_directions(size):
    """Each texel center's unit direction, 6 by size by size by 3."""
    ys, xs = np.mgrid[0:size, 0:size]
    s = (2.0 * xs + 1.0) / size - 1.0
    t = (2.0 * ys + 1.0) / size - 1.0
    return np.stack([normalize(face_direction(f, s, t)) for f in range(6)])


def test_environment(size):
    """The test environment cube as float32 values, 6 by size by size by 4."""
    dirs = texel_directions(size)
    out = np.ones((6, size, size, 4))
    for f in range(6):
        for y in range(size):
            for x in range(size):
                for c in range(3):
                    out[f, y, x, c] = np.float32(env_value(dirs[f, y, x], c))
    return out


# ------------------------------------------------------------- geometry


def spot(d):
    """The face a direction points at and the point (s, t) on it."""
    a = np.abs(d)
    x_major = (a[..., 0] >= a[..., 1]) & (a[..., 0] >= a[..., 2])
    y_major = ~x_major & (a[..., 1] >= a[..., 2])
    face = np.where(x_major, np.where(d[..., 0] > 0, 0, 1), np.where(y_major, np.where(d[..., 1] > 0, 2, 3), np.where(d[..., 2] > 0, 4, 5)))
    with np.errstate(divide="ignore", invalid="ignore"):
        s = np.where(
            x_major,
            np.where(d[..., 0] > 0, -d[..., 2], d[..., 2]) / a[..., 0],
            np.where(y_major, d[..., 0] / a[..., 1], np.where(d[..., 2] > 0, d[..., 0], -d[..., 0]) / a[..., 2]),
        )
        t = np.where(
            x_major,
            -d[..., 1] / a[..., 0],
            np.where(y_major, np.where(d[..., 1] > 0, d[..., 2], -d[..., 2]) / a[..., 1], -d[..., 1] / a[..., 2]),
        )
    return face, s, t


def turned(d, angle):
    """d turned about the vertical axis by angle radians, +X toward -Z."""
    c, s = math.cos(angle), math.sin(angle)
    return np.stack([c * d[..., 0] + s * d[..., 2], d[..., 1], c * d[..., 2] - s * d[..., 0]], -1)


def image_direction(kind, ax, ay, w, h):
    """The direction an image point shows and the solid angle density there."""
    u, v = ax / w, ay / h
    if kind == 0:
        phi, theta = (u - 0.5) * 2.0 * PI, v * PI
        d = np.stack([np.sin(theta) * np.cos(phi), np.cos(theta), np.sin(theta) * np.sin(phi)], -1)
        return d, np.sin(theta)
    if kind == 1:
        a, b = 2.0 * u - 1.0, 1.0 - 2.0 * v
        r = np.hypot(a, b)
        angle = PI * r
        with np.errstate(divide="ignore", invalid="ignore"):
            d = np.stack([np.sin(angle) * a / r, np.sin(angle) * b / r, -np.cos(angle)], -1)
            weight = np.where(r > 0, PI * np.sin(angle) / r, PI * PI)
        d = np.where((r > 0)[..., None], d, np.array([0.0, 0.0, -1.0]))
        weight = np.where(r <= 1.0, weight, 0.0)
        return d, weight
    a, b = 2.0 * u - 1.0, 2.0 * v - 1.0
    up = 1.0 - np.abs(a) - np.abs(b)
    upper = np.stack([a, up, b], -1)
    lower = np.stack([np.sign(a) * (1.0 - np.abs(b)), up, np.sign(b) * (1.0 - np.abs(a))], -1)
    q = np.where((up < 0)[..., None], lower, upper)
    span = np.linalg.norm(q, axis=-1)
    return q / span[..., None], 1.0 / span**3


def image_spot(kind, d, w, h):
    """Where an image shows the unit direction d, in texels."""
    if kind == 0:
        flat = (d[..., 0] == 0) & (d[..., 2] == 0)
        around = np.where(flat, 0.5, np.arctan2(d[..., 2], d[..., 0]) / (2.0 * PI) + 0.5)
        return around * w, np.arccos(np.clip(d[..., 1], -1, 1)) / PI * h
    if kind == 1:
        across = np.hypot(d[..., 0], d[..., 1])
        with np.errstate(divide="ignore", invalid="ignore"):
            reach = np.arccos(np.clip(-d[..., 2], -1, 1)) / PI
            a = np.where(across > 0, d[..., 0] / across * reach, np.where(d[..., 2] > 0, 1.0, 0.0))
            b = np.where(across > 0, d[..., 1] / across * reach, 0.0)
        return (a + 1.0) * 0.5 * w, (1.0 - b) * 0.5 * h
    q = d / np.abs(d).sum(axis=-1, keepdims=True)
    a = np.where(q[..., 1] < 0, np.where(q[..., 0] >= 0, 1.0, -1.0) * (1.0 - np.abs(q[..., 2])), q[..., 0])
    b = np.where(q[..., 1] < 0, np.where(q[..., 2] >= 0, 1.0, -1.0) * (1.0 - np.abs(q[..., 0])), q[..., 2])
    return (a + 1.0) * 0.5 * w, (b + 1.0) * 0.5 * h


def image_texel(img, kind, x, y):
    """Texel (x, y) of an image, a texel past its edges brought inside as the projection carries on."""
    h, w = img.shape[:2]
    if kind == 0:
        top, bottom = y < 0, y >= h
        x = np.where(top | bottom, x + w // 2, x)
        y = np.where(top, -1 - y, np.where(bottom, 2 * h - 1 - y, y))
        x = np.mod(x, w)
        y = np.clip(y, 0, h - 1)
    elif kind == 1:
        a = 2.0 * (x + 0.5) / w - 1.0
        b = 1.0 - 2.0 * (y + 0.5) / h
        r = np.hypot(a, b)
        out = r > 1.0
        reach = np.minimum(2.0 - r, 1.0 - 1.5 / w)
        with np.errstate(divide="ignore", invalid="ignore"):
            ia, ib = -a * reach / r, -b * reach / r
        mx = np.floor(np.where(out, (ia + 1.0) * 0.5 * w, 0)).astype(int)
        my = np.floor(np.where(out, (1.0 - ib) * 0.5 * h, 0)).astype(int)
        x = np.clip(np.where(out, mx, x), 0, w - 1)
        y = np.clip(np.where(out, my, y), 0, h - 1)
    else:
        left, right = x < 0, x >= w
        x2 = np.where(left, -1 - x, np.where(right, 2 * w - 1 - x, x))
        y2 = np.where(left | right, h - 1 - y, y)
        top, bottom = y2 < 0, y2 >= h
        x3 = np.where(top | bottom, w - 1 - x2, x2)
        y3 = np.where(top, -1 - y2, np.where(bottom, 2 * h - 1 - y2, y2))
        x = np.clip(x3, 0, w - 1)
        y = np.clip(y3, 0, h - 1)
    return img[y, x]


def image_at(img, kind, px, py):
    """The image at a point in texels, bilinear between the four texels nearest."""
    fx, fy = px - 0.5, py - 0.5
    bx, by = np.floor(fx), np.floor(fy)
    wx, wy = (fx - bx)[..., None], (fy - by)[..., None]
    ix, iy = bx.astype(int), by.astype(int)
    top = image_texel(img, kind, ix, iy) * (1 - wx) + image_texel(img, kind, ix + 1, iy) * wx
    bottom = image_texel(img, kind, ix, iy + 1) * (1 - wx) + image_texel(img, kind, ix + 1, iy + 1) * wx
    return top * (1 - wy) + bottom * wy


def cube_texel(cube, face, x, y):
    """Texel (x, y) of a face; past the face's edge, the nearest texel of the face beside it."""
    n = cube.shape[1]
    out = (x < 0) | (y < 0) | (x >= n) | (y >= n)
    s = (x + 0.5) / n * 2.0 - 1.0
    t = (y + 0.5) / n * 2.0 - 1.0
    beside, bs, bt = spot(face_direction(face, s, t))
    cx = np.clip(np.floor((bs + 1.0) * 0.5 * n).astype(int), 0, n - 1)
    cy = np.clip(np.floor((bt + 1.0) * 0.5 * n).astype(int), 0, n - 1)
    f = np.where(out, beside, face)
    return cube[f, np.where(out, cy, y), np.where(out, cx, x)]


def cube_level(cube, d):
    """A cube level in direction d, bilinear across faces; past a face's corner, the mean of the three texels there."""
    n = cube.shape[1]
    face, s, t = spot(d)
    fx, fy = (s + 1.0) * 0.5 * n - 0.5, (t + 1.0) * 0.5 * n - 0.5
    bx, by = np.floor(fx), np.floor(fy)
    wx, wy = (fx - bx)[..., None], (fy - by)[..., None]
    ix, iy = bx.astype(int), by.astype(int)
    t00 = cube_texel(cube, face, ix, iy)
    t10 = cube_texel(cube, face, ix + 1, iy)
    t01 = cube_texel(cube, face, ix, iy + 1)
    t11 = cube_texel(cube, face, ix + 1, iy + 1)
    left, right = (ix < 0)[..., None], (ix + 1 >= n)[..., None]
    top, bottom = (iy < 0)[..., None], (iy + 1 >= n)[..., None]
    t00, t10, t01, t11 = (
        np.where(left & top, (t10 + t01 + t11) / 3.0, t00),
        np.where(right & top & ~(left & top), (t00 + t01 + t11) / 3.0, t10),
        np.where(left & bottom & ~(left & top) & ~(right & top), (t00 + t10 + t11) / 3.0, t01),
        np.where(right & bottom & ~(left & top) & ~(right & top) & ~(left & bottom), (t00 + t10 + t01) / 3.0, t11),
    )
    return (t00 * (1 - wx) + t10 * wx) * (1 - wy) + (t01 * (1 - wx) + t11 * wx) * wy


def cube_at(levels, d, lod):
    """The cube in direction d at a mip level between levels."""
    l = np.clip(lod, 0.0, len(levels) - 1.0)
    below = np.floor(l).astype(int)
    above = np.minimum(below + 1, len(levels) - 1)
    values = [cube_level(level, d) for level in levels]
    pick = lambda which: np.choose(which[..., None] * np.ones(values[0].shape, dtype=int), values)
    low, high = pick(below), pick(above)
    frac = (l - below)[..., None]
    return low * (1 - frac) + high * frac


def box_mips(cube):
    """A cube's mip levels by the Box filter, each the mean of the four texels under it."""
    levels = [cube]
    while levels[-1].shape[1] > 1:
        c = levels[-1]
        levels.append((c[:, 0::2, 0::2] + c[:, 0::2, 1::2] + c[:, 1::2, 0::2] + c[:, 1::2, 1::2]) / 4.0)
    return levels


def solid_angle(size, x, y):
    at = lambda i: 2.0 * i / size - 1.0
    area = lambda a, b: np.arctan2(a * b, np.sqrt(a * a + b * b + 1.0))
    x0, x1, y0, y1 = at(x), at(x + 1), at(y), at(y + 1)
    return area(x0, y0) - area(x0, y1) - area(x1, y0) + area(x1, y1)


# ------------------------------------------------------------ resampling


def to_cube(img, kind, size, grid, yaw_degrees):
    h, w = img.shape[:2]
    offsets = (np.arange(grid) + 0.5) / grid
    ys, xs = np.mgrid[0:size, 0:size]
    ax = xs[..., None, None] + offsets[None, None, None, :]
    ay = ys[..., None, None] + offsets[None, None, :, None]
    s, t = ax / size * 2.0 - 1.0, ay / size * 2.0 - 1.0
    r = 1.0 / np.sqrt(1.0 + s * s + t * t)
    weight = r**3
    out = np.zeros((6, size, size, 4))
    for face in range(6):
        d = turned(normalize(face_direction(face, s, t)), -math.radians(yaw_degrees))
        px, py = image_spot(kind, d, w, h)
        v = image_at(img, kind, px, py)
        out[face] = (weight[..., None] * v).sum(axis=(2, 3)) / weight.sum(axis=(2, 3))[..., None]
    return out


def from_cube(cube, kind, w, h, grid, yaw_degrees):
    offsets = (np.arange(grid) + 0.5) / grid
    ys, xs = np.mgrid[0:h, 0:w]
    ax = xs[..., None, None] + offsets[None, None, None, :]
    ay = ys[..., None, None] + offsets[None, None, :, None]
    ax, ay = np.broadcast_arrays(ax, ay)
    d, weight = image_direction(kind, ax, ay, w, h)
    weight = np.where(weight > 0, weight, 0.0)
    v = cube_level(cube, turned(d, math.radians(yaw_degrees)))
    total = weight.sum(axis=(2, 3))
    with np.errstate(divide="ignore", invalid="ignore"):
        image = np.where(total[..., None] > 0, (weight[..., None] * v).sum(axis=(2, 3)) / total[..., None], 0.0)
    return image


# -------------------------------------------------------------- lighting


def irradiance(env, size):
    """Each texel the mean of the environment's texels by solid angle and positive cosine."""
    n = env.shape[1]
    ys, xs = np.mgrid[0:n, 0:n]
    omega = solid_angle(n, xs, ys)
    dirs = texel_directions(n)
    normals = texel_directions(size)
    out = np.zeros((6, size, size, 4))
    out[..., 3] = 1.0
    flat_dirs = dirs.reshape(-1, 3)
    flat_light = env[..., :3].reshape(-1, 3)
    flat_omega = np.broadcast_to(omega, (6, n, n)).reshape(-1)
    for f in range(6):
        for y in range(size):
            for x in range(size):
                cosine = flat_dirs @ normals[f, y, x]
                w = np.where(cosine > 0, cosine, 0.0) * flat_omega
                out[f, y, x, :3] = (w[:, None] * flat_light).sum(0) / w.sum()
    return out


def legendre(l, m, x):
    """The associated Legendre function P_l^m(x), with the Condon-Shortley phase."""
    pmm = 1.0
    if m > 0:
        somx2 = math.sqrt((1.0 - x) * (1.0 + x))
        fact = 1.0
        for _ in range(m):
            pmm *= -fact * somx2
            fact += 2.0
    if l == m:
        return pmm
    pmmp1 = x * (2 * m + 1) * pmm
    if l == m + 1:
        return pmmp1
    pll = 0.0
    for ll in range(m + 2, l + 1):
        pll = ((2 * ll - 1) * x * pmmp1 - (ll + m - 1) * pmm) / (ll - m)
        pmm, pmmp1 = pmmp1, pll
    return pll


def real_harmonic(l, m, d):
    """The real orthonormal spherical harmonic of band l, order m, z the polar axis."""
    theta = math.acos(max(-1.0, min(1.0, d[2])))
    phi = math.atan2(d[1], d[0])
    k = math.sqrt((2 * l + 1) / (4 * PI) * math.factorial(l - abs(m)) / math.factorial(l + abs(m)))
    if m > 0:
        return math.sqrt(2.0) * k * math.cos(m * phi) * legendre(l, m, math.cos(theta))
    if m < 0:
        return math.sqrt(2.0) * k * math.sin(-m * phi) * legendre(l, -m, math.cos(theta))
    return k * legendre(l, 0, math.cos(theta))


def harmonics(env):
    """The irradiance's coefficients, band 0 to 2, index l(l+1)+m, by channel."""
    n = env.shape[1]
    dirs = texel_directions(n)
    cosine = [PI, 2.0 * PI / 3.0, PI / 4.0]
    sums = np.zeros((9, 3))
    for f in range(6):
        for y in range(n):
            for x in range(n):
                omega = solid_angle(n, x, y)
                d = dirs[f, y, x]
                for l in range(3):
                    for m in range(-l, l + 1):
                        sums[l * (l + 1) + m] += env[f, y, x, :3] * real_harmonic(l, m, d) * omega
    for l in range(3):
        for m in range(-l, l + 1):
            sums[l * (l + 1) + m] *= cosine[l]
    return sums


def radical_inverse(i):
    """Van der Corput's radical inverse of i in base 2, rounded to float32 as the shader does."""
    reversed_bits = int("{:032b}".format(int(i) & 0xFFFFFFFF)[::-1], 2)
    return float(np.float32(reversed_bits)) * 2.3283064365386963e-10


def ggx_halves(count, a2):
    """The half vectors of the Hammersley points sampling GGX about +Z, count by 3."""
    u = np.array([(i / count, radical_inverse(i)) for i in range(count)])
    phi = 2.0 * PI * u[:, 0]
    c2 = np.minimum((1.0 - u[:, 1]) / (1.0 + (a2 - 1.0) * u[:, 1]), 1.0)
    s = np.sqrt(1.0 - c2)
    return np.stack([s * np.cos(phi), s * np.sin(phi), np.sqrt(c2)], -1)


def specular_level(levels, size, roughness, count, cmgen):
    """A specular level of faces size across, of an environment's Box mip chain."""
    env_size = levels[0].shape[1]
    alpha = roughness * roughness
    a2 = alpha * alpha
    halves = ggx_halves(count, a2)
    texels = 6.0 * env_size * env_size
    out = np.zeros((6, size, size, 4))
    out[..., 3] = 1.0
    ys, xs = np.mgrid[0:size, 0:size]
    s, t = (xs + 0.5) / size * 2.0 - 1.0, (ys + 0.5) / size * 2.0 - 1.0
    for f in range(6):
        n = normalize(face_direction(f, s, t))
        up = np.where((np.abs(n[..., 2]) < 0.999)[..., None], np.array([0.0, 0.0, 1.0]), np.array([1.0, 0.0, 0.0]))
        tx = normalize(np.cross(up, n))
        ty = np.cross(n, tx)
        total = np.zeros((size, size))
        sums = np.zeros((size, size, 3))
        for h in halves:
            nl = 2.0 * h[2] * h[2] - 1.0
            if nl <= 0:
                continue
            l = tx * (2.0 * h[2] * h[0]) + ty * (2.0 * h[2] * h[1]) + n * nl
            q = h[2] * h[2] * (a2 - 1.0) + 1.0
            pdf = a2 / (4.0 * PI * q * q)
            if cmgen:
                lod = 0.5 * math.log2(texels / (4.0 * PI * count * pdf)) + 1.0
            else:
                lod = 0.5 * math.log2(texels / (count * pdf))
            sums += cube_at(levels, l, np.full((size, size), lod))[..., :3] * nl
            total += nl
        out[f, ..., :3] = sums / total[..., None]
    return out


def brdf(nv, roughness, count):
    alpha = roughness * roughness
    a2 = alpha * alpha
    v = np.array([math.sqrt(1.0 - nv * nv), 0.0, nv])
    scale = bias = 0.0
    for h in ggx_halves(count, a2):
        l = 2.0 * v.dot(h) * h - v
        nl = min(max(l[2], 0.0), 1.0)
        nh = min(max(h[2], 0.0), 1.0)
        vh = min(max(v.dot(h), 0.0), 1.0)
        if nl > 0.0:
            gv = nl * math.sqrt(nv * nv * (1.0 - a2) + a2)
            gl = nv * math.sqrt(nl * nl * (1.0 - a2) + a2)
            weighed = 0.5 / (gv + gl) * vh * nl / nh
            fc = (1.0 - vh) ** 5
            scale += (1.0 - fc) * weighed
            bias += fc * weighed
    return 4.0 * scale / count, 4.0 * bias / count


# ---------------------------------------------------------------- output


def cube_spots(size):
    """Three texels a face: a corner, the middle of the right edge and of the bottom edge."""
    return [(f, x, y) for f in range(6) for (x, y) in [(0, 0), (size - 1, size // 2), (size // 2, size - 1)]]


def image_spots(w, h):
    return [(0, 0), (w - 1, h - 1), (w // 2, h // 2), (w // 4, h // 3), (3 * w // 4, 2 * h // 3), (w // 8, h - 1), (w - 1, 0), (w // 3, h // 8)]


def f32(v):
    """v rounded to float32, in the fewest digits that give that float32 back."""
    text = np.format_float_positional(np.float32(v), unique=True, trim="-")
    return text if "." in text else text + ".0"


def rust_cube(out, name, what, cube, spots):
    unique = list(dict.fromkeys(spots))
    out.append("/// {}: chosen texels, (face, x, y, color).".format(what))
    out.append("pub(crate) const {}: [(usize, u32, u32, [f32; 3]); {}] = [".format(name, len(unique)))
    for f, x, y in unique:
        v = cube[f, y, x]
        out.append("    ({}, {}, {}, [{}, {}, {}]),".format(f, x, y, f32(v[0]), f32(v[1]), f32(v[2])))
    out.append("];")
    out.append("")


def rust_image(out, name, what, image, spots):
    out.append("/// {}: chosen texels, (x, y, color).".format(what))
    out.append("pub(crate) const {}: [(u32, u32, [f32; 3]); {}] = [".format(name, len(spots)))
    for x, y in spots:
        v = image[y, x]
        out.append("    ({}, {}, [{}, {}, {}]),".format(x, y, f32(v[0]), f32(v[1]), f32(v[2])))
    out.append("];")
    out.append("")


HEADER = """//! The values tools/cube_reference.py computes with numpy in doubles, from
//! the formulas the cube module's shaders follow and the test inputs
//! cube::tests builds: chosen texels of each resampling, the irradiance cube
//! and its harmonics, Linear and Quadratic specular levels and the BRDF
//! table. The script writes this file; run it again after a change to what
//! it computes.
"""


def main(path):
    out = [HEADER]
    panorama = test_image(64, 32)
    angular = test_image(64, 64)
    octahedral = test_image(32, 32)
    rust_cube(out, "PANORAMA_TO_CUBE", "A 64x32 panorama resampled into faces 16 across, 3 by 3 samples a texel", to_cube(panorama, 0, 16, 3, 0.0), cube_spots(16))
    rust_cube(out, "ANGULAR_TO_CUBE", "A 64x64 angular map turned 30 degrees, resampled into faces 16 across, 3 by 3 samples a texel", to_cube(angular, 1, 16, 3, 30.0), cube_spots(16))
    rust_cube(out, "OCTAHEDRAL_TO_CUBE", "A 32x32 octahedral map resampled into faces 16 across, 4 by 4 samples a texel", to_cube(octahedral, 2, 16, 4, 0.0), cube_spots(16))
    env = test_environment(16)
    rust_image(out, "CUBE_TO_PANORAMA", "The environment, faces 16 across, resampled into a 64x32 panorama, 4 by 4 samples a texel", from_cube(env, 0, 64, 32, 4, 0.0), image_spots(64, 32))
    rust_image(out, "CUBE_TO_ANGULAR", "The environment resampled into a 64x64 angular map turned -20 degrees, 4 by 4 samples a texel", from_cube(env, 1, 64, 64, 4, -20.0), image_spots(64, 64))
    rust_image(out, "CUBE_TO_OCTAHEDRAL", "The environment resampled into a 32x32 octahedral map, 4 by 4 samples a texel", from_cube(env, 2, 32, 32, 4, 0.0), image_spots(32, 32))
    rust_cube(out, "IRRADIANCE", "The environment's irradiance, faces 8 across", irradiance(env, 8), cube_spots(8))
    sh = harmonics(env)
    out.append("/// The environment's irradiance as harmonics: red, green and blue.")
    out.append("pub(crate) const HARMONICS: [[f64; 9]; 3] = [")
    for c in range(3):
        out.append("    [{}],".format(", ".join("{:.12e}".format(v) for v in sh[:, c])))
    out.append("];")
    out.append("")
    levels = box_mips(env)
    for level in (1, 2, 3):
        size = 16 >> level
        what = "Linear specular level {} of the environment, roughness {}/3, 64 samples a texel".format(level, level)
        rust_cube(out, "SPECULAR_LINEAR_{}".format(level), what, specular_level(levels, size, level / 3.0, 64, False), cube_spots(size))
    env32 = test_environment(32)
    what = "Quadratic specular level 1 of the environment at faces 32 across, roughness 1, 64 samples a texel"
    rust_cube(out, "SPECULAR_QUADRATIC_1", what, specular_level(box_mips(env32), 16, 1.0, 64, True), cube_spots(16))
    out.append("/// A 32 by 32 BRDF table by 256 samples a texel: chosen texels, (x, y, scale and bias).")
    out.append("pub(crate) const BRDF: [(u32, u32, [f32; 2]); 8] = [")
    for x, y in [(31, 0), (0, 0), (16, 16), (31, 31), (5, 27), (27, 5), (0, 31), (12, 3)]:
        a, b = brdf((x + 0.5) / 32, (y + 0.5) / 32, 256)
        out.append("    ({}, {}, [{}, {}]),".format(x, y, f32(a), f32(b)))
    out.append("];")
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(out) + "\n")
    print("wrote", path)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: python -u -I cube_reference.py <path of the Rust file to write>")
    main(sys.argv[1])
