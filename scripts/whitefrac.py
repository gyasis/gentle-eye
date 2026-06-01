#!/usr/bin/env python3
"""Print the integer % of near-white pixels on an X display (sampled).

Used as a *real-content* readiness gate before capturing a browser/app:
a blank white loading page is ~100% white (255), so the naive "max channel
> 30" check trips instantly on white. White-fraction < ~70% means the page
has actually painted content.

Usage:  whitefrac.py :99   ->  prints 0-100
"""
import sys
from PIL import ImageGrab

disp = sys.argv[1] if len(sys.argv) > 1 else ":0"
im = ImageGrab.grab(xdisplay=disp).convert("RGB")
px = im.load()
w, h = im.size
white = tot = 0
for y in range(0, h, 6):
    for x in range(0, w, 6):
        r, g, b = px[x, y]
        tot += 1
        if r > 240 and g > 240 and b > 240:
            white += 1
print(int(100 * white / tot) if tot else 100)
