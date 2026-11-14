<!--
SPDX-FileCopyrightText: 2026 Juan Carlos Isaza
SPDX-License-Identifier: LicenseRef-Propietario
-->

# Análisis de brechas y plan

> **Estado al 2026-08-02**: las fases 1, 2 y 3 están **implementadas y
> probadas** (226 pruebas en verde, incluida una simulación de cumplimiento de
> 27 operaciones contra el binario real por HTTP). Lo único que queda fuera es
> F9 —emisión electrónica ante la DIAN—, por el motivo que se explica al final.
> Las tablas de abajo conservan el estado ANTES de la implementación, porque son
> el diagnóstico que la justifica.

Qué exigen los estándares de [`ESTANDARES.md`](ESTANDARES.md) y qué tiene hoy el
sistema. El estado se mide contra el código, no contra la intención.

Leyenda: **✓** cumple · **~** parcial · **✗** no existe.

---

## 1. Contable — NIC 2 / Sección 13 (Decreto 2420 de 2015)

| # | Exigencia | Estado | Qué falta exactamente |
|---|---|:--:|---|
| C1 | Costo = precio + aranceles + transporte + otros directos − descuentos (NIC 2.11) | ✗ | El kardex recibe `costo_unitario` ya cocinado. No hay forma de repartir un flete entre las líneas de una compra. El costo queda subvaluado y con él el costo de ventas. |
| C2 | Fórmula FIFO **o** promedio ponderado (NIC 2.25) | ~ | Solo promedio ponderado, y cableado. Un negocio de producto perecedero —este lo es— normalmente costea FIFO. |
| C3 | UEPS/LIFO prohibido | ~ | No está implementado, pero tampoco **rechazado con un mensaje**. Un silencio no es una prohibición. |
| C4 | Política de costeo declarada y consistente | ✗ | No hay dónde declararla. |
| C5 | **Medición al menor entre costo y VNR** (NIC 2.9, 28–33) | ✗ | **No existe el deterioro.** El inventario se lleva siempre al costo. Es un incumplimiento directo del principio de medición. |
| C6 | Reversión del deterioro cuando cesan las causas (NIC 2.33) | ✗ | No existe. |
| C7 | Revelaciones (NIC 2.36) | ✗ | No hay informe con importe en libros, deterioro reconocido y revertido, ni costo llevado al gasto. |

## 2. Tributario colombiano

| # | Exigencia | Estado | Qué falta exactamente |
|---|---|:--:|---|
| T1 | Sistema de inventario **permanente** (ET art. 62) | ✓ | Se cumple por construcción: la existencia se deriva del kardex y facturar mueve kardex en la misma transacción. |
| T2 | **Tope del 3 %** por faltantes de fácil destrucción (ET art. 64) | ✗ | Hay conceptos de salida por vencimiento y mal estado, pero nada mide el 3 % contra su base (inventario inicial + compras) ni avisa al pasarse. |
| T3 | **Acta de baja por obsolescencia** con cantidad, descripción, costo fiscal unitario, justificación y firmas (ET art. 64) | ✗ | No existe el documento. Sin él la deducción no se sustenta. |
| T4 | **Retiro de inventarios = venta para IVA**, base valor comercial (ET art. 421 lit. b) | ✗ | **La brecha de mayor riesgo.** Obsequios, consumo interno y retiro de socios salen del kardex sin generar un peso de IVA. En el corpus heredado eso son 161 movimientos. |
| T5 | Inventario y balance al menos una vez al año (CCo art. 52) | ✗ | No hay corte de inventario a una fecha. |
| T6 | Conservación 10 años (Ley 962/2005 art. 28) | ~ | Nada se borra —anular no borra— pero no hay exportación de respaldo que garantice «reproducción exacta». |

## 3. Facturación electrónica — Resolución 000165 de 2023

| # | Exigencia | Estado | Qué falta exactamente |
|---|---|:--:|---|
| F1 | Numeración autorizada con prefijo y rango | ~ | Hay prefijo y rango. Falta la **clave técnica** y la **fecha desde** de la vigencia (solo hay `vence`). |
| F2 | La vigencia y el rango se agotan **por separado** | ~ | Solo se controla el rango. |
| F3 | **Fecha y hora** de generación | ✗ | Solo fecha. La hora con desplazamiento horario entra en el CUFE. |
| F4 | **Unidad de medida** por ítem | ✗ | No existe el campo. |
| F5 | **Código de producto con estándar** (UNSPSC 001 / GTIN 010) | ✗ | Hay `codigo_barras` sin estándar declarado ni validación. |
| F6 | **Medio de pago** cuando es de contado | ~ | El medio de pago vive en el recibo, no en la factura de contado. |
| F7 | Discriminación de **IVA e INC** | ~ | IVA sí. **INC no existe.** |
| F8 | **CUFE** (SHA-384, anexo 1.9 num. 11.2) | ✗ | No existe. |
| F9 | Firma digital, QR, transmisión y validación previa DIAN | ✗ | **Fuera de alcance**: exige certificado digital y habilitación ante la DIAN. Es un proyecto propio. |

## 4. Identificación y gestión (internacional)

| # | Exigencia | Estado | Qué falta |
|---|---|:--:|---|
| G1 | Dígito de control GTIN validado | ✗ | Se guarda cualquier cosa como código de barras. |
| G2 | Punto de reorden con stock de seguridad estadístico | ~ | Solo un mínimo fijo digitado. En el corpus, 1 artículo de 148 lo tenía puesto. |
| G3 | Clasificación ABC | ✗ | No hay. |
| G4 | Rotación y días de inventario | ✗ | No hay. |

---

## Plan

Ordenado por **riesgo legal × factibilidad local**. Todo lo que entra se
implementa completo, con prueba; lo que no entra se dice.

### Fase 1 — Incumplimientos contables y tributarios · **HECHO**

1. **C1 — Costo de adquisición prorrateable.** Una compra puede traer flete,
   arancel y seguro que se reparten entre sus líneas en proporción al valor, y
   descuentos comerciales que se restan. Es NIC 2.11 literal.
2. **C2/C3/C4 — Fórmula de costeo elegible: `promedio` o `peps`.** Capas de
   costo para PEPS. **UEPS se rechaza con un error que lo nombra** y cita la
   norma: una prohibición tiene que ser audible.
3. **C5/C6 — Deterioro por VNR con reversión.** Medición al menor entre costo y
   VNR, rebaja como gasto, reevaluación y reversión limitada al costo original.
4. **T4 — Retiro de inventarios genera IVA.** El concepto de kardex declara si
   causa IVA (art. 421 lit. b); el movimiento calcula el impuesto sobre el
   **valor comercial**, no sobre el costo.
5. **T2/T3 — Control del 3 % y acta de baja del art. 64.** Informe que mide el
   consumido contra la base real y acta con los campos que enumera la norma.
6. **C7/T5 — Corte de inventario y revelaciones a una fecha.**

### Fase 2 — Facturación electrónica, la parte que se puede hacer sin la DIAN · **HECHO**

7. **F8 — CUFE SHA-384**, probado contra el ejemplo oficial del anexo 1.9.
8. **F3/F1/F2 — Hora de generación, clave técnica, vigencia desde/hasta.**
9. **F4/F5/G1 — Unidad de medida, código estándar del producto y validación del
   dígito de control GTIN.**
10. **F7 — INC** como impuesto propio, discriminado en línea y en cabecera.
11. **F6 — Medio de pago** en la factura de contado.

### Fase 3 — Gestión · **HECHO**

12. **G2/G3/G4 — Stock de seguridad, punto de reorden, ABC y rotación**,
    calculados sobre el kardex real.

### Lo que NO entra, y por qué

- **F9 — Emisión electrónica real ante la DIAN.** Necesita certificado digital
  de firma, habilitación del facturador, generación de UBL 2.1 con extensiones
  DIAN, firma XAdES-BES y consumo del servicio de validación previa. Nada de eso
  se puede probar sin credenciales del cliente, y una implementación «a ciegas»
  que no se puede ejercer contra el servicio real sería justo lo que la
  directiva 28 prohíbe dar por terminado. El sistema queda con **todos los datos
  que el anexo exige** —incluido el CUFE— para que ese paso sea conectar, no
  rehacer.
- **Documento equivalente POS electrónico.** Mismo motivo.
- **Retención en la fuente e ICA por municipio.** El campo de retención existe y
  el CUFE reserva el sitio del ICA; las tarifas y su base son un módulo de
  configuración tributaria que merece su propia pasada.


---

## Dónde quedó cada cosa

| # | Qué | Dónde |
|---|---|---|
| C1 | Costo de adquisición con flete, arancel, seguro y descuentos | `dominio/costeo.rs::costo_adquisicion` |
| C2/C3/C4 | PEPS y promedio; UEPS rechazado citando la norma | `dominio/costeo.rs::Formula`, tabla `capa_costo` |
| C5/C6/C7 | Deterioro por VNR, reversión y revelaciones | `dominio/vnr.rs`, `datos/deterioro.rs`, `POST /api/deterioro` |
| T2/T3 | Tope del 3 % y acta de baja | `dominio/retiro.rs`, `datos/tributario.rs`, `GET /api/informes/art64`, `POST /api/actas` |
| T4 | IVA por retiro sobre valor comercial | `concepto.efecto_tributario`, `datos/kardex.rs`, `GET /api/informes/retiros` |
| F1/F2/F3 | Clave técnica, vigencia desde/hasta, hora de generación | tabla `resolucion`, `factura.hora` |
| F4/F5/G1 | Unidad de medida, estándar del producto, dígito GTIN | `dominio/gtin.rs`, `articulo.unidad_medida`, `articulo.tipo_estandar` |
| F6 | Medio de pago en la factura de contado | `factura.medio_pago` |
| F7 | INC discriminado del IVA | `dominio/factura.rs`, `factura.inc`, `factura_linea.inc` |
| F8 | CUFE SHA-384 | `dominio/cufe.rs`, probado contra el ejemplo oficial del anexo 1.9 |
| G2/G3/G4 | Rotación, ABC, punto de reorden | `datos/gestion.rs`, `GET /api/informes/{rotacion,abc,reorden}` |

## Fase 4 — Que el cumplimiento LLEGUE a la pantalla · **HECHO** (2026-08-11)

Las fases 1 a 3 dejaron el cumplimiento implementado en el dominio, probado y
servido por `/api`. Al verificarlo contra el código salió que **eso no basta**:
el cliente que recibe este producto tiene un navegador, no un cliente HTTP, y
seis informes y tres operaciones vivían solo en JSON. Un informe que no cruza la
frontera hacia el usuario no está terminado (directiva 28), y son justo los que
sustentan una deducción y una revelación.

No era regresión de la migración a maud: el Angular tampoco los tenía. Era una
brecha que este documento daba por cerrada.

| Qué faltaba en la pantalla | Norma | Dónde quedó |
|---|---|---|
| Tope del 3 % y su detalle | ET art. 64 | `vista::pantalla_informes` |
| Acta de baja sobre las salidas pendientes | ET art. 64 | `vista::informes_acta`, `datos/tributario.rs::bajas_sin_acta` |
| Retiros gravados con IVA | ET art. 421 lit. b | `vista::pantalla_informes` |
| Correr y leer el corte de deterioro | NIC 2.28–33, 2.36 | `vista::informes_deterioro` |
| Rotación, ABC y punto de reorden | gestión | `vista::pantalla_informes` |
| Corregir el efecto tributario de un concepto | ET art. 421 lit. b | `catalogo::actualizar_efecto_tributario`, `PUT /api/conceptos/{id}/efecto` |
| Registrar resolución y concepto | Res. 000165/2023 | `vista::configuracion_resolucion`, `configuracion_concepto` |
| PIN e identificador del software | CUDE | `vista::configuracion_software` |
| Configurar la empresa la PRIMERA vez | — | `vista::pantalla_configuracion` |

**Lo que faltaba y no era pantalla, sino pieza:** `bajas_sin_acta`. `crear_acta`
exige —bien exigido— el movimiento de kardex que sacó la mercancía, y nada decía
cuáles eran, así que el acta solo se podía levantar adivinando un id. Lo
pendiente se cuenta por unidades sin cubrir, no por «tiene renglón o no»: un
acta puede cubrir parte de un movimiento.

**Y una brecha de verdad, que la prueba nueva cazó al ponerse roja:** el informe
de retiros gravados filtraba por `concepto.efecto_tributario` —la política
vigente— en vez de por lo que el movimiento registró. En cuanto corregir un
concepto es posible, eso hacía aparecer sus movimientos ANTERIORES como retiros
gravados con base y IVA en cero. Ahora filtra por `movimiento.valor_comercial IS
NOT NULL`, que es el hecho ocurrido (directiva 32).

## Lo que la implementación enseñó y no estaba en el plan

- **El migrador no puede deducir el efecto tributario.** El corpus no tiene la
  columna, y adivinarla por el nombre del concepto haría que un obsequio
  liquidara IVA o no según cómo lo hubiera escrito alguien en 2004. Se deja
  neutro y **se reporta cada concepto de salida para revisión humana**.
- **`efecto_tributario` no lleva valor por defecto en el esquema.** Es la
  decisión de diseño que impide que un concepto nuevo de obsequio nazca sin IVA
  por omisión.
- **Un artículo dice qué le falta para la factura electrónica** antes de
  emitirla (`falta_para_factura_electronica`). Descubrirlo al emitir es
  descubrirlo tarde.
- **El código de barras del corpus no es un GTIN.** `41251501` no pasa su dígito
  de control, así que se declara como código interno (`999`), que es la verdad.
