# Ayudita Comandas

App de escritorio (macOS, Windows y Linux) que imprime en una impresora térmica las comandas de
**Ayudita** en cuanto se piden: inicia sesión con la cuenta del local, escucha el canal de Supabase
Realtime del negocio y manda cada comanda a la impresora por ESC/POS.

## Para quien la usa

### Instalar

Se descarga desde Ayudita (Ajustes → Notificaciones → **Impresora de comandas**), que da el
paquete de tu sistema:

- **macOS** — `ayudita-comandas-macos.dmg`. Ábrelo y arrastra **Ayudita Comandas** a
  Aplicaciones. La primera vez, macOS avisa de que la app no viene del App Store: clic derecho
  sobre ella → **Abrir** (en macOS 15, Ajustes del Sistema → Privacidad y seguridad → **Abrir
  igualmente**).
- **Windows** — `ayudita-comandas-windows.exe`. Haz doble clic. Si sale «Windows protegió su PC»,
  **Más información → Ejecutar de todas formas**.
- **Linux** — `ayudita-comandas-linux.tar.gz`. Descomprímelo y ejecuta `instalar.sh`: la añade a
  tus aplicaciones.

### Usarla

1. Abre la app.
2. Escribe el correo y la contraseña de Ayudita y pulsa **Entrar**.
3. En **Impresora**, pulsa **Usar esta impresora** (la predeterminada del sistema) o elige otra.
   Prueba con **Página de prueba**.

A partir de ahí funciona sola: arranca con el ordenador, se queda en la bandeja del sistema al
cerrar la ventana, se reconecta si se cae internet y reintenta si la impresora se queda sin papel.

### La ventana

Tiene el aspecto de Ayudita (Public Sans, grises pizarra, el teal de la marca) y sigue el tema claro
u oscuro del sistema.

| Izquierda | Derecha |
| --- | --- |
| **Cuenta de Ayudita**: inicio de sesión, estado de la conexión (Conectado / Reconectando en N s / Sesión caducada), negocio, usuario y hora de la última comanda. | Pestaña **Comanda actual**: la que está saliendo o la última, tal como sale en papel, con su estado (impresa, imprimiendo, o por qué falló y cuándo se reintenta). |
| **Impresora**: estado (Lista, Sin papel, Tapa abierta, Desconectada…), elección de impresora, papel 80/58 mm, página de prueba. | Pestaña **Cola e historial**: lo pendiente y las 10 últimas, cada una con su importe, con Reimprimir (sale como **COPIA**), Reintentar y Descartar; Pausar la impresión. |

Arriba a la derecha está siempre el peor estado de todos (sesión, conexión o impresora) y, a su
lado, el botón **⚙ Ajustes**, con pestañas: **General** (abrir al encender el ordenador) e
**Impresión** (copias de cada comanda, imprimir precios). La pestaña **Desarrollo** solo aparece al
pulsar **Option + D** (Alt + D en Windows y Linux).

Al salir de la app («Salir» en la bandeja) se borran los archivos temporales: las comandas de la
impresora virtual y los `.tmp`. La cola, los ajustes y la sesión se conservan.

### Lo que llegó mientras estaba apagada

Al conectarse, la app pide a Ayudita lo que llegó mientras no escuchaba y **no lo imprime sola**:
enseña una ventana «Mientras estabas desconectado llegaron N comandas» con la vista previa de cada
una, todas marcadas, y los botones **Imprimir las marcadas** y **Descartar**. Lo mismo pasa con una
anulación cuya comanda original no salió en este equipo. Si la anulación llega cuando la comanda
original todavía estaba en la cola sin imprimir (p. ej. sin papel), no se imprime ninguna de las dos.

## Impresoras

| Tipo | Cómo | Estado |
| --- | --- | --- |
| Del sistema (USB o red, instalada con su controlador) | macOS/Linux: CUPS `lp -o raw`. Windows: cola de impresión en modo RAW. | Atributos IPP de CUPS / estado de Windows |
| De red (IP) | TCP al puerto 9100 | Tiempo real con `DLE EOT` (sin papel, tapa abierta, fuera de línea) |
| USB directo | Sin controlador (`nusb`). macOS y Linux; en Windows solo con WinUSB | `DLE EOT` |

Los bytes son ESC/POS y siempre los mismos, sin nada que configurar:

- **PC850**, la tabla con más caracteres del español (á é í ó ú ü ñ Ñ ¿ ¡ º ª × ·). Lo que no está en
  ella se aproxima («–» → «-», «€» → «EUR»).
- **`FS .`** al empezar: saca a las impresoras chinas (Xprinter y similares) del modo de caracteres
  chinos, en el que las tildes salen como ideogramas. Las que no tienen ese modo lo ignoran.
- **Corte** al final: las que tienen cuchilla cortan; las que no, lo ignoran.

Los productos van en letra doble (ancho y alto) para leerse de un vistazo.

## Desarrollo

Requisitos: [Rust](https://rustup.rs) estable (edición 2024).

```bash
cargo run --release
```

```bash
cargo test
```

En Linux hacen falta, para compilar, las cabeceras habituales de una app gráfica:

```bash
sudo apt install build-essential pkg-config libxkbcommon-dev libgl1-mesa-dev
```

y, para usar USB directo sin ser root, permiso sobre el dispositivo (p. ej. añadir el usuario al
grupo `lp`). La bandeja usa StatusNotifier (KDE, XFCE…; en GNOME hace falta la extensión
AppIndicator). Si no hay bandeja, cerrar la ventana la minimiza.

### La dirección de Ayudita

Va fija en la app: `https://www.ayuditas.app`. Se puede cambiar al compilar:

```bash
AYUDITA_BASE_URL=https://otra.direccion cargo build --release
```

y, para desarrollar, en **Ajustes → Desarrollo** (Option/Alt + D): al activar el modo
desarrollo se usa la dirección de desarrollo (por defecto `http://localhost:3000`); al desactivarlo
vuelve a producción. También con `--ayudita=http://localhost:3000`, que
activa el modo desarrollo con esa dirección. Cada dirección guarda su propia sesión.

> Los endpoints `/api/pos/print/connect`, `/api/pos/print/pending` y `/api/auth/refresh` todavía no
> están desplegados en producción: para probar contra Ayudita hace falta un Ayudita local o una
> dirección donde estén. Mientras tanto, usa el Ayudita simulado de abajo.

### Probar sin impresora

```bash
cargo run -- --sin-impresora
```

Sustituye la impresora por una **impresora virtual** que pasa por todo el camino real (cola,
reintentos, ESC/POS) y guarda cada comanda como `.bin` (bytes ESC/POS) y `.txt` (vista en texto) en
la carpeta `salida` de los datos de la app. No toca la impresora configurada y muestra una banda
«MODO DESARROLLO · SIN IMPRESORA».

```bash
cargo run -- --sin-impresora --simular-fallo=sin-papel
```

hace que la impresora virtual falle (`sin-papel`, `tapa-abierta` o `desconectada`) para ver los
reintentos y avisos. Lo mismo, sin opciones de arranque, en **Ajustes → Desarrollo**:
«Impresora virtual», «Simular fallo», «Comandas de ejemplo» y «Simular comandas perdidas».
`--comanda-demo` mete comandas de ejemplo (nueva, cortesía, cuenta y la anulación de la primera).
`cargo run -- --help` lista todo.

### Ayudita simulado

`examples/mock_ayudita.rs` implementa el login con cookies, `connect`, el token de Supabase,
`refresh`, `pending` y un WebSocket Realtime (Phoenix) mínimo:

```bash
cargo run --example mock_ayudita
```

```bash
AYUDITA_DATOS=/tmp/ayudita-dev AYUDITA_SESION_EN_ARCHIVO=1 cargo run -- --ayudita=http://127.0.0.1:3000 --sin-impresora
```

Contraseña `1234` con cualquier correo (`admin@ayudita.test` no tiene negocio → 403). Con `curl`:

```bash
curl -X POST 127.0.0.1:3000/mock/order
```

| Ruta (POST) | Qué hace |
| --- | --- |
| `/mock/order?kind=cortesia&place=Barra%207&type=barra` | Nueva orden |
| `/mock/void/last` | Anula la última |
| `/mock/prepare/last` | La cocina empieza a prepararla (deja de salir en `pending`) |
| `/mock/drop` | Corta los WebSocket (reconexión) |
| `/mock/ws-down/30` | Rechaza el WebSocket 30 s (esperas 1, 2, 5, 10… s) |
| `/mock/expire` | Caduca las sesiones (401 → pantalla de inicio de sesión) |

`MOCK_TOKEN_TTL=75` hace que el token de Supabase caduque pronto para ver su renovación.

Variables de desarrollo de la app: `AYUDITA_DATOS` (carpeta de datos aislada, con su propia sesión
en archivo: no toca el llavero),
`AYUDITA_SESION_EN_ARCHIVO=1` (cookies en archivo en vez del llavero, evita los avisos del llavero
de macOS al recompilar), `AYUDITA_CAPTURA=archivo.png` (+ `AYUDITA_CAPTURA_ESPERA`,
`AYUDITA_CAPTURA_SALIR=1`, `AYUDITA_CAPTURA_VISTA=cola|ajustes`) para capturas automáticas de la
ventana, y `AYUDITA_TEMA=claro|oscuro` para forzar un tema.

### Prueba con Ayudita real

Crea un turno, cobra una orden y anúlala: deben salir una comanda y una anulación. Para la
reconexión: cierra la app, cobra dos órdenes, empieza a preparar una en la pantalla de Cocina y
vuelve a abrir la app: debe preguntar solo por la otra.

## Cómo está hecha

```
src/main.rs          arranque, opciones de línea de órdenes, runtime tokio + ventana egui
src/conn.rs          sesión, canal Realtime, reconexión, token, latido diario, «lo que se perdió»
src/realtime.rs      protocolo Phoenix de Supabase Realtime (join privado, latido, broadcast)
src/api.rs           llamadas HTTP a Ayudita con las cookies de la sesión
src/session.rs       cookies cifradas en el llavero del sistema (nunca la contraseña)
src/model.rs         la comanda (lectura indulgente: ignora campos nuevos)
src/store.rs         cola persistente, historial, deduplicación (order.id, event), last_sent_at
src/worker.rs        worker de impresión con reintentos y sondeo del estado de la impresora
src/ticket.rs        diseño de la comanda → líneas maquetadas (80/58 mm)
src/escpos.rs        líneas → bytes ESC/POS, página de códigos PC850, estado DLE EOT
src/printer/         sistema (CUPS / Windows), red, USB, archivos
src/ui/              la ventana, el tema de Ayudita, los ajustes y la vista previa en papel
src/tray.rs          icono de la bandeja
src/autostart.rs     arrancar con el sistema (solo en compilaciones release)
src/dev.rs           comandas de ejemplo del modo desarrollador
examples/mock_ayudita.rs   Ayudita simulado
```

Los datos viven en la carpeta de datos de la app del sistema (en macOS,
`~/Library/Application Support/com.Ayudita.Ayudita-Comandas`; el botón «Abrir carpeta de datos» del
modo desarrollo la abre en cualquier sistema): `config.json`, `session.json` (negocio y usuario, sin
secretos) y `cola.json`.

La tipografía Public Sans va incrustada (`assets/fonts`), con su licencia SIL OFL 1.1.

El filtro por `place.type` (para separar cocina y barra) está preparado en `config.json`
(`"place_types": ["barra"]`), sin interfaz todavía.

## Publicar una versión

Al empujar una etiqueta `v*`, `.github/workflows/build.yml` construye en macOS, Windows y Linux,
**abre cada paquete y arranca la app** (con una captura de su ventana, que queda como artefacto) y,
sólo si los tres arrancan, publica el release con:

| Sistema | Archivo | Qué es |
| --- | --- | --- |
| macOS | `ayudita-comandas-macos.dmg` | «Ayudita Comandas.app», Apple Silicon e Intel, firmada ad hoc (`scripts/package-macos.sh`) |
| Windows | `ayudita-comandas-windows.exe` | El ejecutable, con su icono (`build.rs`) |
| Linux | `ayudita-comandas-linux.tar.gz` | La app, su icono e `instalar.sh` (`scripts/package-linux.sh`) |

**Los nombres no cambian entre versiones**: la app de Ayudita enlaza a
`releases/latest/download/<archivo>` según el sistema de quien la visita, así que una etiqueta
nueva es lo que se descarga, sin tocar Ayudita.

```bash
# Sube la versión en Cargo.toml, y después:
git tag v0.1.2
git push origin v0.1.2
```

Los iconos (`assets/icon/`) salen del dibujo de `src/icon.rs`. Si cambia, se regeneran con
`cargo run --example write_icons` y `iconutil -c icns assets/icon/AppIcon.iconset -o
assets/icon/AppIcon.icns`, y se suben.
