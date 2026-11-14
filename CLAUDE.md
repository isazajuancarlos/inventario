# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

**La configuración general está en `~/.claude/CLAUDE.md`** y se carga en toda
sesión: reglas, directivas numeradas y máquinas montadas. Aquí solo va lo propio
de este repositorio. El inventario de propiedades está en
`/mnt/data/conocimiento/MAPA.md`.

Esta instancia se arranca **desde esta carpeta**: `cd /mnt/data/inventario && claude`.

## Qué es

Sistema de inventario, facturación y cartera. Un binario Rust que teje su propia
pantalla con `maud` + htmx, SQLite, offline, una base por instancia. Producto de venta,
`LicenseRef-Propietario`, repositorio privado (directiva 10).

Reemplaza una aplicación **Visual FoxPro 6** cuyos datos siguen en
`/mnt/data/inventario_2004` (48 tablas DBF, operación 2004-2020, envasadora de
agua de Medellín, NIT 811033579-6). **Ese corpus es de un tercero y no se
distribuye**: las pruebas que lo usan se omiten solas si no está.

## Los tres documentos que mandan

No son documentación de cortesía: son la especificación.

1. **`ANALISIS.md`** — cada decisión del diseño responde a un defecto *medido*
   del sistema viejo, con su cifra. Los guiones que las midieron están en
   `herramientas/`. Si un cambio contradice un hallazgo, se revisa el hallazgo
   primero, corriendo el guion.
2. **`docs/ESTANDARES.md`** — qué exigen la NIC 2, el Estatuto Tributario y la
   Resolución DIAN 000165 de 2023, **con la fuente citada**. Lo que no se pudo
   verificar contra el texto normativo está marcado «sin verificar»: no se
   afirma (directiva 2).
3. **`docs/BRECHAS.md`** — el análisis de brechas y dónde quedó cada una.

**Antes de tocar una regla contable o tributaria, léelas.** Y si hace falta
consultar la norma, se consulta la vigente: la del CUFE cambió de SHA-1 a
SHA-384 y de separador, y la versión derogada sigue siendo el primer resultado
en las búsquedas.

## Las siete reglas que no se tocan

Romper cualquiera de estas devuelve el sistema al estado del que se venía. Cada
una tiene su prueba, y el nombre de la prueba está aquí para poder buscarla.

1. **La existencia se DERIVA del kardex.** No puede existir una columna
   `existencia`/`inv_actual`/`stock` en `articulo`.
   → `el_saldo_sale_del_kardex_y_no_de_ningun_campo` (comprueba el `PRAGMA
   table_info` y falla si aparece la columna).
2. **Facturar escribe cabecera, detalle, consecutivo y kardex en UNA
   transacción.** → `facturar_mueve_el_kardex_en_la_misma_transaccion` y
   `sin_existencia_no_queda_ni_factura_ni_numero_quemado`.
3. **Todo camino de escritura abre con `BEGIN IMMEDIATE`**, nunca con
   `pool.begin()`. Una transacción diferida que lee y luego escribe recibe
   `SQLITE_BUSY` **sin reintento** —el `busy_timeout` no aplica— y el cajero ve
   un 500. Medido: con `BEGIN` a secas, 6 de 12 facturas concurrentes morían.
   → `doce_cajeros_a_la_vez_no_repiten_numero`.
   Los `pool.begin()` que quedan son de solo lectura con `rollback`.
4. **La foto del kardex tiene que cuadrar con el recorrido de su historia.**
   → `el_kardex_y_las_fotos_dicen_lo_mismo`, y su prueba gemela
   `la_auditoria_caza_un_movimiento_escrito_por_fuera`, que escribe un
   movimiento saltándose `registrar` y exige que la auditoría lo cace.
5. **Nada de `INSERT OR IGNORE` en el migrador.** Se traga el choque en
   silencio: la primera versión cargó 106 de 148 artículos y el informe decía
   que todo bien. Se comprueba antes y se reporta.
6. **No pidas otra conexión del pool con una transacción abierta.** Con 8
   conexiones y 12 cajeros simultáneos eso es un interbloqueo por agotamiento, y
   el servidor responde 500 a todos. La configuración se lee ANTES de
   `begin_with`. → `doce_cajeros_a_la_vez_no_repiten_numero`.
7. **`concepto.efecto_tributario` NO tiene valor por defecto**, ni en el esquema
   ni en `crear_concepto`. Es lo que impide que un concepto nuevo de obsequio
   nazca sin IVA por omisión (ET art. 421 lit. b). Si alguien le pone un
   `DEFAULT` «para que sea más cómodo», ha reabierto la brecha que costó 161
   movimientos en el sistema anterior.

## Dinero y cantidades

- Dinero: `Centavos(i64)`. Cantidades: `Cantidad(i64)` en **milésimas** de unidad.
- **Ni un `REAL` en la base.** El sistema viejo usaba `N(11,2)` y por eso 369 de
  sus facturas no cuadraban con su propio detalle.
- El redondeo es `dinero::dividir_redondeando` (al más cercano, medio alejándose
  del cero). De él dependen el IVA, el costeo y el prorrateo: si cambia, cambian
  los tres.
- **`dividir_redondeando`, `multiplicar` y `porcentaje` devuelven `Option`, y no
  es ceremonia.** Terminaban en `as i64`, que **trunca en silencio** y que
  `overflow-checks` NO comprueba en ningún perfil, ni en debug. Por ahí una
  factura se persistía con un total que no era cantidad × precio: medido,
  `1.553.255.926.290.448.384` donde el importe real era
  `20.000.000.000.000.000.000`. Se numeraba contra la resolución de la DIAN y
  salía «bien».
  Ahora el tipo lo dice y **cada llamante decidió** (directiva 20): las cinco
  familias de error del dominio ganaron su `ImporteFueraDeRango` —factura,
  compra, kardex, retiro y VNR— porque un importe que no cabe no tiene respuesta
  razonable que dar y saturar sería inventar un número que alguien cobra. Los
  informes de gestión (rotación, ABC) sí caen a cero, porque son INDICADORES y
  no dinero que se cobre; está comentado en cada sitio.
  Lo fijan `un_importe_que_no_cabe_dice_que_no_en_vez_de_truncar` y
  `una_factura_cuyo_importe_no_cabe_se_rechaza_en_vez_de_emitirse`, las dos con
  su pareja: sin ellas, un `None` prematuro que rechazara importes legítimos
  pasaría igual.
- **Y el `.sum()` crudo de las aplicaciones también** (`datos/recibo.rs`, el
  hermano que vivía a diecinueve líneas). El `Sum` de `Centavos` es un `+` sin
  comprobar, y desborda con los pares (factura, importe) que manda el formulario
  de recaudo — o sea entrada de fuera.
  **CORRECCIÓN del 2026-08-13, y es de las que hay que leer**: escribí aquí que
  «en release da la vuelta y se aplican importes que nadie pidió». **Es falso en
  este árbol**: `Cargo.toml` lleva `[profile.release] overflow-checks = true`
  desde el 2026-08-12, con su porqué al lado —y no lo leí—. El desbordamiento es
  PÁNICO en los dos perfiles. Lo cazó la revisión de seguridad midiendo el
  ARTEFACTO: `strings` sobre el binario de release encuentra la comprobación, y
  un crate de control sin la línea no la tiene.
  El arreglo sigue siendo el correcto —el propio `Cargo.toml` dice que «un
  rechazo limpio es mejor que un pánico», y `overflow-checks` es la red de
  debajo—, pero lo que evita es un **500**, no un dato corrupto. Y el mutante que
  di por control salió rojo **por pánico**, no por vuelta: no falló por la vía
  que el texto temía (directiva 33).
- **Y una guarda más arriba: `calcular` no costea contra un saldo que no
  significa nada** (`ErrorKardex::SaldoNoRepresentable`, desde el 2026-08-13).
  Comprobar el IMPORTE no bastaba, y ésta es la parte que no se ve leyendo: el
  cociente `valor × cantidad / existencia` cabe perfectamente aunque el
  **promedio por unidad** del saldo de partida no quepa. Medido: antes devolvía
  `Ok(Centavos(18446744073709552))` sobre un saldo imposible; ahora rechaza.
  No estorba —200.000 saldos en rango de operación, 0 disparos en falso; 45,6
  ns/op frente a los 103,8 del `aplicar` completo—.

  **La salida que se lleva TODA la existencia NO se bloquea, y es deliberado**:
  con promedio ponderado es justo lo que repara la corrupción, y bloquearla
  sería quitar la salida en vez de protegerla (directiva 21).

  **Pero con `Formula::Peps` no repara, y eso es un límite conocido, no un
  descuido.** PEPS reescribe el saldo DESPUÉS de `aplicar`, así que sobre un
  saldo corrupto aterriza en (0, basura) en vez de (0, 0) — y una entrada
  posterior lo BLANQUEA, dejando la guarda muda para siempre. Lo sujeta
  `peps_no_repara_el_saldo_al_llevarselo_todo_y_queda_medido`, que **no afirma
  que esté bien: afirma cuál es el estado**, con el residuo exacto fijado, y se
  pondrá roja el día que alguien lo cierre. Cerrarlo exigiría que existencia
  cero implique valor cero tras el ajuste, y eso puede rechazar datos heredados
  donde las capas y el saldo discrepan por redondeo: es decisión de alcance y va
  con su medición sobre el corpus real primero.

  **El TEXTO del error también está medido**, y no por gusto: el único `assert`
  que había miraba el discriminante, así que seguía verde con el mensaje
  partido, con diez espacios en medio o con dos palabras pegadas — las tres
  formas en que este repositorio ya cayó. Ahora
  `una_salida_parcial_no_se_cuesta_contra_un_saldo_imposible` lo IMPRIME y lo
  comprueba, con un mutante por cada forma.
- El costo de salida es **proporcional al valor acumulado**, no `cantidad ×
  promedio_redondeado`. Lo segundo deja residuo: existencia cero con valor.

## Órdenes

```bash
./construir.sh                    # = cargo build --release. SIN Node.
cargo test --workspace            # 307 pruebas en 6 suites (2026-08-13)
cargo test -p inventario-dominio  # solo reglas puras, sin base (rápido)
cargo test -p inventario-api --test simulacion -- --nocapture   # extremo a extremo
cargo test -p inventario-dominio cufe                            # CUFE vs. ejemplo DIAN
cargo test -p inventario-api migrar -- --nocapture               # ver el informe de migración
```

Una prueba sola: `cargo test -p inventario-datos facturar_mueve_el_kardex`.

**El VERDE de este repositorio está DECLARADO** desde el 2026-08-13, en
`.github/workflows/ci.yml`, y se corre así:

```bash
python3 /mnt/data/conocimiento/cupo/verde_local.py inventario
```

No lo ejecuta GitHub —Actions está apagado en toda la cuenta desde el protocolo
solo-local—: el archivo existe porque `verde_local.py` saca de él los `run:` en
su orden y los ejecuta, para que «el comando exacto del CI» no sea una frase que
alguien copia a mano. Hasta ese día inventario era el ÚNICO producto Rust sin
él, y su verde era lo que alguien recordara haber corrido.

**Lleva `cargo test` en debug Y en release, y eso no es prudencia genérica**: el
`+` de `Centavos` desborda —y con `overflow-checks = true` eso es PÁNICO en los
dos perfiles, no una vuelta silenciosa; ver la corrección de arriba—, así
que una prueba de desbordamiento corrida solo en debug pasa por el motivo
equivocado y deja vivo el camino que se instala en casa del cliente. Medido con
el `.sum()` del reparto de recibos: el mutante se pone rojo en release y no en
debug.

**NO lleva `cargo fmt --check`**: se probó y sale rojo en 428 archivos, porque
el árbol nunca se formateó con el rustfmt por defecto y no hay `rustfmt.toml`
que fije otro estilo. Un verde que nace rojo se acaba silenciando con un
`|| true`, y entonces el archivo miente sobre lo que mide.

**Ya no hay paso de `npm`, ni `frontend/`, ni `build.rs`.** Desde el 2026-08-06
el panel entero —las 9 vistas— lo teje `maud` en el servidor (directiva 26):
cambiar una pantalla es editar `src/vista.rs` y recompilar.

Con Angular se fueron los DOS mecanismos que existían para protegerse de aquel
paso: el orden obligatorio de `construir.sh` (interfaz primero, o el binario
salía con la de la vez anterior) y el `build.rs` que rompía el release si la
interfaz faltaba. No se sustituyeron por otros: dejaron de tener objeto al
desaparecer la causa (directiva 1).

**Los formularios con líneas variables —facturar y recaudar— se parsean a mano**
(`vista::campos`). No es capricho: el cuerpo trae claves repetidas
(`linea_articulo=1&linea_articulo=2&…`) y `axum::Form` no las junta en un `Vec`
—`serde_urlencoded` se queda con la última y el resto desaparece EN SILENCIO—.
Una fila sin artículo se DESCARTA entera y, si no queda ninguna, se dice: una
factura sin líneas emitida en silencio quemaría un número de la resolución.

**La pantalla se prueba DIBUJÁNDOLA** (`tests/simulacion.rs`). Lo demás de ese
archivo habla HTTP crudo y afirma sobre lo que devuelve axum: eso mide el códec.
Con htmx la pantalla la termina el navegador, y un `<script type="module">`
bloqueado por la CSP o con el MIME equivocado devuelve el mismo 200 con el mismo
cuerpo. Las tres pruebas `…se_dibuja…` levantan el binario y lo pintan en
Chromium headless. La afirmación que no se puede hacer sobre el HTML de axum es
`data-boi="listo"` en el `<html>`: solo aparece si el navegador ejecutó el
módulo. Validadas contra sus mutantes; si no hay Chromium, fallan ruidosamente.

**El `|` a `tail`/`head` enmascara el código de salida** y hay un hook que lo
rechaza. Use `set -o pipefail` o redirija a un archivo y mire `$?`.

## Estructura

```
crates/inventario-dominio   reglas puras, sin base de datos ni HTTP
crates/inventario-datos     SQLx/SQLite; nadie de aquí para arriba escribe SQL
crates/inventario-api       axum, sesión, interfaz empotrada, dbf.rs, migrar.rs
crates/inventario-api/src/vista.rs   el panel en maud
crates/inventario-api/estaticos/     htmx 2.0.4, boi y panel.css, vendorizados y embebidos
```

El esquema entero está en una sola migración,
`crates/inventario-datos/migrations/0001_fundacion.sql`, con el porqué de cada
`CHECK` comentado. `movimiento` va **después** de `factura` en el archivo: la
referencia adelantada la resolvería SQLite tarde y el error no aparecería hasta
el primer `INSERT`.

## Cumplimiento: dónde vive cada norma

| Norma | Dónde |
|---|---|
| NIC 2.11 costo de adquisición | `dominio/costeo.rs::costo_adquisicion` |
| NIC 2.25 PEPS/promedio, UEPS prohibido | `dominio/costeo.rs::Formula`, tabla `capa_costo` |
| NIC 2.28–33 deterioro y reversión | `dominio/vnr.rs`, `datos/deterioro.rs` |
| ET art. 421 lit. b) IVA por retiro | `concepto.efecto_tributario`, `datos/kardex.rs` |
| ET art. 64 tope 3 % y acta | `dominio/retiro.rs`, `datos/tributario.rs` |
| Res. DIAN 000165/2023 CUFE | `dominio/cufe.rs` |
| GS1 dígito de control | `dominio/gtin.rs` |
| Res. DIAN 000165/2023 CUDE (notas) | `dominio/cufe.rs::calcular_cude` |
| Ley 962/2005 art. 28 conservación | `api/respaldo.rs` |

El CUFE y el CUDE tienen **prueba de aceptación contra los ejemplos oficiales
publicados** en el anexo técnico 1.9 (`el_ejemplo_oficial_del_anexo_1_9`,
`el_ejemplo_oficial_de_cude_para_nota_credito`,
`el_ejemplo_oficial_de_cude_para_documento_de_transmision`). Si alguna falla tras
un cambio, el cambio está mal — no la prueba.

**Ojo con el CUDE**: es el mismo hash que el CUFE con el **PIN del software** en
lugar de la clave técnica. Un solo campo de diferencia, y confundirlos no se ve
leyendo: se ve cuando la DIAN rechaza el documento.

## Cosas que ya costaron una vez

- **`codigo_barras` se guarda `NULL`, nunca `''`.** SQLite admite muchos `NULL`
  en un índice único pero solo una cadena vacía; con `''` el segundo artículo
  sin código de barras no se puede crear. 85 de los 148 del corpus no lo tienen.
- **Hasta el 2026-08-06 no había NI UNA cabecera de seguridad** —ni CSP, ni
  `nosniff`, ni `frame-options`—, y aun así la prueba del `index.html` de Angular
  decía defender `script-src 'self'`: defendía una política que nunca se aplicó.
  Ahora van en la aplicación (`rutas::cabeceras_de_seguridad`), no solo en el
  nginx de la guía, porque esto se instala en el servidor del cliente y quien lo
  administre puede poner otro proxy. Lo fija
  `toda_respuesta_del_binario_lleva_las_cabeceras_de_seguridad`, validada con su
  mutante: metiendo `'unsafe-inline'` en la CSP, se pone roja.
- **Aquella prueba se omitía SOLA.** Llevaba un `return` silencioso si el binario
  se había compilado sin `./construir.sh`, así que en un árbol sin `ng build`
  pasaba sin comprobar nada (directiva 30). Con maud ese estado no existe: el
  HTML lo compila `rustc`, y si no hay página no hay binario. La sustituta,
  `el_binario_sirve_el_panel_que_lleva_dentro`, no puede saltarse.
- **Una escritura con `Origin` de otro sitio se rechaza; sin `Origin` pasa.**
  `SameSite=Strict` es el control principal, pero es por SITIO y no por ORIGEN:
  en el VPS conviven `inventario`, `chasqui`, `medico` e `informes` bajo
  `xiliux.com`, así que un formulario servido desde un hermano viaja como
  «mismo sitio» con la cookie puesta. Lo cierra `rutas::origen_ajeno`, en el
  middleware —un solo sitio, y cubre las rutas de mañana—. **Sin `Origin` NO se
  bloquea, a propósito** (directiva 21): lo omiten `curl`, la integración de un
  tercero contra `/api` y el propio arnés, que son operación normal. Lo fija
  `una_escritura_con_origen_ajeno_se_rechaza_y_sin_origen_pasa`, con las dos
  mitades y validada con su mutante.
- **`usuario::puede` devuelve `Option<bool>`.** `None` = rol desconocido, y en
  `rutas::exigir` eso NIEGA. Es la única lectura segura de «no sé» en un control
  de acceso.
- **El nombre de una prueba no lleva la cifra de lo que cuenta** si la cifra
  puede cambiar: `mas_de_cien_operaciones...` cuenta y exige el mínimo al final.
- **Una cadena de texto partida en dos líneas muerde de DOS formas, y la que
  este archivo describía era la equivocada** (medido el 2026-08-11 con `rustc`,
  no de memoria). SIN `\` al final conserva el salto **y la sangría del
  código** —`"uno dos\n         tres"`—, y ése fue el que sacó el aviso de
  `/api/demo` a producción con veinte espacios en medio. CON `\` al final sí
  quita las dos cosas, pero junta **sin separador**: `"uno dos\` + `tres"` da
  `"uno dostres"`, así que el espacio hay que dejarlo antes de la barra. Los dos
  fallan en silencio. Para texto que se muestra, `concat!`: cada trozo es una
  cadena entera y el espacio se ve.
  Volvió a pasar el 2026-08-11 en el aviso de revisión del migrador —22 espacios
  incrustados, dos veces—, esta vez ni con barra ni sin ella: escrito en UNA
  línea física larguísima con los espacios tecleados dentro. La lección de
  fondo no es la sintaxis, es que **el texto que lee una persona hay que verlo
  impreso**, y por eso la prueba del migrador afirma sobre el motivo del
  rechazo, no solo sobre que exista.
- **Ningún elemento se dibuja con el atributo `class` dos veces.** `maud` lo
  emite al mezclar la sintaxis corta (`td.num`) con un `class` calculado, y el
  navegador **se queda con el primero y tira el segundo** —medido con Chromium:
  `<td class="num" class="alerta">` llega al DOM como `<td class="num">`—. Así
  estaban las CINCO marcas de alerta del panel: el CSS las definía, el servidor
  creía emitirlas y no se vieron nunca, con la página respondiendo 200 y el
  texto correcto. La forma es `td.num.alerta[condicion]`. Lo fija
  `ninguna_etiqueta_sale_con_el_atributo_class_dos_veces`.
- **Las listas de rutas de las pruebas de guardia se DERIVAN de `rutas.rs`**
  con `include_str!`, no se escriben a mano. Estaban copiadas en tres sitios, y
  una lista copiada se queda corta en la primera ruta nueva sin dar un solo
  error: la que falta es justo la que nadie mide. Lleva su positivo de control
  —si el parser devolviera vacío, los tres bucles saldrían verdes sin comprobar
  nada—, y al escribirlo se cazó que se comía la ruta declarada en varias
  líneas: 8 de 9, y era la última añadida.
- **Lo que el dominio ACEPTA y lo que el esquema GUARDA son dos vocabularios.**
  `Formula::desde_texto` admite `promedio`, `ppp` y `fifo` a propósito; el
  `CHECK` solo admite `promedio_ponderado` y `peps`. `guardar_empresa` validaba
  con el primero y ligaba la cadena cruda, así que el desplegable del panel
  —que manda `promedio`— contestaba un 500 opaco en la operación más común del
  primer día. Se guarda siempre la forma canónica que devuelve el dominio.
- **El respaldo NO compara sus cuentas contra el original.** La copia es una foto
  de un instante y el original sigue moviéndose; compararlos es medir dos
  instantes y llamarlo «reproducción exacta». Se verifica lo que es propiedad de
  la copia sola: `integrity_check`, `foreign_key_check` y que todas las tablas
  respondan.

## La demostración pública

`https://inventario.xiliux.com` — usuario `demo`, clave `inventario2026`.

Corre en el VPS como `inventario-demo.service` (`/opt/inventario-demo`, puerto
loopback 8020, nginx delante). El kit está en `despliegue/`; el procedimiento
general del servidor, en `/mnt/data/conocimiento/despliegue/VPS.md`.

**Lo que la convierte en demostración son DOS variables de entorno**
(`INVENTARIO_DEMO_USUARIO` e `INVENTARIO_DEMO_CLAVE`), no un binario distinto.
Sin ellas `/api/demo` responde 204 y la pantalla de acceso no muestra ningún
cartel. Si alguien mueve eso a una bandera de compilación, habrá dos artefactos
que mantener y uno se quedará atrás.

`GET /api/empresa` es **público** —lo pide la pantalla de acceso antes de que
nadie entre— y por eso devuelve `EmpresaPublica`, sin el PIN del software. La
vista completa vive en `/api/empresa/configuracion`, con sesión. La prueba
`la_vista_publica_de_la_empresa_no_lleva_el_pin_del_software` lo fija.

## Antes de mergear

Este repo maneja dinero y numeración fiscal. Correr `security-review` **desde
esta carpeta** antes de mergear a rama protegida (directiva 25 — la lista
nombra los repos cripto y legales; este entra por el mismo criterio: factura
contra resolución DIAN y guarda cartera de terceros).
