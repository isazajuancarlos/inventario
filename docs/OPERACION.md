<!--
SPDX-FileCopyrightText: 2026 Juan Carlos Isaza
SPDX-License-Identifier: LicenseRef-Propietario
-->

# Manual de operación

Cómo se instala, se configura y se usa. Está escrito para quien va a operarlo,
no para quien lo programó.

---

## 1. Instalar

El sistema es **un solo archivo**. No hay servidor de base de datos, ni
intérprete, ni dependencias que instalar.

```bash
# 1. Copiar el binario donde vaya a vivir.
sudo install -d /opt/inventario
sudo cp inventario-api /opt/inventario/

# 2. Generar la clave de firma de sesión. UNA sola vez.
/opt/inventario/inventario-api clave-firma
#    → INVENTARIO_CLAVE_FIRMA=a3f9...   (cópiela al archivo de entorno)

# 3. Archivo de entorno.
sudo tee /etc/inventario.env <<'EOF'
INVENTARIO_BD=sqlite:///opt/inventario/inventario.db
INVENTARIO_DIRECCION=127.0.0.1:8080
INVENTARIO_CLAVE_FIRMA=a3f9...
INVENTARIO_HORAS_SESION=12
EOF

# 4. Crear los conceptos de kardex y el administrador.
INVENTARIO_CLAVE_ADMIN='una clave larga y propia' \
  INVENTARIO_BD=sqlite:///opt/inventario/inventario.db \
  /opt/inventario/inventario-api instalar

# 5. Arrancar.
/opt/inventario/inventario-api
```

Abra `http://127.0.0.1:8080` y entre con `admin` y la clave del paso 4.

**Si el sistema no arranca**, el mensaje dice qué falta. Sin
`INVENTARIO_CLAVE_FIRMA` no arranca a propósito: generar una al vuelo cerraría
todas las sesiones en cada reinicio y nadie sabría por qué.

Para dejarlo como servicio, `despliegue/inventario-demo.service` sirve de
plantilla: cambie el usuario, la carpeta y quite las dos variables
`INVENTARIO_DEMO_*` (esas son lo que convierte una instalación en demostración).

---

## 2. Configurar — el orden importa

Abra **Alistamiento** (el icono de lista, arriba a la derecha). Esa pantalla
dice qué falta y en qué orden, mirando el estado real. No hay que recorrer un
asistente de siete pasos: solo aparece lo que falta.

El orden natural es:

| # | Qué | Dónde |
|---|---|---|
| 1 | **Empresa**: nombre, NIT, dirección | Configuración → Empresa |
| 2 | **Política de costeo**: promedio ponderado o PEPS | Configuración → Empresa |
| 3 | **Resolución de facturación** | Configuración → Resoluciones |
| 4 | **Conceptos de kardex** — sobre todo su efecto tributario | Configuración → Conceptos |
| 5 | **Usuarios** de mostrador y de bodega | Configuración → Usuarios |
| 6 | **Artículos** y **terceros** | Artículos → «Nuevo artículo» / Terceros → «Nuevo tercero», o migrando |
| 7 | **Existencias iniciales** | Inventario → concepto «Saldo inicial» |

### Lo que hay que decidir y no se puede posponer

**La fórmula de costeo.** Promedio ponderado o PEPS (NIC 2 párrafo 25). Se elige
una y se mantiene: cambiarla con movimientos ya registrados hace que el costo de
ventas de antes y el de después no sean comparables. UEPS no está: lo prohíben
las NIIF.

**El efecto tributario de cada concepto de salida.** Es lo que decide si un
movimiento liquida IVA:

| Efecto | Cuándo | Qué hace |
|---|---|---|
| `venta` | Sale por una factura | El IVA lo lleva la factura |
| `retiro_gravado` | Obsequios, consumo interno, retiro de socios, paso a activo fijo | **Causa IVA sobre el valor comercial** (ET art. 421 lit. b) |
| `faltante_art64` | Merma, rotura, evaporación | Consume el tope del 3 % (ET art. 64) |
| `baja_con_acta` | Vencido u obsoleto que se destruye | Exige acta de destrucción (ET art. 64) |
| `no_afecta_impuestos` | Traslados entre bodegas, ajustes de conteo | Nada |

Si migró desde otro sistema, el diagnóstico marca **todos** los conceptos de
salida para revisión, porque el sistema anterior no tenía este dato y adivinarlo
por el nombre sería una regla frágil. Revíselos antes de operar: después habrá
movimientos sin impuesto que hay que corregir uno a uno.

---

## 3. El día a día

### Cargar mercancía

**Inventario** → concepto «Compras a proveedores» → artículo, cantidad, costo
unitario, documento soporte.

El costo unitario que ponga es el que entra al inventario. Si la compra trajo
flete o aranceles, **ese costo también es del inventario** (NIC 2.11): súmelo
antes de digitarlo, o registre el flete como un movimiento aparte sobre los
mismos artículos.

### Facturar

**Facturar** → cliente, resolución, forma de pago, artículos.

- Si es **de contado**, el medio de pago es obligatorio (Res. DIAN 000165/2023).
- Si no hay existencia, la factura **no se emite** y no se quema un número de
  resolución. Es a propósito.
- La factura muestra su **CUFE** si la resolución tiene clave técnica.

### Regalar, consumir o retirar mercancía

**Inventario** → concepto de retiro. La pantalla pide el **valor comercial**, y
es obligatorio: el IVA se liquida sobre él, no sobre el costo. Si el sistema lo
rechaza es porque el concepto está marcado como retiro gravado y falta ese dato.

### Recaudar

**Recaudar** → cliente, medio de pago, valor. El recibo se reparte de la factura
más vieja a la más nueva. **Lo que sobre no se aplica a ninguna factura**: queda
dicho en pantalla y a favor del cliente.

### Corregir una factura ya emitida

Una factura electrónica **no se edita**. Se corrige con:

- **Nota crédito**: rebaja lo que el cliente debe. Si la mercancía volvió, marque
  «reintegra inventario» y el kardex se mueve solo.
- **Nota débito**: aumenta lo que debe.
- **Anular**: solo si la factura no tiene abonos. Devuelve el inventario al costo
  con el que salió y **conserva la factura** en estado anulado.

---

## 4. Los cierres

### Mensual

| Qué | Dónde | Por qué |
|---|---|---|
| **Corte de deterioro** | Informes → Deterioro | NIC 2.28-33: el inventario se mide al menor entre costo y valor neto realizable, y hay que **reevaluar cada periodo** |
| **Tope del artículo 64** | Informes → Art. 64 | Avisa si los faltantes pasaron del 3 % del inventario inicial más las compras |
| **Retiros gravados** | Informes → Retiros | El IVA que causaron los obsequios y consumos del mes |
| **Rotación y ABC** | Informes | Qué se mueve y qué está quieto |

El corte de deterioro necesita que los artículos tengan **precio de venta
estimado**. Los que no lo tengan salen listados aparte: no se miden con cero,
porque eso los deterioraría al 100 % y el balance diría una barbaridad con toda
la apariencia de estar calculada.

### Anual

El Código de Comercio, art. 52, exige un **inventario y balance general** al
menos una vez al año. Corra el corte de deterioro con fecha 31 de diciembre: esa
medición queda guardada y se puede consultar años después sin recalcular.

---

## 5. Respaldos

**Diez años** (Ley 962 de 2005, art. 28), en un medio que garantice
«reproducción exacta».

```bash
INVENTARIO_BD=sqlite:///opt/inventario/inventario.db \
  /opt/inventario/inventario-api respaldar /respaldos/inventario-2026-08-03.db
```

Se puede correr **con el sistema funcionando**: usa `VACUUM INTO`, que toma una
foto consistente sin bloquear a quien está facturando.

Lo que hace el comando, y por qué importa:

1. Copia con `VACUUM INTO`. Un `cp` de un SQLite con WAL abierto produce un
   archivo que **abre sin quejarse** y al que le faltan las últimas
   transacciones.
2. **Abre la copia y la verifica**: integridad física, integridad referencial y
   que todas las tablas respondan. Un respaldo que no se ha leído no es un
   respaldo.
3. Escribe un **manifiesto** al lado con las cuentas y la fecha.
4. **No pisa un respaldo anterior.** Use un nombre con fecha.

Si el comando falla, **no hay respaldo**. No existe el respaldo «casi bien».

Guarde el par (archivo + manifiesto) fuera del servidor.

### Restaurar

Con el servicio **detenido**, copie el archivo sobre `INVENTARIO_BD` y arranque.

---

## 6. Cuando algo no funciona

| Síntoma | Qué pasa |
|---|---|
| «existencia insuficiente: hay 3 y se piden 4» | No hay stock. Cárguelo por Inventario, o encienda «permitir existencia negativa» si el negocio lo admite |
| «la resolución se agotó» | Se acabaron los números. Pida una autorización nueva en el portal de la DIAN |
| «la resolución venció» | Se acabó la vigencia aunque queden números. Se piden por separado |
| «una factura de contado exige el medio de pago» | Falta el medio de pago (Res. 000165/2023, art. 11) |
| «un retiro gravado exige el valor comercial» | El concepto causa IVA y falta la base gravable |
| La factura sale **sin CUFE** | Falta la clave técnica en la resolución |
| La nota sale **sin CUDE** | Falta el PIN del software en Configuración → Empresa |
| «hace falta iniciar sesión» tras un rato | La sesión venció. `INVENTARIO_HORAS_SESION` la controla |

**Todos los mensajes del sistema dicen qué pasó y qué hacer.** Si alguno no lo
dice, es un defecto: repórtelo.

### Los registros

```bash
journalctl -u inventario -n 50 --no-pager
```

---

## 7. Migrar desde el sistema anterior

```bash
INVENTARIO_BD=sqlite:///opt/inventario/inventario.db \
  /opt/inventario/inventario-api migrar /ruta/a/los/DBF
```

Migra **maestros y saldos**, no el histórico de facturas. El informe dice qué se
cargó y **qué se rechazó, con el motivo**. Léalo entero antes de operar: lo
rechazado no está en el sistema nuevo.

Después de migrar, abra **Alistamiento**: dirá qué quedó pendiente de decidir.
