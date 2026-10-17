#!/usr/bin/env python3
from __future__ import annotations

"""Module docstring to strip."""
import functools

__all__ = ["public_func", "Helper"]


def deco(fn):
    @functools.wraps(fn)
    def wrapper(*args, **kwargs):
        result = fn(*args, **kwargs)
        return result

    return wrapper


def public_func(items):
    width = 6
    total = 0
    for it in items:
        total += it
    lines = [f"item {it:>{width}}!" for it in items]
    nested = f"outer {f'inner {total}'} done"
    conv = f"{total!r} ({total!s})"
    braced = f"{{literal}} {total}"
    lookup = {"key": total}
    picked = f"{lookup['key']}"
    print(lines, nested, conv, braced, picked)
    return total


class Helper:
    ratio = 1.5
    count = 0

    def __init__(self, name="anon"):
        self.name = name
        Helper.count += 1

    @staticmethod
    def twice(x):
        return x * 2

    @classmethod
    def make(cls, label="dflt"):
        return cls(name=label)

    def show(self):
        match self.name:
            case "anon":
                tag = "anonymous"
            case other:
                tag = f"named:{other}"
        print(f"{tag} ratio={self.ratio}")
        return tag


@deco
def hidden_impl(value):
    Helper.count += 10
    if (n := value + 1) > 100:
        return n
    return n * Helper.twice(3)


def counter():
    state = 0

    def bump(step=1):
        nonlocal state
        state += step
        return state

    bump()
    bump(5)
    return state


def use_globals():
    global GLOB
    GLOB = "set-here"
    raw = r"C:\path\to\file"
    blob = b"\x00\x01bytes"
    uni = "héllo wörld"
    multi = """line1
line2"""
    print(raw, blob, uni, "|", multi.replace("\n", "/"))
    return GLOB


if __name__ == "__main__":
    print(public_func([1, 20, 300]))
    helper = Helper.make(label="bob")
    helper.show()
    print(hidden_impl(41), counter(), use_globals(), Helper.count)
