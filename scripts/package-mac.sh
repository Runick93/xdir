#!/bin/sh
# Empaqueta xdir para macOS: xdir.app (binario universal Apple Silicon + Intel), firmado, y una
# imagen .dmg para instalar arrastrando a Aplicaciones. Deja en dist/: xdir-<versión>-macos.dmg y
# xdir-<versión>-macos.zip (la app sola, para actualizar).
#
# Uso: sh scripts/package-mac.sh [--sin-compilar]
#
# Firma y notarización (opcionales; sin ellas la app queda firmada "ad hoc" y Gatekeeper pide
# abrirla con clic derecho > Abrir la primera vez):
#   XDIR_SIGN_ID        identidad "Developer ID Application: ..." del llavero
#   APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD   cuenta para notarytool
set -e
cd "$(dirname "$0")/.."

version=$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -1)
[ -n "$version" ] || { echo "No se encontró la versión en Cargo.toml"; exit 1; }
export MACOSX_DEPLOYMENT_TARGET=12.0

if [ "$1" != "--sin-compilar" ]; then
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    cargo build --release --locked -p xdir-mac --target aarch64-apple-darwin
    cargo build --release --locked -p xdir-mac --target x86_64-apple-darwin
fi

out=dist
app="$out/xdir.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
# El binario de la crate se llama xdir-mac; dentro de la app queda como xdir.
lipo -create -output "$app/Contents/MacOS/xdir" \
    target/aarch64-apple-darwin/release/xdir-mac target/x86_64-apple-darwin/release/xdir-mac
cp assets/xdir.icns "$app/Contents/Resources/xdir.icns"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>xdir</string>
    <key>CFBundleDisplayName</key><string>xdir</string>
    <key>CFBundleIdentifier</key><string>com.runick.xdir</string>
    <key>CFBundleExecutable</key><string>xdir</string>
    <key>CFBundleIconFile</key><string>xdir</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <!-- En español: así el sistema da en español los nombres de tipos de archivo y carpetas. -->
    <key>CFBundleDevelopmentRegion</key><string>es</string>
    <key>CFBundleLocalizations</key><array><string>es</string></array>
    <key>LSMinimumSystemVersion</key><string>12.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
    <key>NSHumanReadableCopyright</key><string>Runick</string>
    <!-- Textos de los permisos que pide macOS al abrir esas carpetas. -->
    <key>NSDesktopFolderUsageDescription</key><string>xdir muestra y administra los archivos del Escritorio.</string>
    <key>NSDocumentsFolderUsageDescription</key><string>xdir muestra y administra los archivos de Documentos.</string>
    <key>NSDownloadsFolderUsageDescription</key><string>xdir muestra y administra los archivos de Descargas.</string>
    <key>NSRemovableVolumesUsageDescription</key><string>xdir muestra y administra los archivos de discos externos.</string>
    <key>NSNetworkVolumesUsageDescription</key><string>xdir muestra y administra los archivos de discos de red.</string>
    <key>NSAppleEventsUsageDescription</key><string>xdir le pide al Finder la ventana de información de los archivos.</string>
</dict>
</plist>
PLIST
printf 'APPL????' > "$app/Contents/PkgInfo"
mkdir -p "$app/Contents/Resources/es.lproj"

# Firma: Developer ID si está configurada (con "hardened runtime", que pide la notarización);
# si no, ad hoc.
if [ -n "$XDIR_SIGN_ID" ]; then
    codesign --force --options runtime --timestamp --sign "$XDIR_SIGN_ID" "$app"
else
    codesign --force --sign - "$app"
fi

zip="$out/xdir-$version-macos.zip"
rm -f "$zip"
ditto -c -k --keepParent "$app" "$zip"

if [ -n "$XDIR_SIGN_ID" ] && [ -n "$APPLE_ID" ] && [ -n "$APPLE_TEAM_ID" ] && [ -n "$APPLE_APP_PASSWORD" ]; then
    xcrun notarytool submit "$zip" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_APP_PASSWORD" --wait
    xcrun stapler staple "$app"
    rm -f "$zip"
    ditto -c -k --keepParent "$app" "$zip"
fi

# Imagen de disco: la app y un acceso a Aplicaciones para arrastrarla.
stage=$(mktemp -d)
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Aplicaciones"
dmg="$out/xdir-$version-macos.dmg"
rm -f "$dmg"
hdiutil create -volname "xdir $version" -srcfolder "$stage" -ov -format UDZO "$dmg" >/dev/null
rm -rf "$stage"
if [ -n "$XDIR_SIGN_ID" ]; then
    codesign --force --sign "$XDIR_SIGN_ID" "$dmg"
fi

echo "App:  $app"
echo "Zip:  $zip"
echo "Dmg:  $dmg"
