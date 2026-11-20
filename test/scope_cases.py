"""Scope-aware renaming (v2) regression cases.

Locals confined to one module-level function body are pooled onto short
names that are reused across sibling scopes; everything else (module-level
names, names shared between scopes, ``global``/``nonlocal`` names, f-string
mentions outside the owning scope, same text in two functions) must stay
globally unique.
"""

BASE = 100
GLOBAL_STATE = 40


def alpha_scope():
    value = 1
    extra = value + BASE
    return extra


def beta_scope():
    other = 2
    more = other * BASE
    return more


def dup_one():
    counter = 1
    counter += 1
    return counter


def dup_two():
    counter = 10
    counter *= 2
    return counter


def closure_user():
    hidden = 7

    def inner():
        return hidden * 2

    return inner()


def global_user():
    global GLOBAL_STATE
    GLOBAL_STATE = GLOBAL_STATE + 1
    return GLOBAL_STATE


def fstring_user():
    local = 5
    return f"local={local} base={BASE}"


def param_pool(x, y):
    total = x + y
    return total * BASE


def comp_user():
    squares = [n * n for n in range(4)]
    joined = ",".join(str(s) for s in squares)
    return joined


def shadow_probe():
    total = 1
    if total:
        total = total + 2
    return total


def walrus_user():
    if got := BASE // 4:
        return got
    return -1


class Renamed:
    def __init__(self, factor):
        self.factor = factor

    def apply(self, amount):
        scaled = amount * self.factor
        adjusted = scaled + BASE
        return adjusted

    @staticmethod
    def factory(tag):
        label = tag.upper()
        return label


def call_variants():
    obj = Renamed(3)
    print("apply", obj.apply(4))
    print("tag", Renamed.factory("mix"))
    return obj


def main():
    print("alpha_scope", alpha_scope())
    print("beta_scope", beta_scope())
    print("dup", dup_one(), dup_two())
    print("closure_user", closure_user())
    print("global_user", global_user(), global_user())
    print("fstring_user", fstring_user())
    print("param_pool", param_pool(3, 4))
    print("comp_user", comp_user())
    print("shadow_probe", shadow_probe())
    print("walrus_user", walrus_user())
    call_variants()
    print("BASE", BASE)


main()
