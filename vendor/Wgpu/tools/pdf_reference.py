"""PDF files for Wgpu's tests of its PDF import, and what each must draw.

Pages are painted with rectangles on whole points, in DeviceRGB and
DeviceGray colors that are whole 8-bit levels, so a page drawn at 72 or 144
dots per inch covers whole pixels and has one right answer. The files: such
a page; the same page turned by /Rotate 90; a page whose CropBox shows part
of its MediaBox, with a rectangle outside it; the page encrypted with a
user password, and with an owner password alone; two pages, the second
black; and a line of text in Helvetica, a font a PDF reader supplies. Each
painted file is drawn through PDFium (pypdfium2), at 72 and 144 dots per
inch on white and on nothing, and the script stops unless PDFium draws
exactly what the rectangles say. pikepdf, through qpdf, encrypts with
AES-256, whose salts are random, so the encrypted files differ from run to
run.

It writes the Rust test constants in src/pdf_reference.rs, and the page of
rectangles as the wiki's page.pdf. From the repository's folder, with
pypdfium2, pikepdf and NumPy installed:
python -u -I tools/pdf_reference.py src/pdf_reference.rs wiki/static/images/page.pdf
"""

import io
import sys

import numpy as np
import pikepdf
import pypdfium2 as pdfium
import pypdfium2.raw as pdfium_c

# The page the rectangles are painted on, in points.
WIDTH, HEIGHT = 40, 30
# Each rectangle: left and top in points from the page's top left, width,
# height, and three DeviceRGB levels or one DeviceGray level.
RECTS = [
    (2, 2, 16, 12, (204, 51, 0)),
    (10, 8, 20, 14, (51, 102, 204)),
    (24, 4, 12, 22, (0, 153, 51)),
    (4, 18, 10, 8, (153,)),
]
# The cropped file's MediaBox, the part its CropBox shows (left, top, width
# and height from the MediaBox's top left), and a rectangle outside it.
MEDIA = (60, 50)
SHOWN = (10, 10, WIDTH, HEIGHT)
HIDDEN = (0, 0, 8, 8, (0, 0, 0))
PASSWORD = "wgpu"
OWNER = "wgpu-owner"
# The text file's page and where its line of text starts, in points from
# the page's bottom left, as PDF places it.
TEXT_PAGE = (60, 20)
TEXT_AT = (4, 5)


def level(value):
    """An 8-bit level as the PDF number of its fraction of 255."""
    return f"{value / 255:.6g}"


def painted(rects, page_height, dx=0, dy=0):
    """A content stream painting `rects`, moved right by `dx` and down by
    `dy`, on a page `page_height` points high."""
    ops = []
    for x, y, w, h, color in rects:
        operator = "rg" if len(color) == 3 else "g"
        ops.append(f"{' '.join(level(v) for v in color)} {operator} {x + dx} {page_height - (y + dy) - h} {w} {h} re f")
    return "\n".join(ops)


def stream(content):
    data = content.encode("ascii")
    return f"<< /Length {len(data)} >>\nstream\n".encode("ascii") + data + b"\nendstream"


def pdf(pages):
    """A PDF file of `pages`, each the entries its page dictionary holds
    beside /Type, /Parent and /Contents, and its content stream."""
    objects = [b"<< /Type /Catalog /Pages 2 0 R >>", b""]
    kids = []
    for entries, content in pages:
        number = len(objects) + 1
        kids.append(f"{number} 0 R")
        objects.append(f"<< /Type /Page /Parent 2 0 R {entries} /Contents {number + 1} 0 R >>".encode("ascii"))
        objects.append(stream(content))
    objects[1] = f"<< /Type /Pages /Kids [{' '.join(kids)}] /Count {len(kids)} >>".encode("ascii")
    out = io.BytesIO()
    out.write(b"%PDF-1.7\n")
    offsets = []
    for number, body in enumerate(objects, 1):
        offsets.append(out.tell())
        out.write(f"{number} 0 obj\n".encode("ascii") + body + b"\nendobj\n")
    xref = out.tell()
    out.write(f"xref\n0 {len(objects) + 1}\n0000000000 65535 f \n".encode("ascii"))
    for offset in offsets:
        out.write(f"{offset:010d} 00000 n \n".encode("ascii"))
    out.write(f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode("ascii"))
    return out.getvalue()


def encrypted(data, user):
    """`data` encrypted with AES-256 under the user password `user` and
    the owner password OWNER."""
    with pikepdf.open(io.BytesIO(data)) as document:
        out = io.BytesIO()
        document.save(out, encryption=pikepdf.Encryption(user=user, owner=OWNER, R=6))
        return out.getvalue()


def expected(scale, background):
    """The page of RECTS drawn at `scale` pixels a point, on white when
    `background`, else on nothing, as top-down RGBA rows."""
    canvas = np.full((HEIGHT * scale, WIDTH * scale, 4), 255 if background else 0, np.uint8)
    for x, y, w, h, color in RECTS:
        rgb = color * 3 if len(color) == 1 else color
        canvas[y * scale:(y + h) * scale, x * scale:(x + w) * scale] = (*rgb, 255)
    return canvas


def drawn_by_pdfium(data, scale, background, password=None):
    """The first page of `data` as PDFium draws it at `scale` pixels a
    point, on white or on nothing, as top-down RGBA rows."""
    document = pdfium.PdfDocument(data, password=password)
    fill = (255, 255, 255, 255) if background else (0, 0, 0, 0)
    bitmap = document[0].render(scale=scale, fill_color=fill, rev_byteorder=True, force_bitmap_format=pdfium_c.FPDFBitmap_BGRA)
    rows = bitmap.to_numpy().copy()
    document.close()
    return rows


def check(name, data, want, password=None):
    """Stops unless PDFium draws `data` as `want(scale, background)` says,
    at 72 and 144 dots per inch, on white and on nothing."""
    for scale in (1, 2):
        for background in (True, False):
            got = drawn_by_pdfium(data, scale, background, password)
            wanted = want(scale, background)
            if got.shape != wanted.shape or not np.array_equal(got, wanted):
                where = "shape" if got.shape != wanted.shape else f"{int(np.count_nonzero(np.any(got != wanted, axis=2)))} pixels"
                raise SystemExit(f"{name} at {72 * scale} dpi {'on white' if background else 'on nothing'}: PDFium differs from the rectangles ({where})")
    print(f"{name}: PDFium draws it as the rectangles say", flush=True)


def rust_bytes(name, data, what):
    rows = [", ".join(str(b) for b in data[i:i + 24]) for i in range(0, len(data), 24)]
    body = ",\n    ".join(rows)
    return f"/// {what}\npub(crate) const {name}: [u8; {len(data)}] = [\n    {body},\n];\n"


def main(out, wiki_page):
    page = f"/MediaBox [0 0 {WIDTH} {HEIGHT}] /Resources << >>"
    rects = pdf([(page, painted(RECTS, HEIGHT))])
    rotated = pdf([(page + " /Rotate 90", painted(RECTS, HEIGHT))])
    sx, sy, sw, sh = SHOWN
    cropped = pdf([(
        f"/MediaBox [0 0 {MEDIA[0]} {MEDIA[1]}] /CropBox [{sx} {MEDIA[1] - sy - sh} {sx + sw} {MEDIA[1] - sy}] /Resources << >>",
        painted(RECTS, MEDIA[1], sx, sy) + "\n" + painted([HIDDEN], MEDIA[1]),
    )])
    protected = encrypted(rects, PASSWORD)
    owner_only = encrypted(rects, "")
    two_pages = pdf([(page, painted(RECTS, HEIGHT)), (page, painted([(0, 0, WIDTH, HEIGHT, (0, 0, 0))], HEIGHT))])
    font = "/Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >>"
    text = pdf([(f"/MediaBox [0 0 {TEXT_PAGE[0]} {TEXT_PAGE[1]}] {font}", f"BT /F1 12 Tf {TEXT_AT[0]} {TEXT_AT[1]} Td (Wgpu) Tj ET")])

    check("RECTS_FILE", rects, expected)
    check("ROTATED_FILE", rotated, lambda scale, background: np.ascontiguousarray(np.rot90(expected(scale, background), k=-1)))
    check("CROPPED_FILE", cropped, expected)
    check("PROTECTED_FILE", protected, expected, PASSWORD)
    check("OWNER_ONLY_FILE", owner_only, expected)
    check("TWO_PAGES_FILE", two_pages, expected)

    rgb = [(x, y, w, h, color * 3 if len(color) == 1 else color) for x, y, w, h, color in RECTS]
    rect_rows = ",\n    ".join(f"({x}, {y}, {w}, {h}, [{r}, {g}, {b}])" for x, y, w, h, (r, g, b) in rgb)
    parts = [
        "//! PDF files and what each draws, for the tests of the module's PDF",
        "//! import: written by tools/pdf_reference.py, which checks that PDFium",
        "//! draws each painted file exactly as its rectangles say.",
        "",
        "/// The painted page's width and height in points.",
        f"pub(crate) const WIDTH: u32 = {WIDTH};",
        f"pub(crate) const HEIGHT: u32 = {HEIGHT};",
        "",
        "/// The rectangles the painted page holds, in order: left and top in",
        "/// points from its top left, width, height, and color.",
        f"pub(crate) const RECTS: [(u32, u32, u32, u32, [u8; 3]); {len(rgb)}] = [\n    {rect_rows},\n];",
        "",
        "/// The user password PROTECTED_FILE opens with.",
        f"pub(crate) const PASSWORD: &str = \"{PASSWORD}\";",
        "",
        rust_bytes("RECTS_FILE", rects, "A page of RECTS."),
        rust_bytes("ROTATED_FILE", rotated, "A page of RECTS turned by /Rotate 90."),
        rust_bytes("CROPPED_FILE", cropped, f"A {MEDIA[0]} by {MEDIA[1]} MediaBox whose CropBox shows RECTS, with a black rectangle outside it."),
        rust_bytes("PROTECTED_FILE", protected, "RECTS_FILE encrypted with AES-256 under the user password PASSWORD."),
        rust_bytes("OWNER_ONLY_FILE", owner_only, "RECTS_FILE encrypted with AES-256 under an owner password alone."),
        rust_bytes("TWO_PAGES_FILE", two_pages, "A page of RECTS, then a black page."),
        rust_bytes("TEXT_FILE", text, f"A {TEXT_PAGE[0]} by {TEXT_PAGE[1]} page with Wgpu written in 12-point Helvetica, from {TEXT_AT[0]} points across and {TEXT_AT[1]} up."),
    ]
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(parts))
    with open(wiki_page, "wb") as f:
        f.write(rects)
    print("PDF REFERENCE DONE", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    main(sys.argv[1], sys.argv[2])
