#!/bin/sh
# El .dmg de macOS: «Ayudita Comandas.app» (Apple Silicon e Intel en un solo binario) y un
# acceso a Aplicaciones para arrastrarla. Deja `dist/ayudita-comandas-<versión>-macos.dmg`.
#
#   sh scripts/package-macos.sh
#   TARGETS=aarch64-apple-darwin sh scripts/package-macos.sh   # sólo Apple Silicon, más rápido
#
# La dirección de Ayudita se fija al compilar con AYUDITA_BASE_URL (ver README).
set -eu

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
NAME="Ayudita Comandas"
APP="dist/$NAME.app"
DMG="dist/ayudita-comandas-$VERSION-macos.dmg"

rm -rf dist
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" dist/dmg

bins=""
for target in ${TARGETS:-aarch64-apple-darwin x86_64-apple-darwin}; do
  cargo build --release --target "$target"
  bins="$bins target/$target/release/ayudita-comandas"
done
# shellcheck disable=SC2086
lipo -create $bins -output "$APP/Contents/MacOS/ayudita-comandas"
cp assets/icon/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$NAME</string>
  <key>CFBundleDisplayName</key><string>$NAME</string>
  <key>CFBundleIdentifier</key><string>app.ayuditas.comandas</string>
  <key>CFBundleExecutable</key><string>ayudita-comandas</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleDevelopmentRegion</key><string>es</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.business</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Firma ad hoc: sin ella, en Apple Silicon el sistema dice que la app «está dañada». Con ella
# sale el aviso normal de una app de fuera del App Store (README, «Instalar»).
codesign --force --deep --sign - "$APP"

cp -R "$APP" dist/dmg/
ln -s /Applications dist/dmg/Applications
hdiutil create -volname "$NAME" -srcfolder dist/dmg -ov -format UDZO "$DMG" >/dev/null
rm -rf dist/dmg

echo "✓ $DMG ($VERSION)"
