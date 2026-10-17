#!/usr/bin/env python3
"""Sample module docstring - should be stripped."""

import os
import sys
from math import sqrt as square_root

# A comment that must disappear.

CONSTANT_GREETING = "Hello, World!"
MAX_RETRIES = 3


def make_greeter(prefix):
    """Create a greeter closure."""
    suffix = "!"
    count = 42

    def greet(name, punctuation=suffix, times=2):
        result = []
        for i in range(times):
            message = f"{prefix} {name}{punctuation} x{i}"
            result.append(message)
        return result

    return greet


def factorial(n):
    if n <= 1:
        return 1
    return n * factorial(n - 1)


class Calculator:
    """Simple calculator class."""

    def __init__(self, base=10):
        self.base = base
        self.history = []

    def add(self, a, b):
        value = a + b + self.base
        self.history.append(value)
        return value

    def scaled(self, items, factor=2):
        return [x * factor for x in items if x > 0]


def check_flags(enabled=True, mode=None):
    if enabled is True and mode is None:
        return "default-on"
    if not enabled:
        return "off"
    return "custom"


def main():
    greeter = make_greeter("Hi")
    print(greeter("Alice"))
    print("fact5 =", factorial(5))
    calc = Calculator(base=100)
    print("add =", calc.add(1, 2))
    print("scaled =", calc.scaled([-1, 2, 3]))
    print("flags =", check_flags(), check_flags(False), check_flags(True, "x"))
    print("sqrt16 =", square_root(16))
    print("retries =", MAX_RETRIES, "greet =", CONSTANT_GREETING)
    total = 0
    for k, v in {"a": 1, "b": 2}.items():
        total += v
    print("total =", total, "key =", k)
    try:
        with open("nonexistent_xyz.txt") as fh:
            data = fh.read()
    except FileNotFoundError as exc:
        print("caught:", type(exc).__name__)
    double = lambda x: x * 2
    print("double =", double(21), "cwd-ok =", isinstance(os.getcwd(), str))
    print("argv-len =", len(sys.argv))


if __name__ == "__main__":
    main()
