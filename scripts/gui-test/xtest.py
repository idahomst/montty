#!/usr/bin/env python3
"""Send keys and pointer events to the nested X server with XTest (ctypes, no extra packages).

Usage: scripts/gui-test/xtest.py COMMAND... (commands run in order)

The display is GUI_TEST_DISPLAY (default :5), never $DISPLAY, so events cannot reach the real desktop.

  focus               give input focus to the newest top-level window (Xephyr has no window manager)
  key COMBO           e.g. key ctrl+shift+t, key enter, key alt+Up (modifiers: ctrl alt shift super)
  type TEXT           type text; "\\n" in TEXT presses Enter
  move X Y            move the pointer to root coordinates
  click X Y           move, then left click
  down / up           press / release the left button
  button N down|up    press / release button N (4/5 are wheel up/down)
  glide X0 Y0 X1 Y1   move in 12 steps (for drags)
  resize W H          resize the newest top-level window
  sleep S             wait S seconds
"""

import ctypes
import os
import sys
import time

x = ctypes.CDLL("libX11.so.6")
xt = ctypes.CDLL("libXtst.so.6")
P, UL, I = ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int
x.XOpenDisplay.restype = P
x.XOpenDisplay.argtypes = [ctypes.c_char_p]
x.XDefaultRootWindow.restype = UL
x.XDefaultRootWindow.argtypes = [P]
x.XStringToKeysym.restype = UL
x.XStringToKeysym.argtypes = [ctypes.c_char_p]
x.XKeysymToKeycode.restype = ctypes.c_ubyte
x.XKeysymToKeycode.argtypes = [P, UL]
x.XkbKeycodeToKeysym.restype = UL
x.XkbKeycodeToKeysym.argtypes = [P, ctypes.c_ubyte, I, I]
x.XFlush.argtypes = [P]
x.XSync.argtypes = [P, I]
x.XSetInputFocus.argtypes = [P, UL, I, UL]
x.XResizeWindow.argtypes = [P, UL, ctypes.c_uint, ctypes.c_uint]
x.XQueryTree.argtypes = [P, UL, ctypes.POINTER(UL), ctypes.POINTER(UL),
                         ctypes.POINTER(ctypes.POINTER(UL)), ctypes.POINTER(ctypes.c_uint)]
xt.XTestFakeKeyEvent.argtypes = [P, ctypes.c_uint, I, UL]
xt.XTestFakeMotionEvent.argtypes = [P, I, I, I, UL]
xt.XTestFakeButtonEvent.argtypes = [P, ctypes.c_uint, I, UL]

DISPLAY = os.environ.get("GUI_TEST_DISPLAY", ":5")
d = x.XOpenDisplay(DISPLAY.encode())
if not d:
    sys.exit("cannot open display " + DISPLAY)
root = x.XDefaultRootWindow(d)

MODS = {"ctrl": "Control_L", "alt": "Alt_L", "shift": "Shift_L", "super": "Super_L"}
ALIASES = {"enter": "Return", "esc": "Escape", "tab": "Tab", "space": "space", "backspace": "BackSpace"}
# Keysym names of printable ASCII characters that are not their own name.
CHARS = {" ": "space", "!": "exclam", '"': "quotedbl", "#": "numbersign", "$": "dollar", "%": "percent",
         "&": "ampersand", "'": "apostrophe", "(": "parenleft", ")": "parenright", "*": "asterisk",
         "+": "plus", ",": "comma", "-": "minus", ".": "period", "/": "slash", ":": "colon",
         ";": "semicolon", "<": "less", "=": "equal", ">": "greater", "?": "question", "@": "at",
         "[": "bracketleft", "\\": "backslash", "]": "bracketright", "^": "asciicircum",
         "_": "underscore", "`": "grave", "{": "braceleft", "|": "bar", "}": "braceright",
         "~": "asciitilde"}


def sync():
    x.XFlush(d)
    x.XSync(d, 0)


def keysym(name):
    ks = x.XStringToKeysym(name.encode())
    if not ks:
        sys.exit("unknown key " + name)
    return ks


def keycode(name):
    return x.XKeysymToKeycode(d, keysym(name))


def newest_window():
    r, p, n = UL(), UL(), ctypes.c_uint()
    ch = ctypes.POINTER(UL)()
    x.XQueryTree(d, root, ctypes.byref(r), ctypes.byref(p), ctypes.byref(ch), ctypes.byref(n))
    if n.value == 0:
        sys.exit("no window on the display")
    return ch[n.value - 1]


def press(codes):
    for c in codes:
        xt.XTestFakeKeyEvent(d, c, 1, 0)
    for c in reversed(codes):
        xt.XTestFakeKeyEvent(d, c, 0, 0)
    sync()
    time.sleep(0.05)


def key(combo):
    parts = combo.split("+")
    name = ALIASES.get(parts[-1].lower(), parts[-1])
    press([keycode(MODS[m.lower()]) for m in parts[:-1]] + [keycode(name)])


def type_text(text):
    for ch in text:
        if ch == "\n":
            key("enter")
            continue
        ks = keysym(CHARS.get(ch, ch))
        code = x.XKeysymToKeycode(d, ks)
        # A keysym that is not on the key's first level needs Shift (capitals, most symbols).
        shifted = x.XkbKeycodeToKeysym(d, code, 0, 0) != ks
        press(([keycode("Shift_L")] if shifted else []) + [code])


def move(px, py):
    xt.XTestFakeMotionEvent(d, -1, int(float(px)), int(float(py)), 0)
    sync()


def button(n, down):
    xt.XTestFakeButtonEvent(d, int(n), 1 if down else 0, 0)
    sync()


def main(args):
    i = 0
    while i < len(args):
        c = args[i]
        if c == "focus":
            x.XSetInputFocus(d, newest_window(), 1, 0)
            sync()
        elif c == "key":
            i += 1
            key(args[i])
        elif c == "type":
            i += 1
            type_text(args[i].replace("\\n", "\n"))
        elif c == "move":
            move(args[i + 1], args[i + 2])
            i += 2
        elif c == "click":
            move(args[i + 1], args[i + 2])
            button(1, True)
            button(1, False)
            i += 2
        elif c == "down":
            button(1, True)
        elif c == "up":
            button(1, False)
        elif c == "button":
            button(args[i + 1], args[i + 2] == "down")
            i += 2
        elif c == "glide":
            a, b, e, f = (float(v) for v in args[i + 1:i + 5])
            for s in range(1, 13):
                move(a + (e - a) * s / 12, b + (f - b) * s / 12)
                time.sleep(0.04)
            i += 4
        elif c == "resize":
            x.XResizeWindow(d, newest_window(), int(args[i + 1]), int(args[i + 2]))
            sync()
            i += 2
        elif c == "sleep":
            i += 1
            time.sleep(float(args[i]))
        else:
            sys.exit(f"unknown command {c!r}; see --help")
        i += 1


if __name__ == "__main__":
    if len(sys.argv) < 2 or sys.argv[1] in ("-h", "--help"):
        print(__doc__)
    else:
        main(sys.argv[1:])
