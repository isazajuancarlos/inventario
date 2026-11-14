#!/usr/bin/env bash
# Despliega la DEMOSTRACIÓN de inventario en el VPS.
#
#   ./despliegue/desplegar-demo.sh debian@192.99.42.108
#
# Deja el servicio corriendo en 127.0.0.1:8020 y comprobado por loopback. NO
# toca nginx ni pide certificado: eso es `publicar-demo.sh`, que se corre
# después y necesita que el DNS ya exista.
#
# Es idempotente: se puede correr en cada actualización.
set -euo pipefail

DESTINO="${1:-}"
[ -n "$DESTINO" ] || { echo "Uso: $0 usuario@servidor" >&2; exit 2; }

RAIZ="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$RAIZ"
BIN="target/release/inventario-api"
PUERTO=8020

echo "==> 0/7  el artefacto tiene que existir Y llevar la interfaz dentro"
[ -f "$BIN" ] || { echo "falta $BIN. Corra ./construir.sh" >&2; exit 3; }
# ESTA COMPROBACIÓN ESTABA MUERTA Y HABRÍA PARADO EL DESPLIEGUE (arreglado el
# 2026-08-12, encontrado ANTES de tocar el servidor). Buscaba `app-raiz`, que es
# la etiqueta raíz que Angular deja en el `index.html` — y Angular salió del árbol
# con la migración a maud+htmx. O sea que el guion abortaba sobre cualquier
# binario nuevo, con el mensaje exactamente al revés de lo que pasaba: la interfaz
# SÍ está, lo que ya no está es el marcador que buscaba. Es el mismo defecto que
# tenía el guion de medico, y la misma directiva: un veredicto anclado a un
# artefacto que cambió (31).
#
# Y el PORQUÉ también cambió. El «binario sin interfaz» era de la era en que el
# frontal se copiaba a `estaticos/` en un paso aparte y podía faltar. Con `maud`
# las plantillas las compila `rustc`: si faltaran, no compilaría. Lo que sigue
# mereciendo un grep es lo que el compilador NO puede cazar — que `$BIN` sea el
# binario recién construido y no otro que quedó ahí.
for marca in "htmx" "boi" "inventario-api"; do
    grep -qa "$marca" "$BIN" || {
        echo "el binario no lleva «$marca» dentro: ¿es el que acaba de construir" >&2
        echo "\`cargo build --release\`? No se subió nada." >&2
        exit 3
    }
done
echo "    $(du -h "$BIN" | cut -f1)  ✔"

echo "==> 1/7  acceso a $DESTINO"
ssh -o ConnectTimeout=10 "$DESTINO" true

echo "==> 2/7  usuario, carpetas y entorno"
# La clave de firma y la de la demo se generan EN EL SERVIDOR y solo si no
# existen ya. Nunca se suben desde local ni se imprimen aquí: una clave que pasa
# por el registro de una sesión de terminal deja de ser una clave.
ssh "$DESTINO" "set -e
    id inventario >/dev/null 2>&1 || sudo -n useradd --system --home /opt/inventario-demo \
        --shell /usr/sbin/nologin inventario
    sudo -n install -d -o inventario -g inventario -m 0750 /opt/inventario-demo
    sudo -n install -d -o root -g inventario -m 0750 /etc/inventario-demo
    if ! sudo -n test -f /etc/inventario-demo/inventario.env; then
        FIRMA=\$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')
        sudo -n tee /etc/inventario-demo/inventario.env >/dev/null <<ENV
INVENTARIO_BD=sqlite:///opt/inventario-demo/inventario.db
INVENTARIO_DIRECCION=127.0.0.1:$PUERTO
INVENTARIO_CLAVE_FIRMA=\$FIRMA
INVENTARIO_HORAS_SESION=8
# Estas dos son lo que convierte esta instalación en una DEMOSTRACIÓN. Sin
# ellas /api/demo responde 204 y no se publica nada.
INVENTARIO_DEMO_USUARIO=demo
INVENTARIO_DEMO_CLAVE=inventario2026
INVENTARIO_CLAVE_ADMIN=inventario2026
ENV
        sudo -n chown root:inventario /etc/inventario-demo/inventario.env
        sudo -n chmod 0640 /etc/inventario-demo/inventario.env
        echo '    entorno creado'
    else
        echo '    el entorno ya existía: NO se toca (tiene la clave de firma en uso)'
    fi"

echo "==> 3/7  respaldo de la versión anterior, si la hay"
ssh "$DESTINO" "if sudo -n test -f /opt/inventario-demo/inventario-api; then
        sudo -n cp -a /opt/inventario-demo/inventario-api \
            /opt/inventario-demo/inventario-api.bak-\$(date +%Y%m%d-%H%M%S)
        echo '    respaldado'
    else echo '    primera instalación'; fi"

echo "==> 4/7  subir el binario y comprobar que llegó igual"
scp -q "$BIN" "$DESTINO:/tmp/inventario-api"
LOCAL="$(md5sum "$BIN" | cut -d' ' -f1)"
REMOTO="$(ssh "$DESTINO" "md5sum /tmp/inventario-api | cut -d' ' -f1")"
[ "$LOCAL" = "$REMOTO" ] || { echo "el binario llegó CORRUPTO ($LOCAL != $REMOTO)" >&2; exit 4; }
ssh "$DESTINO" "sudo -n install -o inventario -g inventario -m 0750 /tmp/inventario-api \
    /opt/inventario-demo/inventario-api && rm -f /tmp/inventario-api"
echo "    md5 $LOCAL  ✔"

echo "==> 5/7  unidad systemd"
scp -q despliegue/inventario-demo.service "$DESTINO:/tmp/inventario-demo.service"
ssh "$DESTINO" "set -e
    sudo -n install -o root -g root -m 0644 /tmp/inventario-demo.service \
        /etc/systemd/system/inventario-demo.service
    rm -f /tmp/inventario-demo.service
    sudo -n systemctl daemon-reload
    sudo -n systemctl enable inventario-demo >/dev/null"

echo "==> 6/7  migrar, sembrar la demostración y arrancar"
ssh "$DESTINO" "set -e
    sudo -n systemctl stop inventario-demo 2>/dev/null || true
    # El sembrado corre como el usuario del servicio para que la base y el WAL
    # queden con su dueño. Hacerlo como root deja archivos que el servicio no
    # puede escribir, y el fallo aparece al primer movimiento.
    sudo -n -u inventario env \$(sudo -n cat /etc/inventario-demo/inventario.env | grep -v '^#' | xargs) \
        /opt/inventario-demo/inventario-api sembrar-demo
    sudo -n systemctl start inventario-demo
    sleep 4
    systemctl is-active inventario-demo"

echo "==> 7/7  responde por loopback (la verdad no es is-active)"
ssh "$DESTINO" "for r in /api/salud /api/demo /; do
        printf '    %-14s %s\n' \"\$r\" \
            \"\$(curl -s -o /dev/null -w '%{http_code}' --max-time 10 http://127.0.0.1:$PUERTO\$r)\"
    done
    echo '    --- últimas líneas del log ---'
    journalctl -u inventario-demo -n 8 --no-pager"

echo
echo "Listo. Ahora: ./despliegue/publicar-demo.sh $DESTINO"
