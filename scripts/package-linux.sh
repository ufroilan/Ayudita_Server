#!/bin/sh
# El paquete de Linux: un .tar.gz (que, a diferencia de un binario suelto, conserva el permiso de
# ejecución) con la app, su icono y un `instalar.sh` que la añade al menú de aplicaciones.
# Deja `dist/ayudita-comandas-<versión>-linux.tar.gz`.
set -eu

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
DIR="dist/ayudita-comandas"
rm -rf dist
mkdir -p "$DIR"

cargo build --release
cp target/release/ayudita-comandas "$DIR/"
cp assets/icon/icon-256.png "$DIR/ayudita-comandas.png"

cat > "$DIR/instalar.sh" <<'SH'
#!/bin/sh
# Copia Ayudita Comandas a ~/.local y la añade al menú de aplicaciones.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications" "$HOME/.local/share/icons/hicolor/256x256/apps"
install -m 755 "$here/ayudita-comandas" "$HOME/.local/bin/ayudita-comandas"
install -m 644 "$here/ayudita-comandas.png" "$HOME/.local/share/icons/hicolor/256x256/apps/ayudita-comandas.png"
cat > "$HOME/.local/share/applications/ayudita-comandas.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Ayudita Comandas
Comment=Imprime las comandas de Ayudita
Exec=$HOME/.local/bin/ayudita-comandas
Icon=ayudita-comandas
Categories=Office;
Terminal=false
DESKTOP
echo "Listo: busca «Ayudita Comandas» en tus aplicaciones."
SH
chmod +x "$DIR/instalar.sh" "$DIR/ayudita-comandas"

tar -C dist -czf "dist/ayudita-comandas-$VERSION-linux.tar.gz" ayudita-comandas
echo "✓ dist/ayudita-comandas-$VERSION-linux.tar.gz"
