<!--
SPDX-FileCopyrightText: 2026 Juan Carlos Isaza
SPDX-License-Identifier: LicenseRef-Propietario
-->

# Habilitación como facturador electrónico ante la DIAN

Qué hay que hacer para que un cliente pueda emitir factura electrónica con este
sistema, **qué trae ya el sistema y qué le toca al cliente**.

Fuentes: [proceso de registro y habilitación
(DIAN)](https://micrositios.dian.gov.co/sistema-de-facturacion-electronica/proceso-de-registro-y-habilitacion-como-facturador-electronico/),
Resolución 000165 de 2023 y su Anexo Técnico 1.9. Ver
[`ESTANDARES.md`](ESTANDARES.md) §3.

---

## 1. Qué modo de operación es este

La DIAN reconoce tres, y cambian el número de documentos del set de pruebas:

| Modo | Set de pruebas |
|---|---|
| Proveedor tecnológico | 6 facturas + 2 notas débito + 2 notas crédito |
| **Software propio** ← **este** | **2 facturas + 1 nota débito + 1 nota crédito** |
| Facturación gratuita DIAN | 2 facturas + 1 nota débito + 1 nota crédito |

Este sistema es **software propio del facturador**: se instala en su casa, con
su NIT, y él lo registra a su nombre. No somos proveedor tecnológico y no
transmitimos por cuenta de nadie.

---

## 2. Los pasos

### Fase 1 — Registro (lo hace el cliente en el portal de la DIAN)

1. Entrar al portal **Habilitación** y generar el token de acceso.
2. Pulsar **Registrar** y salir del sistema para que el cambio se aplique.
3. Volver a entrar, generar token nuevo.
4. Elegir modo de operación: **Software de Factura Electrónica**.
5. La DIAN **genera automáticamente los rangos de prueba** y asigna:
   - el **identificador del software**, y
   - el **PIN del software**.

Los dos van a **Configuración → Empresa** en este sistema. Sin el PIN no se
puede calcular el CUDE de las notas, y el diagnóstico de alistamiento lo dice.

### Fase 2 — Set de pruebas

Hay que enviar y que la DIAN valide **sin errores**:

- 2 facturas electrónicas de venta,
- 1 nota crédito,
- 1 nota débito.

Se firman con el **certificado digital** del facturador. Los documentos llevan
información ficticia: se está probando el software, no facturando.

Este sistema ya **genera y guarda**, para cada uno de esos cuatro documentos,
todo lo que el anexo técnico exige:

| Requisito | Estado |
|---|---|
| Numeración consecutiva autorizada con prefijo | ✔ |
| Fecha **y hora** de generación con desplazamiento horario | ✔ |
| Cantidad, **unidad de medida** y descripción por ítem | ✔ |
| Código de producto con **estándar declarado** (GTIN / UNSPSC / código propio) | ✔ |
| Forma de pago y **medio de pago** cuando es de contado | ✔ |
| **IVA e INC discriminados** por línea y en cabecera | ✔ |
| **CUFE** de la factura (SHA-384, num. 11.2) | ✔ |
| **CUDE** de las notas (SHA-384 con PIN, num. 11.4) | ✔ |
| Notas crédito y débito sobre factura, con su concepto | ✔ |

**El CUFE y el CUDE están probados contra los ejemplos oficiales publicados en
el anexo 1.9** (`el_ejemplo_oficial_del_anexo_1_9`,
`el_ejemplo_oficial_de_cude_para_nota_credito`,
`el_ejemplo_oficial_de_cude_para_documento_de_transmision`). No es «debería
coincidir»: coincide con el resultado que publica la DIAN.

### Fase 3 — Habilitación

1. Entrar al portal **Facturando Electrónicamente** con el token.
2. Elegir la **fecha de inicio** de facturación electrónica.
3. En **MUISCA**, solicitar los rangos de numeración de producción y el
   **talonario de contingencia**.
4. Asociar los rangos en el portal.
5. En la fecha elegida, **el RUT se actualiza solo** con la responsabilidad de
   facturador electrónico.

Al recibir los rangos de producción, en este sistema:

- **Configuración → Resoluciones**: registrar el rango con su número, prefijo,
  vigencia (desde/hasta) y **clave técnica**.
- **Configuración → Empresa**: cambiar el ambiente de `pruebas` a
  **`produccion`**.

El diagnóstico de alistamiento avisa mientras el ambiente siga en pruebas: los
documentos que se emitan ahí **no tienen validez fiscal**.

---

## 3. Lo que este sistema NO hace todavía

Hay que decirlo claro, porque es lo que separa «listo para certificar» de
«certificado»:

| Falta | Por qué no está |
|---|---|
| Generar el **XML UBL 2.1** con las extensiones DIAN | Se puede escribir, pero solo se valida de verdad contra el servicio de la DIAN |
| **Firmar** con XAdES-BES | Necesita el **certificado digital del cliente**, que solo él tiene |
| **Transmitir** al servicio de validación previa | Necesita las credenciales de habilitación del cliente |
| **Código QR** y representación gráfica impresa | Depende del CUFE, que ya está; es trabajo de presentación |
| **Documento equivalente POS** electrónico | Mismo camino que la factura |

**Ninguno de esos cuatro se puede probar sin credenciales de un facturador
real.** Escribirlos «a ciegas» y darlos por terminados sería exactamente lo que
la directiva 28 prohíbe: una pieza que cruza la frontera del programa y que
nunca se ejerció contra lo real.

Por eso el sistema queda con **todos los datos que el anexo exige ya calculados
y guardados**. Cuando aparezca el primer cliente y tenga su certificado, el
trabajo pendiente es **conectar, no rehacer**: tomar los datos que ya están,
serializarlos a UBL, firmar y transmitir.

---

## 4. Qué necesita el cliente antes de empezar

1. **RUT** con la actividad económica correspondiente.
2. **Certificado digital de firma** vigente, emitido por una entidad de
   certificación autorizada en Colombia. Es de él, no nuestro.
3. **Acceso al portal de la DIAN** con su instrumento de firma electrónica.
4. Decidir la **fecha de inicio** de facturación electrónica.

Lo que ponemos nosotros: el software, su instalación, la configuración y el
acompañamiento en el set de pruebas.

---

## 5. Mientras tanto: qué se puede hacer hoy

Un cliente puede **operar desde el primer día** sin estar habilitado:

- inventario, kardex y costeo completos,
- facturación con numeración autorizada y control de resoluciones,
- cartera y recaudo,
- deterioro, artículo 64 e IVA por retiros,
- informes de gestión.

El diagnóstico de alistamiento separa esas dos cosas a propósito: **«se puede
operar»** y **«factura electrónica lista»** son dos semáforos distintos. Un
negocio puede arrancar con el primero en verde mientras termina la habilitación.
