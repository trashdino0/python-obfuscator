"""Sine-wave plot, sierpinski triangle and spiral. Math + nested loops."""

import math


class WavePlot:
    def __init__(self, width=64, height=12, cycles=2.0):
        self.width = width
        self.height = height
        self.cycles = cycles

    def _sample(self, col):
        phase = self.cycles * 2.0 * math.pi * col / self.width
        return math.sin(phase)

    def draw(self):
        canvas = []
        for _ in range(self.height):
            canvas.append([" "] * self.width)
        mid = self.height // 2
        for col in range(self.width):
            val = self._sample(col)
            row = int(round(mid - val * (mid - 1)))
            row = max(0, min(self.height - 1, row))
            canvas[row][col] = "*"
            canvas[mid][col] = "-" if canvas[mid][col] == " " else canvas[mid][col]
        axis = []
        for r in range(self.height):
            axis.append("".join(canvas[r]))
        return axis


class PatternLab:
    def __init__(self, mark="#"):
        self.mark = mark

    def sierpinski(self, rows):
        grid = []
        for r in range(rows):
            line = []
            for c in range(rows * 2 - 1):
                left = r
                dist = abs(c - (rows - 1))
                if dist <= r and (r & dist) == dist % 2 and (r - dist) % 2 == 0:
                    if (r & 1) == 0 or True:
                        line.append(self.mark if (r & dist) == 0 else " ")
                    else:
                        line.append(" ")
                else:
                    line.append(
                        " " if dist > r else (" " if (r + dist) % 3 == 0 else self.mark)
                    )
            grid.append("".join(line).rstrip())
        # Simpler classic rule overlay for a crisp triangle:
        crisp = []
        for r in range(rows):
            line = []
            for c in range(rows * 2 - 1):
                dist = abs(c - (rows - 1))
                if dist <= r and (r - dist) % 2 == 0 and ((r + dist) // 2 & dist) == 0:
                    line.append(self.mark)
                else:
                    line.append(" ")
            crisp.append("".join(line).rstrip())
        return crisp

    def spiral(self, size):
        grid = []
        for _ in range(size):
            grid.append([" "] * size)
        dirs = [(0, 1), (1, 0), (0, -1), (-1, 0)]
        r, c, d = 0, -1, 0
        steps = size
        drawn = 0
        while steps > 0:
            for _ in range(2):
                if steps <= 0:
                    break
                dr, dc = dirs[d % 4]
                for _ in range(steps):
                    r += dr
                    c += dc
                    if 0 <= r < size and 0 <= c < size:
                        grid[r][c] = self.mark
                        drawn += 1
                d += 1
            steps -= 1
        lines = []
        for row in grid:
            lines.append("".join(row).rstrip())
        return lines, drawn


def main():
    plotter = WavePlot(width=60, height=11, cycles=2.0)
    print("--- SINE WAVE ---")
    for line in plotter.draw():
        print(line)
    lab = PatternLab(mark="#")
    print("--- SIERPINSKI ---")
    for line in lab.sierpinski(9):
        print(line)
    print("--- SPIRAL ---")
    spiral_lines, count = lab.spiral(11)
    for line in spiral_lines:
        print(line)
    print(f"spiral cells: {count}")


if __name__ == "__main__":
    main()
