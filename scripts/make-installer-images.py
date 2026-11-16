"""Genera las imágenes del asistente del instalador (installer/*.bmp) con el logo de Runick
(primer cuadro de assets/runick-logo.png) y el ícono de xdir. Una por escala de pantalla.
Uso: python scripts/make-installer-images.py
"""
from PIL import Image

BG = (14, 15, 20)  # la franja oscura del diálogo «Acerca de»
logo = Image.open("assets/runick-logo.png").convert("RGBA")
icon = Image.open("assets/icon-png/xdir-256.png").convert("RGBA")

# Panel lateral (bienvenida y final): 164x314 a 100 %.
for w, h in ((164, 314), (192, 386), (246, 459), (273, 514), (328, 628), (355, 728)):
    im = Image.new("RGB", (w, h), BG)
    lw = round(w * 0.88)
    lg = logo.resize((lw, round(logo.height * lw / logo.width)), Image.LANCZOS)
    im.paste(lg, ((w - lw) // 2, (h - lg.height) // 2), lg)
    im.save(f"installer/wizard-{w}.bmp")

# Ícono de arriba a la derecha: 55x55 a 100 %, sobre blanco.
for s in (55, 64, 73, 82, 92, 110):
    im = Image.new("RGB", (s, s), (255, 255, 255))
    ic = icon.resize((s, s), Image.LANCZOS)
    im.paste(ic, (0, 0), ic)
    im.save(f"installer/wizard-small-{s}.bmp")

# Ícono a la izquierda del título de la bienvenida (128x128, se muestra a 48 dips), sobre blanco.
im = Image.new("RGB", (128, 128), (255, 255, 255))
ic = icon.resize((128, 128), Image.LANCZOS)
im.paste(ic, (0, 0), ic)
im.save("installer/wizard-title-icon.bmp")
