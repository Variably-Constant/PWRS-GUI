"""Reference values for Wgpu's image filters, from scipy.ndimage and
scikit-image: Gaussian blurs with each FilterEdge, unsharp masks, the
Sobel, Scharr and Prewitt gradients and the Laplacian, a Sobel gradient of
an sRGB texture's stored values, and Canny's edges.

The test images are made of integer and dyadic arithmetic, so the float32
texels here and those filter::tests builds are the same bits. The script
refuses to write when a decision Canny or the unsharp mask makes on them
is within a hair of going the other way, as float32 on a GPU could tip it.

It writes the Rust test constants in src/filter_reference.rs. From the
repository's folder, with numpy, scipy and scikit-image installed:
python -u -I tools/filter_reference.py src/filter_reference.rs
"""

import sys

import numpy as np
import scipy
import scipy.ndimage as ndi
import skimage
from skimage import feature

WIDTH, HEIGHT = 45, 37
MODES = {"Clamp": "nearest", "Wrap": "wrap", "Mirror": "reflect"}
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


HEADER = """//! The values tools/filter_reference.py computes with scipy.ndimage {} and
//! scikit-image {}, numpy {}, from the test images filter::tests builds:
//! the texels at SPOTS of Gaussian blurs with each FilterEdge, unsharp
//! masks, gradients and Laplacians, and Canny's edges of the gray image.
//! The script writes this file; run it again after a change to what it
//! computes.

/// The test images' width and height.
pub(crate) const WIDTH: u32 = {};
pub(crate) const HEIGHT: u32 = {};

/// The texels the constants of colors give, (x, y).
pub(crate) const SPOTS: [(u32, u32); {}] = [{}];
"""


def main(path):
    out = [HEADER.format(scipy.__version__, skimage.__version__, np.__version__, WIDTH, HEIGHT, len(SPOTS), ", ".join("({}, {})".format(x, y) for x, y in SPOTS))]
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
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(out))
    print("wrote", path)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: python -u -I filter_reference.py <path of the Rust file to write>")
    main(sys.argv[1])
