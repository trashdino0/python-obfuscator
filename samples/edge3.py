"""Implicit concatenation, long literals, percent-format precedence."""

who = "world"
print(who)
r = r"C:\path" " tail"
print(r)
triple = "abc"
print(triple)
blurb = "The quick brown fox jumps over the lazy dog. Pack my box with five dozen liquor jugs. Sphinx of black quartz, judge my vow!"
print(len(blurb), blurb[:23], blurb[-12:])
msg = "%s-%s" % ("left", "right")
print(msg)
combo = "prefix:" + blurb
print(len(combo), combo[-6:])
fs = f"{who}! extra"
print(fs)
