#!/bin/sh
# Genera assets/xdir.icns (ícono de la app en macOS) a partir de assets/xdir.svg.
# Uso: sh scripts/make-icns.sh   (necesita rsvg-convert e ImageMagick: brew install librsvg imagemagick)
# El .icns queda versionado; solo hace falta correrlo si cambia el diseño.
set -e
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
set_dir="$tmp/xdir.iconset"
mkdir -p "$set_dir"
# macOS deja un margen alrededor del ícono (la grilla de Apple usa ~80 % del cuadro).
for size in 16 32 64 128 256 512 1024; do
    inner=$((size * 82 / 100))
    rsvg-convert -w "$inner" -h "$inner" assets/xdir.svg -o "$tmp/inner.png"
    magick "$tmp/inner.png" -background none -gravity center -extent "${size}x${size}" "$tmp/$size.png"
done
cp "$tmp/16.png" "$set_dir/icon_16x16.png"
cp "$tmp/32.png" "$set_dir/icon_16x16@2x.png"
cp "$tmp/32.png" "$set_dir/icon_32x32.png"
cp "$tmp/64.png" "$set_dir/icon_32x32@2x.png"
cp "$tmp/128.png" "$set_dir/icon_128x128.png"
cp "$tmp/256.png" "$set_dir/icon_128x128@2x.png"
cp "$tmp/256.png" "$set_dir/icon_256x256.png"
cp "$tmp/512.png" "$set_dir/icon_256x256@2x.png"
cp "$tmp/512.png" "$set_dir/icon_512x512.png"
cp "$tmp/1024.png" "$set_dir/icon_512x512@2x.png"
iconutil -c icns "$set_dir" -o assets/xdir.icns
rm -rf "$tmp"
echo "assets/xdir.icns listo"
