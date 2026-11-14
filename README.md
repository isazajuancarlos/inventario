<!--
SPDX-FileCopyrightText: 2026 Juan Carlos Isaza
SPDX-License-Identifier: LicenseRef-Propietario
-->

# inventario

Sistema de **inventario, facturación y cartera**. Un solo ejecutable que
funciona sin internet, con la interfaz dentro y una base por instancia.

Nace de un encargo concreto: reemplazar la aplicación **Visual FoxPro 6** que
vive en `/mnt/data/inventario_2004` —una envasadora de agua de Medellín, 48
tablas DBF, operación de 2004 a 2020—. Ese sistema es la **prueba**, no la
especificación: lo que se construye sirve para cualquier negocio que compre,
almacene, venda y cobre.

**Demostración pública: <https://inventario.xiliux.com>** — usuario `demo`,
clave `inventario2026`. Cualquiera puede escribir en ella y se reinicia
periódicamente.

Cinco documentos sostienen el trabajo, y conviene leerlos en este orden:

| Documento | Qué contiene |
|---|---|
| **[`ANALISIS.md`](ANALISIS.md)** | El análisis **medido** del sistema heredado: 48 tablas DBF, sus defectos con cifra, y la justificación de cada decisión estructural. |
| **[`docs/ESTANDARES.md`](docs/ESTANDARES.md)** | Qué exigen la **NIC 2**, el **Estatuto Tributario** y la **Resolución DIAN 000165 de 2023**, con la fuente citada y lo no verificado marcado como tal. |
| **[`docs/BRECHAS.md`](docs/BRECHAS.md)** | El análisis de brechas contra esos estándares y dónde quedó implementada cada una. |
| **[`docs/OPERACION.md`](docs/OPERACION.md)** | Manual de operación: instalar, configurar, el día a día, cierres, respaldos y qué hacer cuando algo falla. |
| **[`docs/DIAN.md`](docs/DIAN.md)** | La habilitación como facturador electrónico: los pasos, qué trae el sistema y qué le toca al cliente. |

## Lo que hace distinto a este sistema

Cada punto responde a un defecto **medido** en el que reemplaza.

1. **La existencia se deriva del kardex.** No hay ningún campo `inv_actual` que
   se pueda actualizar por otro camino. El sistema viejo lo tenía, y facturó
   dieciséis años con el kardex parado en septiembre de 2004: la existencia que
   mostraba era ficción.
2. **Facturar mueve el inventario en la misma transacción.** No hay un paso
   aparte que olvidar. Si no hay existencia, la transacción entera se cae y no
   queda ni factura ni número de resolución quemado.
3. **`UNIQUE(prefijo, numero)`**, con el número asignado dentro de la
   transacción y contra el rango de la resolución. El corpus tenía 15 números
   duplicados.
4. **El dinero es entero (centavos) y las cantidades enteras (milésimas).**
   Ni un `REAL` en toda la base.
5. **Costo promedio ponderado exacto**, llevado con dos acumuladores
   (existencia y valor) y costeado proporcionalmente. Sin proceso por lotes y
   sin deriva de redondeo.
6. **Anular no borra.** Estado `anulada` más movimiento de reverso al costo
   original.
7. **Lo que no se puede mapear se rechaza con motivo.** El migrador produce un
   informe de rechazos; no rellena nada con valores por defecto.

## Cumplimiento

Implementado y probado. El detalle normativo está en
[`docs/ESTANDARES.md`](docs/ESTANDARES.md); lo que sigue es qué hace el sistema.

**Contable — NIC 2 / Sección 13 (Decreto 2420 de 2015)**

- **Costo de adquisición completo** (NIC 2.11): flete, arancel, seguro y otros
  directos se **reparten** entre las líneas de la compra en proporción al valor,
  y los descuentos comerciales se restan. Sin esto el inventario queda
  subvaluado y el error se arrastra a todos los periodos siguientes.
- **PEPS o promedio ponderado** (NIC 2.25), a elección de la entidad y con capas
  de costo auditables. **UEPS/LIFO se rechaza con un error que cita la norma**:
  una prohibición silenciosa no es una prohibición.
- **Medición al menor entre costo y valor neto realizable**, con reevaluación en
  cada corte y **reversión limitada al costo original** (NIC 2.28–33). Las
  rebajas y las reversiones se revelan **por separado**, no compensadas
  (NIC 2.36). La comparación es **partida por partida**: medir sobre el total
  esconde el deterioro (NIC 2.29).

**Tributario — Estatuto Tributario**

- **El retiro de inventarios causa IVA sobre el valor comercial** (art. 421
  lit. b). Cada concepto de kardex declara su efecto tributario, y un retiro
  gravado **no se puede registrar sin el valor comercial**: liquidarlo sobre el
  costo sería subdeclararlo en silencio. En el corpus heredado esto eran 161
  movimientos sin un peso de impuesto.
- **Tope del 3 % por faltantes** (art. 64) medido contra su base real —inventario
  inicial más compras— con advertencia al pasarse, y **acta de baja** con
  cantidad, descripción, costo fiscal unitario, justificación y firmantes. El
  costo del acta sale del kardex, no de quien la digita.
- **Sistema de inventario permanente** (art. 62) por construcción.

**Facturación electrónica — Resolución DIAN 000165 de 2023**

- **CUFE SHA-384** del anexo técnico 1.9, numeral 11.2, **probado contra el
  ejemplo oficial publicado** y con SHA-384 propio validado contra vectores de
  NIST.
- Hora de generación con desplazamiento horario, clave técnica, vigencia
  desde/hasta separada del rango, medio de pago en las de contado, unidad de
  medida por ítem y **INC discriminado del IVA**.
- Código de producto bajo estándar declarado (UNSPSC / GTIN / interno), con el
  **dígito de control GS1 validado**: un código que no lo pasa se guarda como
  interno, porque declararlo GTIN sería informar un identificador internacional
  inexistente.

**Gestión**

- Rotación y días de inventario, clasificación ABC por valor consumido, y
  **punto de reorden estadístico** (`z × σ × √tiempo de reposición`) en lugar del
  mínimo fijo digitado que en el corpus tenía 1 artículo de 148.

## Alistamiento: qué falta para operar

El sistema trae un **diagnóstico de su propio estado**. Es lo que separa «está
construido» de «se puede entregar»: mira la base real y enumera qué impide
operar, qué impide facturar electrónicamente y qué conviene revisar — cada punto
con **qué hacer**, no solo con qué falta.

Los dos semáforos van por separado a propósito:

- **«Se puede operar»** — facturar, mover inventario, recaudar.
- **«Factura electrónica lista»** — CUFE, CUDE y los datos que exige la DIAN.

Un negocio puede arrancar con el primero en verde mientras termina su
habilitación ante la DIAN, que es **su** trámite y no un defecto del software.

## Demostración pública

```bash
INVENTARIO_DEMO_USUARIO=demo INVENTARIO_DEMO_CLAVE=... \
  ./inventario-api sembrar-demo
```

Esas dos variables son **lo único** que distingue el binario de la demostración
del que va a casa de un cliente: sin ellas `/api/demo` responde 204 y la
pantalla de acceso no muestra ningún cartel. No hay dos compilaciones.

El kit de despliegue está en `despliegue/`:

```bash
./despliegue/desplegar-demo.sh debian@servidor    # binario + servicio + siembra
./despliegue/publicar-demo.sh  debian@servidor    # nginx + certificado + verificación
```

El segundo **verifica contra el dominio real**, no contra `systemctl is-active`:
un servicio puede estar «activo» sirviendo la versión anterior.

## Construir

Un solo comando produce el artefacto que recibe el cliente:

```bash
./construir.sh          # = cargo build --release. SIN Node.
```

**El build no toca Node.** El panel lo teje `maud` en el servidor: el HTML lo
compila `rustc` junto con el resto del binario, así que una plantilla rota es un
error de compilación y no una pantalla en blanco a las tres semanas. htmx 2.0.4
y la micro-librería propia `boi` van **vendorizados y embebidos** con
`include_str!`, servidos desde el propio binario y nunca por CDN: esto corre en
una envasadora que puede quedarse sin internet, y lo que no esté dentro no
existe. Que la versión de htmx sea la declarada lo fija
`htmx_va_vendorizado_en_la_version_declarada`, que la lee del artefacto embebido.

Con Angular se fueron los dos mecanismos que existían **solo** para protegerse
de aquel paso —el orden obligatorio del guion y el `build.rs` que rompía el
*release* si faltaba la interfaz—. No se sustituyeron: dejaron de tener objeto
al desaparecer la causa.

Resultado: `target/release/inventario-api`, **5,3 MB**.

## Instalar y arrancar

```bash
# 1. Clave de firma de sesión. Se genera UNA vez y va al entorno.
./inventario-api clave-firma

export INVENTARIO_CLAVE_FIRMA=...      # obligatoria, mínimo 32 caracteres
export INVENTARIO_BD=sqlite://inventario.db
export INVENTARIO_DIRECCION=127.0.0.1:8080

# 2. Conceptos de kardex y usuario administrador.
INVENTARIO_CLAVE_ADMIN='una clave larga' ./inventario-api instalar

# 3. Servir.
./inventario-api
```

No hay valores por defecto para lo que puede causar daño en silencio: sin
`INVENTARIO_CLAVE_FIRMA` el servidor **no arranca**, y sin
`INVENTARIO_CLAVE_ADMIN` **no se crea** un administrador con clave por defecto.

| Variable | Por defecto | Notas |
|---|---|---|
| `INVENTARIO_CLAVE_FIRMA` | *(ninguno)* | Obligatoria, ≥ 32 caracteres |
| `INVENTARIO_BD` | `sqlite://inventario.db` | Se crea si no existe |
| `INVENTARIO_DIRECCION` | `127.0.0.1:8080` | `:0` = puerto que elija el sistema |
| `INVENTARIO_HORAS_SESION` | `12` | Entero positivo |
| `INVENTARIO_COOKIE_SEGURA` | *(apagado)* | `1` detrás de HTTPS |

## Migrar desde Visual FoxPro

```bash
INVENTARIO_BD=sqlite://inventario.db ./inventario-api migrar /ruta/a/los/DBF
```

Migra **maestros y saldos**, no el histórico. Es una decisión, no un olvido: 15
números de factura del corpus están duplicados y su kardex lleva parado desde
2004, así que cargar el histórico daría un sistema nuevo con los defectos del
viejo dentro y apariencia de estar bien. El histórico se conserva en los DBF,
que son el respaldo.

Sobre el corpus real, la migración carga 148 artículos, 789 clientes, 7
proveedores, 260 precios de convenio y 5 resoluciones, y **rechaza 78 filas con
su motivo**: códigos de barras compartidos, un artículo con existencia negativa,
cinco clientes cuyo «NIT» era `"---"` o `"BEBE"`, una ciudad que era BOLÍVAR y
CALI a la vez, y **cada concepto de salida marcado para revisión** porque el
corpus no dice cuáles causan IVA.

Además informa —una vez, no sesenta— que **60 artículos traen un «código de
barras» que no pasa el dígito de control de GS1**: no son GTIN, así que se
declaran con el estándar 999 (código del vendedor). Informarlos como GTIN sería
declararle a la DIAN un identificador internacional que no existe.

## Estructura

| Crate | Qué contiene |
|---|---|
| `inventario-dominio` | Reglas puras: dinero, kardex, costeo, factura, cartera, numeración, monto en letras, NIT. **Sin base de datos.** |
| `inventario-datos` | SQLx sobre SQLite (WAL, claves foráneas encendidas). Nadie de aquí para arriba escribe SQL. |
| `inventario-api` | axum, sesión, **el panel en `maud`** (`src/vista.rs`), lector de DBF y migrador. |

El panel y la API son **dos consumidores del mismo dominio**, no dos capas:
`/api` (JSON) es la superficie de integración y no cambia; `vista.rs` teje las
nueve pantallas en el servidor. Lo vendorizado —htmx, `boi` y el CSS— vive en
`crates/inventario-api/estaticos/`.

## Pruebas

```bash
cargo test --workspace          # 280 pruebas
```

| Suite | Pruebas | Qué cubre |
|---|---:|---|
| `inventario-dominio` | 109 | Reglas puras: 500 movimientos de kardex, 200 facturas sintéticas, **CUFE y CUDE contra los ejemplos oficiales de la DIAN**, GTIN, VNR, PEPS |
| `inventario-datos` | 99 | Esquema, transacciones, 120 movimientos, 120 recibos, deterioro, art. 64, notas, alistamiento, informes |
| `inventario-api` | 51 | Sesión, claves, códigos HTTP, lector DBF, migración del corpus real, respaldo verificado, demostración |
| `tests/simulacion.rs` | 21 | **Extremo a extremo: arranca el binario y le habla por HTTP**, y tres que lo DIBUJAN en Chromium |

Hay **dos** simulaciones de extremo a extremo. La larga hace **137 operaciones**
contra el proceso real —montar, comprar,
60 facturas, 30 recibos, 6 anulaciones— y comprueba desde fuera que la
existencia servida por la API coincide con la suma del kardex servido por la
API. Incluye concurrencia con techo de tiempo (doce cajeros facturando a la
vez), camino de error inyectado (se corrompe la base bajo el proceso) y
pruebas que **discriminan**: se rompe el costeo a propósito y se comprueba que
cae justo la prueba que lo cubre.

La segunda, `el_sistema_cumple_las_obligaciones_contables_y_tributarias`, hace
**27 operaciones** que ejercen el cumplimiento por HTTP: que un retiro sin valor
comercial se rechace citando el art. 421, que el informe del art. 64 advierta al
pasar el 3 %, que un acta sin responsables no se acepte, que el deterioro se
reconozca y luego se revierta sin superar el costo, que el INC viaje separado
del IVA, y que **el kardex siga cuadrando después de todo eso**.

El panel tiene además su propio recorrido por HTTP: configurar la instalación
desde cero, corregir el efecto tributario de un concepto heredado, levantar el
acta del art. 64 sobre la baja que la pide y correr el corte de deterioro. Y
tres guardias que no miran una pantalla concreta sino una regla de todas:
ninguna ruta de escritura pasa sin sesión ni sin permiso —**la lista se deriva
de `rutas.rs` en tiempo de compilación**, así que una ruta nueva entra al banco
el día que se escribe—, ningún elemento sale con el atributo `class` repetido
—el navegador tira el segundo y la marca de alerta no se ve nunca— y el servidor
no teje ni un manejador de evento en línea, que la CSP no ejecutaría.

Las pruebas que necesitan el corpus real se **omiten** si no está: nadie tiene
que tener los datos de un cliente para poder compilar.

## Alcance no cubierto

Dicho explícitamente para que no pase por hecho:

- **Emisión electrónica ante la DIAN.** El sistema calcula el **CUFE** y guarda
  todos los datos que exige el anexo técnico, pero no genera el UBL 2.1 con
  extensiones DIAN, no lo firma con XAdES-BES y no consume el servicio de
  validación previa. Eso necesita certificado digital y habilitación del
  facturador, y **no se puede ejercer contra el servicio real sin credenciales
  del cliente**: darlo por terminado a ciegas es justo lo que la directiva 28
  prohíbe. El paso que falta es conectar, no rehacer.
- **Documento equivalente POS electrónico.** Mismo motivo.
- **Talonario de contingencia.** Se solicita en MUISCA junto con los rangos de
  producción; el sistema todavía no lo modela.
- **Retención en la fuente e ICA por municipio.** El campo de retención existe y
  el CUFE reserva el sitio del ICA; las tarifas y sus bases son un módulo de
  configuración tributaria aparte.
- **Pedidos y despachos.** Existen en el corpus y el esquema les deja sitio; no
  hay API ni pantalla.
- **Multiempresa.** Una base por instancia, como el resto del estándar.

## Titularidad

Software propietario. © 2026 Juan Carlos Isaza. Ver [`NOTICE`](NOTICE) y
[`LICENSE`](LICENSE). «Xiliux» es una marca, no una persona jurídica.
