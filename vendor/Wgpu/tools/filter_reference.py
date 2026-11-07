"""Reference values for Wgpu's image filters, from scipy.ndimage,
scikit-image and OpenCV: Gaussian blurs with each FilterEdge, unsharp
masks, the Sobel, Scharr and Prewitt gradients and the Laplacian, a Sobel
gradient of an sRGB texture's stored values, Canny's edges, the
smoothing's bilateral filter, and clarity's detail against such a filter
of the luminance on the log scale, each summed exactly by OpenCV's joint
bilateral filter and through the bilateral grid by a model of the
module's own.

The test images are made of integer and dyadic arithmetic, so the float32
texels here and those filter::tests builds are the same bits. The script
refuses to write when a decision Canny or the unsharp mask makes on them
is within a hair of going the other way, as float32 on a GPU could tip it.

It writes the Rust test constants in src/filter_reference.rs. From the
repository's folder, with numpy, scipy, scikit-image and
opencv-contrib-python-headless installed:
python -u -I tools/filter_reference.py src/filter_reference.rs
"""

import sys

import cv2
import numpy as np
import scipy
import scipy.ndimage as ndi
import skimage
from skimage import feature

WIDTH, HEIGHT = 45, 37
MODES = {"Clamp": "nearest", "Wrap": "wrap", "Mirror": "reflect"}
# The same edges for OpenCV's filters and numpy's padding.
CV_BORDERS = {"Clamp": cv2.BORDER_REPLICATE, "Wrap": cv2.BORDER_WRAP, "Mirror": cv2.BORDER_REFLECT}
PAD_MODES = {"Clamp": "edge", "Wrap": "wrap", "Mirror": "symmetric"}
# The log scale's step of precision for a float texture: half float's
# smallest normal.
FLOAT_STEP = 2.0 ** -14
SPOTS = [
    (0, 0), (44, 0), (0, 36), (44, 36), (22, 0), (22, 36), (0, 18), (44, 18),
    (1, 1), (43, 35), (10, 15), (11, 16), (30, 31), (31, 32), (19, 15), (20, 6),
    (27, 9), (5, 30), (40, 20), (22, 22), (8, 24), (37, 3), (14, 33), (33, 14),
]
# The smallest distance a decision may have from its threshold, in the
# scale of its comparison, for the GPU's float32 to make it the same way:
# about ten times the float32 error of an unsharp mask's difference, 2e-6,
# and of a Canny gradient in scikit-image's scale, 8e-6.
SHARPEN_MARGIN = 2e-5
CANNY_MARGIN = 1e-4


def grid():
    ys, xs = np.mgrid[0:HEIGHT, 0:WIDTH]
    return xs.astype(np.int64), ys.astype(np.int64)


def color_image(opaque):
    """The color test image as float32, HEIGHT by WIDTH by 4."""
    xs, ys = grid()
    r = ((xs * 5 + ys * 3) % 17) / 16.0
    g = ((xs * xs + 2 * ys) % 13) / 16.0
    b = np.clip((101.25 - (xs - 19.375) ** 2 - (ys - 15.625) ** 2) / 8.0, 0.0, 1.0)
    a = np.ones_like(r) if opaque else ((xs * 3 + ys) % 5) / 4.0
    return np.stack([r, g, b, a], -1).astype(np.float32)


def gray_image():
    """The gray test image as float32, HEIGHT by WIDTH: a disc, a faint disc
    and a step whose contrast fades along it."""
    xs, ys = grid()
    disc = np.clip((49.5 - (xs - 12.375) ** 2 - (ys - 11.625) ** 2) / 4.0, 0.0, 1.0)
    faint = 0.15625 * np.clip((20.25 - (xs - 30.625) ** 2 - (ys - 9.375) ** 2) / 4.0, 0.0, 1.0)
    step = (1.0 - xs * 0.0234375) * np.clip((ys - 22.0 - xs * 0.3125) * 2.0 + 0.5, 0.0, 1.0)
    return np.clip(disc + faint + step, 0.0, 1.0).astype(np.float32)


def stored_bytes():
    """The sRGB test texture's stored bytes, HEIGHT by WIDTH by 4."""
    xs, ys = grid()
    r = (xs * 37 + ys * 91) % 256
    g = (xs * xs * 3 + ys * 5) % 256
    b = (xs * ys * 7) % 256
    return np.stack([r, g, b, np.full_like(r, 255)], -1).astype(np.uint8)


def blur(image, sigmas, mode):
    """image's color weighed by alpha, and its alpha, blurred across by
    sigmas[0] and down by sigmas[1], with straight color again."""
    weighed = image.astype(np.float64)
    weighed[..., :3] *= weighed[..., 3:4]
    out = ndi.gaussian_filter1d(weighed, sigmas[0], axis=1, mode=mode, truncate=4.0)
    out = ndi.gaussian_filter1d(out, sigmas[1], axis=0, mode=mode, truncate=4.0)
    alpha = out[..., 3:4]
    with np.errstate(divide="ignore", invalid="ignore"):
        color = np.where(alpha > 0, out[..., :3] / alpha, 0.0)
    return np.concatenate([color, alpha], -1)


def sharpen(image, amount, radius, threshold, mode):
    """The unsharp mask: amount times each channel's difference from the
    blur added where it is at least threshold; alpha kept, and a texel of
    alpha 0 transparent black."""
    alpha = image[..., 3:4].astype(np.float64)
    color = np.where(alpha > 0, image[..., :3].astype(np.float64), 0.0)
    detail = color - blur(image, [radius, radius], mode)[..., :3]
    if threshold > 0:
        near = np.abs(np.abs(detail) - threshold)[np.broadcast_to(alpha > 0, detail.shape)]
        if near.min() < SHARPEN_MARGIN:
            raise SystemExit("a difference of the unsharp mask is {:.3g} from its threshold".format(near.min()))
    sharp = np.where(np.abs(detail) < threshold, color, color + amount * detail)
    return np.concatenate([np.where(alpha > 0, sharp, 0.0), alpha], -1)


def gradient(values, weights, mode):
    """The size of each channel's gradient, the derivative [-1, 0, 1]
    across the rows or columns weights weighs, divided by their sum."""
    d = np.array([-1.0, 0.0, 1.0])
    w = np.array(weights, dtype=np.float64)
    gx = ndi.correlate1d(ndi.correlate1d(values, d, axis=1, mode=mode), w, axis=0, mode=mode)
    gy = ndi.correlate1d(ndi.correlate1d(values, d, axis=0, mode=mode), w, axis=1, mode=mode)
    return np.sqrt(gx * gx + gy * gy) / w.sum()


def with_alpha(color, alpha=1.0):
    return np.concatenate([color, np.full(color.shape[:2] + (1,), alpha)], -1)


def canny(gray, sigma, low, high, mode):
    """Canny's edges as scikit-image finds them, its thresholds in its own
    scale, where a step from 0 to 1 reads 4; refused where a decision is
    too near its threshold."""
    values = gray.astype(np.float64)
    smoothed = ndi.gaussian_filter(values, sigma, mode=mode, truncate=4.0)
    isobel = ndi.sobel(smoothed, axis=0)
    jsobel = ndi.sobel(smoothed, axis=1)
    size = np.hypot(isobel, jsobel)
    lo, hi = 4.0 * low, 4.0 * high
    nearest = np.inf
    for x in range(1, HEIGHT - 1):
        for y in range(1, WIDTH - 1):
            m = size[x, y]
            nearest = min(nearest, abs(m - lo), abs(m - hi))
            if m < lo:
                continue
            ai, aj = abs(isobel[x, y]), abs(jsobel[x, y])
            up, down = isobel[x, y] >= 0, isobel[x, y] <= 0
            left, right = jsobel[x, y] <= 0, jsobel[x, y] >= 0
            if (up and right) or (down and left):
                if ai > aj:
                    w, n = aj / ai, (size[x + 1, y], size[x + 1, y + 1], size[x - 1, y], size[x - 1, y - 1])
                else:
                    w, n = ai / aj, (size[x, y + 1], size[x + 1, y + 1], size[x, y - 1], size[x - 1, y - 1])
            elif ai < aj:
                w, n = ai / aj, (size[x, y + 1], size[x - 1, y + 1], size[x, y - 1], size[x + 1, y - 1])
            else:
                w, n = aj / ai, (size[x - 1, y], size[x - 1, y + 1], size[x + 1, y], size[x + 1, y - 1])
            nearest = min(nearest, abs(n[1] * w + n[0] * (1 - w) - m), abs(n[3] * w + n[2] * (1 - w) - m))
    if nearest < CANNY_MARGIN:
        raise SystemExit("Canny with sigma {} and mode {} decides {:.3g} from a threshold".format(sigma, mode, nearest))
    edges = feature.canny(values, sigma=sigma, low_threshold=lo, high_threshold=hi, mode=mode)
    return edges, nearest


def encoded(v):
    """v encoded by the sRGB curve, extended above 1 and below 0."""
    return np.where(v <= 0.0031308, v * 12.92, 1.055 * np.maximum(v, 0.0031308) ** (1.0 / 2.4) - 0.055)


def scaled(v, scale, step):
    """v on the smoothing's scale: encoded as sRGB stores it, the base 10
    log continued below step as the line tangent to it, or linear."""
    if scale == "Encoded":
        return encoded(v)
    if scale == "Log":
        return np.where(v >= step, np.log10(np.maximum(v, step)), np.log10(step) + (v - step) / (step * np.log(10.0)))
    return v


def smooth_keys(image, by, scale, step):
    """The keys the smoothing compares, HEIGHT by WIDTH by 4: by luminance,
    Rec. 709 of the straight color on the scale, in the first channel; by
    channel, each color on the scale, and alpha as it is."""
    alpha = image[..., 3].astype(np.float64)
    color = np.where(alpha[..., None] > 0, image[..., :3].astype(np.float64), 0.0)
    if by == "Luminance":
        y = scaled(color @ np.array([0.2126, 0.7152, 0.0722]), scale, step)
        return np.stack([y, np.zeros_like(y), np.zeros_like(y), np.zeros_like(y)], -1)
    return np.concatenate([scaled(color, scale, step), alpha[..., None]], -1)


def weighed(image):
    """image's color weighed by alpha, beside its alpha."""
    out = image.astype(np.float64)
    out[..., :3] *= out[..., 3:4]
    return out


def joint(guide, values, sigma, rng, mode):
    """OpenCV's joint bilateral filter of values keyed by guide over the
    circular window of radius floor(4 sigma + 1/2). OpenCV leaves an image
    whose guide is one value as it is, unsmoothed, so such a guide is
    refused."""
    if guide.max() - guide.min() < 1e-6:
        raise SystemExit("a guide of one value is left unsmoothed by OpenCV")
    radius = int(np.floor(4.0 * sigma + 0.5))
    return cv2.ximgproc.jointBilateralFilter(guide.astype(np.float32), values.astype(np.float32), 2 * radius + 1, rng, sigma, borderType=CV_BORDERS[mode]).astype(np.float64)


def straight_and_alpha(color, alpha):
    """Colors and alpha as the store writes them: a texel of alpha 0
    transparent black."""
    return np.concatenate([np.where(alpha[..., None] > 0, color, 0.0), alpha[..., None]], -1)


def smooth_exact(image, sigma, rng, by, scale, step, mode):
    """The smoothing summed exactly: by luminance, the texels weighed by
    alpha averaged by their keys' weights; by channel, each color's mean
    weighed by alpha and its own key's weights, and alpha's by its own."""
    keys, values = smooth_keys(image, by, scale, step), weighed(image)
    if by == "Luminance":
        alpha = joint(keys[..., 0], values[..., 3], sigma, rng, mode)
        sums = joint(keys[..., 0], values[..., :3], sigma, rng, mode)
        with np.errstate(divide="ignore", invalid="ignore"):
            return straight_and_alpha(sums / alpha[..., None], alpha)
    colors = []
    for c in range(3):
        num = joint(keys[..., c], values[..., c], sigma, rng, mode)
        den = joint(keys[..., c], values[..., 3], sigma, rng, mode)
        with np.errstate(divide="ignore", invalid="ignore"):
            colors.append(np.where(den > 0, num / den, 0.0))
    return straight_and_alpha(np.stack(colors, -1), joint(keys[..., 3], values[..., 3], sigma, rng, mode))


def gaussian_weights(sigma):
    """A Gaussian's weights reaching 4 standard deviations, rounded as
    scipy's gaussian_filter rounds its truncate of 4, summing to 1."""
    radius = int(np.floor(4.0 * sigma + 0.5))
    raw = np.exp(-0.5 * (np.arange(-radius, radius + 1) / sigma) ** 2)
    return raw / raw.sum()


def grid_sums(values, guide, sigma, rng, spacing, mode):
    """The bilateral grid's sums of values, HEIGHT by WIDTH by n, and of
    weight, keyed by guide, as the module works it: the texels within the
    window's reach past the edges placed by tents into cells spacing
    standard deviations apart across, down and along the key from the
    image's smallest key; the cells blurred on each axis by a Gaussian of
    sqrt(1 / spacing^2 - 1 / 3) cells, empty past the grid; each texel read
    back by trilinear interpolation."""
    reach = int(np.floor(4.0 * sigma + 0.5))
    ds, dr = spacing * sigma, spacing * rng
    low = guide.min()
    padded_values = np.pad(values, ((reach, reach), (reach, reach), (0, 0)), mode=PAD_MODES[mode])
    padded_guide = np.pad(guide, reach, mode=PAD_MODES[mode])
    h, w = padded_guide.shape
    cells = (int(np.floor((HEIGHT - 1 + 2 * reach) / ds)) + 2, int(np.floor((WIDTH - 1 + 2 * reach) / ds)) + 2, int(np.floor((guide.max() - low) / dr)) + 2)
    ys, xs = np.mgrid[0:h, 0:w]
    coords = [ys / ds, xs / ds, (padded_guide - low) / dr]
    payload = np.zeros(cells + (values.shape[-1],))
    mass = np.zeros(cells)
    base = [np.floor(c).astype(np.int64) for c in coords]
    frac = [c - b for c, b in zip(coords, base)]
    for corner in range(8):
        share = np.ones(padded_guide.shape)
        cell = []
        for axis in range(3):
            up = (corner >> axis) & 1
            share = share * (frac[axis] if up else 1.0 - frac[axis])
            cell.append(base[axis] + up)
        np.add.at(payload, tuple(cell), share[..., None] * padded_values)
        np.add.at(mass, tuple(cell), share)
    spread = np.sqrt(max(1.0 / spacing ** 2 - 1.0 / 3.0, 0.0))
    if spread > 0:
        weights = gaussian_weights(spread)
        for axis in range(3):
            payload = ndi.correlate1d(payload, weights, axis=axis, mode="constant")
            mass = ndi.correlate1d(mass, weights, axis=axis, mode="constant")
    inner = (slice(reach, h - reach), slice(reach, w - reach))
    at = [c[inner] for c in coords]
    sums = np.stack([ndi.map_coordinates(payload[..., i], at, order=1, mode="constant") for i in range(values.shape[-1])], -1)
    return sums, ndi.map_coordinates(mass, at, order=1, mode="constant")


def smooth_grid(image, sigma, rng, by, scale, step, spacing, mode):
    """The smoothing through the bilateral grid, as smooth_exact sums it."""
    keys, values = smooth_keys(image, by, scale, step), weighed(image)
    if by == "Luminance":
        sums, weight = grid_sums(values, keys[..., 0], sigma, rng, spacing, mode)
        texels = sums / weight[..., None]
        with np.errstate(divide="ignore", invalid="ignore"):
            return straight_and_alpha(texels[..., :3] / texels[..., 3:4], texels[..., 3])
    colors = []
    for c in range(3):
        sums, _ = grid_sums(np.stack([values[..., c], values[..., 3]], -1), keys[..., c], sigma, rng, spacing, mode)
        with np.errstate(divide="ignore", invalid="ignore"):
            colors.append(np.where(sums[..., 1] > 0, sums[..., 0] / sums[..., 1], 0.0))
    sums, weight = grid_sums(np.stack([values[..., 3], values[..., 3]], -1), keys[..., 3], sigma, rng, spacing, mode)
    return straight_and_alpha(np.stack(colors, -1), sums[..., 0] / weight)


def clarity_inputs(image):
    """Clarity's keys, each texel's luminance on the log scale, with the
    texel's straight color and its alpha."""
    keys = smooth_keys(image, "Luminance", "Log", FLOAT_STEP)[..., 0]
    alpha = image[..., 3].astype(np.float64)
    color = np.where(alpha[..., None] > 0, image[..., :3].astype(np.float64), 0.0)
    return keys, color, alpha


def clarity_scaled(keys, color, alpha, sums, weight, amount):
    """The color scaled by 10 to the power of amount / 100 times the
    detail, how far each key is from the filter's mean, sums over weight;
    a texel of alpha 0 transparent black."""
    with np.errstate(divide="ignore", invalid="ignore"):
        detail = np.where(alpha > 0, keys - sums / weight, 0.0)
    return straight_and_alpha(color * (10.0 ** ((amount / 100.0) * detail))[..., None], alpha)


def clarity_exact(image, amount, radius, rng, mode):
    """Clarity summed exactly: the filter's mean is OpenCV's joint
    bilateral filter of the luminance on the log scale weighed by alpha,
    over that of alpha, keyed by that luminance."""
    keys, color, alpha = clarity_inputs(image)
    return clarity_scaled(keys, color, alpha, joint(keys, keys * alpha, radius, rng, mode), joint(keys, alpha, radius, rng, mode), amount)


def clarity_grid(image, amount, radius, rng, spacing, mode):
    """Clarity through the bilateral grid, as clarity_exact sums it, the
    grid carrying the key, the key weighed by alpha, 0 and alpha, as the
    module's does."""
    keys, color, alpha = clarity_inputs(image)
    sums, _ = grid_sums(np.stack([keys, keys * alpha, np.zeros_like(keys), alpha], -1), keys, radius, rng, spacing, mode)
    return clarity_scaled(keys, color, alpha, sums[..., 1], sums[..., 3], amount)


def f32(v):
    """v rounded to float32, in the fewest digits that give that float32 back."""
    text = np.format_float_positional(np.float32(v), unique=True, trim="-")
    return text if "." in text else text + ".0"


def rust_spots(out, name, what, image):
    out.append("/// {}: the texels at SPOTS, red, green, blue and alpha.".format(what))
    out.append("pub(crate) const {}: [[f32; 4]; {}] = [".format(name, len(SPOTS)))
    for x, y in SPOTS:
        out.append("    [{}],".format(", ".join(f32(v) for v in image[y, x])))
    out.append("];")
    out.append("")


def rust_edges(out, name, what, edges, nearest):
    out.append("/// {}: a row of 0s and 1s for each row of texels. The decision nearest a".format(what))
    out.append("/// threshold is {:.3g} from it, where a step from 0 to 1 reads 4.".format(nearest))
    out.append("pub(crate) const {}: [&str; {}] = [".format(name, HEIGHT))
    for row in edges:
        out.append('    "{}",'.format("".join("1" if e else "0" for e in row)))
    out.append("];")
    out.append("")


HEADER = """//! The values tools/filter_reference.py computes with scipy.ndimage {},
//! scikit-image {}, OpenCV {} and numpy {}, from the test images
//! filter::tests builds: the texels at SPOTS of Gaussian blurs with each
//! FilterEdge, unsharp masks, gradients and Laplacians, Canny's edges of
//! the gray image, and the smoothing and clarity summed exactly by
//! OpenCV's joint bilateral filter and through the bilateral grid by a
//! model of the module's. The script writes this file; run it again after
//! a change to what it computes.

/// The test images' width and height.
pub(crate) const WIDTH: u32 = {};
pub(crate) const HEIGHT: u32 = {};

/// The texels the constants of colors give, (x, y).
pub(crate) const SPOTS: [(u32, u32); {}] = [{}];
"""


def main(path):
    out = [HEADER.format(scipy.__version__, skimage.__version__, cv2.__version__, np.__version__, WIDTH, HEIGHT, len(SPOTS), ", ".join("({}, {})".format(x, y) for x, y in SPOTS))]
    color = color_image(False)
    for name, sigmas, mode in [("BLUR_CLAMP", (1.5, 0.75), "Clamp"), ("BLUR_WRAP", (2.0, 2.0), "Wrap"), ("BLUR_MIRROR", (0.625, 1.75), "Mirror"), ("BLUR_WRAP_WIDE", (12.0, 10.0), "Wrap"), ("BLUR_MIRROR_WIDE", (12.0, 10.0), "Mirror")]:
        what = "The color image blurred by standard deviations {} across and {} down, {}".format(sigmas[0], sigmas[1], mode)
        rust_spots(out, name, what, blur(color, sigmas, MODES[mode]))
    what = "The opaque color image blurred by standard deviations 1.5 across and 0.75 down, Clamp"
    rust_spots(out, "BLUR_CLAMP_OPAQUE", what, blur(color_image(True), (1.5, 0.75), "nearest"))
    for name, amount, radius, threshold, mode in [("SHARPEN_CLAMP", 1.5, 1.25, 0.046875, "Clamp"), ("SHARPEN_WRAP", 0.75, 2.0, 0.0, "Wrap")]:
        what = "The color image sharpened by {} times its difference from a blur of standard deviation {}, at least {}, {}".format(amount, radius, threshold, mode)
        rust_spots(out, name, what, sharpen(color, amount, radius, threshold, MODES[mode]))
    opaque = color_image(True)[..., :3].astype(np.float64)
    for name, operator, weights, mode in [("SOBEL_CLAMP", "Sobel", (1, 2, 1), "Clamp"), ("SCHARR_CLAMP", "Scharr", (3, 10, 3), "Clamp"), ("PREWITT_CLAMP", "Prewitt", (1, 1, 1), "Clamp"), ("SOBEL_WRAP", "Sobel", (1, 2, 1), "Wrap"), ("SOBEL_MIRROR", "Sobel", (1, 2, 1), "Mirror")]:
        what = "The {} gradient of the opaque color image, {}".format(operator, mode)
        rust_spots(out, name, what, with_alpha(gradient(opaque, weights, MODES[mode])))
    laplacian = np.abs(np.stack([ndi.laplace(opaque[..., c], mode="nearest") for c in range(3)], -1))
    rust_spots(out, "LAPLACIAN_CLAMP", "The size of the Laplacian of the opaque color image, Clamp", with_alpha(laplacian))
    stored = stored_bytes()[..., :3].astype(np.float64) / 255.0
    srgb = np.clip(gradient(stored, (1, 2, 1), "nearest"), 0.0, 1.0) * 255.0
    rust_spots(out, "SOBEL_SRGB_STORED", "The Sobel gradient of the sRGB texture's stored values, Clamp, as stored values from 0 to 255", with_alpha(srgb, 255.0))
    gray = gray_image()
    for name, sigma, low, high, mode in [("CANNY_CLAMP", 1.0, 0.1, 0.2, "Clamp"), ("CANNY_WRAP", 1.0, 0.09375, 0.1875, "Wrap"), ("CANNY_MIRROR", 1.0, 0.1, 0.2, "Mirror"), ("CANNY_WIDE", 1.375, 0.0625, 0.15625, "Clamp")]:
        edges, nearest = canny(gray, sigma, low, high, MODES[mode])
        what = "Canny's edges of the gray image, standard deviation {}, thresholds {} and {}, {}".format(sigma, low, high, mode)
        rust_edges(out, name, what, edges, nearest)
        print("{}: {} edge texels, nearest decision {:.3g}".format(name, int(edges.sum()), nearest))
    for name, sigma, rng, by, scale, mode in [
        ("SMOOTH_EXACT_CLAMP", 2.0, 0.1, "Luminance", "Encoded", "Clamp"),
        ("SMOOTH_EXACT_CHANNELS_WRAP", 1.5, 0.2, "Channels", "Linear", "Wrap"),
        ("SMOOTH_EXACT_LOG_MIRROR", 2.5, 0.15, "Luminance", "Log", "Mirror"),
    ]:
        what = "The color image smoothed exactly, standard deviation {}, range {} by {} on the {} scale, {}".format(sigma, rng, by, scale, mode)
        rust_spots(out, name, what, smooth_exact(color, sigma, rng, by, scale, FLOAT_STEP, mode))
    for name, sigma, rng, by, scale, spacing, mode in [
        ("SMOOTH_GRID_CLAMP", 3.0, 0.1, "Luminance", "Encoded", 0.5, "Clamp"),
        ("SMOOTH_GRID_CHANNELS_MIRROR", 2.0, 0.1, "Channels", "Encoded", 0.5, "Mirror"),
        ("SMOOTH_GRID_FINE_WRAP", 1.5, 0.01, "Luminance", "Linear", 0.75, "Wrap"),
    ]:
        model = smooth_grid(color, sigma, rng, by, scale, FLOAT_STEP, spacing, mode)
        exact = smooth_exact(color, sigma, rng, by, scale, FLOAT_STEP, mode)
        # Compared weighed by alpha, as they are seen: a texel's color is a
        # ratio of two sums that both near 0 as its alpha does.
        apart = np.abs(weighed(model) - weighed(exact))
        print("{}: the grid model is within {:.4g} of OpenCV's exact filter weighed by alpha, {:.3g} on average".format(name, apart.max(), apart.mean()))
        what = "The color image smoothed through the bilateral grid, cells {} standard deviations apart, standard deviation {}, range {} by {} on the {} scale, {}".format(spacing, sigma, rng, by, scale, mode)
        rust_spots(out, name, what, model)
    for name, amount, radius, rng, mode in [("CLARITY_EXACT_CLAMP", 50.0, 2.0, 0.4, "Clamp"), ("CLARITY_EXACT_WRAP", -60.0, 1.5, 0.25, "Wrap")]:
        what = "The color image's detail scaled by clarity {:g} against a filter summed exactly, standard deviation {}, range {} in the base 10 log, {}".format(amount, radius, rng, mode)
        rust_spots(out, name, what, clarity_exact(color, amount, radius, rng, mode))
    for name, amount, radius, rng, spacing, mode in [("CLARITY_GRID_MIRROR", 80.0, 3.0, 0.4, 0.5, "Mirror")]:
        model = clarity_grid(color, amount, radius, rng, spacing, mode)
        apart = np.abs(weighed(model) - weighed(clarity_exact(color, amount, radius, rng, mode)))
        print("{}: the grid model is within {:.4g} of OpenCV's exact filter weighed by alpha, {:.3g} on average".format(name, apart.max(), apart.mean()))
        what = "The color image's detail scaled by clarity {:g} against a filter through the bilateral grid, cells {} standard deviations apart, standard deviation {}, range {} in the base 10 log, {}".format(amount, spacing, radius, rng, mode)
        rust_spots(out, name, what, model)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(out))
    print("wrote", path)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: python -u -I filter_reference.py <path of the Rust file to write>")
    main(sys.argv[1])
