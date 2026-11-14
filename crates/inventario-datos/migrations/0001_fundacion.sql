-- SPDX-FileCopyrightText: 2026 Juan Carlos Isaza
-- SPDX-License-Identifier: LicenseRef-Propietario
--
-- Fundación del esquema.
--
-- Dos convenciones que valen para todo el archivo:
--
--   * El DINERO es INTEGER en CENTAVOS. Las CANTIDADES son INTEGER en
--     MILÉSIMAS de unidad. Ni un REAL en toda la base: el sistema que se
--     reemplaza guardaba N(11,2) y por eso 369 de sus facturas no cuadraban
--     con su propio detalle.
--   * Las FECHAS son TEXT ISO-8601 ('2026-08-02'). Ordenan bien como texto y
--     se leen sin herramientas.
--
-- POR QUÉ ESTE ARCHIVO SIGUE CRECIENDO EN VEZ DE APARECER UN 0002:
-- el producto NO tiene instalaciones. Mientras no exista una base en casa de un
-- cliente, consolidar en la fundación deja un esquema que se lee de una sentada;
-- un 0002 que reconstruye tablas para cambiar un CHECK sería ceremonia, y además
-- en SQLite obliga a apagar las claves foráneas, cosa que no se puede hacer
-- dentro de la transacción en la que corren las migraciones.
-- EL DÍA QUE HAYA LA PRIMERA INSTALACIÓN, ESTE ARCHIVO SE CONGELA y todo cambio
-- pasa a ser un archivo nuevo. Ese día es el que hay que no dejar pasar.

-- ---------------------------------------------------------------------------
-- Parámetros de la instalación. Una sola fila, y el CHECK lo garantiza.
-- ---------------------------------------------------------------------------
CREATE TABLE empresa (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    nombre        TEXT    NOT NULL,
    nit           TEXT    NOT NULL,
    dv            INTEGER NOT NULL,
    direccion     TEXT    NOT NULL DEFAULT '',
    telefono      TEXT    NOT NULL DEFAULT '',
    -- Si el negocio admite sacar más de lo que hay. Apagado por defecto: el
    -- sistema viejo tenía un artículo en -3 unidades, que es una venta que
    -- nadie pudo entregar. Se enciende a propósito, no por descuido.
    permite_negativo INTEGER NOT NULL DEFAULT 0 CHECK (permite_negativo IN (0, 1)),
    -- Fórmula de cálculo del costo. NIC 2 párrafo 25 admite EXACTAMENTE estas
    -- dos; UEPS/LIFO está prohibido y el CHECK lo hace imposible de guardar.
    -- Es política de la entidad —tiene que ser la misma para inventarios de
    -- naturaleza similar, y hay que revelarla—, no una opción por movimiento.
    formula_costeo TEXT NOT NULL DEFAULT 'promedio_ponderado'
        CHECK (formula_costeo IN ('promedio_ponderado', 'peps')),
    -- Ambiente de la DIAN: 1 producción, 2 pruebas. Entra en el CUFE, así que
    -- equivocarlo produce un código que la DIAN rechaza.
    ambiente_dian TEXT NOT NULL DEFAULT 'pruebas'
        CHECK (ambiente_dian IN ('produccion', 'pruebas')),
    -- PIN del software, que la DIAN asigna al registrar el software propio.
    -- Es al CUDE lo que la clave técnica es al CUFE: sin él no se pueden
    -- emitir notas crédito ni débito electrónicas.
    pin_software TEXT,
    -- Identificador del software en el catálogo de participantes de la DIAN.
    id_software  TEXT,
    creado_en     TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE usuario (
    id          INTEGER PRIMARY KEY,
    codigo      TEXT    NOT NULL UNIQUE,
    nombre      TEXT    NOT NULL,
    hash_clave  TEXT    NOT NULL,
    -- 'admin' manda; 'cajero' factura y recauda; 'bodega' mueve inventario;
    -- 'consulta' solo mira. El CHECK es la lista cerrada a propósito: un rol
    -- desconocido no puede resolverse a «el más permisivo».
    rol         TEXT    NOT NULL CHECK (rol IN ('admin', 'cajero', 'bodega', 'consulta')),
    activo      INTEGER NOT NULL DEFAULT 1 CHECK (activo IN (0, 1)),
    creado_en   TEXT    NOT NULL DEFAULT (datetime('now'))
);

-- ---------------------------------------------------------------------------
-- Geografía y fuerza de ventas. Del corpus: ciudades(129), zonas(19),
-- barrios(61), vendedor(17).
-- ---------------------------------------------------------------------------
CREATE TABLE ciudad (
    id     INTEGER PRIMARY KEY,
    codigo TEXT NOT NULL UNIQUE,
    nombre TEXT NOT NULL
);

CREATE TABLE zona (
    id        INTEGER PRIMARY KEY,
    codigo    TEXT NOT NULL UNIQUE,
    nombre    TEXT NOT NULL,
    ciudad_id INTEGER REFERENCES ciudad (id) ON DELETE SET NULL
);

CREATE TABLE barrio (
    id     INTEGER PRIMARY KEY,
    codigo TEXT NOT NULL UNIQUE,
    nombre TEXT NOT NULL
);

CREATE TABLE vendedor (
    id     INTEGER PRIMARY KEY,
    codigo TEXT    NOT NULL UNIQUE,
    nombre TEXT    NOT NULL,
    cedula TEXT    NOT NULL DEFAULT '',
    activo INTEGER NOT NULL DEFAULT 1 CHECK (activo IN (0, 1))
);

-- ---------------------------------------------------------------------------
-- Catálogo.
-- ---------------------------------------------------------------------------
CREATE TABLE categoria (
    id     INTEGER PRIMARY KEY,
    codigo TEXT    NOT NULL UNIQUE,
    nombre TEXT    NOT NULL,
    activa INTEGER NOT NULL DEFAULT 1 CHECK (activa IN (0, 1))
);

CREATE TABLE articulo (
    id             INTEGER PRIMARY KEY,
    codigo         TEXT    NOT NULL UNIQUE,
    -- El código de barras NO es único con UNIQUE a secas: 85 de los 148
    -- artículos del corpus lo tienen vacío, y en SQLite varios NULL conviven
    -- en un índice único pero varias cadenas vacías no. Se guarda NULL cuando
    -- no hay, y el índice parcial de más abajo lo hace único solo cuando existe.
    codigo_barras  TEXT,
    descripcion    TEXT    NOT NULL,
    presentacion   TEXT    NOT NULL DEFAULT '',
    categoria_id   INTEGER REFERENCES categoria (id) ON DELETE SET NULL,
    -- Tarifa entera: 0, 5, 19… y el 16 que traía el corpus de 2004. Se
    -- rechaza lo imposible (fuera de 0..100), no lo inusual.
    iva_porcentaje INTEGER NOT NULL DEFAULT 0 CHECK (iva_porcentaje BETWEEN 0 AND 100),
    precio_venta   INTEGER NOT NULL DEFAULT 0 CHECK (precio_venta >= 0),
    punto_minimo   INTEGER NOT NULL DEFAULT 0 CHECK (punto_minimo >= 0),
    punto_maximo   INTEGER NOT NULL DEFAULT 0 CHECK (punto_maximo >= 0),
    -- Impuesto nacional al consumo. Es un impuesto DISTINTO del IVA y la
    -- factura electrónica exige discriminarlos por separado (Res. 000165/2023
    -- art. 11). Sumarlo al IVA descuadra la declaración de los dos.
    inc_porcentaje INTEGER NOT NULL DEFAULT 0 CHECK (inc_porcentaje BETWEEN 0 AND 100),
    -- Unidad de medida del ítem, obligatoria en la factura electrónica.
    -- NULL = no declarada, y así se dice: no se inventa un 'unidad' por
    -- defecto que después viaje a la DIAN como si alguien lo hubiera decidido.
    unidad_medida  TEXT,
    -- Código del producto bajo un estándar declarado (@schemeID del anexo):
    -- 'unspsc' = 001, 'gtin' = 010, 'interno' = 999. El GTIN solo se acepta si
    -- pasa su dígito de control; si no, se guarda como 'interno'.
    codigo_estandar TEXT,
    tipo_estandar  TEXT CHECK (tipo_estandar IN ('unspsc', 'gtin', 'interno')),
    -- Datos para medir el deterioro (NIC 2.28-33). NULL = no estimado, que es
    -- distinto de cero.
    precio_venta_estimado INTEGER CHECK (precio_venta_estimado IS NULL OR precio_venta_estimado >= 0),
    costos_para_vender    INTEGER CHECK (costos_para_vender IS NULL OR costos_para_vender >= 0),
    activo         INTEGER NOT NULL DEFAULT 1 CHECK (activo IN (0, 1)),
    creado_en      TEXT    NOT NULL DEFAULT (datetime('now')),
    CHECK (punto_maximo = 0 OR punto_maximo >= punto_minimo),
    CHECK ((codigo_estandar IS NULL) = (tipo_estandar IS NULL))
);
CREATE UNIQUE INDEX articulo_barras ON articulo (codigo_barras)
    WHERE codigo_barras IS NOT NULL;
CREATE INDEX articulo_descripcion ON articulo (descripcion);

-- Listas de precio. En el corpus: convenios(9) x valorconvenio(261 vivos).
CREATE TABLE convenio (
    id     INTEGER PRIMARY KEY,
    codigo TEXT    NOT NULL UNIQUE,
    nombre TEXT    NOT NULL,
    activo INTEGER NOT NULL DEFAULT 1 CHECK (activo IN (0, 1))
);

CREATE TABLE precio_convenio (
    convenio_id INTEGER NOT NULL REFERENCES convenio (id) ON DELETE CASCADE,
    articulo_id INTEGER NOT NULL REFERENCES articulo (id) ON DELETE CASCADE,
    precio      INTEGER NOT NULL CHECK (precio >= 0),
    PRIMARY KEY (convenio_id, articulo_id)
) WITHOUT ROWID;

-- ---------------------------------------------------------------------------
-- Terceros. Clientes y proveedores en una sola tabla: el corpus los tenía
-- separados y repetía dirección y teléfono del mismo negocio en las dos.
-- ---------------------------------------------------------------------------
CREATE TABLE tercero (
    id          INTEGER PRIMARY KEY,
    -- NIT normalizado (solo dígitos). El DV se calcula, no se digita.
    nit         TEXT    NOT NULL,
    dv          INTEGER NOT NULL CHECK (dv BETWEEN 0 AND 9),
    nombre      TEXT    NOT NULL,
    es_cliente   INTEGER NOT NULL DEFAULT 1 CHECK (es_cliente IN (0, 1)),
    es_proveedor INTEGER NOT NULL DEFAULT 0 CHECK (es_proveedor IN (0, 1)),
    direccion   TEXT    NOT NULL DEFAULT '',
    telefono    TEXT    NOT NULL DEFAULT '',
    telefono2   TEXT    NOT NULL DEFAULT '',
    correo      TEXT    NOT NULL DEFAULT '',
    ciudad_id   INTEGER REFERENCES ciudad (id) ON DELETE SET NULL,
    zona_id     INTEGER REFERENCES zona (id) ON DELETE SET NULL,
    barrio_id   INTEGER REFERENCES barrio (id) ON DELETE SET NULL,
    vendedor_id INTEGER REFERENCES vendedor (id) ON DELETE SET NULL,
    convenio_id INTEGER REFERENCES convenio (id) ON DELETE SET NULL,
    -- Días de plazo. El corpus guardaba esto en C(3) y tenía '0.', '' y '02'
    -- entre los valores: 239 filas que no eran un número.
    plazo_dias  INTEGER NOT NULL DEFAULT 0 CHECK (plazo_dias >= 0),
    activo      INTEGER NOT NULL DEFAULT 1 CHECK (activo IN (0, 1)),
    creado_en   TEXT    NOT NULL DEFAULT (datetime('now')),
    CHECK (es_cliente = 1 OR es_proveedor = 1)
);
-- El NIT se repetía 47 veces en el corpus y no se puede hacer único sin
-- perder datos al migrar. Se indexa para buscarlo y el duplicado se reporta,
-- no se prohíbe: dos sedes de la misma empresa con el mismo NIT existen.
CREATE INDEX tercero_nit ON tercero (nit);
CREATE INDEX tercero_nombre ON tercero (nombre);

-- ---------------------------------------------------------------------------
-- Kardex.
-- ---------------------------------------------------------------------------
CREATE TABLE concepto (
    id       INTEGER PRIMARY KEY,
    codigo   TEXT    NOT NULL UNIQUE,
    nombre   TEXT    NOT NULL,
    -- Sin tercer valor posible. En el corpus 3 movimientos tenían el concepto
    -- vacío; aquí eso no se puede guardar.
    signo    TEXT    NOT NULL CHECK (signo IN ('entrada', 'salida')),
    -- Para qué sirve. 'venta' y 'devolucion_venta' los usa la facturación y no
    -- se digitan a mano.
    clase    TEXT    NOT NULL CHECK (clase IN (
                 'compra', 'venta', 'devolucion_venta', 'devolucion_compra',
                 'ajuste', 'traslado', 'inicial', 'consumo')),
    exige_tercero INTEGER NOT NULL DEFAULT 0 CHECK (exige_tercero IN (0, 1)),
    -- QUÉ IMPUESTO DISPARA ESTE CONCEPTO. Es la columna que le faltaba al
    -- sistema anterior y por la que 161 de sus movimientos —obsequios, consumo
    -- interno, retiro de socios— salieron sin liquidar un peso de IVA.
    --
    --   'retiro_gravado'      ET art. 421 lit. b): causa IVA sobre el VALOR
    --                         COMERCIAL, no sobre el costo.
    --   'faltante_art64'      consume el tope del 3 % del art. 64.
    --   'baja_con_acta'       exige acta de destrucción (art. 64).
    --   'venta'               el IVA lo lleva la factura.
    --   'no_afecta_impuestos' traslados y ajustes de conteo.
    --
    -- Sin DEFAULT a propósito: al crear un concepto hay que DECIDIR. Un defecto
    -- aquí dejaría cada obsequio nuevo sin IVA por omisión (directiva 20).
    efecto_tributario TEXT NOT NULL CHECK (efecto_tributario IN (
        'venta', 'retiro_gravado', 'faltante_art64', 'baja_con_acta', 'no_afecta_impuestos')),
    activo   INTEGER NOT NULL DEFAULT 1 CHECK (activo IN (0, 1))
);

-- ---------------------------------------------------------------------------
-- Facturación.
-- ---------------------------------------------------------------------------
CREATE TABLE resolucion (
    id           INTEGER PRIMARY KEY,
    nombre       TEXT    NOT NULL,
    prefijo      TEXT    NOT NULL DEFAULT '',
    numero       TEXT    NOT NULL DEFAULT '',
    fecha        TEXT,
    -- Vigencia de la AUTORIZACIÓN, que la DIAN otorga por 6, 12, 18 o 24 meses
    -- (máximo 2 años). El rango numérico y la vigencia se agotan por SEPARADO:
    -- una resolución puede quedarse sin días teniendo números, y al revés.
    rige_desde   TEXT,
    vence        TEXT,
    -- Clave técnica del rango, la que entrega la DIAN. Entra en el CUFE, así
    -- que sin ella no se puede emitir factura electrónica.
    clave_tecnica TEXT,
    desde        INTEGER NOT NULL,
    hasta        INTEGER NOT NULL,
    -- NULL = no se ha emitido ninguna. Distinto de 0, que en un rango que
    -- empieza en 1 sería un número fuera del rango.
    ultimo_emitido INTEGER,
    activa       INTEGER NOT NULL DEFAULT 1 CHECK (activa IN (0, 1)),
    CHECK (desde <= hasta),
    CHECK (ultimo_emitido IS NULL OR ultimo_emitido BETWEEN desde AND hasta)
);
CREATE UNIQUE INDEX resolucion_prefijo ON resolucion (prefijo, desde);

CREATE TABLE factura (
    id            INTEGER PRIMARY KEY,
    resolucion_id INTEGER NOT NULL REFERENCES resolucion (id) ON DELETE RESTRICT,
    prefijo       TEXT    NOT NULL,
    numero        INTEGER NOT NULL,
    tercero_id    INTEGER NOT NULL REFERENCES tercero (id) ON DELETE RESTRICT,
    vendedor_id   INTEGER REFERENCES vendedor (id) ON DELETE SET NULL,
    fecha         TEXT    NOT NULL,
    -- Hora de generación CON desplazamiento horario ('10:53:10-05:00'). La
    -- exige el art. 11 de la Res. 000165/2023 y entra en el CUFE: sin ella el
    -- código no se puede calcular.
    hora          TEXT    NOT NULL DEFAULT '00:00:00-05:00',
    vence         TEXT    NOT NULL,
    forma_pago    TEXT    NOT NULL CHECK (forma_pago IN ('contado', 'credito')),
    -- Medio de pago, obligatorio cuando es de contado (art. 11).
    medio_pago    TEXT CHECK (medio_pago IS NULL OR medio_pago IN
        ('efectivo', 'cheque', 'tarjeta_credito', 'tarjeta_debito', 'transferencia')),
    -- Anular NO borra: el corpus se «iniciaba» con `dele all` + `pack`, que se
    -- llevaba el histórico por delante.
    estado        TEXT    NOT NULL DEFAULT 'activa' CHECK (estado IN ('activa', 'anulada')),
    subtotal      INTEGER NOT NULL CHECK (subtotal >= 0),
    descuento     INTEGER NOT NULL DEFAULT 0 CHECK (descuento >= 0),
    iva           INTEGER NOT NULL DEFAULT 0 CHECK (iva >= 0),
    -- Impuesto nacional al consumo, separado del IVA porque la DIAN los pide
    -- discriminados y porque tienen bases y declaraciones distintas.
    inc           INTEGER NOT NULL DEFAULT 0 CHECK (inc >= 0),
    retencion     INTEGER NOT NULL DEFAULT 0 CHECK (retencion >= 0),
    total         INTEGER NOT NULL,
    -- CUFE (SHA-384, anexo técnico 1.9 num. 11.2). NULL mientras la
    -- instalación no tenga clave técnica: se dice, no se rellena.
    cufe          TEXT,
    observacion   TEXT    NOT NULL DEFAULT '',
    usuario_id    INTEGER REFERENCES usuario (id) ON DELETE SET NULL,
    anulada_en    TEXT,
    anulada_por   INTEGER REFERENCES usuario (id) ON DELETE SET NULL,
    motivo_anulacion TEXT,
    creado_en     TEXT    NOT NULL DEFAULT (datetime('now')),
    -- LA regla que el sistema viejo no tenía: 15 de sus números estaban
    -- repetidos, y por eso 29 de sus facturas «no cuadraban» (dos facturas
    -- compartían detalle).
    UNIQUE (prefijo, numero),
    -- El total tiene que ser el que dice la aritmética. Es la misma regla que
    -- comprueba el dominio, escrita también aquí: una fila que llegue por
    -- cualquier otro camino tampoco puede descuadrar.
    CHECK (total = subtotal + iva + inc - retencion),
    CHECK (estado = 'activa' OR anulada_en IS NOT NULL)
);
CREATE INDEX factura_tercero ON factura (tercero_id, fecha);
CREATE INDEX factura_fecha ON factura (fecha);

CREATE TABLE factura_linea (
    id             INTEGER PRIMARY KEY,
    factura_id     INTEGER NOT NULL REFERENCES factura (id) ON DELETE CASCADE,
    item           INTEGER NOT NULL,
    articulo_id    INTEGER NOT NULL REFERENCES articulo (id) ON DELETE RESTRICT,
    cantidad       INTEGER NOT NULL CHECK (cantidad > 0),
    precio_unitario INTEGER NOT NULL CHECK (precio_unitario >= 0),
    bruto          INTEGER NOT NULL CHECK (bruto >= 0),
    descuento      INTEGER NOT NULL DEFAULT 0 CHECK (descuento >= 0),
    base           INTEGER NOT NULL CHECK (base >= 0),
    iva_porcentaje INTEGER NOT NULL CHECK (iva_porcentaje BETWEEN 0 AND 100),
    iva            INTEGER NOT NULL CHECK (iva >= 0),
    inc_porcentaje INTEGER NOT NULL DEFAULT 0 CHECK (inc_porcentaje BETWEEN 0 AND 100),
    inc            INTEGER NOT NULL DEFAULT 0 CHECK (inc >= 0),
    -- Se congela la unidad de medida del artículo EN la línea: si mañana
    -- alguien cambia la unidad del artículo, la factura ya emitida tiene que
    -- seguir diciendo lo que dijo.
    unidad_medida  TEXT,
    total          INTEGER NOT NULL,
    -- Costo con el que salió del kardex. Es lo que permite la utilidad por
    -- línea sin recalcular nada más tarde con un promedio que ya cambió.
    costo          INTEGER NOT NULL DEFAULT 0,
    UNIQUE (factura_id, item),
    -- Un artículo no puede repetirse en la misma factura: haría ambiguo el
    -- costeo y descuadraría el detalle contra el kardex.
    UNIQUE (factura_id, articulo_id),
    CHECK (base = bruto - descuento),
    CHECK (total = base + iva + inc)
);

-- ---------------------------------------------------------------------------
-- El kardex propiamente dicho. Va DESPUÉS de `factura` porque la referencia
-- SQLite la resolvería tarde, y una tabla que se crea apuntando a otra que
-- todavía no existe es un error que no aparece hasta el primer INSERT.
-- ---------------------------------------------------------------------------
CREATE TABLE movimiento (
    id            INTEGER PRIMARY KEY,
    fecha         TEXT    NOT NULL,
    concepto_id   INTEGER NOT NULL REFERENCES concepto (id) ON DELETE RESTRICT,
    articulo_id   INTEGER NOT NULL REFERENCES articulo (id) ON DELETE RESTRICT,
    tercero_id    INTEGER REFERENCES tercero (id) ON DELETE SET NULL,
    -- Documento soporte: la factura del proveedor, el número de ajuste…
    documento     TEXT    NOT NULL DEFAULT '',
    -- Siempre positiva; la dirección la pone el signo del concepto.
    cantidad      INTEGER NOT NULL CHECK (cantidad > 0),
    -- Lo que ese movimiento suma o resta al valor del inventario.
    valor         INTEGER NOT NULL,
    -- LA FOTO, inmutable: cómo quedó el artículo justo después. Permite leer
    -- el saldo sin recorrer la historia, y permite AUDITARLA: la prueba
    -- `el_kardex_y_las_fotos_dicen_lo_mismo` recorre la historia y exige que
    -- coincida. El sistema viejo tenía el campo mutable y sin nadie que lo
    -- contrastara, y por eso su existencia era ficción.
    existencia_despues INTEGER NOT NULL,
    valor_despues      INTEGER NOT NULL,
    -- Valor comercial del retiro (ET art. 421 lit. b). NULL cuando el
    -- movimiento no es un retiro gravado. NO se rellena con el costo: usar el
    -- costo como base gravable subdeclara el impuesto en silencio.
    valor_comercial INTEGER CHECK (valor_comercial IS NULL OR valor_comercial >= 0),
    -- IVA que causa el retiro. Cero cuando no lo causa.
    iva_generado  INTEGER NOT NULL DEFAULT 0 CHECK (iva_generado >= 0),
    factura_id    INTEGER REFERENCES factura (id) ON DELETE RESTRICT,
    usuario_id    INTEGER REFERENCES usuario (id) ON DELETE SET NULL,
    -- Movimiento que este reversa (anulación). NULL si es original.
    reversa_de    INTEGER REFERENCES movimiento (id) ON DELETE RESTRICT,
    creado_en     TEXT    NOT NULL DEFAULT (datetime('now'))
);
-- El orden del kardex de un artículo. `id` desempata los del mismo día, y como
-- es monótono creciente basta él para reconstruir la secuencia.
CREATE INDEX movimiento_articulo ON movimiento (articulo_id, id);
CREATE INDEX movimiento_fecha ON movimiento (fecha);
CREATE INDEX movimiento_factura ON movimiento (factura_id) WHERE factura_id IS NOT NULL;
CREATE UNIQUE INDEX movimiento_reversa ON movimiento (reversa_de) WHERE reversa_de IS NOT NULL;

-- ---------------------------------------------------------------------------
-- Cartera.
-- ---------------------------------------------------------------------------
CREATE TABLE banco (
    id     INTEGER PRIMARY KEY,
    codigo TEXT NOT NULL UNIQUE,
    nombre TEXT NOT NULL
);

CREATE TABLE recibo (
    id         INTEGER PRIMARY KEY,
    numero     INTEGER NOT NULL UNIQUE,
    fecha      TEXT    NOT NULL,
    tercero_id INTEGER NOT NULL REFERENCES tercero (id) ON DELETE RESTRICT,
    total      INTEGER NOT NULL CHECK (total > 0),
    estado     TEXT    NOT NULL DEFAULT 'activo' CHECK (estado IN ('activo', 'anulado')),
    observacion TEXT   NOT NULL DEFAULT '',
    usuario_id INTEGER REFERENCES usuario (id) ON DELETE SET NULL,
    creado_en  TEXT    NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX recibo_tercero ON recibo (tercero_id, fecha);

-- ON DELETE CASCADE, y las claves foráneas ENCENDIDAS al abrir la base
-- (SQLite las trae apagadas). El corpus conservaba 313 detalles de recibo
-- cuya cabecera había sido borrada: los abonos existían y el recibo que los
-- explicaba, no.
CREATE TABLE recibo_aplicacion (
    id         INTEGER PRIMARY KEY,
    recibo_id  INTEGER NOT NULL REFERENCES recibo (id) ON DELETE CASCADE,
    factura_id INTEGER NOT NULL REFERENCES factura (id) ON DELETE RESTRICT,
    valor      INTEGER NOT NULL CHECK (valor > 0),
    UNIQUE (recibo_id, factura_id)
);
CREATE INDEX aplicacion_factura ON recibo_aplicacion (factura_id);

CREATE TABLE recibo_pago (
    id         INTEGER PRIMARY KEY,
    recibo_id  INTEGER NOT NULL REFERENCES recibo (id) ON DELETE CASCADE,
    medio      TEXT    NOT NULL CHECK (medio IN
                   ('efectivo', 'cheque', 'tarjeta_credito', 'tarjeta_debito', 'transferencia')),
    banco_id   INTEGER REFERENCES banco (id) ON DELETE SET NULL,
    referencia TEXT    NOT NULL DEFAULT '',
    -- Fecha de vencimiento del cheque posfechado.
    vence      TEXT,
    valor      INTEGER NOT NULL CHECK (valor > 0)
);
CREATE INDEX pago_recibo ON recibo_pago (recibo_id);

-- Notas crédito sobre factura.
-- ---------------------------------------------------------------------------
-- Notas crédito y débito.
--
-- Las dos van en la MISMA tabla con una columna `clase`, y no en dos tablas
-- gemelas: comparten numeración, CUDE, referencia a la factura y ciclo de vida,
-- y lo único que cambia es el signo con el que afectan la cartera. Dos tablas
-- iguales serían dos sitios donde arreglar el mismo error.
--
-- Llevan **CUDE**, no CUFE (anexo técnico 1.9, num. 11.4): el mismo hash con el
-- PIN del software en lugar de la clave técnica.
--
-- El set de pruebas de habilitación de la DIAN para software propio exige
-- 2 facturas + 1 nota crédito + 1 nota débito, así que sin estas dos no se
-- puede certificar.
-- ---------------------------------------------------------------------------
CREATE TABLE nota (
    id         INTEGER PRIMARY KEY,
    -- 'credito' rebaja lo que el cliente debe; 'debito' lo aumenta.
    clase      TEXT    NOT NULL CHECK (clase IN ('credito', 'debito')),
    prefijo    TEXT    NOT NULL DEFAULT '',
    numero     INTEGER NOT NULL,
    fecha      TEXT    NOT NULL,
    hora       TEXT    NOT NULL DEFAULT '00:00:00-05:00',
    factura_id INTEGER NOT NULL REFERENCES factura (id) ON DELETE RESTRICT,
    -- Código de concepto de la DIAN (devolución, anulación, descuento…). Se
    -- guarda como texto porque la tabla la publica la DIAN y cambia sin que
    -- cambie el software.
    concepto_dian TEXT NOT NULL DEFAULT '',
    motivo     TEXT    NOT NULL,
    subtotal   INTEGER NOT NULL CHECK (subtotal >= 0),
    iva        INTEGER NOT NULL DEFAULT 0 CHECK (iva >= 0),
    inc        INTEGER NOT NULL DEFAULT 0 CHECK (inc >= 0),
    total      INTEGER NOT NULL CHECK (total > 0),
    -- Si devuelve mercancía al inventario. Cuando es 1 hay movimientos de
    -- kardex asociados; cuando es 0 es un ajuste de precio y no los hay.
    reintegra_inventario INTEGER NOT NULL DEFAULT 0 CHECK (reintegra_inventario IN (0, 1)),
    cude       TEXT,
    estado     TEXT    NOT NULL DEFAULT 'activa' CHECK (estado IN ('activa', 'anulada')),
    usuario_id INTEGER REFERENCES usuario (id) ON DELETE SET NULL,
    creado_en  TEXT    NOT NULL DEFAULT (datetime('now')),
    UNIQUE (clase, prefijo, numero),
    CHECK (total = subtotal + iva + inc),
    CHECK (length(trim(motivo)) > 0),
    -- Una nota débito NO devuelve mercancía: aumenta lo que se debe.
    CHECK (clase = 'credito' OR reintegra_inventario = 0)
);
CREATE INDEX nota_factura ON nota (factura_id, clase);

CREATE TABLE nota_linea (
    id          INTEGER PRIMARY KEY,
    nota_id     INTEGER NOT NULL REFERENCES nota (id) ON DELETE CASCADE,
    item        INTEGER NOT NULL,
    articulo_id INTEGER NOT NULL REFERENCES articulo (id) ON DELETE RESTRICT,
    cantidad    INTEGER NOT NULL CHECK (cantidad > 0),
    precio_unitario INTEGER NOT NULL CHECK (precio_unitario >= 0),
    base        INTEGER NOT NULL CHECK (base >= 0),
    iva_porcentaje INTEGER NOT NULL DEFAULT 0 CHECK (iva_porcentaje BETWEEN 0 AND 100),
    iva         INTEGER NOT NULL DEFAULT 0 CHECK (iva >= 0),
    inc_porcentaje INTEGER NOT NULL DEFAULT 0 CHECK (inc_porcentaje BETWEEN 0 AND 100),
    inc         INTEGER NOT NULL DEFAULT 0 CHECK (inc >= 0),
    total       INTEGER NOT NULL,
    unidad_medida TEXT,
    UNIQUE (nota_id, item),
    UNIQUE (nota_id, articulo_id),
    CHECK (total = base + iva + inc)
);

-- ---------------------------------------------------------------------------
-- Capas de costo (PEPS).
--
-- Solo se usan cuando `empresa.formula_costeo = 'peps'`. Cada entrada crea una
-- capa; cada salida consume las más viejas primero. La capa guarda de qué
-- movimiento salió, que es lo que permite auditar de qué compra concreta viene
-- cada peso del costo de ventas — algo que el promedio ponderado, por su propia
-- naturaleza, no puede decir.
--
-- Con `formula_costeo = 'promedio_ponderado'` esta tabla queda vacía y el saldo
-- sale de los dos acumuladores del movimiento, como siempre. Las dos fórmulas
-- comparten la MISMA invariante: la existencia y el valor del artículo son los
-- de la última foto del kardex.
-- ---------------------------------------------------------------------------
CREATE TABLE capa_costo (
    id             INTEGER PRIMARY KEY,
    movimiento_id  INTEGER NOT NULL UNIQUE REFERENCES movimiento (id) ON DELETE RESTRICT,
    articulo_id    INTEGER NOT NULL REFERENCES articulo (id) ON DELETE RESTRICT,
    -- Lo que queda SIN consumir. Llega a cero y ahí se queda: no se borra, para
    -- que el kardex pueda reconstruirse.
    cantidad_restante INTEGER NOT NULL CHECK (cantidad_restante >= 0),
    cantidad_inicial  INTEGER NOT NULL CHECK (cantidad_inicial > 0),
    costo_unitario    INTEGER NOT NULL CHECK (costo_unitario >= 0),
    creado_en      TEXT    NOT NULL DEFAULT (datetime('now')),
    CHECK (cantidad_restante <= cantidad_inicial)
);
-- El orden de consumo: por `id`, que es el orden de entrada.
CREATE INDEX capa_pendiente ON capa_costo (articulo_id, id) WHERE cantidad_restante > 0;

-- Qué capa alimentó cada salida. Es el rastro que hace auditable el costo de
-- ventas bajo PEPS.
CREATE TABLE consumo_capa (
    id            INTEGER PRIMARY KEY,
    movimiento_id INTEGER NOT NULL REFERENCES movimiento (id) ON DELETE CASCADE,
    capa_id       INTEGER NOT NULL REFERENCES capa_costo (id) ON DELETE RESTRICT,
    cantidad      INTEGER NOT NULL CHECK (cantidad > 0),
    costo         INTEGER NOT NULL CHECK (costo >= 0),
    UNIQUE (movimiento_id, capa_id)
);

-- ---------------------------------------------------------------------------
-- Deterioro del inventario — NIC 2 párrafos 28 a 33.
--
-- Medición al menor entre costo y valor neto realizable. Cada corte guarda su
-- fila: la norma exige REEVALUAR en cada periodo y revelar por separado las
-- rebajas y las reversiones del periodo, y eso no se puede reconstruir si solo
-- se guarda el saldo actual.
-- ---------------------------------------------------------------------------
CREATE TABLE deterioro (
    id             INTEGER PRIMARY KEY,
    fecha          TEXT    NOT NULL,
    articulo_id    INTEGER NOT NULL REFERENCES articulo (id) ON DELETE RESTRICT,
    -- Foto de la medición, para poder explicarla años después sin recalcular
    -- con precios que ya cambiaron.
    existencia     INTEGER NOT NULL,
    costo          INTEGER NOT NULL CHECK (costo >= 0),
    vnr            INTEGER NOT NULL CHECK (vnr >= 0),
    valor_en_libros INTEGER NOT NULL CHECK (valor_en_libros >= 0),
    -- El deterioro acumulado que debe quedar reconocido a esta fecha.
    deterioro_acumulado INTEGER NOT NULL CHECK (deterioro_acumulado >= 0),
    -- El asiento del periodo: positivo = nueva rebaja (gasto);
    -- negativo = reversión (menor gasto). Se guarda con signo para que la
    -- revelación pueda separar las dos cifras sin compensarlas.
    movimiento     INTEGER NOT NULL,
    usuario_id     INTEGER REFERENCES usuario (id) ON DELETE SET NULL,
    creado_en      TEXT    NOT NULL DEFAULT (datetime('now')),
    -- Una sola medición por artículo y fecha de corte.
    UNIQUE (articulo_id, fecha),
    CHECK (valor_en_libros = costo - deterioro_acumulado)
);
CREATE INDEX deterioro_fecha ON deterioro (fecha);

-- ---------------------------------------------------------------------------
-- Actas de baja — ET art. 64.
--
-- La deducción por obsolescencia solo procede si los bienes se destruyen,
-- reciclan o chatarrizan Y existe documento con cantidad, descripción, costo
-- fiscal unitario y justificación, firmado por el representante legal y los
-- responsables. Las columnas son exactamente esas: si la norma cambia, se ve
-- aquí qué hay que cambiar.
-- ---------------------------------------------------------------------------
CREATE TABLE acta_baja (
    id          INTEGER PRIMARY KEY,
    numero      INTEGER NOT NULL UNIQUE,
    fecha       TEXT    NOT NULL,
    metodo      TEXT    NOT NULL CHECK (metodo IN ('destruccion', 'reciclaje', 'chatarrizacion')),
    representante_legal TEXT NOT NULL,
    -- Nombres de quienes ejecutaron y presenciaron la destrucción, separados
    -- por «;». La firma va en papel o con el mecanismo del cliente; aquí queda
    -- constancia de quiénes son.
    responsables TEXT   NOT NULL,
    -- Solo es deducible lo NO cubierto por indemnización o seguro (art. 64).
    cubierto_por_seguro INTEGER NOT NULL DEFAULT 0 CHECK (cubierto_por_seguro >= 0),
    costo_total INTEGER NOT NULL CHECK (costo_total >= 0),
    deducible   INTEGER NOT NULL CHECK (deducible >= 0),
    usuario_id  INTEGER REFERENCES usuario (id) ON DELETE SET NULL,
    creado_en   TEXT    NOT NULL DEFAULT (datetime('now')),
    CHECK (deducible = costo_total - cubierto_por_seguro),
    CHECK (length(trim(representante_legal)) > 0),
    CHECK (length(trim(responsables)) > 0)
);

CREATE TABLE acta_renglon (
    id          INTEGER PRIMARY KEY,
    acta_id     INTEGER NOT NULL REFERENCES acta_baja (id) ON DELETE CASCADE,
    articulo_id INTEGER NOT NULL REFERENCES articulo (id) ON DELETE RESTRICT,
    descripcion TEXT    NOT NULL,
    cantidad    INTEGER NOT NULL CHECK (cantidad > 0),
    costo_fiscal_unitario INTEGER NOT NULL CHECK (costo_fiscal_unitario >= 0),
    justificacion TEXT  NOT NULL,
    -- El movimiento de kardex que sacó la mercancía. Es lo que impide un acta
    -- que dice que se destruyó algo que nunca salió del inventario.
    movimiento_id INTEGER REFERENCES movimiento (id) ON DELETE RESTRICT,
    CHECK (length(trim(descripcion)) > 0),
    CHECK (length(trim(justificacion)) > 0)
);
CREATE INDEX renglon_acta ON acta_renglon (acta_id);
