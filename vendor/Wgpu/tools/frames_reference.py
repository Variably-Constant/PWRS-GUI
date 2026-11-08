"""Animated and multi-page files for Wgpu's tests of frames and pages as
array layers, each with the frames a reference decoder gives it: Pillow
for GIF, APNG, WebP and TIFF, its WebP through libwebp, and libjxl through
ffmpeg for JPEG XL.

The files: an animated GIF whose later frames cover part of the canvas and
dispose to nothing, to the background and to the previous frame; two small
GIFs, one that repeats twice and one with no NETSCAPE2.0 extension; an APNG
whose frames blend over the canvas and replace it, played twice; a lossless
animated WebP of the same frames and a still lossy WebP of the first; a
TIFF of two pages of different sizes; and the same frames as a JPEG XL
animation, encoded losslessly by libjxl. Each frame is given as RGBA, with
its delay and the times the file plays: a GIF's repeat count counts the
plays after the first, so it plays one time more, and a GIF without one
plays once. Two more APNGs test the compositing: one of four frames whose
second clears its area and whose third puts its area back, and a 16-bit one
ffmpeg's encoder writes, with the frames ffmpeg's decoder composites from it.

It writes the Rust test constants in src/frames_reference.rs. From the
repository's folder, with Pillow installed and ffmpeg on the PATH with
libjxl's animated encoder and decoder (ffmpeg -encoders lists libjxl_anim):
python -u -I tools/frames_reference.py src/frames_reference.rs
"""

import io
import os
import struct
import subprocess
import sys
import tempfile

from PIL import Image

WIDTH, HEIGHT = 8, 6
CLEAR = (0, 0, 0, 0)
RED, GREEN, BLUE, WHITE = (204, 51, 0, 255), (0, 153, 51, 255), (51, 102, 204, 255), (255, 255, 255, 255)
DELAYS = [100, 200, 300]
# The JPEG XL animation's frames per second, which gives each frame a delay
# of 250 milliseconds.
JXL_RATE = 4


def canvas(fill):
    return Image.new("RGBA", (WIDTH, HEIGHT), fill)


def frames():
    """Three frames: red with a blue square, then a green block over part of
    it, then a white bar on a clear canvas."""
    first = canvas(RED)
    first.paste(BLUE, (1, 1, 4, 4))
    second = first.copy()
    second.paste(GREEN, (4, 2, 7, 5))
    third = canvas(CLEAR)
    third.paste(WHITE, (0, 5, 8, 6))
    return [first, second, third]


def written(first, rest, fmt, **params):
    buffer = io.BytesIO()
    first.save(buffer, fmt, save_all=True, append_images=rest, **params)
    return buffer.getvalue()


def pillow_frames(data):
    """Each frame Pillow decodes from `data`, as its width, height and RGBA
    samples, with each frame's delay in milliseconds and the file's loop
    value, None where it has none."""
    image = Image.open(io.BytesIO(data))
    out = []
    delays = []
    for index in range(getattr(image, "n_frames", 1)):
        image.seek(index)
        out.append((image.size[0], image.size[1], image.convert("RGBA").tobytes()))
        delays.append(round(image.info.get("duration", 0)))
    return out, delays, image.info.get("loop")


def gif_plays(loop):
    """The times a GIF plays by Pillow's loop value, its NETSCAPE2.0 repeat
    count: 0 for endlessly, the repeats and one more, or once without one."""
    if loop is None:
        return 1
    return 0 if loop == 0 else loop + 1


def jxl_animation(images, folder):
    """The JPEG XL animation libjxl makes of `images`, losslessly, at
    JXL_RATE frames a second, with the frames libjxl decodes from it through
    ffmpeg, composited, and each frame's delay at that rate in
    milliseconds."""
    path = os.path.join(folder, "animated.jxl")
    raw = b"".join(image.tobytes() for image in images)
    # ffmpeg has no JPEG XL muxer; the encoder's packets piped out are the file.
    encoded = subprocess.run(["ffmpeg", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", f"{WIDTH}x{HEIGHT}", "-framerate", str(JXL_RATE),
                              "-i", "-", "-c:v", "libjxl_anim", "-distance", "0", "-f", "image2pipe", "-"], input=raw, capture_output=True, check=True).stdout
    with open(path, "wb") as f:
        f.write(encoded)
    # ffmpeg's jpegxl_anim demuxer finds no frame size in the file, so the
    # pipe demuxer hands it to libjxl's decoder, which composites each frame.
    decoded = subprocess.run(["ffmpeg", "-v", "error", "-f", "jpegxl_pipe", "-c:v", "libjxl", "-i", path, "-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "rgba", "-"],
                             capture_output=True, check=True).stdout
    size = WIDTH * HEIGHT * 4
    if len(decoded) != size * len(images):
        raise SystemExit(f"libjxl decoded {len(decoded)} bytes, not {len(images)} {WIDTH} by {HEIGHT} RGBA frames")
    out = [(WIDTH, HEIGHT, decoded[i:i + size]) for i in range(0, len(decoded), size)]
    return encoded, out, [1000 // JXL_RATE] * len(images)


def rust_bytes(name, data, what, gate=""):
    rows = [", ".join(str(b) for b in data[i:i + 24]) for i in range(0, len(data), 24)]
    body = ",\n    ".join(rows)
    return f"/// {what}\n{gate}pub(crate) const {name}: [u8; {len(data)}] = [\n    {body},\n];\n"


def rust_frames(name, decoded, what, gate=""):
    entries = []
    for w, h, rgba in decoded:
        rows = [", ".join(str(b) for b in rgba[i:i + 24]) for i in range(0, len(rgba), 24)]
        entries.append(f"({w}, {h}, &[\n        " + ",\n        ".join(rows) + ",\n    ])")
    body = ",\n    ".join(entries)
    return f"/// {what}\n{gate}pub(crate) const {name}: [(u32, u32, &[u8]); {len(decoded)}] = [\n    {body},\n];\n"


def rust_frames16(name, decoded, what):
    entries = []
    for w, h, samples in decoded:
        rows = [", ".join(str(v) for v in samples[i:i + 16]) for i in range(0, len(samples), 16)]
        entries.append(f"({w}, {h}, &[\n        " + ",\n        ".join(rows) + ",\n    ])")
    body = ",\n    ".join(entries)
    return f"/// {what}\npub(crate) const {name}: [(u32, u32, &[u16]); {len(decoded)}] = [\n    {body},\n];\n"


def sixteen(image):
    """`image`'s RGBA samples at 16 bits: each 8-bit level times 257, a
    color's low byte then changed by its place so it differs from its high
    byte; alpha stays 0 or full where it is."""
    rgba = image.tobytes()
    out = []
    for index in range(WIDTH * HEIGHT):
        r, g, b, a = rgba[index * 4:index * 4 + 4]
        x, y = index % WIDTH, index // WIDTH
        jitter = (x * 37 + y * 11) % 200 + 1
        out += [(v * 257) ^ jitter for v in (r, g, b)] + [a * 257]
    return out


def apng16(images, folder):
    """`images` as a 16-bit APNG that ffmpeg's encoder writes at 10 frames a
    second, played twice, with the 16-bit RGBA frames its decoder composites
    from it and each frame's delay in milliseconds."""
    path = os.path.join(folder, "animated16.png")
    raw = b"".join(struct.pack(f">{len(s)}H", *s) for s in (sixteen(image) for image in images))
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba64be", "-s", f"{WIDTH}x{HEIGHT}", "-framerate", "10",
                    "-i", "-", "-plays", "2", "-f", "apng", path], input=raw, check=True)
    decoded = subprocess.run(["ffmpeg", "-v", "error", "-i", path, "-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "rgba64be", "-"],
                             capture_output=True, check=True).stdout
    size = WIDTH * HEIGHT * 4 * 2
    if len(decoded) != size * len(images):
        raise SystemExit(f"ffmpeg decoded {len(decoded)} bytes, not {len(images)} 16-bit {WIDTH} by {HEIGHT} RGBA frames")
    frames = [(WIDTH, HEIGHT, list(struct.unpack(f">{size // 2}H", decoded[i:i + size]))) for i in range(0, len(decoded), size)]
    with open(path, "rb") as f:
        return f.read(), frames, [100] * len(images)


def rust_u32s(name, values, what, gate=""):
    return f"/// {what}\n{gate}pub(crate) const {name}: [u32; {len(values)}] = [{', '.join(str(v) for v in values)}];\n"


def rust_u32(name, value, what):
    return f"/// {what}\npub(crate) const {name}: u32 = {value};\n"


def main(out):
    first, second, third = frames()
    parts = [
        "//! Animated and multi-page files and the frames reference decoders give",
        "//! them, for the tests of the module's frames and pages: written by",
        "//! tools/frames_reference.py, with Pillow for GIF, APNG and TIFF and",
        "//! libjxl through ffmpeg for JPEG XL. Each frame is its width, height",
        "//! and 8-bit RGBA samples.",
        "",
    ]

    gif = written(first, [second, third], "GIF", duration=DELAYS, loop=0, disposal=[1, 2, 3], optimize=False)
    decoded, delays, loop = pillow_frames(gif)
    parts.append(rust_bytes("GIF_FILE", gif, f"An animated GIF of {len(decoded)} frames on a {WIDTH} by {HEIGHT} canvas, whose later frames\n/// dispose to nothing, to the background and to the previous frame."))
    parts.append(rust_frames("GIF_FRAMES", decoded, "GIF_FILE's frames as Pillow composites them."))
    parts.append(rust_u32s("GIF_DELAYS", delays, "GIF_FILE's delays in milliseconds."))
    parts.append(rust_u32("GIF_PLAYS", gif_plays(loop), "How many times GIF_FILE plays, 0 for endlessly."))
    print(f"GIF: {len(gif)} bytes, {len(decoded)} frames, delays {delays}, loop {loop}", flush=True)

    for name, params, what in [("GIF_TWICE", {"loop": 2}, "repeats twice"), ("GIF_ONCE", {}, "has no NETSCAPE2.0 extension")]:
        data = written(first, [second], "GIF", duration=DELAYS[:2], optimize=False, **params)
        _, _, loop = pillow_frames(data)
        parts.append(rust_bytes(f"{name}_FILE", data, f"An animated GIF of two frames that {what}."))
        parts.append(rust_u32(f"{name}_PLAYS", gif_plays(loop), f"How many times {name}_FILE plays."))
        print(f"{name}: {len(data)} bytes, loop {loop}", flush=True)

    apng = written(first, [second, third], "PNG", duration=DELAYS, loop=2, disposal=[0, 1, 2], blend=[0, 1, 0])
    decoded, delays, loop = pillow_frames(apng)
    parts.append(rust_bytes("APNG_FILE", apng, f"An APNG of {len(decoded)} frames on a {WIDTH} by {HEIGHT} canvas, whose frames blend\n/// over the canvas and replace it and dispose to nothing, to the background\n/// and to the previous frame."))
    parts.append(rust_frames("APNG_FRAMES", decoded, "APNG_FILE's frames as Pillow composites them."))
    parts.append(rust_u32s("APNG_DELAYS", delays, "APNG_FILE's delays in milliseconds."))
    parts.append(rust_u32("APNG_PLAYS", loop, "How many times APNG_FILE plays, its num_plays."))
    print(f"APNG: {len(apng)} bytes, {len(decoded)} frames, delays {delays}, loop {loop}", flush=True)

    fourth = canvas(CLEAR)
    fourth.paste(BLUE, (2, 0, 6, 3))
    apng4 = written(first, [second, third, fourth], "PNG", duration=DELAYS + [400], loop=0, disposal=[0, 1, 2, 0], blend=[0, 1, 0, 1])
    decoded, delays, loop = pillow_frames(apng4)
    parts.append(rust_bytes("APNG_PUT_BACK_FILE", apng4, f"An APNG of {len(decoded)} frames whose second clears its area and whose third puts its\n/// area back as it was before that frame."))
    parts.append(rust_frames("APNG_PUT_BACK_FRAMES", decoded, "APNG_PUT_BACK_FILE's frames as Pillow composites them."))
    parts.append(rust_u32s("APNG_PUT_BACK_DELAYS", delays, "APNG_PUT_BACK_FILE's delays in milliseconds."))
    print(f"APNG put back: {len(apng4)} bytes, {len(decoded)} frames, delays {delays}, loop {loop}", flush=True)

    with tempfile.TemporaryDirectory() as folder:
        apng16_file, decoded, delays = apng16([first, second, third], folder)
    parts.append(rust_bytes("APNG16_FILE", apng16_file, f"A 16-bit APNG of {len(decoded)} frames, which ffmpeg's encoder writes at 10 frames a\n/// second, played twice."))
    parts.append(rust_frames16("APNG16_FRAMES", decoded, "APNG16_FILE's frames as ffmpeg's decoder composites them, 16-bit RGBA."))
    parts.append(rust_u32s("APNG16_DELAYS", delays, "APNG16_FILE's delays in milliseconds, each a frame's at 10 frames a second."))
    print(f"APNG16: {len(apng16_file)} bytes, {len(decoded)} frames", flush=True)

    # The module decodes WebP through libwebp, as Pillow does, only in a
    # build with its webp feature.
    webp_gate = '#[cfg(feature = "webp")]\n'
    webp = written(first, [second, third], "WEBP", duration=DELAYS, loop=0, lossless=True)
    decoded, delays, loop = pillow_frames(webp)
    parts.append(rust_bytes("WEBP_FILE", webp, f"A lossless animated WebP of {len(decoded)} frames on a {WIDTH} by {HEIGHT} canvas.", webp_gate))
    parts.append(rust_frames("WEBP_FRAMES", decoded, "WEBP_FILE's frames as Pillow composites them through libwebp.", webp_gate))
    parts.append(rust_u32s("WEBP_DELAYS", delays, "WEBP_FILE's delays in milliseconds.", webp_gate))
    parts.append(f"/// How many times WEBP_FILE plays, its loop count.\n{webp_gate}pub(crate) const WEBP_PLAYS: u32 = {loop};\n")
    print(f"WEBP: {len(webp)} bytes, {len(decoded)} frames, delays {delays}, loop {loop}", flush=True)

    buffer = io.BytesIO()
    first.convert("RGB").save(buffer, "WEBP", quality=80)
    lossy = buffer.getvalue()
    decoded, _, _ = pillow_frames(lossy)
    parts.append(rust_bytes("WEBP_LOSSY_FILE", lossy, f"A still lossy WebP of the first frame at quality 80, {WIDTH} by {HEIGHT}.", webp_gate))
    parts.append(rust_frames("WEBP_LOSSY_PIXELS", decoded, "WEBP_LOSSY_FILE as Pillow decodes it through libwebp.", webp_gate))
    print(f"WEBP lossy: {len(lossy)} bytes", flush=True)

    tiff = written(first, [Image.new("RGBA", (5, 9), BLUE)], "TIFF", compression="tiff_deflate")
    decoded, _, _ = pillow_frames(tiff)
    sizes = ", ".join(f"{w} by {h}" for w, h, _ in decoded)
    parts.append(rust_bytes("TIFF_FILE", tiff, f"A TIFF of {len(decoded)} RGBA pages, {sizes}, compressed with deflate."))
    parts.append(rust_frames("TIFF_PAGES", decoded, "TIFF_FILE's pages as Pillow decodes them."))
    print(f"TIFF: {len(tiff)} bytes, pages {sizes}", flush=True)

    with tempfile.TemporaryDirectory() as folder:
        jxl, decoded, delays = jxl_animation([first, second, third], folder)
    # The module reads JPEG XL only in a build with its jxl feature.
    gate = '#[cfg(feature = "jxl")]\n'
    parts.append(rust_bytes("JXL_FILE", jxl, f"A JPEG XL animation of {len(decoded)} frames on a {WIDTH} by {HEIGHT} canvas, encoded\n/// losslessly by libjxl at {JXL_RATE} frames a second.", gate))
    parts.append(rust_frames("JXL_FRAMES", decoded, "JXL_FILE's frames as libjxl decodes them, composited.", gate))
    parts.append(rust_u32s("JXL_DELAYS", delays, f"JXL_FILE's delays in milliseconds, each a frame's at {JXL_RATE} frames a second.", gate))
    print(f"JXL: {len(jxl)} bytes, {len(decoded)} frames, delays {delays}", flush=True)

    with open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(parts))
    print("FRAMES REFERENCE DONE", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    main(sys.argv[1])
