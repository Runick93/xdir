"""Genera assets/runick-logo.bgra (y el PNG del primer cuadro) a partir de assets/runick-logo.gif.

Formato: u32 ancho, u32 alto, u32 cuadros, u32 duración en ms por cuadro, y luego cada cuadro en
BGRA premultiplicado. El fondo negro del GIF pasa a transparente (alfa = canal más alto).
Uso: python scripts/make-logo.py
"""
import struct
import numpy as np
from PIL import Image

W = 688
im = Image.open("assets/runick-logo.gif")
frames, durs = [], []
for i in range(im.n_frames):
    im.seek(i)
    f = im.convert("RGB")
    f = f.resize((W, round(f.height * W / f.width)), Image.LANCZOS)
    a = np.asarray(f).astype(np.float32)
    al = a.max(axis=2)
    rgb = np.where(al[..., None] > 0, a / np.maximum(al[..., None], 1) * 255, 0)
    out = np.dstack([rgb, al]).clip(0, 255).astype(np.uint8)
    frames.append(out)
    durs.append(max(im.info.get("duration", 100), 20))
h, w = frames[0].shape[:2]
Image.fromarray(frames[0], "RGBA").save("assets/runick-logo.png", optimize=True)
with open("assets/runick-logo.bgra", "wb") as fh:
    fh.write(struct.pack("<III", w, h, len(frames)))
    fh.write(struct.pack(f"<{len(durs)}I", *durs))
    for o in frames:
        pm = (o[..., :3].astype(np.float32) * o[..., 3:4] / 255).round().astype(np.uint8)
        fh.write(np.dstack([pm[..., 2], pm[..., 1], pm[..., 0], o[..., 3]]).tobytes())
print(w, h, len(frames), sum(durs), "ms")
