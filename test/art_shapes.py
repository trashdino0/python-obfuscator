"""Geometric ASCII shapes: pyramids, diamonds, boards. Loop-heavy."""


class ShapePrinter:
    def __init__(self, fill="#", empty=" "):
        self.fill = fill
        self.empty = empty
        self.drawn = 0

    def _row(self, left, middle):
        return f"{self.empty * left}{self.fill * middle}"

    def pyramid(self, height):
        lines = []
        for row in range(1, height + 1):
            side = height - row
            blocks = row * 2 - 1
            lines.append(self._row(side, blocks))
        self.drawn += 1
        return lines

    def diamond(self, half):
        lines = []
        for row in range(1, half + 1):
            lines.append(self._row(half - row, row * 2 - 1))
        for row in range(half - 1, 0, -1):
            lines.append(self._row(half - row, row * 2 - 1))
        self.drawn += 1
        return lines

    def checkerboard(self, size):
        lines = []
        for r in range(size):
            cells = []
            for c in range(size):
                if (r + c) % 2 == 0:
                    cells.append(self.fill * 2)
                else:
                    cells.append(self.empty * 2)
            lines.append("".join(cells))
        self.drawn += 1
        return lines

    def hourglass(self, height):
        lines = []
        width = height * 2 - 1
        for row in range(height):
            gap = row
            blocks = width - gap * 2
            lines.append(f"{self.empty * gap}{self.fill * blocks}")
        for row in range(height - 2, -1, -1):
            gap = row
            blocks = width - gap * 2
            lines.append(f"{self.empty * gap}{self.fill * blocks}")
        self.drawn += 1
        return lines

    def number_triangle(self, rows):
        lines = []
        for row in range(1, rows + 1):
            parts = []
            for num in range(1, row + 1):
                parts.append(str(num % 10))
            for num in range(row - 1, 0, -1):
                parts.append(str(num % 10))
            lines.append(" ".join(parts))
        self.drawn += 1
        return lines

    def show(self, title, lines):
        print(f"--- {title} ---")
        for line in lines:
            print(line.rstrip())
        print(f"(shapes drawn so far: {self.drawn})")


def main():
    printer = ShapePrinter(fill="#")
    printer.show("PYRAMID", printer.pyramid(5))
    printer.show("DIAMOND", printer.diamond(4))
    printer.show("CHECKERBOARD", printer.checkerboard(6))
    printer.show("HOURGLASS", printer.hourglass(4))
    printer.show("NUMBER TRIANGLE", printer.number_triangle(6))


if __name__ == "__main__":
    main()
