"""Torture test: inheritance, dunders, decorators with args, generators,
closures, match/case, custom exceptions, nested comprehensions."""

import math

REGISTRY = []
TOTAL_MARKS = 0


class CanvasError(Exception):
    pass


class OutOfBounds(CanvasError):
    def __init__(self, x, y):
        super().__init__(f"plot out of bounds: {(x, y)}!")
        self.point = (x, y)


def layer(label):
    def deco(fn):
        fn._layer = label
        REGISTRY.append(fn)
        return fn

    return deco


class Canvas:
    kind = "base"

    def __init__(self, width=40, height=20, fill=" "):
        self.width = width
        self.height = height
        self.fill = fill
        self.cells = [[fill] * width for _ in range(height)]

    def __len__(self):
        return self.width * self.height

    def __iter__(self):
        for row in self.cells:
            yield "".join(row).rstrip()

    def __add__(self, other):
        merged = Canvas(self.width, self.height, self.fill)
        for y in range(self.height):
            for x in range(self.width):
                a = self.cells[y][x]
                b = other.cells[y][x]
                merged.cells[y][x] = a if a != self.fill else b
        return merged

    def __call__(self):
        return "\n".join(iter(self))

    def plot(self, x, y, ch="#"):
        if not (0 <= x < self.width and 0 <= y < self.height):
            raise OutOfBounds(x, y)
        global TOTAL_MARKS
        TOTAL_MARKS += 1
        self.cells[y][x] = ch

    def stats(self):
        counts = {}
        for row in self.cells:
            for ch in row:
                counts[ch] = counts.get(ch, 0) + 1
        return {k: v for k, v in sorted(counts.items()) if v > 0}


class Glyph(Canvas):
    kind = "glyph"

    def blit(self, other, ox=0, oy=0, *extra, mode="over"):
        for row in extra:
            for x, y, ch in row:
                try:
                    if mode == "over" or self.cells[oy + y][ox + x] == self.fill:
                        self.plot(ox + x, oy + y, ch)
                except OutOfBounds as exc:
                    print(f"skip {exc.point}")
        for y in range(other.height):
            for x in range(other.width):
                ch = other.cells[y][x]
                if ch != other.fill:
                    try:
                        self.plot(ox + x, oy + y, ch)
                    except OutOfBounds:
                        pass

    @property
    def area(self):
        return len(self)

    @staticmethod
    def blank(w, h):
        return Glyph(w, h)

    @classmethod
    def named(cls, name="anon", **kw):
        g = cls(kw.get("w", 10), kw.get("h", 5))
        g.tag = name
        return g


class Frame(Canvas):
    def __init__(self, inner, pad=1):
        super().__init__(inner.width + pad * 2, inner.height + pad * 2)
        self.inner = inner
        self.pad = pad

    def __enter__(self):
        for x in range(self.width):
            self.plot(x, 0, "-")
            self.plot(x, self.height - 1, "-")
        for y in range(self.height):
            self.plot(0, y, "|")
            self.plot(self.width - 1, y, "|")
        return self

    def __exit__(self, etype, evalue, tb):
        for y in range(self.inner.height):
            for x in range(self.inner.width):
                ch = self.inner.cells[y][x]
                if ch != self.inner.fill:
                    self.cells[y + self.pad][x + self.pad] = ch
        return False


def dragon_string(depth):
    seq = "FX"
    for _ in range(depth):
        out = []
        for ch in seq:
            if ch == "X":
                out.append("X+YF+")
            elif ch == "Y":
                out.append("-FX-Y")
            else:
                out.append(ch)
        seq = "".join(out)
    return seq


def make_tally():
    seen = 0

    def tally(point):
        nonlocal seen
        seen += 1
        return seen

    return tally


def walk_dragon(canvas, seq, step_marks):
    x, y = canvas.width // 2, canvas.height // 2
    dirs = [(1, 0), (0, 1), (-1, 0), (0, -1)]
    d = 0
    for cmd in seq:
        match cmd:
            case "F":
                dx, dy = dirs[d]
                x += dx
                y += dy
                try:
                    canvas.plot(x, y, "#")
                    step_marks.append((x, y))
                except OutOfBounds:
                    break
            case "+":
                d = (d + 1) % 4
            case "-":
                d = (d - 1) % 4
            case "X" | "Y":
                pass
            case _:
                raise CanvasError(f"bad dragon command: {cmd!r}")


@layer("dragon")
def paint_dragon(size=8):
    g = Glyph(size * 8, size * 4)
    seq = dragon_string(size)
    marks = []
    walk_dragon(g, seq, marks)
    tally = make_tally()
    kept = [pt for pt in marks if tally(pt) > 0]
    return g, len(kept)


@layer("rings")
def paint_rings(n=9):
    g = Glyph(n * 4 + 1, n * 2 + 1)
    cx, cy = (n * 4) // 2, n
    for r in range(n, 0, -1):
        for deg in range(0, 360, 6):
            rad = math.radians(deg)
            x = int(round(cx + r * 2 * math.cos(rad)))
            y = int(round(cy + r * math.sin(rad)))
            try:
                g.plot(x, y, str(r % 10))
            except OutOfBounds:
                pass
    return g, n


@layer("pascal")
def paint_pascal(rows=16):
    g = Glyph(rows * 2, rows)
    for r in range(rows):
        for c in range(r + 1):
            if math.comb(r, c) % 2 == 1:
                g.plot(c * 2 + (rows - r), r, "#")
    return g, rows


def first_rows(canvas, n):
    for i, line in enumerate(canvas):
        if i >= n:
            break
        yield f"{i:02d}:{line}"


def classify(count):
    match count:
        case 0:
            return "empty"
        case n if n < 100:
            return f"small:{n:03d}"
        case 1000:
            return "exactly one thousand"
        case _:
            return "big"


def demo_dunders():
    a = Glyph.blank(12, 5)
    b = Glyph.named(name="bee", w=12, h=5)
    a.plot(1, 1, "A")
    b.plot(3, 2, "B")
    combo = a + b
    print(combo())
    print(f"areas: {a.area} {len(b)} stats={combo.stats()}")
    tiny = Glyph(6, 3)
    tiny.plot(2, 1, "Q")
    with Frame(tiny) as framed:
        print(framed())
    try:
        tiny.plot(99, 99, "Z")
    except OutOfBounds as exc:
        print(f"caught: {exc} at {exc.point}")
    finally:
        print("bounds check done")


def main():
    ordered = sorted(REGISTRY, key=lambda fn: fn._layer)
    for fn in ordered:
        canvas, count = fn()
        print(f"=== {fn._layer} ({classify(count)}) ===")
        for line in first_rows(canvas, 40):
            print(line)
    demo_dunders()
    print(f"total marks: {TOTAL_MARKS} layers: {len(ordered)}")


if __name__ == "__main__":
    main()
