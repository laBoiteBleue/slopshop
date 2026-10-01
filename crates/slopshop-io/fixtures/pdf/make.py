"""Writes the PDF test files (see README.md). Run from this folder: python make.py"""


def pdf(name, pages):
    """pages: list of (media box [w, h], extra page entries, content stream)."""
    objects = []  # bodies, object n is objects[n - 1]

    def add(body):
        objects.append(body)
        return len(objects)

    catalog = add(None)
    tree = add(None)
    font = add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    kids = []
    for (w, h), extra, content in pages:
        stream = add(b"<< /Length %d >>\nstream\n" % len(content) + content + b"\nendstream")
        kids.append(add(b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 %d %d] %s"
                        b"/Resources << /Font << /F1 %d 0 R >> >> /Contents %d 0 R >>"
                        % (tree, w, h, extra, font, stream)))
    objects[catalog - 1] = b"<< /Type /Catalog /Pages %d 0 R >>" % tree
    objects[tree - 1] = b"<< /Type /Pages /Kids [%s] /Count %d >>" % (
        b" ".join(b"%d 0 R" % k for k in kids), len(kids))
    out = bytearray(b"%PDF-1.4\n")
    offsets = []
    for n, body in enumerate(objects, 1):
        offsets.append(len(out))
        out += b"%d 0 obj\n" % n + body + b"\nendobj\n"
    xref = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objects) + 1)
    for o in offsets:
        out += b"%010d 00000 n \n" % o
    out += b"trailer\n<< /Size %d /Root %d 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (
        len(objects) + 1, catalog, xref)
    open(name, "wb").write(out)


# Page 1: 72 x 36 pt, its left half filled with pure red, the right half left empty.
# Page 2: 50 x 100 pt shown rotated a quarter turn (100 x 50), filled with pure blue.
# Page 3: 200 x 50 pt, black Helvetica text on nothing.
pdf("pages.pdf", [
    ((72, 36), b"", b"1 0 0 rg 0 0 36 36 re f"),
    ((50, 100), b"/Rotate 90 ", b"0 0 1 rg 0 0 50 100 re f"),
    ((200, 50), b"", b"BT /F1 36 Tf 10 12 Td (Slop) Tj ET"),
])
