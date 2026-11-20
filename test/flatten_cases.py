"""Control-flow flattening / function junk regression cases.

Every function here is intentionally small and flat so the flattener applies;
the few that are not flat exercise the bail-out paths (loops, generators,
nested defs, decorators around defs).
"""

import asyncio

COUNTER = 0


def apply_stats(a, b):
    total = a + b
    doubled = total * 2
    adjusted = doubled - a
    return adjusted


def no_return(a):
    x = a + 1
    y = x * 2
    z = y - 3
    print("no_return", z)


def early_exit(a):
    x = a + 1
    return x
    y = x * 2
    return y


def multi_line_call(a, b):
    x = a + b
    y = max(
        x,
        a * 2,
        b * 3,
    )
    z = y + 1
    return z


def del_case():
    x = 5
    y = x * 2
    del x
    return y


def bump():
    global COUNTER
    COUNTER += 1
    COUNTER *= 2
    return COUNTER


def must_fail(n):
    x = n - n
    y = 10 // x
    return y


def make_counter():
    v = 10

    def inner():
        nonlocal v
        v += 1
        v *= 2
        return v

    return inner


def postfix(fn):
    def wrapper(*args, **kwargs):
        r = fn(*args, **kwargs)
        r = r
        return r

    return wrapper


@postfix
def sixty_four(a):
    b = a * 2
    c = b * 4
    d = c * 8
    return d


async def flat_async(x):
    y = x * 3
    z = y + 1
    return z


def gen(n):
    for i in range(n):
        yield i * i


class Shape:
    def __init__(self, name, sides):
        self.name = name
        self.sides = sides

    def describe(self):
        s = self.sides
        label = f"{self.name} has {s} sides"
        return label

    @classmethod
    def square(cls, name):
        return cls(name=name, sides=4)


def main():
    print("apply_stats", apply_stats(3, 4))
    no_return(7)
    print("early_exit", early_exit(5))
    print("multi_line_call", multi_line_call(2, 5))
    print("del_case", del_case())
    print("bump1", bump())
    print("bump2", bump())
    try:
        must_fail(3)
    except ZeroDivisionError as e:
        print("must_fail", type(e).__name__)
    inner = make_counter()
    print("inner", inner(), inner())
    print("sixty_four", sixty_four(2))
    print("async", asyncio.run(flat_async(4)))
    print("gen", list(gen(5)))
    sq = Shape.square("box")
    print("shape", sq.describe())
    print("shape2", Shape("tri", 3).describe())


main()
