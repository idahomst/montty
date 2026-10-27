#!/usr/bin/env python3
"""Key-to-screen latency: press a key with XTest, poll the screen until the echo appears.

  latency.py [--flood] [--samples N] [--style exec|tilix] <terminal-cmd...>
  latency.py --self-test

Run through `scripts/bench.sh latency`, which supplies --style. Python 3 standard
library only (ctypes on libX11.so.6 and libXtst.so.6). Needs DISPLAY.

What it does
  1. Starts the terminal running `cat` (the tty echoes typed characters, so the
     measured path is key -> terminal -> pty -> terminal -> screen). With --flood
     it starts montty only, with a temporary XDG_CONFIG_HOME holding a two-pane
     session: `cat` focused on the left, `yes` flooding on the right.
  2. Finds the terminal's window: the _NET_CLIENT_LIST entry whose _NET_WM_PID is
     the terminal's pid. Fallback for a display without a window manager (no
     _NET_CLIENT_LIST, e.g. a bare Xephyr) or a terminal without _NET_WM_PID:
     the newest top-level window that did not exist before the launch and whose
     WM_CLASS contains the terminal's basename. Waits up to 10 s.
  3. Remembers the focused window, focuses the terminal (_NET_ACTIVE_WINDOW and
     XSetInputFocus), waits 2 s.
  4. Calibrates the cell size by typing `x` twice and diffing screenshots.
  5. Per sample: random 50-150 ms pause, XTestFakeKeyEvent of `x`, poll XGetImage
     of the next cell until it equals the calibrated `x` image; 1 s -> timeout.
     Ctrl+U (erases the echoed line) after every 40 characters, or fewer when the
     pane is narrower (so the echo never wraps). With --flood only the left 40 % of
     the window is compared during calibration, because the right pane scrolls.
  6. Three runs of N samples (default 100), one summary line per run:
     latency <name>: n=<ok>/<N> timeouts=<t> min=<ms> median=<ms> p95=<ms> max=<ms>

LATENCY_DEBUG=1 also prints every sample (ms, - = timeout) to stderr.

It sends only unmodified `x` and Ctrl+U, and focuses only the window it started.
On every exit path (including SIGINT/SIGTERM) it releases keys, kills and reaps the
terminal, restores the previous focus and closes the display.
"""
import argparse
import ctypes
import json
import math
import os
import random
import shutil
import signal
import subprocess
import sys
import tempfile
import time

RUNS = 3
TIMEOUT_S = 1.0
FIND_TIMEOUT_S = 10.0
LINE_CHARS = 40

# ---------------------------------------------------------------- pure helpers


def diff_bbox(a, b, w, h, bpp):
    """Bounding box (x, y, w, h) of the pixels that differ, or None. Packed rows."""
    if a == b:
        return None
    stride = w * bpp
    x0, y0, x1, y1 = w, h, -1, -1
    for y in range(h):
        ra = a[y * stride:(y + 1) * stride]
        rb = b[y * stride:(y + 1) * stride]
        if ra == rb:
            continue
        y0 = min(y0, y)
        y1 = max(y1, y)
        for x in range(w):
            if ra[x * bpp:(x + 1) * bpp] != rb[x * bpp:(x + 1) * bpp]:
                x0 = min(x0, x)
                x1 = max(x1, x)
    if x1 < 0:
        return None
    return (x0, y0, x1 - x0 + 1, y1 - y0 + 1)


def crop(img, w, bpp, box):
    """The pixels inside box of a packed image of width w, as packed rows."""
    x, y, bw, bh = box
    stride = w * bpp
    return b"".join(
        img[(y + r) * stride + x * bpp:(y + r) * stride + (x + bw) * bpp]
        for r in range(bh)
    )


def unpad(raw, bytes_per_line, w, h, bpp):
    """Packed rows of w pixels from rows of bytes_per_line bytes (XImage rows may be padded)."""
    row = w * bpp
    if bytes_per_line == row:
        return raw
    return b"".join(raw[r * bytes_per_line:r * bytes_per_line + row] for r in range(h))


def cell_equals(img, w, bpp, box, expected):
    """True when the pixels of img inside box equal expected (packed box rows)."""
    return crop(img, w, bpp, box) == expected


def summarize(samples):
    """samples: ms values, None for a timeout. Percentiles are nearest-rank."""
    ok = sorted(s for s in samples if s is not None)
    out = {"n": len(ok), "total": len(samples), "timeouts": len(samples) - len(ok)}
    if not ok:
        out.update(min=None, median=None, p95=None, max=None)
        return out

    def rank(p):
        return ok[max(1, math.ceil(p * len(ok))) - 1]

    mid = len(ok) // 2
    out["min"] = ok[0]
    out["median"] = ok[mid] if len(ok) % 2 else (ok[mid - 1] + ok[mid]) / 2
    out["p95"] = rank(0.95)
    out["max"] = ok[-1]
    return out


def format_summary(name, s):
    def f(v):
        return "-" if v is None else "%.1f" % v

    return "latency %s: n=%d/%d timeouts=%d min=%s median=%s p95=%s max=%s" % (
        name, s["n"], s["total"], s["timeouts"],
        f(s["min"]), f(s["median"]), f(s["p95"]), f(s["max"]))


def self_test():
    w, h, bpp = 10, 12, 4
    a = bytes(w * h * bpp)
    assert diff_bbox(a, a, w, h, bpp) is None
    b = bytearray(a)
    o = (7 * w + 5) * bpp
    b[o:o + bpp] = b"\x01\x02\x03\x04"
    assert diff_bbox(a, bytes(b), w, h, bpp) == (5, 7, 1, 1)
    b[((9 * w) + 2) * bpp] = 9
    assert diff_bbox(a, bytes(b), w, h, bpp) == (2, 7, 4, 3)

    # cell_match_ignores_other_pixels
    box = (2, 3, 3, 2)
    img = bytearray(a)
    for y in range(3, 5):
        for x in range(2, 5):
            img[(y * w + x) * bpp:(y * w + x + 1) * bpp] = bytes([x, y, 0, 0])
    expected = crop(bytes(img), w, bpp, box)
    assert cell_equals(bytes(img), w, bpp, box, expected)
    img[0] = 77                      # outside the box: ignored
    img[(11 * w + 9) * bpp] = 5
    assert cell_equals(bytes(img), w, bpp, box, expected)
    img[(3 * w + 4) * bpp] = 200     # inside the box: seen
    assert not cell_equals(bytes(img), w, bpp, box, expected)

    # a box touching the right and bottom edges
    edge = (w - 3, h - 2, 3, 2)
    img = bytearray(a)
    for y in range(h - 2, h):
        for x in range(w - 3, w):
            img[(y * w + x) * bpp:(y * w + x + 1) * bpp] = bytes([x, y, 1, 1])
    want = b"".join(bytes([x, y, 1, 1]) for y in range(h - 2, h) for x in range(w - 3, w))
    assert crop(bytes(img), w, bpp, edge) == want
    assert cell_equals(bytes(img), w, bpp, edge, want)
    img[(h * w - 1) * bpp] = 0       # the last pixel of the image: seen
    assert not cell_equals(bytes(img), w, bpp, edge, want)

    # rows padded to a larger stride (XImage bytes_per_line) are packed before comparing
    pad = 3
    bpl = w * bpp + pad
    packed = bytes((i * 7) % 251 for i in range(w * h * bpp))
    padded = b"".join(packed[r * w * bpp:(r + 1) * w * bpp] + b"\xff" * pad for r in range(h))
    assert unpad(padded, bpl, w, h, bpp) == packed
    assert unpad(packed, w * bpp, w, h, bpp) == packed

    # summary_counts_timeouts
    s = summarize([10.0, 20.0, None, 30.0])
    assert (s["n"], s["timeouts"], s["median"], s["max"], s["min"]) == (3, 1, 20.0, 30.0, 10.0)
    assert s["p95"] == 30.0
    assert summarize([float(i) for i in range(1, 101)])["p95"] == 95.0
    assert summarize([1.0, 2.0, 3.0, 4.0])["median"] == 2.5
    assert summarize([None])["median"] is None
    line = format_summary("t", summarize([10.0, None]))
    assert line == "latency t: n=1/2 timeouts=1 min=10.0 median=10.0 p95=10.0 max=10.0", line
    print("self-test ok")


# ------------------------------------------------------------------- X11 layer


class XImage(ctypes.Structure):
    pass


_DESTROY = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.POINTER(XImage))
_VOIDFN = ctypes.c_void_p
XImage._fields_ = [
    ("width", ctypes.c_int), ("height", ctypes.c_int), ("xoffset", ctypes.c_int),
    ("format", ctypes.c_int), ("data", ctypes.c_void_p), ("byte_order", ctypes.c_int),
    ("bitmap_unit", ctypes.c_int), ("bitmap_bit_order", ctypes.c_int),
    ("bitmap_pad", ctypes.c_int), ("depth", ctypes.c_int),
    ("bytes_per_line", ctypes.c_int), ("bits_per_pixel", ctypes.c_int),
    ("red_mask", ctypes.c_ulong), ("green_mask", ctypes.c_ulong),
    ("blue_mask", ctypes.c_ulong), ("obdata", ctypes.c_void_p),
    # struct funcs f: XDestroyImage is a macro calling f.destroy_image
    ("create_image", _VOIDFN), ("destroy_image", _DESTROY), ("get_pixel", _VOIDFN),
    ("put_pixel", _VOIDFN), ("sub_image", _VOIDFN), ("add_pixel", _VOIDFN),
]


class XWindowAttributes(ctypes.Structure):
    _fields_ = [
        ("x", ctypes.c_int), ("y", ctypes.c_int), ("width", ctypes.c_int),
        ("height", ctypes.c_int), ("border_width", ctypes.c_int),
        ("depth", ctypes.c_int), ("visual", ctypes.c_void_p),
        ("root", ctypes.c_ulong), ("class_", ctypes.c_int),
        ("bit_gravity", ctypes.c_int), ("win_gravity", ctypes.c_int),
        ("backing_store", ctypes.c_int), ("backing_planes", ctypes.c_ulong),
        ("backing_pixel", ctypes.c_ulong), ("save_under", ctypes.c_int),
        ("colormap", ctypes.c_ulong), ("map_installed", ctypes.c_int),
        ("map_state", ctypes.c_int), ("all_event_masks", ctypes.c_long),
        ("your_event_mask", ctypes.c_long), ("do_not_propagate_mask", ctypes.c_long),
        ("override_redirect", ctypes.c_int), ("screen", ctypes.c_void_p),
    ]


class XClientMessage(ctypes.Structure):
    _fields_ = [
        ("type", ctypes.c_int), ("serial", ctypes.c_ulong), ("send_event", ctypes.c_int),
        ("display", ctypes.c_void_p), ("window", ctypes.c_ulong),
        ("message_type", ctypes.c_ulong), ("format", ctypes.c_int),
        ("data", ctypes.c_long * 5),
    ]


class XPropertyEvent(ctypes.Structure):
    _fields_ = [
        ("type", ctypes.c_int), ("serial", ctypes.c_ulong), ("send_event", ctypes.c_int),
        ("display", ctypes.c_void_p), ("window", ctypes.c_ulong), ("atom", ctypes.c_ulong),
        ("time", ctypes.c_ulong), ("state", ctypes.c_int),
    ]


class XEvent(ctypes.Union):
    _fields_ = [("client", XClientMessage), ("property", XPropertyEvent),
                ("pad", ctypes.c_long * 24)]


ERR_HANDLER = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_void_p, ctypes.c_void_p)
IO_ERR_HANDLER = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_void_p)
CLIENT_MESSAGE = 33
PROPERTY_NOTIFY = 28
PROPERTY_CHANGE_MASK = 1 << 22
SUBSTRUCTURE_MASK = (1 << 20) | (1 << 19)
ZPIXMAP = 2
ALL_PLANES = ctypes.c_ulong(~0)
IS_VIEWABLE = 2


class X:
    def __init__(self):
        self.x11 = x = ctypes.CDLL("libX11.so.6")
        self.xt = ctypes.CDLL("libXtst.so.6")
        P, UL, I, U = ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_uint
        x.XOpenDisplay.restype = P
        x.XOpenDisplay.argtypes = [ctypes.c_char_p]
        x.XCloseDisplay.argtypes = [P]
        x.XDefaultRootWindow.restype = UL
        x.XDefaultRootWindow.argtypes = [P]
        x.XInternAtom.restype = UL
        x.XInternAtom.argtypes = [P, ctypes.c_char_p, I]
        x.XStringToKeysym.restype = UL
        x.XStringToKeysym.argtypes = [ctypes.c_char_p]
        x.XKeysymToKeycode.restype = ctypes.c_ubyte
        x.XKeysymToKeycode.argtypes = [P, UL]
        x.XFlush.argtypes = [P]
        x.XSync.argtypes = [P, I]
        x.XSetInputFocus.argtypes = [P, UL, I, UL]
        x.XGetInputFocus.argtypes = [P, ctypes.POINTER(UL), ctypes.POINTER(I)]
        x.XGetImage.restype = ctypes.POINTER(XImage)
        x.XGetImage.argtypes = [P, UL, I, I, U, U, UL, I]
        x.XGetWindowAttributes.argtypes = [P, UL, ctypes.POINTER(XWindowAttributes)]
        x.XGetWindowProperty.argtypes = [
            P, UL, UL, ctypes.c_long, ctypes.c_long, I, UL, ctypes.POINTER(UL),
            ctypes.POINTER(I), ctypes.POINTER(UL), ctypes.POINTER(UL),
            ctypes.POINTER(ctypes.c_void_p)]
        x.XFree.argtypes = [P]
        x.XQueryTree.argtypes = [
            P, UL, ctypes.POINTER(UL), ctypes.POINTER(UL),
            ctypes.POINTER(ctypes.POINTER(UL)), ctypes.POINTER(U)]
        x.XSendEvent.argtypes = [P, UL, I, ctypes.c_long, ctypes.POINTER(XEvent)]
        x.XSetErrorHandler.argtypes = [ERR_HANDLER]
        x.XCreateSimpleWindow.restype = UL
        x.XCreateSimpleWindow.argtypes = [P, UL, I, I, U, U, U, UL, UL]
        x.XDestroyWindow.argtypes = [P, UL]
        x.XSelectInput.argtypes = [P, UL, ctypes.c_long]
        x.XChangeProperty.argtypes = [P, UL, UL, UL, I, I, ctypes.c_char_p, I]
        x.XCheckTypedWindowEvent.argtypes = [P, UL, I, ctypes.POINTER(XEvent)]
        x.XRaiseWindow.argtypes = [P, UL]
        self.xt.XTestFakeKeyEvent.argtypes = [P, U, I, UL]

        self.x_errors = 0

        def on_error(_d, _e):
            self.x_errors += 1
            return 0

        self.on_io_error = None
        self._handler = ERR_HANDLER(on_error)   # keep alive
        x.XSetErrorHandler(self._handler)

        def on_io_error(_d):
            # Xlib calls exit() once this returns: only the essential cleanup is possible.
            if self.on_io_error:
                self.on_io_error()
            return 0

        x.XSetIOErrorHandler.argtypes = [IO_ERR_HANDLER]
        self._io_handler = IO_ERR_HANDLER(on_io_error)
        x.XSetIOErrorHandler(self._io_handler)
        self.d = x.XOpenDisplay(None)
        if not self.d:
            raise SystemExit("latency: cannot open display (DISPLAY=%s)" % os.environ.get("DISPLAY"))
        self.root = x.XDefaultRootWindow(self.d)
        self.pressed = set()
        self.guard = None       # set by run(): raises Fatal when keys must not be sent
        self._time_win = None   # server_time()'s window, created on first use
        self._atoms = {}

    def atom(self, name):
        if name not in self._atoms:
            self._atoms[name] = self.x11.XInternAtom(self.d, name.encode(), 0)
        return self._atoms[name]

    def keycode(self, keysym_name):
        c = self.x11.XKeysymToKeycode(self.d, self.x11.XStringToKeysym(keysym_name.encode()))
        if not c:
            raise SystemExit("latency: no keycode for " + keysym_name)
        return c

    def key(self, code, down):
        if down:
            self.pressed.add(code)      # first: an interrupt must still release it
        self.xt.XTestFakeKeyEvent(self.d, code, 1 if down else 0, 0)
        if not down:
            self.pressed.discard(code)

    def flush(self):
        self.x11.XFlush(self.d)

    def release_all(self):
        for c in list(self.pressed):
            self.key(c, False)
        self.flush()

    def prop(self, win, name):
        """Property as a list of ints (format 32) or bytes (format 8); None if absent."""
        typ, fmt, n, after = ctypes.c_ulong(), ctypes.c_int(), ctypes.c_ulong(), ctypes.c_ulong()
        data = ctypes.c_void_p()
        r = self.x11.XGetWindowProperty(
            self.d, win, self.atom(name), 0, 1 << 16, 0, 0,
            ctypes.byref(typ), ctypes.byref(fmt), ctypes.byref(n), ctypes.byref(after),
            ctypes.byref(data))
        if r != 0 or not data.value:
            return None
        try:
            if fmt.value == 32:
                return list((ctypes.c_ulong * n.value).from_address(data.value))
            if fmt.value == 8:
                return ctypes.string_at(data.value, n.value)
            return None
        finally:
            self.x11.XFree(data)

    def tree(self, win):
        """(parent, children) of win; children bottom-to-top, newest last."""
        root, parent, kids, n = ctypes.c_ulong(), ctypes.c_ulong(), ctypes.POINTER(ctypes.c_ulong)(), ctypes.c_uint()
        if not self.x11.XQueryTree(self.d, win, ctypes.byref(root), ctypes.byref(parent),
                                   ctypes.byref(kids), ctypes.byref(n)):
            return 0, []
        try:
            return parent.value, [kids[i] for i in range(n.value)]
        finally:
            if kids:
                self.x11.XFree(ctypes.cast(kids, ctypes.c_void_p))

    def children(self):
        return self.tree(self.root)[1]

    def focus_in(self, win):
        """True when the input focus is win or one of its descendants."""
        w = self.focus_window()[0]
        for _ in range(16):
            if w == win:
                return True
            if w <= 1 or w == self.root:
                return False
            w = self.tree(w)[0]
        return False

    def check(self):
        """Called before every key press; raises Fatal when typing would go astray."""
        if self.guard:
            self.guard()

    def attrs(self, win):
        a = XWindowAttributes()
        before = self.x_errors
        ok = self.x11.XGetWindowAttributes(self.d, win, ctypes.byref(a))
        self.x11.XSync(self.d, 0)
        return a if ok and self.x_errors == before else None

    def focus_window(self):
        w, rev = ctypes.c_ulong(), ctypes.c_int()
        self.x11.XGetInputFocus(self.d, ctypes.byref(w), ctypes.byref(rev))
        return w.value, rev.value

    def set_focus(self, win, revert=2):
        self.x11.XSetInputFocus(self.d, win, revert, 0)
        self.x11.XSync(self.d, 0)

    def has_wm(self):
        return bool(self.prop(self.root, "_NET_SUPPORTING_WM_CHECK")
                    or self.prop(self.root, "_NET_ACTIVE_WINDOW"))

    def server_time(self):
        """A current server timestamp: the time of a PropertyNotify on a private window."""
        if self._time_win is None:
            self._time_win = self.x11.XCreateSimpleWindow(self.d, self.root, 0, 0, 1, 1, 0, 0, 0)
            self.x11.XSelectInput(self.d, self._time_win, PROPERTY_CHANGE_MASK)
        w = self._time_win
        ev = XEvent()
        while self.x11.XCheckTypedWindowEvent(self.d, w, PROPERTY_NOTIFY, ctypes.byref(ev)):
            pass    # a late notify from an earlier timed-out call would be stale
        self.x11.XChangeProperty(self.d, w, self.atom("_LATENCY_TIME"), 31, 8, 2, None, 0)
        self.x11.XSync(self.d, 0)
        deadline = time.monotonic() + 1.0
        while time.monotonic() < deadline:
            if self.x11.XCheckTypedWindowEvent(self.d, w, PROPERTY_NOTIFY, ctypes.byref(ev)):
                return ev.property.time
            time.sleep(0.005)
        print("latency: no server timestamp; activating with time 0", file=sys.stderr)
        return 0

    def activate(self, win):
        """_NET_ACTIVE_WINDOW as a pager with a real timestamp, then raise (WM present)."""
        ev = XEvent()
        ev.client.type = CLIENT_MESSAGE
        ev.client.window = win
        ev.client.message_type = self.atom("_NET_ACTIVE_WINDOW")
        ev.client.format = 32
        ev.client.data[0] = 2   # source: pager
        ev.client.data[1] = self.server_time()
        ev.client.data[2] = 0
        self.x11.XSendEvent(self.d, self.root, 0, SUBSTRUCTURE_MASK, ctypes.byref(ev))
        self.x11.XSync(self.d, 0)

    def take_focus(self, win, wm):
        """Give win the input focus: through the WM when there is one, else directly."""
        if not wm:
            self.set_focus(win)
            return
        self.activate(win)
        self.x11.XRaiseWindow(self.d, win)
        self.x11.XSync(self.d, 0)
        deadline = time.monotonic() + 2.0
        while time.monotonic() < deadline:
            if self.focus_in(win):
                return
            time.sleep(0.05)
        raise Fatal("the window manager did not give the terminal the input focus within 2 s")

    def get_image(self, win, box):
        """Packed bytes of the box, plus bytes per pixel. None on X error."""
        x, y, w, h = box
        before = self.x_errors
        im = self.x11.XGetImage(self.d, win, x, y, w, h, ALL_PLANES, ZPIXMAP)
        if not im or self.x_errors != before:
            return None, 0
        try:
            i = im.contents
            bpp = i.bits_per_pixel // 8
            raw = ctypes.string_at(i.data, i.bytes_per_line * i.height)
            return unpad(raw, i.bytes_per_line, w, h, bpp), bpp
        finally:
            i.destroy_image(im)

    def close(self):
        if self._time_win is not None:
            self.x11.XDestroyWindow(self.d, self._time_win)
        self.x11.XCloseDisplay(self.d)


# -------------------------------------------------------------------- the tool


class Fatal(Exception):
    pass


def find_window(x, proc, base, before, deadline):
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            raise Fatal("terminal exited early (status %s)" % proc.returncode)
        clients = x.prop(x.root, "_NET_CLIENT_LIST") or []
        cands = clients if clients else x.children()
        for w in cands:
            pid = x.prop(w, "_NET_WM_PID")
            a = x.attrs(w) if pid and pid[0] == proc.pid else None
            if a and a.map_state == IS_VIEWABLE:
                return w
        for w in reversed(x.children()):   # newest first
            if w in before:
                continue
            cls = x.prop(w, "WM_CLASS")
            a = x.attrs(w)
            if cls and base.lower().encode() in cls.lower() and a and a.map_state == IS_VIEWABLE:
                return w
        time.sleep(0.1)
    raise Fatal("no window for the terminal within %.0f s" % FIND_TIMEOUT_S)


def build_argv(style, term, flood):
    if flood:
        return term + ["--session", "flood"]
    if style == "tilix":
        return term + ["--new-process", "-e", "cat"]
    return term + ["-e", "cat"]


def write_flood_session(cfg):
    d = os.path.join(cfg, "montty", "sessions")
    os.makedirs(d)
    session = {
        "version": 1,
        "active_tab": 0,
        "tabs": [{
            "focused": 0,
            "root": {
                "split": "right", "ratio": 0.5,
                "a": {"profile": "", "command": ["cat"]},
                "b": {"profile": "", "command": ["sh", "-c", "exec yes"]},
            },
        }],
    }
    with open(os.path.join(d, "flood.json"), "w") as f:
        json.dump(session, f)


def calibrate(x, win, key, flood, wait=0.3):
    """Returns (box of column 1, expected image of `x`, characters per line).

    The line length is LINE_CHARS, or fewer when the pane is narrower (the echo would wrap).
    Retries over cursor blink.

    With --flood the right half scrolls `yes`, so only the left 40 % is compared."""
    for _ in range(5):
        w, h = size_of(x, win)
        if flood:
            w = w * 2 // 5
        shots = []
        for _ in range(2):
            x.check()
            x.key(key, True)
            x.key(key, False)
            x.flush()
            time.sleep(wait)
            img, bpp = x.get_image(win, (0, 0, w, h))
            if img is None:
                raise Fatal("XGetImage failed on the window")
            shots.append(img)
        bb = diff_bbox(shots[0], shots[1], w, h, bpp)
        if bb and bb[2] % 2 == 0 and bb[2] >= 2:
            cw = bb[2] // 2
            box = (bb[0], bb[1], cw, bb[3])
            cols = w // cw
            if cols < 6:
                raise Fatal("the terminal pane is only %d columns wide" % cols)
            return box, crop(shots[1], w, bpp, box), min(LINE_CHARS, cols - 2)
        # cursor blink or a stray echo: erase what was typed and try again
        ctrl_u(x, win)
    raise Fatal("calibration failed: the two typed cells were not found (cursor blink? wrong window?)")


def size_of(x, win):
    a = x.attrs(win)
    if not a:
        raise Fatal("window vanished")
    return a.width, a.height


def ctrl_u(x, win):
    x.check()
    ctrl, u = x.keycode("Control_L"), x.keycode("u")
    x.key(ctrl, True)
    x.key(u, True)
    x.key(u, False)
    x.key(ctrl, False)
    x.flush()
    time.sleep(0.3)


def verify_cell(x, win, key, box, expected, col):
    """Types one more `x` at col (unmeasured) and requires it to equal the calibrated image."""
    bx, by, cw, ch = box
    cell = (bx + (col - 1) * cw, by, cw, ch)
    x.check()
    x.key(key, True)
    x.key(key, False)
    x.flush()
    deadline = time.monotonic() + 2.0
    while time.monotonic() < deadline:
        img, _ = x.get_image(win, cell)
        if img == expected:
            return
        time.sleep(0.02)
    raise Fatal("calibration check failed: the third x does not match the calibrated cell")


def measure(x, win, key, box, expected, col, n, line_chars):
    bx, by, cw, ch = box
    out = []
    misses = 0
    for _ in range(n):
        time.sleep(random.uniform(0.05, 0.15))
        cell = (bx + (col - 1) * cw, by, cw, ch)
        stale, _ = x.get_image(win, cell)
        if stale == expected:       # an old `x` is still there: a false match
            raise Fatal("the next cell already shows an x (the line was not erased)")
        x.check()
        t0 = time.perf_counter()
        x.key(key, True)
        x.key(key, False)
        x.flush()
        result = None
        while True:
            img, bpp = x.get_image(win, cell)
            t1 = time.perf_counter()
            if img is not None and img == expected:
                result = (t1 - t0) * 1000.0
                break
            if t1 - t0 > TIMEOUT_S:
                break
        out.append(result)
        misses = misses + 1 if result is None else 0
        if misses >= 3:
            raise Fatal("3 consecutive timeouts: the echo is not appearing")
        col += 1
        if col >= line_chars:
            ctrl_u(x, win)
            col = 0
    return out, col


def stop(proc):
    if proc.poll() is None:
        for sig in (signal.SIGTERM, signal.SIGKILL):
            try:
                os.killpg(proc.pid, sig)
            except ProcessLookupError:
                break
            try:
                proc.wait(timeout=3)
                break
            except subprocess.TimeoutExpired:
                continue
    try:
        proc.wait(timeout=3)
    except subprocess.TimeoutExpired:
        pass


def run(args):
    term = args.terminal
    base = os.path.basename(term[0])
    if args.flood and base != "montty":
        raise SystemExit("latency: --flood needs montty")
    x = X()
    proc = None
    tmp = None
    prev = None
    prev_active = None
    win = None
    old_handlers = {}

    def on_signal(signum, _frame):
        raise KeyboardInterrupt

    for s in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP, signal.SIGTSTP):
        old_handlers[s] = signal.signal(s, on_signal)
    try:
        env = dict(os.environ)
        if base == "montty":
            tmp = tempfile.mkdtemp(prefix="montty-latency-")
            env["XDG_CONFIG_HOME"] = tmp
            if args.flood:
                write_flood_session(tmp)
        before = set(x.children())
        prev = x.focus_window()
        pa = x.prop(x.root, "_NET_ACTIVE_WINDOW")
        prev_active = pa[0] if pa else None
        proc = subprocess.Popen(build_argv(args.style, term, args.flood), env=env,
                                stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL, start_new_session=True)
        def killpg_and_clean():
            if proc is not None:   # even when the leader exited, its session may live on
                try:
                    os.killpg(proc.pid, signal.SIGKILL)
                except (ProcessLookupError, PermissionError):
                    pass
            if tmp:
                shutil.rmtree(tmp, ignore_errors=True)

        x.on_io_error = killpg_and_clean
        win = find_window(x, proc, base, before, time.monotonic() + FIND_TIMEOUT_S)

        def guard():
            if proc.poll() is not None:
                raise Fatal("terminal exited (status %s); no more keys sent" % proc.returncode)
            if not x.focus_in(win):
                raise Fatal("the terminal lost the input focus; no more keys sent")

        x.guard = guard
        wm = x.has_wm()
        x.take_focus(win, wm)
        time.sleep(2.0)
        if not wm:
            x.set_focus(win)
        key = x.keycode("x")
        box, expected, line_chars = calibrate(x, win, key, args.flood)
        verify_cell(x, win, key, box, expected, 2)
        col = 3
        for _ in range(RUNS):
            samples, col = measure(x, win, key, box, expected, col, args.samples, line_chars)
            if os.environ.get("LATENCY_DEBUG"):
                print(["-" if v is None else round(v, 1) for v in samples], file=sys.stderr)
            print(format_summary(base, summarize(samples)), flush=True)
        if proc.poll() is not None:
            raise Fatal("terminal exited during the run")
    finally:
        for s, h in old_handlers.items():
            signal.signal(s, signal.SIG_IGN)   # no second interrupt during cleanup
        try:
            x.release_all()
        except Exception:
            pass
        # Restore focus only if it is still ours (or nobody's): when the owner clicked into
        # another window, giving the old one focus back would steal their next keystrokes.
        restore = True
        try:
            if win is not None:
                w = x.focus_window()[0]
                restore = x.focus_in(win) or w <= 1 or w == x.root
        except Exception:
            pass
        if proc is not None:
            stop(proc)
        try:
            if not restore:
                pass
            elif x.has_wm():
                if prev_active:
                    x.activate(prev_active)     # never XSetInputFocus behind the WM's back
            elif prev:
                x.set_focus(prev[0], prev[1] or 2)
            x.flush()
        except Exception:
            pass
        x.close()
        if tmp:
            shutil.rmtree(tmp, ignore_errors=True)
        for s, h in old_handlers.items():
            signal.signal(s, h)


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--self-test", action="store_true")
    p.add_argument("--flood", action="store_true")
    p.add_argument("--samples", type=int, default=100)
    p.add_argument("--style", choices=("exec", "tilix"), default="exec")
    p.add_argument("terminal", nargs=argparse.REMAINDER)
    args = p.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not args.terminal or args.samples < 1:
        p.error("need <terminal-cmd...> and --samples >= 1")
    try:
        run(args)
    except Fatal as e:
        print("latency: " + str(e), file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("latency: interrupted", file=sys.stderr)
        return 130
    return 0


if __name__ == "__main__":
    sys.exit(main())
