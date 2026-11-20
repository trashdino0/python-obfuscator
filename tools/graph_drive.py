"""Drive GUI Python apps under stub tkinter/turtle modules for behavior testing.

Usage:
    python tools/graph_drive.py ["test/graphing calculator.py"]

Runs the target script with fake GUI modules, simulates user interaction
(pick equation -> Submit -> Enter, twice with different equations, plus the
Restart/Add/Exit buttons), records every GUI call with its arguments, and
prints a digest. Run it against the original, the obfuscated copy, and a
packed build: identical "RESULT" lines prove identical behavior for the
driven paths (the built-in `verify` command can't run `mainloop()` apps,
they time out).
"""

import hashlib
import os
import random
import sys
import types

TRACE = []


def rec(line):
    TRACE.append(line)


# ---------------------------------------------------------------- turtle ---
class MasterStub:
    pass


class CanvasStub:
    def __init__(self):
        self.master = MasterStub()


class ScreenStub:
    def getcanvas(self):
        rec("Screen.getcanvas()")
        return CanvasStub()

    def __getattr__(self, name):
        def f(*a, **k):
            rec(f"Screen.{name}{list(a)}")

        return f


class TurtleStub:
    def __getattr__(self, name):
        def f(*a, **k):
            rec(f"Turtle.{name}{list(a)}")

        return f


# -------------------------------------------------------------- tkinter ---
BUTTON_COMMANDS = {}
OPTION_VARS = []
ENTRY_VALUES = []
_entry_idx = [0]


class TkStub:
    def __init__(self, *a, **k):
        rec("Tk()")

    def title(self, t):
        rec(f"Tk.title({t!r})")

    def geometry(self, g):
        rec(f"Tk.geometry({g!r})")

    def wm_geometry(self, g):
        rec(f"Tk.wm_geometry({g!r})")

    def withdraw(self):
        rec("Tk.withdraw()")

    def deiconify(self):
        rec("Tk.deiconify()")

    def destroy(self):
        rec("Tk.destroy()")

    def mainloop(self):
        rec("Tk.mainloop()")

    def __getattr__(self, name):
        def f(*a, **k):
            rec(f"Tk.{name}{list(a)}")

        return f


class FrameStub:
    def __init__(self, *a, **k):
        rec("Frame()")

    def grid(self, *a, **k):
        rec(f"Frame.grid{list(a)}")

    def tkraise(self):
        rec("Frame.tkraise()")


class LabelStub:
    def __init__(self, master=None, text=None, **k):
        rec(f"Label({text!r})")

    def pack(self, *a, **k):
        rec(f"Label.pack{list(a)}")


class EntryStub:
    def __init__(self, *a, **k):
        self.idx = _entry_idx[0]
        _entry_idx[0] += 1
        rec(f"Entry(idx={self.idx})")

    def get(self):
        return ENTRY_VALUES[self.idx % len(ENTRY_VALUES)]

    def pack(self, *a, **k):
        rec(f"Entry.pack{list(a)}")


class ButtonStub:
    def __init__(self, master=None, text=None, command=None, **k):
        rec(f"Button({text!r})")
        if command is not None:
            BUTTON_COMMANDS[text] = command

    def pack(self, *a, **k):
        rec(f"Button.pack{list(a)}")

    def place(self, *a, **k):
        rec(f"Button.place({k})")


class StringVarStub:
    def __init__(self, *a, **k):
        self._v = ""

    def set(self, v):
        rec(f"StringVar.set({v!r})")
        self._v = v

    def get(self):
        return self._v


class OptionMenuStub:
    def __init__(self, master, var, *options, **k):
        rec(f"OptionMenu({list(options)!r})")
        OPTION_VARS.append(var)

    def pack(self, *a, **k):
        rec("OptionMenu.pack()")


tk_mod = types.ModuleType("tkinter")
tk_mod.Tk = TkStub
tk_mod.Frame = FrameStub
tk_mod.Label = LabelStub
tk_mod.Entry = EntryStub
tk_mod.Button = ButtonStub
tk_mod.StringVar = StringVarStub
tk_mod.OptionMenu = OptionMenuStub

turtle_mod = types.ModuleType("turtle")
turtle_mod.Screen = ScreenStub
turtle_mod.Turtle = TurtleStub


def main():
    target = (
        sys.argv[1]
        if len(sys.argv) > 1
        else os.path.join("test", "graphing calculator.py")
    )
    random.seed(12345)
    ENTRY_VALUES.extend(["2", "-3", "4", "0", "1", "1", "0", "0"])

    sys.modules["turtle"] = turtle_mod
    sys.modules["tkinter"] = tk_mod

    with open(target, encoding="utf-8") as fh:
        src = fh.read()
    code = compile(src, target, "exec")
    g = {"__name__": "__main__", "__file__": target}
    exec(code, g)

    # Drive: default choice "Quadratics" -> Submit -> Enter (a=2, b=-3, c=4).
    BUTTON_COMMANDS["Submit"]()
    BUTTON_COMMANDS["Enter"]()
    # Drive: choose "Sin" -> Submit -> Enter (a=1, f=1, h=0, v=0).
    OPTION_VARS[0].set("Sin")
    BUTTON_COMMANDS["Submit"]()
    BUTTON_COMMANDS["Enter"]()
    # Remaining buttons.
    BUTTON_COMMANDS["Restart"]()
    BUTTON_COMMANDS["Add"]()
    BUTTON_COMMANDS["Exit"]()

    digest = hashlib.sha1("\n".join(TRACE).encode("utf-8")).hexdigest()
    print(f"RESULT {digest} lines={len(TRACE)}")


main()
