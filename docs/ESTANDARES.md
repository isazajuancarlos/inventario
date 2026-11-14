<!--
SPDX-FileCopyrightText: 2026 Juan Carlos Isaza
SPDX-License-Identifier: LicenseRef-Propietario
-->

# Estándares aplicables a un sistema de inventario

Investigación con fuente citada. Lo que no se pudo verificar contra el texto
normativo se marca como **sin verificar**, no se afirma (directiva 2).

Consultado el 2026-08-02.

---

## 1. Marco contable

### 1.1 Colombia: qué norma aplica

Las NIIF están incorporadas al derecho colombiano por el **Decreto 2420 de
2015** (Decreto Único Reglamentario de las normas de contabilidad), que las
compila en anexos por grupo:

| Grupo | Norma de inventarios |
|---|---|
| 1 (NIIF plenas) | **NIC 2 — Inventarios** (Anexo 1) |
| 2 (NIIF para PYMES) | **Sección 13 — Inventarios** (Anexo 2) |
| 3 (microempresas) | Decreto 2706, capítulo 8 |

El principio de medición es **el mismo en los tres**: costo o valor neto
realizable, **el menor**.

### 1.2 NIC 2: las reglas que un sistema tiene que poder cumplir

**Costo de adquisición (NIC 2.11).** Literal:

> «El costo de adquisición de los inventarios comprenderá el precio de compra,
> los aranceles de importación y otros impuestos (que no sean recuperables
> posteriormente de las autoridades fiscales) y transporte, manejo y otros
> costos directamente atribuibles a la adquisición de mercaderías, materiales y
> servicios. Los descuentos comerciales, las rebajas y otras partidas similares
> se deducirán para determinar el costo de adquisición.»

Consecuencia para el software: **el costo unitario no es el de la factura del
proveedor**. Hay que poder repartir flete, arancel y seguro entre las líneas de
una compra, y restar los descuentos comerciales.

**Fórmulas de cálculo del costo (NIC 2.25).**

> «El costo de los inventarios […] se asignará utilizando los métodos de primera
> entrada primera salida (FIFO) o costo promedio ponderado.»

**UEPS/LIFO está prohibido** bajo NIIF. (Sigue permitido en US GAAP, ASC 330 —
por eso una comparación internacional lo encuentra; en Colombia no.)

La entidad **debe usar la misma fórmula** para todos los inventarios de
naturaleza y uso similar, y la política debe revelarse.

**Valor neto realizable (NIC 2.9, 28–33).** Los inventarios se miden al **menor
entre el costo y el VNR**. El VNR es el precio estimado de venta menos los
costos estimados de terminación y de venta. La rebaja se reconoce como gasto del
periodo. En **cada periodo posterior** se vuelve a evaluar, y si las causas de
la rebaja dejaron de existir, **la rebaja se revierte** (limitada al costo
original).

**Revelaciones (NIC 2.36).** Políticas y fórmulas usadas, importe en libros por
categorías, importe reconocido como gasto, importe de las rebajas reconocidas y
**revertidas** en el periodo, y las circunstancias que llevaron a la reversión.

### 1.3 Estándar internacional distinto que conviene conocer

**US GAAP (ASC 330)** sí admite LIFO, y mide al *lower of cost or market* para
LIFO/retail y *lower of cost and net realizable value* para lo demás. Un sistema
que quiera servir en los dos marcos necesita que la fórmula de costeo sea
**configurable y auditable**, no cableada. Aquí se implementan las dos que
permite NIIF y se **rechaza** UEPS con un error que lo dice.

---

## 2. Marco tributario colombiano

### 2.1 Sistema de determinación del costo — ET art. 62

Modificado por la Ley 1819 de 2016. Dos sistemas: **juego de inventarios
(periódico)** e **inventarios permanentes (continuos)**. Los contribuyentes
obligados a presentar declaración firmada por revisor fiscal o contador público
**deben** usar el permanente.

Un sistema informático de inventario es, por construcción, permanente — pero
solo si **cada** movimiento queda registrado. El sistema que se reemplazó
facturó dieciséis años sin registrar salidas: era permanente en el papel y
periódico —peor, inexistente— en los hechos.

### 2.2 Disminución del inventario — ET art. 64

Modificado por el art. 43 de la Ley 1819 de 2016. Tres supuestos:

1. **Faltantes de fácil destrucción o pérdida**: el inventario final puede
   disminuirse **hasta en un 3 % de la suma del inventario inicial más las
   compras**. Por encima de ese tope hace falta demostrar fuerza mayor o caso
   fortuito.
2. **Baja por obsolescencia**: deducible al precio de adquisición más costos
   directamente atribuibles, **siempre que los bienes se destruyan, reciclen o
   chatarricen**, y exista documento que contenga **cantidad, descripción, costo
   fiscal unitario y justificación**, firmado por el representante legal y los
   responsables de la destrucción.
3. **Inventarios asegurados**: solo es deducible la parte **no cubierta** por la
   indemnización o el seguro.

Además: no puede usarse más de una afectación para el mismo valor, y toda
recuperación posterior es renta líquida por recuperación de deducciones.

Consecuencia para el software: hay que **medir el 3 % contra su base real**
(inventario inicial + compras del periodo) y **producir el acta** con los campos
que la norma enumera. Un concepto de kardex llamado «salida por vencimiento» sin
esos dos controles no sirve para sustentar la deducción.

### 2.3 Retiro de inventarios — ET art. 421 literal b)

> Se consideran venta «el retiro de bienes corporales muebles e inmuebles hechos
> por el responsable para su uso o para formar parte de los activos fijos de la
> empresa».

**La base gravable es el valor comercial** de los bienes.

Esto es lo más fácil de incumplir sin darse cuenta. En el corpus heredado hay
**91 salidas por obsequios, 49 por retiro de socios y 21 por consumo interno**:
cada una es un hecho generador de IVA que el sistema viejo no calculaba ni
registraba en ninguna parte.

### 2.4 Libros y conservación

- **Código de Comercio art. 52**: al iniciar actividades y **al menos una vez al
  año**, todo comerciante debe elaborar un **inventario y balance general** que
  muestre de manera clara y completa el estado de su patrimonio.
- **Ley 962 de 2005, art. 28**: los libros y papeles del comerciante se
  conservan **diez (10) años** contados desde el último asiento, documento o
  comprobante, en papel **o en cualquier medio técnico, magnético o electrónico
  que garantice su reproducción exacta**.

Consecuencia: el sistema tiene que poder **congelar y exportar** el inventario a
una fecha, y no puede borrar histórico.

---

## 3. Facturación electrónica — Resolución DIAN 000165 de 2023

Norma vigente que compila el sistema de facturación y adopta el **Anexo Técnico
1.9**.

### 3.1 Requisitos de la factura electrónica de venta (art. 11)

Los que tocan directamente al modelo de datos:

| Requisito | Qué implica en el esquema |
|---|---|
| Numeración consecutiva **autorizada**, con prefijo | Rango + prefijo + **clave técnica** + **vigencia** de la autorización |
| **Fecha y hora** de generación | Hora con desplazamiento horario, no solo fecha |
| Detalle con cantidad, **unidad de medida** y descripción | Código de unidad de medida por artículo |
| Código del producto según un **estándar** | `@schemeID` = `001` (UNSPSC) o `010` (GTIN, con `@schemeAgencyID = 9`) |
| **Forma de pago** (contado/crédito) y **medio de pago** cuando es contado | Medio de pago en la factura, no solo en el recibo |
| Discriminación de **IVA e impuesto nacional al consumo** | INC como impuesto propio, distinto del IVA |
| **CUFE** | Ver 3.2 |
| Firma digital, QR, transmisión y validación previa DIAN | Requiere certificado y habilitación: **fuera del alcance local** |

### 3.2 CUFE — Anexo Técnico 1.9, numeral 11.2

Algoritmo **SHA-384** sobre la concatenación, **sin separadores**:

```
CUFE = SHA-384( NumFac + FecFac + HorFac + ValFac
              + "01" + ValImp1(IVA)
              + "04" + ValImp2(INC)
              + "03" + ValImp3(ICA)
              + ValTot + NitFE + NumAdq + ClTec + TipoAmbiente )
```

- `NumFac` = prefijo **concatenado** con el número.
- `ValFac` = valor **sin impuestos**, punto decimal, **dos decimales
  truncados**, sin separador de miles ni símbolo.
- Un impuesto no referenciado se representa `0.00` (no se omite).
- `NitFE` y `NumAdq` van **sin puntos, sin guiones y sin dígito de
  verificación**.
- `TipoAmbiente`: 1 producción, 2 pruebas.

**Ejemplo oficial del anexo**, que sirve de prueba de aceptación:

```
NumFac  323200000129     FecFac  2019-01-16     HorFac 10:53:10-05:00
ValFac  1500000.00       IVA     285000.00      INC 0.00    ICA 0.00
ValTot  1785000.00       NitFE   700085371      NumAdq 800199436
ClTec   693ff6f2a553c3646a063436fd4dd9ded0311471                TipoAmbiente 1

CUFE = 8bb918b19ba22a694f1da11c643b5e9de39adf60311cf179179e9b33381030bcd4c3c3f156c506ed5908f9276f5bd9b4
```

> **Ojo con la documentación vieja.** El Anexo Técnico 003 de la Resolución
> 000019 de 2016 usaba **SHA-1**, separador `;` y un orden de campos distinto
> (`CodImp2` era `02`, hoy es `04`). Está derogado. Implementar aquel produce un
> CUFE que la DIAN rechaza.

### 3.3 Autorización de numeración

La autorización tiene **vigencia máxima de 2 años**, y la DIAN la otorga por 6,
12, 18 o 24 meses. Existe la **habilitación** de rangos: cuando la vigencia se
agota pero la numeración no, se solicita con **15 días hábiles** de antelación al
vencimiento.

Consecuencia: una resolución tiene **fecha desde y fecha hasta**, no solo un
rango numérico, y las dos cosas pueden agotarse por separado.

### 3.4 Documento equivalente POS

Con la Resolución 000165 el tiquete POS electrónico puede expedirse
**independientemente de la cuantía** —desapareció el tope de 5 UVT— y da derecho
a impuestos descontables si identifica al adquirente.

---

## 4. Estándares internacionales de identificación

### 4.1 GTIN (GS1)

El Anexo Técnico 1.9 (cap. 14.2) adopta explícitamente **GTIN** como estándar de
identificación de producto. Estructuras GTIN-8, GTIN-12, GTIN-13 y GTIN-14, todas
almacenadas en un campo de 14 dígitos.

**Dígito de control**: módulo 10 con pesos alternados 3 y 1 desde la derecha.

```
suma = Σ (dígito_i × peso_i),  pesos 3,1,3,1… desde la posición más a la derecha
dv   = (10 − (suma mod 10)) mod 10
```

Un código de barras que no cumple su dígito de control **no es un GTIN**: es un
número interno. El corpus heredado guarda `41251501` (8 dígitos) en 63
artículos, y varios artículos distintos comparten el mismo — así que ahí no
había GTIN, había una clasificación interna.

### 4.2 UNSPSC

Clasificación estándar de productos y servicios de las Naciones Unidas.
`@schemeID = 001` en el anexo de la DIAN. Es una **clasificación**, no un
identificador único: dos artículos distintos pueden compartir código UNSPSC.

### 4.3 Unidades de medida

El anexo trae la tabla 13.3.6 de unidades de medida y la exige a nivel de ítem.
Los códigos siguen la familia UN/ECE (referencias a `unece.org/trade/untdid` en
el propio anexo). **Sin verificar**: no se pudo confirmar contra el texto que la
tabla 13.3.6 sea exactamente la Recomendación 20 de UN/CEFACT sin cambios; la
tabla vive en la «Caja de Herramientas» de la DIAN, aparte del anexo.

---

## 5. Gestión de inventarios (no normativo, pero es el estado del arte)

**Stock de seguridad** con nivel de servicio:

```
SS = z × σ_demanda × √(tiempo de reposición)
```

con `z` = 1,28 (90 %), 1,65 (95 %), 2,33 (99 %).

**Punto de reorden**: `ROP = demanda media × tiempo de reposición + SS`.

Un «punto mínimo» fijo digitado a mano —lo que tenía el sistema viejo, y con un
solo artículo configurado de 148— no es esto: no reacciona a la variabilidad de
la demanda ni al tiempo de reposición.

**Clasificación ABC** (Pareto): A ≈ 80 % del valor consumido, B ≈ 15 %, C ≈ 5 %.
Sirve para decidir dónde poner el esfuerzo de conteo y de control.

**Rotación**: `costo de ventas del periodo / inventario promedio`; y **días de
inventario** = `365 / rotación`.

---

## Fuentes

- [Art. 62 ET — Sistema para establecer el costo de los inventarios](https://estatuto.co/62)
- [Art. 63 ET — Limitación a la valuación (Actualícese)](https://actualicese.com/estatutotributario/63-2/)
- [Art. 64 ET — Disminución del inventario (texto vigente)](https://www.contadia.com/estatuto-tributario/articulo-64-disminucion-del-inventario)
- [Art. 421 ET — Hechos que se consideran venta / retiro de inventarios](https://www.gerencie.com/impuesto-a-las-ventas-en-el-retiro-de-inventarios.html)
- [Decreto 2420 de 2015 — Anexo 2 (Función Pública)](https://www.funcionpublica.gov.co/eva/gestornormativo/norma.php?i=74535)
- [NIC 2 Inventarios — texto (MEF Perú, misma norma IASB)](https://www.mef.gob.pe/contenidos/conta_publ/con_nor_co/vigentes/nic/2_NIC.pdf)
- [NIC 2 / Sección 13 — medición al menor entre costo y VNR (Actualícese)](https://actualicese.com/archivo/medicion-y-costo-de-los-inventarios-segun-nic-2-y-seccion-13-para-pymes/)
- [Resolución DIAN 000165 de 2023 — texto compilado](https://normograma.dian.gov.co/dian/compilacion/docs/resolucion_dian_0165_2023.htm)
- [Anexo Técnico Factura Electrónica de Venta v1.9 (DIAN)](https://www.dian.gov.co/impuestos/factura-electronica/Documents/Anexo-Tecnico-Factura-Electronica-de-Venta-vr-1-9.pdf)
- [Anexo Técnico 003 de 2016 — CUFE con SHA-1, **derogado**](https://facturaenlinea.co/facturaenlinea/Anexo_3_Resolucion_000019_24_Febrero_2016_FACTURA_ELECTRONICA.pdf)
- [DIAN — Autorización de numeración de facturación](https://micrositios.dian.gov.co/sistema-de-facturacion-electronica/numeracion-autorizacion/)
- [DIAN — Habilitación de numeración](https://micrositios.dian.gov.co/sistema-de-facturacion-electronica/numeracion-habilitacion/)
- [Ley 962 de 2005, art. 28 — conservación diez años](https://leyes.co/se_dictan_disposiciones_sobre_racionalizacion_de_tramites_y_procedimientos_administrativos_de_los_organismos_y_entidades_del_estado/28.htm)
- [Libros contables y conservación de documentos (Forvis Mazars)](https://www.forvismazars.com/co/es/insights/nuestras-publicaciones/actualidad-juridica-y-tributaria/2023/libros-contables-y-conservacion-de-documentos)
- [Global Trade Item Number — estructura y dígito de control](https://en.wikipedia.org/wiki/Global_Trade_Item_Number)
- [GS1 — cálculo del dígito de control](https://gs1ca.org/check-digit/)
- [Safety stock con desviación estándar y nivel de servicio (NetSuite)](https://www.netsuite.com/portal/resource/articles/inventory-management/safety-stock.shtml)
