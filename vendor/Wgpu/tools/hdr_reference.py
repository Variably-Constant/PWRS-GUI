"""Reference values for Wgpu's loading of an HDR AVIF's light, from
colour-science (colour-science.org): 16-bit PQ and HLG signals in BT.2020
primaries taken to linear light in sRGB's primaries, BT.709's, with
BT.2408's HDR reference white of 203 nits at 1 and light in colors sRGB
lacks clipped at 0. PQ goes through colour.models.eotf_ST2084, HLG
through colour.models.oetf_inverse_BT2100_HLG and
colour.models.ootf_BT2100_HLG for a display of 1,000 nits with no black
lift, and the primaries through colour.RGB_to_RGB, both whites D65.

It writes the Rust test constants in src/hdr_reference.rs. From the
repository's folder, with colour-science installed:
python -u -I tools/hdr_reference.py src/hdr_reference.rs
"""

import sys

import colour
import numpy as np

# 16-bit signals: black, the brightest, PQ's 58 percent and HLG's 75
# percent (BT.2408's reference white in each), steps between, and colors,
# a green among them that sRGB lacks.
PIXELS = [
    (0, 0, 0),
    (65535, 65535, 65535),
    (38050, 38050, 38050),
    (49151, 49151, 49151),
    (32768, 32768, 32768),
    (16384, 16384, 16384),
    (52000, 20000, 9000),
    (10000, 50000, 12000),
    (30000, 30000, 60000),
]
REFERENCE_WHITE = 203.0


def shortest(value):
    """A float32 as the fewest significant digits that read back as it, as
    Rust prints one, so clippy finds no excess precision."""
    value = np.float32(value)
    for digits in range(1, 10):
        text = f"{float(value):.{digits}g}"
        if np.float32(text) == value:
            break
    if "." not in text and "e" not in text:
        text += ".0"
    return text + "_f32"


def rust_array(name, kind, values, doc):
    lines = [f"/// {doc}", f"pub(crate) const {name}: [{kind}; {len(values)}] = ["]
    for start in range(0, len(values), 9):
        lines.append("    " + ", ".join(values[start:start + 9]) + ",")
    lines.append("];")
    return "\n".join(lines)


def main(out_path):
    signals = np.array(PIXELS, dtype=np.float64) / 65535.0
    light = {
        "PQ": colour.models.eotf_ST2084(signals),
        "HLG": colour.models.ootf_BT2100_HLG(colour.models.oetf_inverse_BT2100_HLG(signals), L_B=0, L_W=1000),
    }
    parts = [
        "\n".join([
            "//! Reference values for image_decode::tests, written by",
            "//! tools/hdr_reference.py from colour-science: 16-bit PQ and HLG signals",
            "//! in BT.2020 primaries as linear light in sRGB's, BT.2408's HDR",
            "//! reference white of 203 nits at 1.",
        ]),
        rust_array("HDR_PIXELS", "u16", [str(v) for pixel in PIXELS for v in pixel], "The signals, red, green and blue, of each pixel."),
    ]
    for name, nits in light.items():
        linear = colour.RGB_to_RGB(nits / REFERENCE_WHITE, "ITU-R BT.2020", "ITU-R BT.709", chromatic_adaptation_transform=None)
        values = np.maximum(linear, 0.0).astype(np.float32).flatten()
        parts.append(rust_array(f"HDR_{name}", "f32", [shortest(v) for v in values], f"The pixels as {name} signals, as linear light in sRGB's primaries."))
        print(f"wrote {name}", flush=True)
    with open(out_path, "w", newline="\n") as handle:
        handle.write("\n\n".join(parts) + "\n")
    print(f"WROTE {out_path}", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    main(sys.argv[1])
