"""Big-letter ASCII banner renderer with border. Dict + method driven."""


class Banner:
    FONT = {
        "A": [" *** ", "*   *", "*****", "*   *", "*   *"],
        "B": ["**** ", "*   *", "**** ", "*   *", "**** "],
        "C": [" ****", "*    ", "*    ", "*    ", " ****"],
        "D": ["**** ", "*   *", "*   *", "*   *", "**** "],
        "E": ["*****", "*    ", "**** ", "*    ", "*****"],
        "F": ["*****", "*    ", "**** ", "*    ", "*    "],
        "G": [" ****", "*    ", "* ***", "*   *", " ****"],
        "H": ["*   *", "*   *", "*****", "*   *", "*   *"],
        "I": ["*****", "  *  ", "  *  ", "  *  ", "*****"],
        "J": ["  ***", "   * ", "   * ", "*  * ", " **  "],
        "K": ["*   *", "*  * ", "***  ", "*  * ", "*   *"],
        "L": ["*    ", "*    ", "*    ", "*    ", "*****"],
        "M": ["*   *", "*****", "*****", "*   *", "*   *"],
        "N": ["*   *", "**  *", "* * *", "*  **", "*   *"],
        "O": [" *** ", "*   *", "*   *", "*   *", " *** "],
        "P": ["**** ", "*   *", "**** ", "*    ", "*    "],
        "Q": [" *** ", "*   *", "*   *", "*  **", " ****"],
        "R": ["**** ", "*   *", "**** ", "*  * ", "*   *"],
        "S": [" ****", "*    ", " *** ", "    *", "**** "],
        "T": ["*****", "  *  ", "  *  ", "  *  ", "  *  "],
        "U": ["*   *", "*   *", "*   *", "*   *", " *** "],
        "V": ["*   *", "*   *", "*   *", "*   *", " * * "],
        "W": ["*   *", "*   *", "*****", "*****", "*   *"],
        "X": ["*   *", "*   *", "  *  ", "*   *", "*   *"],
        "Y": ["*   *", "*   *", "  *  ", "  *  ", "  *  "],
        "Z": ["*****", "   * ", "  *  ", " *   ", "*****"],
        "0": [" *** ", "*  **", "* * *", "**  *", " *** "],
        "1": ["  *  ", " **  ", "  *  ", "  *  ", "*****"],
        "2": [" *** ", "*   *", "   * ", "  *  ", "*****"],
        "3": ["**** ", "    *", " *** ", "    *", "**** "],
        " ": ["     ", "     ", "     ", "     ", "     "],
        "?": ["***  ", "  *  ", "  *  ", "     ", "  *  "],
        "!": ["  *  ", "  *  ", "  *  ", "     ", "  *  "],
    }

    def __init__(self, glyph=None, pad=1):
        self.glyph = glyph or "#"
        self.pad = pad
        self.rendered = 0

    def _stylize(self, rows):
        return [row.replace("*", self.glyph) for row in rows]

    def render(self, text):
        text = text.upper()
        out_rows = [""] * 5
        for ch in text:
            Drawing = self.FONT.get(ch, self.FONT["?"])
            styled = self._stylize(Drawing)
            for i in range(5):
                out_rows[i] += styled[i] + " " * self.pad
        self.rendered += 1
        return [row.rstrip() for row in out_rows]

    def border(self, rows, corner="+", edge="-", side="|"):
        width = 0
        for row in rows:
            if len(row) > width:
                width = len(row)
        top = corner + edge * (width + 2) + corner
        boxed = [top]
        for row in rows:
            boxed.append(f"{side} {row.ljust(width)} {side}")
        boxed.append(top)
        return boxed

    def show(self, text):
        print(f"### banner #{self.rendered + 1}: {text}")
        for line in self.border(self.render(text)):
            print(line)


def main():
    maker = Banner(glyph="#")
    maker.show("Hello!")
    maker.show("ABC 123")
    thin = Banner(glyph="*", pad=2)
    thin.show("Hi?")


if __name__ == "__main__":
    main()
