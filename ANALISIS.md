<!--
SPDX-FileCopyrightText: 2026 Juan Carlos Isaza
SPDX-License-Identifier: LicenseRef-Propietario
-->

# Análisis del sistema heredado (`/mnt/data/inventario_2004`)

Todo número de este documento está **medido**, no estimado. Los guiones que lo
midieron están en `herramientas/` y se pueden volver a correr.

## 1. Qué es el sistema viejo

Aplicación **Visual FoxPro 6** (`inventario.pjx`, `VFP6RUN.EXE`) de inventario,
facturación y cartera para una **envasadora y distribuidora de agua** de
Medellín — NIT `811033579-6`, CRA 43A #30-74. Los artículos son botellones y
PET de 335/500 c.c., y los conceptos de movimiento nombran el negocio sin
ambigüedad: *DEVOLUCIÓN DE BOTELLONES X CLIENTES*, *VENTAS DE CONTADO
RELÁJATE*, *TIENDA DEL AGUA*, *VENTAS SPA*.

48 tablas DBF, 26 índices CDX, formularios `.scx`, informes `.frx`, un menú
`.mnx` y tres programas `.prg` con la lógica compartida.

## 2. Qué está vivo y qué está muerto

| Tabla | Registros vivos | Borrados | Rango de fechas |
|---|---:|---:|---|
| `facturas` | 3.154 | 0 | 2004-05-03 … 2020-09-01 |
| `fac_articulo` (detalle) | 3.712 | 0 | — |
| `mov_inv` (kardex) | 1.321 | 0 | **2004-05-01 … 2004-09-30** |
| `CLIENTES` | 794 | 0 | 2004-03-08 … 2004-11-18 |
| `ARTICULOS` | 148 | 0 | sin fecha |
| `DETRECIBO` | 323 | 19 | — |
| `ENCRECIBO` | **10** | **332** | 2004-09-01 |
| `valorconvenio` | 261 | 2.596 | — |
| `CATEGORIAS` | 6 | 6 | — |
| `CONCEPTOS` | 33 | 0 | — |

### El hallazgo que decide el diseño

**El kardex se abandonó en septiembre de 2004; la facturación siguió hasta
2020.** Dieciséis años facturando sin registrar un solo movimiento de
inventario. La existencia que muestra el sistema (`ARTICULOS.INV_ACTUAL`) es
por tanto **ficción**: 27 de 148 artículos tienen existencia distinta de cero, y
uno la tiene **negativa** (`1102001`, −3 unidades).

La causa es estructural, no de disciplina: `INV_ACTUAL` es un **campo mutable**
que se actualiza aparte del movimiento, y `recalsalidas.prg` es un proceso por
lotes que lo recalcula *filtrando por un mes y un año* guardados en
`CONTROL.MES_INV` / `CONTROL.ANO_INV` — hoy congelados en octubre de 2023. Si
alguien no corre el proceso, o lo corre con el periodo mal, el número miente y
nada se cae.

## 3. Reglas de negocio recuperadas del código VFP

Extraídas de `programas/procesos.prg` y `programas/recalsalidas.prg`.

**Aritmética de la factura** — verificada contra las 3.154 facturas reales:

```
VALORFAC = Σ(cantidad × valor) + VALORIVA − DESCTO
```

Se cumple en **3.065 de 3.094** facturas que tienen detalle. Desglose medido:

| Caso | Facturas |
|---|---:|
| cabecera = detalle | 2.725 |
| cabecera = detalle + IVA | 335 |
| cabecera = detalle + IVA − descuento | 5 |
| sin detalle | 60 |
| **no explicado** | **29** |

Los 29 no explicados **no son un fallo de aritmética**: son exactamente los
**15 números de factura duplicados** (`3065`, `3214`, `3446`, `3447`, `3951`–`3954`,
`4151`–`4153`, `5351`, `5352`, `5356`, `674`). Dos facturas distintas comparten
número, así que el mismo detalle se le suma a las dos. La clave primaria está
rota, no la fórmula.

**Costo promedio ponderado** (`recalsalidas.prg`): sobre las entradas del mes
con concepto de signo `+`,

```
costo_salida = (saldo_inicial × costo_inicial + Σ cantidad × (valor − descuento/cantidad))
             ÷ (saldo_inicial + Σ cantidad)
```

**Cartera**: saldo de una factura = `VALORFAC` − Σ`DETRECIBO.VALOR` (por
`nrofac` **y** `prefijo`) − Σ`NOTAFACT.VALOR`.

**Numeración**: `CONFIFAC` guarda seis grupos de facturación con resolución
DIAN, rango `[NROMINFAC..NROMAXFAC]` y `ULT_FACTU`. Dos de ellos ya se agotaron
(`08`: último 5650 con tope 5650; `05`: 5349 de 5350).

**Monto en letras**: tabla `MONTOESC` de 108 filas más el procedimiento
`montoesc` — un conversor de número a español con casos de «MILLÓN/MILLONES» y
«CIENTO».

## 4. Los 33 conceptos de movimiento

El kardex es configurable por concepto, con `SIGNO` `+`/`−` y banderas de
clase (proveedor / saldo inicial / gastos / cliente). Los realmente usados:

| Código | Concepto | Signo | Usos |
|---|---|:-:|---:|
| 41 | Entradas por ajustes de inventario | + | 341 |
| 42 | Salidas por ajustes de inventario | − | 216 |
| 71 | Salidas insumos por producciones | − | 203 |
| 12 | Entradas de producto terminado | + | 145 |
| 32 | Devolución de botellones x clientes | + | 120 |
| 76 | Salidas por obsequios | − | 91 |
| 13 | Saldos iniciales | + | 71 |
| 75 | Salidas por retiro socios | − | 49 |
| 79 | Consumo interno | − | 21 |
| 73 | Salidas por mal estado | − | 20 |
| 31 | Entradas por traslados | + | 14 |
| 11 | Compras de insumos / proveedores | + | 13 |
| 78 | Insumos vencidos o malos | − | 7 |
| 77 | Salidas x traslados | − | 5 |
| 22 | Devolución por mal estado | + | 2 |
| *(vacío)* | **no existe en `CONCEPTOS`** | ? | **3** |

**Ningún movimiento tiene concepto de venta** (51–55), aunque hay 3.154
facturas. Confirma lo del punto 2: facturar nunca generó kardex.

## 5. Defectos de integridad medidos

Esto es la deuda que el migrador tiene que **rechazar con motivo**, nunca
rellenar con un defecto (directiva 20):

| Defecto | Cantidad |
|---|---:|
| `NROFAC` duplicados (incluso con prefijo) | 15 pares |
| `fac_articulo` → factura inexistente | 34 líneas, 27 números |
| `facturas` → cliente inexistente | 5 |
| `fac_articulo` / `mov_inv` / `valorconvenio` → artículo inexistente | 1 cada uno |
| `mov_inv` con concepto vacío | 3 |
| NIT de cliente duplicado | 47 |
| **`IDCLIENTE` duplicado** (era la clave) | **4** |
| `DETRECIBO` sin cabecera `ENCRECIBO` viva | 313 de 323 |
| Clientes sin zona / sin ciudad | 26 / 17 |
| `PAGO` con valores no numéricos (`'0.'`, `''`) | 239 |

### Defectos que solo aparecieron al migrar

No estaban en el perfilado inicial: los descubrió el migrador al chocar contra
las restricciones del esquema nuevo. Todos se **reportan**, ninguno se rellena.

| Defecto | Cantidad |
|---|---:|
| Artículos que comparten código de barras con otro | 42 de 148 |
| Códigos de ciudad colisionados (`101` = BOLÍVAR **y** CALI) | 1 |
| Códigos de zona colisionados (17 filas → 5 códigos; `01` = ORIENTE **y** CENTRO) | 12 |
| Clientes cuyo «NIT» no tiene un solo dígito (`"---"`, `"BEBE"`, `"USA"`, `"SIN #"`, `"-"`) | 5 |
| Resoluciones con prefijo y arranque repetidos | 1 |

El de los códigos de barras es el más instructivo. La primera versión del
migrador usaba `INSERT OR IGNORE`, que **no falla**: se traga el choque en
silencio. Entraron 106 de 148 artículos y el informe decía que todo había ido
bien. Lo cazó la prueba que exigía los 148 exactos. Es la directiva 20 en su
forma más peligrosa —un sistema que *parece* funcionar— y por eso ahora el
choque se comprueba antes de insertar y sale por escrito.

`IDCLIENTE` no es un identificador: es el NIT concatenado con un consecutivo,
con longitudes de 6 a 22 caracteres, y **se repite**. `DETRECIBO` conserva 313
detalles de recibo cuya cabecera fue borrada: los abonos existen, el recibo que
los explica no.

## 6. Decisiones de diseño del sistema nuevo

Cada una responde a un defecto medido arriba. El caso de 2004 es la **prueba**,
no la especificación (directiva 19): el sistema se diseña para cualquier
negocio de inventario y facturación.

| # | Decisión | Defecto que elimina |
|---|---|---|
| 1 | **La existencia se DERIVA del kardex.** No hay campo `inv_actual` que se pueda actualizar por otro camino. Cada movimiento persiste su `existencia_despues` como foto inmutable, y una prueba comprueba que la suma del kardex y la última foto coinciden. | La existencia-ficción y `recalsalidas.prg` |
| 2 | **Facturar emite el kardex en la MISMA transacción.** No se puede facturar sin mover inventario: es la misma escritura. | 16 años de facturas sin kardex |
| 3 | **`UNIQUE(prefijo, numero)`** y el número se asigna dentro de la transacción, contra el rango de la resolución. | Los 15 números duplicados |
| 4 | **El dinero es entero (centavos); las cantidades, enteras (milésimas).** Nunca coma flotante. | `N(11,2)` y los redondeos que descuadran cabecera y detalle |
| 5 | **Costo promedio ponderado calculado EN cada movimiento** y persistido en él. No hay proceso por lotes ni periodo global. | `CONTROL.MES_INV`=10/2023 congelado |
| 6 | **Clave primaria propia (`INTEGER`); el NIT es un atributo** con su dígito de verificación. | 4 `IDCLIENTE` duplicados, 47 NIT repetidos |
| 7 | **El detalle de recibo no existe sin cabecera** (FK `ON DELETE CASCADE`, y las claves foráneas ENCENDIDAS: SQLite las trae apagadas). | Los 313 abonos huérfanos |
| 8 | **Anular no borra.** Estado `anulada` + movimiento de reverso. | `dele all` + `pack` de `iniciar inventario.txt`, que borraba el histórico |
| 9 | **El monto en letras es código, no una tabla de 108 filas.** | `MONTOESC.DBF` |

## 7. Alcance que NO se toma en esta pasada

Se dice explícitamente para que no pase por hecho:

- **Facturación electrónica DIAN (FEV/RADIAN)**. El sistema modela resoluciones
  y rangos —que es lo que el corpus prueba— pero no emite ni firma documento
  electrónico. Es un proyecto propio, con su propia validación contra la DIAN.
- **Pedidos y despachos**. Existen en el corpus (77 vivos, 2004–2017) y el
  esquema les deja sitio, pero no hay pantalla ni API en esta entrega.
- **Multiempresa**. Una base por instancia, como el resto del estándar.
