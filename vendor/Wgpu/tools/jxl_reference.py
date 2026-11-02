"""JPEG XL files for Wgpu's tests of its JPEG XL import: small images of
known samples encoded losslessly (distance 0) by libjxl through ffmpeg,
which must be on the PATH with its libjxl encoder (ffmpeg -encoders lists
it), each with the samples it was made from. An 8-bit RGB, RGBA and gray
image and a 16-bit RGB one, all in sRGB, a 16-bit RGB image whose
samples are PQ signals in BT.2020's primaries, tagged so, and the color
test image tools/profile_reference.py converts, tagged Display P3, whose
conversion the tests compare with littleCMS's of the Display P3 profile.

It writes the Rust test constants in src/jxl_reference.rs. From the
repository's folder, with NumPy installed:
python -u -I tools/jxl_reference.py src/jxl_reference.rs
"""

import os
import subprocess
import sys
import tempfile

import numpy as np

WIDTH, HEIGHT = 4, 3
# The size of tools/profile_reference.py's color test image.
PROFILE_WIDTH, PROFILE_HEIGHT = 16, 8
DISPLAY_P3 = ["-color_primaries", "smpte432", "-color_trc", "iec61966-2-1", "-colorspace", "rgb"]


def profile_test_image():
    """tools/profile_reference.py's color test image by its integer formula,
    interleaved RGB."""
    return np.array([[(x * 37 + y * 91) % 256, (x * x * 3 + y * 5) % 256, (x * y * 7 + 13) % 256]
                     for y in range(PROFILE_HEIGHT) for x in range(PROFILE_WIDTH)], dtype=np.int64).reshape(-1)


def samples(channels, top):
    """A WIDTH by HEIGHT image's samples, interleaved by `channels`, each
    channel a different ramp up to `top`, no two pixels alike."""
    count = WIDTH * HEIGHT * channels
    ramp = np.arange(count, dtype=np.int64)
    return (ramp * 7919 + 13) % (top + 1)


# Each case: its name, ffmpeg's raw pixel format, the channels, the largest
# sample, the color tags ffmpeg writes into the file, and the Rust type of
# its samples.
CASES = [
    ("RGB8", "rgb24", 3, 255, [], "u8"),
    ("RGBA8", "rgba", 4, 255, [], "u8"),
    ("GRAY8", "gray", 1, 255, [], "u8"),
    ("RGB16", "rgb48le", 3, 65535, [], "u16"),
    ("PQ16", "rgb48le", 3, 65535, ["-color_primaries", "bt2020", "-color_trc", "smpte2084", "-colorspace", "rgb"], "u16"),
]


def encoded(raw, pixel_format, tags, folder, size=(WIDTH, HEIGHT)):
    """The JPEG XL file libjxl makes of `raw`, losslessly."""
    path = os.path.join(folder, "out.jxl")
    # The color tags describe the raw input, so they come before -i; ffmpeg
    # writes sRGB's for an input that has none.
    command = ["ffmpeg", "-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", pixel_format, "-s", f"{size[0]}x{size[1]}",
               *tags, "-i", "-", "-c:v", "libjxl", "-distance", "0", "-frames:v", "1", path]
    subprocess.run(command, input=raw, check=True)
    with open(path, "rb") as f:
        return f.read()


def rust_bytes(name, data, what):
    rows = [", ".join(str(b) for b in data[i:i + 24]) for i in range(0, len(data), 24)]
    body = ",\n    ".join(rows)
    return f"/// {what}\npub(crate) const {name}: [u8; {len(data)}] = [\n    {body},\n];\n"


def rust_samples(name, values, kind, what):
    rows = [", ".join(str(int(v)) for v in values[i:i + 24]) for i in range(0, len(values), 24)]
    body = ",\n    ".join(rows)
    return f"/// {what}\npub(crate) const {name}: [{kind}; {len(values)}] = [\n    {body},\n];\n"


def main(out):
    parts = [
        "//! JPEG XL files and the samples they were made from, for the tests of",
        "//! the module's JPEG XL import: written by tools/jxl_reference.py, which",
        "//! encodes each image losslessly with libjxl through ffmpeg.",
        "",
        f"/// The images' width and height.",
        f"pub(crate) const WIDTH: u32 = {WIDTH};",
        f"pub(crate) const HEIGHT: u32 = {HEIGHT};",
        "",
    ]
    with tempfile.TemporaryDirectory() as folder:
        for name, pixel_format, channels, top, tags, kind in CASES:
            values = samples(channels, top)
            dtype = "<u2" if kind == "u16" else "u1"
            data = encoded(values.astype(dtype).tobytes(), pixel_format, tags, folder)
            tagged = "PQ signals in BT.2020's primaries" if tags else "sRGB"
            parts.append(rust_bytes(f"{name}_FILE", data, f"A {WIDTH} by {HEIGHT} {name} JPEG XL file, {tagged}, encoded losslessly."))
            parts.append(rust_samples(f"{name}_SAMPLES", values, kind, f"The samples {name}_FILE was made from, interleaved."))
            print(f"{name}: {len(data)} bytes", flush=True)
        data = encoded(profile_test_image().astype("u1").tobytes(), "rgb24", DISPLAY_P3, folder, (PROFILE_WIDTH, PROFILE_HEIGHT))
        parts.append(rust_bytes("DISPLAYP3_FILE", data, f"A {PROFILE_WIDTH} by {PROFILE_HEIGHT} 8-bit RGB JPEG XL file in Display P3 holding the\n/// color test image tools/profile_reference.py converts, encoded losslessly."))
        print(f"DISPLAYP3: {len(data)} bytes", flush=True)
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(parts))
    print("JXL REFERENCE DONE", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    main(sys.argv[1])
