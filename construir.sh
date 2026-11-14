#!/usr/bin/env bash
# Construye el artefacto que recibe el cliente: UN archivo.
#
#   ./construir.sh
#
# Desde el 2026-08-06 esto es `cargo build --release` y nada más. **El paso de
# Angular desapareció**: el panel lo teje `maud` en el servidor (directiva 26),
# así que el HTML lo compila `rustc` junto con el resto del binario y htmx y
# `boi` van embebidos con `include_str!`.
#
# Con ello se van los dos mecanismos que existían para protegerse de ese paso:
# el ORDEN obligatorio (interfaz primero, o el binario salía con la interfaz de
# la vez anterior) y el `build.rs` que rompía el release si la interfaz faltaba.
# No se sustituyen por otros: dejaron de tener objeto cuando desapareció la
# causa, que es la directiva 1 —quitar el primer mecanismo en vez de añadir un
# segundo que lo contenga—.
#
# El guion se conserva porque es el nombre por el que se le llama en la guía de
# despliegue y en el kit del VPS.
set -euo pipefail

RAIZ="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$RAIZ"

echo "==> compilando el binario (sin Node)"
cargo build --release

BIN="target/release/inventario-api"
[ -x "$BIN" ] || { echo "no salió el binario en $BIN" >&2; exit 1; }

echo "==> listo: $BIN  ($(du -h "$BIN" | cut -f1))"
