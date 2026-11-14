//! Los informes que exige el Estatuto Tributario, sobre datos reales.
//!
//! - **Art. 64**: el tope del 3 % por faltantes y el acta de baja.
//! - **Art. 421 lit. b)**: el IVA que causaron los retiros del periodo.
//!
//! Estos informes no son un adorno: son la diferencia entre un concepto de
//! kardex llamado «salida por vencimiento» y una deducción que se puede
//! sustentar ante la DIAN.

use crate::{Error, Pool, Resultado};
use inventario_dominio::{
    Cantidad, Centavos,
    retiro::{self, Acta, MetodoDestruccion, RenglonActa},
};
use sqlx::Row;

// ---------------------------------------------------------------------------
// Artículo 64 — el tope del 3 %
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct InformeArt64 {
    pub desde: String,
    pub hasta: String,
    /// Unidades del inventario **al inicio** del periodo.
    pub inventario_inicial: i64,
    pub compras: i64,
    pub base: i64,
    pub tope: i64,
    pub consumido: i64,
    pub disponible: i64,
    pub exceso: i64,
    /// En centésimas de punto: 250 = 2,50 %.
    pub porcentaje_consumido: i64,
    pub se_paso: bool,
    /// Qué movimientos consumieron el tope, para poder explicarlos.
    pub detalle: Vec<FilaFaltante>,
    /// El aviso que hay que leer, en una frase.
    pub advertencia: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FilaFaltante {
    pub movimiento_id: i64,
    pub fecha: String,
    pub articulo: String,
    pub concepto: String,
    pub cantidad: i64,
    pub costo: i64,
}

/// Calcula el tope del art. 64 para un periodo.
///
/// La base son **unidades**, como dice la norma: «las unidades del inventario
/// final […] de la suma del inventario inicial más las compras». Se calcula
/// sobre el kardex entero, no sobre un artículo.
pub async fn art64(pool: &Pool, desde: &str, hasta: &str) -> Resultado<InformeArt64> {
    for f in [desde, hasta] {
        chrono::NaiveDate::parse_from_str(f, "%Y-%m-%d")
            .map_err(|_| Error::Regla(format!("fecha inválida: {f:?}")))?;
    }
    if desde > hasta {
        return Err(Error::Regla(format!(
            "el periodo está al revés: {desde} .. {hasta}"
        )));
    }

    // Inventario inicial en unidades: todo lo que se movió ANTES del periodo.
    let (inicial,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(CASE WHEN c.signo = 'entrada' THEN m.cantidad ELSE -m.cantidad END), 0)
           FROM movimiento m JOIN concepto c ON c.id = m.concepto_id
          WHERE m.fecha < ?",
    )
    .bind(desde)
    .fetch_one(pool)
    .await?;

    // Compras del periodo: solo los conceptos de clase compra e inicial. Un
    // ajuste de entrada NO es una compra, y meterlo inflaría la base y con ella
    // el tope — a favor del contribuyente y en contra de la norma.
    let (compras,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(m.cantidad), 0)
           FROM movimiento m JOIN concepto c ON c.id = m.concepto_id
          WHERE m.fecha BETWEEN ? AND ? AND c.clase IN ('compra', 'inicial')",
    )
    .bind(desde)
    .bind(hasta)
    .fetch_one(pool)
    .await?;

    let filas = sqlx::query(
        "SELECT m.id, m.fecha, a.descripcion AS articulo, c.nombre AS concepto,
                m.cantidad, m.valor
           FROM movimiento m
           JOIN concepto c ON c.id = m.concepto_id
           JOIN articulo a ON a.id = m.articulo_id
          WHERE m.fecha BETWEEN ? AND ? AND c.efecto_tributario = 'faltante_art64'
          ORDER BY m.fecha, m.id",
    )
    .bind(desde)
    .bind(hasta)
    .fetch_all(pool)
    .await?;

    let detalle: Vec<FilaFaltante> = filas
        .into_iter()
        .map(|f| FilaFaltante {
            movimiento_id: f.get("id"),
            fecha: f.get("fecha"),
            articulo: f.get("articulo"),
            concepto: f.get("concepto"),
            cantidad: f.get("cantidad"),
            costo: f.get("valor"),
        })
        .collect();
    // SUMA COMPROBADA, y es la que más pesa de las tres: este número es el
    // `consumido` del informe del artículo 64, del que sale el EXCESO que se
    // declara ante la DIAN. Un `.sum()` crudo sobre `i64` desborda, y en este
    // árbol eso es PÁNICO en los dos perfiles —`Cargo.toml` lleva
    // `overflow-checks = true` en release desde el 2026-08-12—, o sea un 500 en
    // la cara de quien pide el informe. Se rechaza con un mensaje.
    //
    // Sobrevivió a la primera pasada de este arreglo —se cerraron los de
    // `recibo.rs` y no éste— y lo señaló la revisión de seguridad del
    // 2026-08-13. Alcanzable por un usuario con `MoverInventario` que registre
    // cantidades absurdas a costo cero: ahí `multiplicar` devuelve `Some(0)` y
    // no rechaza nada, así que el importe no protege a la cantidad.
    let mut consumido: i64 = 0;
    for d in &detalle {
        consumido = consumido.checked_add(d.cantidad).ok_or_else(|| {
            Error::Regla(
                "las cantidades consumidas del periodo suman un número que no cabe; \
                 el informe del art. 64 no se puede calcular con datos así"
                    .into(),
            )
        })?;
    }

    let t = retiro::tope_art64(
        Cantidad(inicial),
        Cantidad(compras),
        Cantidad(consumido),
    );

    // El porcentaje consumido en centésimas. `None` solo si no cabe en `i64`,
    // que exige una base ridícula frente a lo consumido; se lee UNA vez y se
    // reutiliza, en vez de llamar tres veces a la función como estaba.
    //
    // El respaldo es 10.000 = 100 %: si el porcentaje no se puede calcular, lo
    // que NO se puede hacer es dar por bueno un tope que no se ha medido. Con
    // 100 % la advertencia sale y el informe dice que se pasó, que es la
    // lectura conservadora para un dato que va a una declaración de renta.
    let consumido_centesimas = t.porcentaje_consumido_centesimas().unwrap_or(10_000);

    let advertencia = if t.se_paso() {
        Some(format!(
            "El periodo consumió el {:.2} % por faltantes, y el artículo 64 del Estatuto \
             Tributario admite hasta el 3 %. El exceso ({} unidades) solo es deducible si se \
             demuestra fuerza mayor o caso fortuito.",
            consumido_centesimas as f64 / 100.0,
            Cantidad(t.exceso.0)
        ))
    } else if t.base.0 > 0 && consumido_centesimas >= 250 {
        Some(format!(
            "Va en el {:.2} % del tope del 3 % (art. 64). Queda poco margen.",
            consumido_centesimas as f64 / 100.0
        ))
    } else {
        None
    };

    Ok(InformeArt64 {
        desde: desde.into(),
        hasta: hasta.into(),
        inventario_inicial: t.inventario_inicial.0,
        compras: t.compras.0,
        base: t.base.0,
        tope: t.tope.0,
        consumido: t.consumido.0,
        disponible: t.disponible.0,
        exceso: t.exceso.0,
        porcentaje_consumido: consumido_centesimas,
        se_paso: t.se_paso(),
        detalle,
        advertencia,
    })
}

// ---------------------------------------------------------------------------
// Artículo 64 — el acta de baja
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PeticionActa {
    pub fecha: String,
    pub metodo: String,
    pub representante_legal: String,
    pub responsables: Vec<String>,
    #[serde(default)]
    pub cubierto_por_seguro: i64,
    pub renglones: Vec<PeticionRenglon>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PeticionRenglon {
    pub articulo_id: i64,
    pub cantidad: i64,
    pub justificacion: String,
    /// El movimiento de kardex que sacó la mercancía. **Obligatorio**: un acta
    /// que diga que se destruyó algo que nunca salió del inventario no
    /// sustenta nada.
    pub movimiento_id: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ActaGuardada {
    pub id: i64,
    pub numero: i64,
    pub costo_total: i64,
    pub cubierto_por_seguro: i64,
    pub deducible: i64,
}

/// Una baja por obsolescencia que **todavía no tiene acta**, o que la tiene
/// solo por parte de sus unidades.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BajaPendiente {
    pub movimiento_id: i64,
    pub fecha: String,
    pub articulo_id: i64,
    pub articulo: String,
    /// Lo que sacó el movimiento.
    pub cantidad: i64,
    /// Lo que ya cubre un acta. Se resta: un acta puede cubrir parte.
    pub con_acta: i64,
    /// `cantidad - con_acta`. Es lo que falta por sustentar.
    pub pendiente: i64,
    pub costo: i64,
    /// Costo fiscal unitario, derivado del kardex igual que en `crear_acta`.
    /// Se enseña para que quien levanta el acta vea lo que va a quedar escrito
    /// **antes** de firmarla, no después.
    pub costo_fiscal_unitario: i64,
}

/// Las bajas por obsolescencia del periodo que aún no están sustentadas.
///
/// Existe porque sin ella el acta era un documento que solo se podía levantar
/// adivinando el `movimiento_id`: `crear_acta` exige el movimiento que sacó la
/// mercancía —bien exigido, un acta sobre una salida que no ocurrió no sustenta
/// nada— y nada decía cuáles eran. La deducción del art. 64 se cae por no
/// tener el documento, no por no tener la salida.
///
/// **Cuenta lo pendiente, no la existencia de un renglón**: un acta puede
/// cubrir parte de un movimiento, y dar por sustentado el resto sería
/// exactamente el silencio que la norma castiga.
pub async fn bajas_sin_acta(pool: &Pool, desde: &str, hasta: &str) -> Resultado<Vec<BajaPendiente>> {
    for f in [desde, hasta] {
        chrono::NaiveDate::parse_from_str(f, "%Y-%m-%d")
            .map_err(|_| Error::Regla(format!("fecha inválida: {f:?}")))?;
    }
    let filas = sqlx::query(
        "SELECT m.id, m.fecha, m.articulo_id, a.descripcion, m.cantidad, m.valor,
                COALESCE((SELECT SUM(r.cantidad) FROM acta_renglon r
                           WHERE r.movimiento_id = m.id), 0) AS con_acta
           FROM movimiento m
           JOIN concepto c ON c.id = m.concepto_id
           JOIN articulo a ON a.id = m.articulo_id
          WHERE m.fecha BETWEEN ? AND ?
            AND c.efecto_tributario = 'baja_con_acta'
          ORDER BY m.fecha, m.id",
    )
    .bind(desde)
    .bind(hasta)
    .fetch_all(pool)
    .await?;

    // BUCLE Y NO `filter_map`, desde el 2026-08-13. El cierre devuelve `Option`,
    // así que un costo unitario que no cabe solo se podía resolver con un
    // relleno —había un `unwrap_or(0)`— o tirando la fila en silencio, y las dos
    // son la directiva 30: una baja que desaparece de la lista de pendientes es
    // una baja que nadie va a meter en el acta, y nadie se entera.
    let mut salida = Vec::with_capacity(filas.len());
    for f in filas {
        let cantidad: i64 = f.get("cantidad");
        let con_acta: i64 = f.get("con_acta");
        let pendiente = cantidad - con_acta;
        if pendiente <= 0 {
            continue;
        }
        let valor: i64 = f.get("valor");
        // Este unitario acaba en el acta del art. 64, que se presenta ante la
        // DIAN: un cero de relleno ahí es un costo fiscal falso. La premisa que
        // justificaba el cero —«cabe por construcción, la cantidad es ≥ 1000
        // milésimas»— es falsa: el esquema solo exige `cantidad > 0`, y una
        // venta al peso de 1 milésima multiplica el valor por mil.
        let unitario = if cantidad == 0 {
            0
        } else {
            inventario_dominio::dinero::dividir_redondeando(
                valor as i128 * inventario_dominio::dinero::MIL as i128,
                cantidad as i128,
            )
            .ok_or_else(|| {
                Error::Regla(format!(
                    "el costo fiscal unitario del movimiento {} no cabe en centavos \
                     (valor {valor}, cantidad {cantidad}); no entra en el acta",
                    f.get::<i64, _>("id")
                ))
            })?
        };
        salida.push(BajaPendiente {
            movimiento_id: f.get("id"),
            fecha: f.get("fecha"),
            articulo_id: f.get("articulo_id"),
            articulo: f.get("descripcion"),
            cantidad,
            con_acta,
            pendiente,
            costo: valor,
            costo_fiscal_unitario: unitario,
        });
    }
    Ok(salida)
}

/// Registra un acta de baja del art. 64.
///
/// La descripción y el **costo fiscal unitario** no se digitan: se toman del
/// artículo y del movimiento que sacó la mercancía. Digitarlos permitiría un
/// acta que dice un costo y un kardex que dice otro.
pub async fn crear_acta(
    pool: &Pool,
    p: &PeticionActa,
    usuario_id: Option<i64>,
) -> Resultado<ActaGuardada> {
    chrono::NaiveDate::parse_from_str(&p.fecha, "%Y-%m-%d")
        .map_err(|_| Error::Regla(format!("fecha inválida: {:?}", p.fecha)))?;
    let metodo = MetodoDestruccion::desde_texto(&p.metodo).ok_or_else(|| {
        Error::Regla(format!(
            "método de destrucción {:?} desconocido: el art. 64 admite destrucción, \
             reciclaje o chatarrización",
            p.metodo
        ))
    })?;
    if p.renglones.is_empty() {
        return Err(Error::Regla("el acta necesita al menos un renglón".into()));
    }

    let mut renglones = Vec::new();
    for r in &p.renglones {
        let m = sqlx::query(
            "SELECT m.cantidad, m.valor, a.descripcion, c.efecto_tributario
               FROM movimiento m
               JOIN articulo a ON a.id = m.articulo_id
               JOIN concepto c ON c.id = m.concepto_id
              WHERE m.id = ? AND m.articulo_id = ?",
        )
        .bind(r.movimiento_id)
        .bind(r.articulo_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| {
            Error::Regla(format!(
                "el movimiento {} no existe o no es del artículo {}: no se puede levantar \
                 un acta sobre una salida que no ocurrió",
                r.movimiento_id, r.articulo_id
            ))
        })?;

        // El movimiento tiene que ser de los que exigen acta. Levantar un acta
        // sobre una venta o un traslado sería sustentar una deducción que no
        // existe.
        let efecto: String = m.get("efecto_tributario");
        if efecto != "baja_con_acta" {
            return Err(Error::Regla(format!(
                "el movimiento {} es de efecto «{efecto}»; el acta del art. 64 solo aplica a \
                 movimientos de baja por obsolescencia («baja_con_acta»)",
                r.movimiento_id
            )));
        }
        let cantidad_mov: i64 = m.get("cantidad");
        if r.cantidad > cantidad_mov {
            return Err(Error::Regla(format!(
                "el acta pretende dar de baja {} unidades y el movimiento {} solo sacó {}",
                Cantidad(r.cantidad),
                r.movimiento_id,
                Cantidad(cantidad_mov)
            )));
        }
        // Costo fiscal UNITARIO, tal como lo pide la norma, derivado del costo
        // con que salió del kardex.
        let valor: i64 = m.get("valor");
        // Mismo cálculo, mismo destino y misma decisión que en `bajas_sin_acta`:
        // los dos tienen que dar el MISMO número o el acta diría un costo y el
        // kardex otro. Aquí el valor se escribe de verdad en el acta.
        //
        // (Aquí quedaron dos comentarios pegados que se contradecían —el viejo
        // seguía hablando de un «respaldo» que la línea de abajo acababa de
        // quitar— y los dos nombraban `bajas_pendientes`, que es un nombre que
        // esta función no tiene. Lo señaló la revisión del 2026-08-13.)
        let unitario = if cantidad_mov == 0 {
            0
        } else {
            inventario_dominio::dinero::dividir_redondeando(
                valor as i128 * inventario_dominio::dinero::MIL as i128,
                cantidad_mov as i128,
            )
            .ok_or_else(|| {
                Error::Regla(format!(
                    "el costo fiscal unitario del movimiento {} no cabe en centavos \
                     (valor {valor}, cantidad {cantidad_mov}); no entra en el acta",
                    r.movimiento_id
                ))
            })?
        };
        renglones.push((
            r.movimiento_id,
            RenglonActa {
                articulo_id: r.articulo_id,
                descripcion: m.get("descripcion"),
                cantidad: Cantidad(r.cantidad),
                costo_fiscal_unitario: Centavos(unitario),
                justificacion: r.justificacion.clone(),
            },
        ));
    }

    let acta = Acta {
        fecha: p.fecha.clone(),
        metodo,
        renglones: renglones.iter().map(|(_, r)| r.clone()).collect(),
        representante_legal: p.representante_legal.clone(),
        responsables: p.responsables.clone(),
        cubierto_por_seguro: Centavos(p.cubierto_por_seguro),
    };
    let resultado = retiro::validar_acta(&acta).map_err(|e| Error::Regla(e.to_string()))?;

    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let numero: i64 = sqlx::query("SELECT COALESCE(MAX(numero), 0) + 1 AS n FROM acta_baja")
        .fetch_one(&mut *tx)
        .await?
        .get("n");
    let id: i64 = sqlx::query(
        "INSERT INTO acta_baja
            (numero, fecha, metodo, representante_legal, responsables,
             cubierto_por_seguro, costo_total, deducible, usuario_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(numero)
    .bind(&p.fecha)
    .bind(metodo.como_texto())
    .bind(p.representante_legal.trim())
    .bind(p.responsables.join("; "))
    .bind(p.cubierto_por_seguro)
    .bind(resultado.costo_total.0)
    .bind(resultado.deducible.0)
    .bind(usuario_id)
    .fetch_one(&mut *tx)
    .await?
    .get("id");

    for (movimiento_id, r) in &renglones {
        sqlx::query(
            "INSERT INTO acta_renglon
                (acta_id, articulo_id, descripcion, cantidad, costo_fiscal_unitario,
                 justificacion, movimiento_id)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(r.articulo_id)
        .bind(&r.descripcion)
        .bind(r.cantidad.0)
        .bind(r.costo_fiscal_unitario.0)
        .bind(&r.justificacion)
        .bind(movimiento_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    Ok(ActaGuardada {
        id,
        numero,
        costo_total: resultado.costo_total.0,
        cubierto_por_seguro: resultado.cubierto_por_seguro.0,
        deducible: resultado.deducible.0,
    })
}

// ---------------------------------------------------------------------------
// Artículo 421 literal b) — IVA por retiro de inventarios
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct InformeRetiros {
    pub desde: String,
    pub hasta: String,
    pub base_total: i64,
    pub iva_total: i64,
    pub costo_total: i64,
    pub filas: Vec<FilaRetiro>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FilaRetiro {
    pub movimiento_id: i64,
    pub fecha: String,
    pub articulo: String,
    pub concepto: String,
    pub cantidad: i64,
    pub costo: i64,
    pub valor_comercial: i64,
    pub iva: i64,
}

/// El IVA que causaron los retiros del periodo (ET art. 421 lit. b).
///
/// Es el informe que el sistema anterior no podía dar, porque ni siquiera
/// guardaba el dato: sus 161 obsequios, consumos internos y retiros de socios
/// salieron del inventario sin liquidar un peso.
/// Los retiros que causaron IVA en el periodo (ET art. 421 lit. b).
///
/// **El filtro es lo que el MOVIMIENTO registró, no lo que el concepto dice
/// hoy.** Son dos cosas distintas y colapsarlas produce un informe falso
/// (directiva 32): `concepto.efecto_tributario` es la política VIGENTE y se
/// puede corregir —hace falta que se pueda, porque el migrador deja los
/// conceptos heredados en neutro y pide revisión—; `movimiento.valor_comercial`
/// es el hecho ocurrido, y solo se escribe cuando el retiro liquidó.
///
/// Filtrar por el concepto hacía que, al corregir uno, todos sus movimientos
/// ANTERIORES aparecieran de golpe como retiros gravados con base y IVA en
/// cero. El informe crecía en filas sin crecer en impuesto, y esas filas son
/// indistinguibles de un retiro legítimo de artículo exento. Lo cazó
/// `corregir_el_concepto_heredado_hace_que_el_obsequio_empiece_a_liquidar_iva`.
///
/// `valor_comercial IS NOT NULL` y no `iva_generado > 0`: un retiro de artículo
/// con tarifa 0 % es un retiro gravado que liquidó cero, y tiene que salir.
pub async fn retiros_gravados(pool: &Pool, desde: &str, hasta: &str) -> Resultado<InformeRetiros> {
    for f in [desde, hasta] {
        chrono::NaiveDate::parse_from_str(f, "%Y-%m-%d")
            .map_err(|_| Error::Regla(format!("fecha inválida: {f:?}")))?;
    }
    let filas = sqlx::query(
        "SELECT m.id, m.fecha, a.descripcion AS articulo, c.nombre AS concepto,
                m.cantidad, m.valor, COALESCE(m.valor_comercial, 0) AS valor_comercial,
                m.iva_generado
           FROM movimiento m
           JOIN concepto c ON c.id = m.concepto_id
           JOIN articulo a ON a.id = m.articulo_id
          WHERE m.fecha BETWEEN ? AND ?
            AND m.valor_comercial IS NOT NULL
          ORDER BY m.fecha, m.id",
    )
    .bind(desde)
    .bind(hasta)
    .fetch_all(pool)
    .await?;

    let filas: Vec<FilaRetiro> = filas
        .into_iter()
        .map(|f| FilaRetiro {
            movimiento_id: f.get("id"),
            fecha: f.get("fecha"),
            articulo: f.get("articulo"),
            concepto: f.get("concepto"),
            cantidad: f.get("cantidad"),
            costo: f.get("valor"),
            valor_comercial: f.get("valor_comercial"),
            iva: f.get("iva_generado"),
        })
        .collect();

    Ok(InformeRetiros {
        desde: desde.into(),
        hasta: hasta.into(),
        base_total: filas.iter().map(|f| f.valor_comercial).sum(),
        iva_total: filas.iter().map(|f| f.iva).sum(),
        costo_total: filas.iter().map(|f| f.costo).sum(),
        filas,
    })
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::prueba_apoyo;

    /// **LAS GUARDAS DE ESTE ARCHIVO NO LAS ALCANZABA NADIE**, y lo midió la
    /// revisión de seguridad del 2026-08-13 de la única forma que vale:
    /// revirtiendo los `ok_or` a `unwrap_or(0)` y viendo la suite quedarse en
    /// **103/103 verde**. Con su positivo de control —romper el RESULTADO en vez
    /// de la guarda pone 7 pruebas rojas—, o sea que la suite sí ejecuta estas
    /// funciones; lo que no cubría es el rechazo.
    ///
    /// Un `ok_or` que nunca dispara es indistinguible de uno cableado a la
    /// condición equivocada (directiva 33).
    ///
    /// Escribe un movimiento con un `valor` absurdo SALTÁNDOSE `registrar` —que
    /// es justo lo que el defecto original dejaba persistido— para que el costo
    /// fiscal unitario derivado no quepa.
    /// El concepto que `bajas_sin_acta` mira es `baja_con_acta`, no
    /// `faltante_art64` — lo aprendí escribiendo esta prueba mal la primera vez:
    /// salió roja porque la consulta no devolvía NADA, no porque el código
    /// fallara. Un caso rojo que falla por el arnés no mide el código.
    async fn concepto_de_baja(pool: &Pool) -> i64 {
        sqlx::query(
            "INSERT INTO concepto (codigo, nombre, signo, clase, efecto_tributario)
             VALUES ('78', 'Baja con acta', 'salida', 'ajuste', 'baja_con_acta')
             RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
        .get("id")
    }

    async fn movimiento_con_valor_imposible(pool: &Pool, articulo: i64, concepto: i64) -> i64 {
        sqlx::query(
            "INSERT INTO movimiento
               (fecha, articulo_id, concepto_id, cantidad, valor,
                existencia_despues, valor_despues)
             VALUES ('2026-08-01', ?, ?, 1, ?, 0, 0)
             RETURNING id",
        )
        .bind(articulo)
        .bind(concepto)
        // cantidad = 1 milésima, así que el unitario derivado es valor × 1000:
        // con este valor no cabe en i64.
        .bind(i64::MAX / 500)
        .fetch_one(pool)
        .await
        .unwrap()
        .get("id")
    }

    /// La mitad que TIENE que rechazar: un costo fiscal unitario que no cabe no
    /// entra en el acta del art. 64, que se presenta ante la DIAN.
    #[tokio::test]
    async fn una_baja_con_costo_fiscal_imposible_no_se_lista_en_silencio() {
        let pool = prueba_apoyo::base().await;
        let (a, _, _) = montar(&pool).await;
        let baja = concepto_de_baja(&pool).await;
        movimiento_con_valor_imposible(&pool, a, baja).await;

        let e = bajas_sin_acta(&pool, "2026-01-01", "2026-12-31").await;
        assert!(
            matches!(e, Err(Error::Regla(_))),
            "una baja cuyo costo fiscal no cabe se listó como si nada: {e:?}"
        );
    }

    /// LA PAREJA, y es la que impide que un rechazo indiscriminado pase por
    /// arreglo: una baja NORMAL sí se lista, con su costo unitario.
    #[tokio::test]
    async fn una_baja_normal_si_se_lista_con_su_costo() {
        let pool = prueba_apoyo::base().await;
        let (a, _, _) = montar(&pool).await;
        let baja = concepto_de_baja(&pool).await;
        mover(&pool, prueba_apoyo::COMPRA, a, 100, "2026-08-01", None)
            .await
            .unwrap();
        mover(&pool, baja, a, 2, "2026-08-02", None).await.unwrap();

        let bajas = bajas_sin_acta(&pool, "2026-01-01", "2026-12-31")
            .await
            .expect("una baja normal tiene que listarse");
        assert_eq!(bajas.len(), 1, "no se listó la baja");
        assert!(
            bajas[0].costo_fiscal_unitario > 0,
            "la baja normal salió con costo cero"
        );
    }

    async fn montar(pool: &Pool) -> (i64, i64, i64) {
        let a: i64 = sqlx::query(
            "INSERT INTO articulo (codigo, descripcion, iva_porcentaje, precio_venta)
             VALUES ('A1', 'Botellón 20 L', 19, 500000) RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
        .get("id");
        let faltante: i64 = sqlx::query(
            "INSERT INTO concepto (codigo, nombre, signo, clase, efecto_tributario)
             VALUES ('72', 'Salida por vencimiento', 'salida', 'ajuste', 'faltante_art64')
             RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
        .get("id");
        let obsequio: i64 = sqlx::query(
            "INSERT INTO concepto (codigo, nombre, signo, clase, efecto_tributario)
             VALUES ('76', 'Salidas por obsequios', 'salida', 'consumo', 'retiro_gravado')
             RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
        .get("id");
        (a, faltante, obsequio)
    }

    async fn mover(
        pool: &Pool,
        concepto: i64,
        articulo: i64,
        u: i64,
        fecha: &str,
        comercial: Option<i64>,
    ) -> Resultado<i64> {
        crate::kardex::registrar(
            pool,
            &crate::kardex::Peticion {
                fecha: fecha.into(),
                concepto_id: concepto,
                articulo_id: articulo,
                tercero_id: None,
                documento: String::new(),
                cantidad: Cantidad::de_unidades(u),
                costo_unitario: Centavos::de_pesos(3_000),
                factura_id: None,
                usuario_id: None,
                valor_comercial_unitario: comercial.map(Centavos),
            },
        )
        .await
    }

    // -- Art. 421 lit. b) ---------------------------------------------------

    /// LA prueba de la brecha de mayor riesgo: un obsequio **genera IVA**, y
    /// sobre el valor comercial.
    #[tokio::test]
    async fn un_obsequio_liquida_iva_sobre_el_valor_comercial() {
        let pool = prueba_apoyo::base().await;
        let (a, _, obsequio) = montar(&pool).await;
        mover(&pool, prueba_apoyo::COMPRA, a, 100, "2026-08-01", None)
            .await
            .unwrap();

        mover(&pool, obsequio, a, 10, "2026-08-05", Some(Centavos::de_pesos(5_000).0))
            .await
            .unwrap();

        let r = retiros_gravados(&pool, "2026-08-01", "2026-08-31").await.unwrap();
        assert_eq!(r.filas.len(), 1);
        assert_eq!(r.base_total, Centavos::de_pesos(50_000).0, "base = valor comercial");
        assert_eq!(r.iva_total, Centavos::de_pesos(9_500).0, "19 % de 50.000");
        assert_eq!(r.costo_total, Centavos::de_pesos(30_000).0, "costo = 10 × 3.000");
        // Y esto es lo que discrimina: si la base fuera el costo, el IVA sería
        // 5.700. El sistema viejo no liquidaba NADA.
        assert_ne!(r.iva_total, Centavos::de_pesos(5_700).0);
    }

    /// **La revisión que el migrador pide se puede ejecutar de verdad.**
    ///
    /// El migrador deja cada concepto de salida heredado en
    /// `no_afecta_impuestos` y lo marca «REVISAR el efecto tributario». Esta
    /// prueba recorre esa revisión entera por la vía real —no comprobando que
    /// la columna cambió, sino que el IVA empieza a salir—: se retira un
    /// obsequio con el concepto SIN corregir y no se liquida un peso, que es
    /// exactamente el estado del sistema anterior; se corrige el concepto; y el
    /// siguiente retiro sí liquida.
    ///
    /// Los dos retiros son la pareja (directiva 33): con el segundo solo,
    /// pasaría igual un sistema que le liquidara IVA a todo.
    #[tokio::test]
    async fn corregir_el_concepto_heredado_hace_que_el_obsequio_empiece_a_liquidar_iva() {
        let pool = prueba_apoyo::base().await;
        let (a, _, _) = montar(&pool).await;
        mover(&pool, prueba_apoyo::COMPRA, a, 100, "2026-08-01", None)
            .await
            .unwrap();

        // Así lo deja el migrador: el nombre dice obsequio y el efecto dice que
        // no toca impuestos, porque el corpus no traía el dato.
        let heredado: i64 = sqlx::query(
            "INSERT INTO concepto (codigo, nombre, signo, clase, efecto_tributario)
             VALUES ('79', 'SALIDAS POR OBSEQUIOS', 'salida', 'consumo', 'no_afecta_impuestos')
             RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .get("id");

        // ROJO: sin corregir, el obsequio sale del inventario y no liquida nada
        // —ni siquiera hace falta declarar el valor comercial—.
        mover(&pool, heredado, a, 10, "2026-08-05", None).await.unwrap();
        let antes = retiros_gravados(&pool, "2026-08-01", "2026-08-31").await.unwrap();
        assert_eq!(antes.filas.len(), 0, "un concepto neutro no debe aparecer");
        assert_eq!(antes.iva_total, 0);

        // La corrección, y su aviso: ya hay UN movimiento escrito con el
        // concepto viejo. Ese número es el que le dice a quien corrige que
        // llegó tarde; un cero silencioso no distinguiría «no había ninguno».
        let ya = crate::catalogo::actualizar_efecto_tributario(&pool, heredado, "retiro_gravado")
            .await
            .unwrap();
        assert_eq!(ya, 1, "no avisó de los movimientos ya registrados");

        // VERDE: el mismo concepto, ahora, exige el valor comercial…
        let e = mover(&pool, heredado, a, 10, "2026-08-06", None).await.unwrap_err();
        assert!(e.to_string().contains("421"), "el error no cita la norma: {e}");
        // …y liquida el 19 % sobre él.
        mover(&pool, heredado, a, 10, "2026-08-06", Some(Centavos::de_pesos(5_000).0))
            .await
            .unwrap();

        let despues = retiros_gravados(&pool, "2026-08-01", "2026-08-31").await.unwrap();
        assert_eq!(despues.filas.len(), 1, "solo el retiro POSTERIOR a la corrección");
        assert_eq!(despues.base_total, Centavos::de_pesos(50_000).0);
        assert_eq!(despues.iva_total, Centavos::de_pesos(9_500).0);

        // Y esto es lo que la función promete y hay que sostener: corrige hacia
        // adelante. El movimiento del día 5 guardó su impuesto —ninguno— cuando
        // se escribió, y cambiar el concepto no lo reescribe. Quien necesite
        // arreglar el pasado tiene que reversar el movimiento, no editar una
        // tabla por debajo.
        let (con_iva,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM movimiento WHERE fecha = '2026-08-05' AND iva_generado > 0",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(con_iva, 0, "la corrección reescribió el pasado");
    }

    /// Un efecto tributario que no existe **no se guarda**, y el id inventado
    /// tampoco: los dos son la pareja del caso de arriba.
    #[tokio::test]
    async fn la_correccion_rechaza_el_efecto_inventado_y_el_concepto_que_no_esta() {
        let pool = prueba_apoyo::base().await;
        let (_, _, obsequio) = montar(&pool).await;

        let e = crate::catalogo::actualizar_efecto_tributario(&pool, obsequio, "exento_porque_si")
            .await
            .unwrap_err();
        assert!(e.to_string().contains("desconocido"), "{e}");

        let e = crate::catalogo::actualizar_efecto_tributario(&pool, 99_999, "retiro_gravado")
            .await
            .unwrap_err();
        assert!(e.to_string().contains("99999"), "{e}");

        // Y el concepto que sí existe se quedó como estaba.
        let (efecto,): (String,) =
            sqlx::query_as("SELECT efecto_tributario FROM concepto WHERE id = ?")
                .bind(obsequio)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(efecto, "retiro_gravado");
    }

    /// Sin valor comercial el movimiento **no se guarda**. No hay forma de que
    /// un obsequio entre al kardex sin su IVA.
    #[tokio::test]
    async fn un_retiro_sin_valor_comercial_no_entra_al_kardex() {
        let pool = prueba_apoyo::base().await;
        let (a, _, obsequio) = montar(&pool).await;
        mover(&pool, prueba_apoyo::COMPRA, a, 100, "2026-08-01", None)
            .await
            .unwrap();

        let e = mover(&pool, obsequio, a, 10, "2026-08-05", None).await.unwrap_err();
        assert!(e.to_string().contains("421"), "el error no cita la norma: {e}");

        // Y el inventario NO se movió.
        assert_eq!(
            crate::kardex::saldo(&pool, a).await.unwrap().existencia,
            100_000
        );
    }

    // -- Art. 64 ------------------------------------------------------------

    #[tokio::test]
    async fn el_informe_del_articulo_64_mide_el_tres_por_ciento_contra_su_base_real() {
        let pool = prueba_apoyo::base().await;
        let (a, faltante, _) = montar(&pool).await;
        // Compras del periodo: 1.000 unidades.
        mover(&pool, prueba_apoyo::COMPRA, a, 1_000, "2026-08-01", None)
            .await
            .unwrap();
        // Faltantes: 20 unidades = 2 % de la base.
        mover(&pool, faltante, a, 20, "2026-08-10", None).await.unwrap();

        let r = art64(&pool, "2026-08-01", "2026-08-31").await.unwrap();
        assert_eq!(r.inventario_inicial, 0);
        assert_eq!(r.compras, 1_000_000, "1.000 unidades en milésimas");
        assert_eq!(r.tope, 30_000, "3 % de 1.000 = 30 unidades");
        assert_eq!(r.consumido, 20_000);
        assert_eq!(r.disponible, 10_000);
        assert!(!r.se_paso);
        assert_eq!(r.porcentaje_consumido, 200, "2,00 %");
        assert_eq!(r.detalle.len(), 1);
        assert!(r.advertencia.is_none());
    }

    /// El aviso que el sistema viejo no daba.
    #[tokio::test]
    async fn pasarse_del_tope_produce_una_advertencia_que_cita_la_norma() {
        let pool = prueba_apoyo::base().await;
        let (a, faltante, _) = montar(&pool).await;
        mover(&pool, prueba_apoyo::COMPRA, a, 1_000, "2026-08-01", None)
            .await
            .unwrap();
        mover(&pool, faltante, a, 45, "2026-08-10", None).await.unwrap();

        let r = art64(&pool, "2026-08-01", "2026-08-31").await.unwrap();
        assert!(r.se_paso);
        assert_eq!(r.exceso, 15_000);
        let aviso = r.advertencia.expect("tiene que advertir");
        assert!(aviso.contains("64"), "no cita el artículo: {aviso}");
        assert!(aviso.contains("fuerza mayor"), "no dice cómo se salva: {aviso}");
    }

    /// Un ajuste de entrada NO es una compra. Meterlo en la base inflaría el
    /// tope a favor del contribuyente.
    #[tokio::test]
    async fn un_ajuste_de_entrada_no_infla_la_base_del_tope() {
        let pool = prueba_apoyo::base().await;
        let (a, faltante, _) = montar(&pool).await;
        mover(&pool, prueba_apoyo::COMPRA, a, 1_000, "2026-08-01", None)
            .await
            .unwrap();
        // Concepto 3 del sembrado de prueba = «Entradas por ajuste», clase ajuste.
        mover(&pool, 3, a, 10_000, "2026-08-02", None).await.unwrap();
        mover(&pool, faltante, a, 40, "2026-08-10", None).await.unwrap();

        let r = art64(&pool, "2026-08-01", "2026-08-31").await.unwrap();
        assert_eq!(r.compras, 1_000_000, "el ajuste no cuenta como compra");
        assert!(r.se_paso, "con la base correcta, 40 de 1.000 SÍ se pasa del 3 %");
    }

    #[tokio::test]
    async fn el_periodo_al_reves_o_con_fecha_invalida_se_rechaza() {
        let pool = prueba_apoyo::base().await;
        assert!(art64(&pool, "2026-08-31", "2026-08-01").await.is_err());
        assert!(art64(&pool, "ayer", "hoy").await.is_err());
    }

    // -- Acta ---------------------------------------------------------------

    async fn concepto_baja(pool: &Pool) -> i64 {
        sqlx::query(
            "INSERT INTO concepto (codigo, nombre, signo, clase, efecto_tributario)
             VALUES ('BAJ', 'Baja por obsolescencia', 'salida', 'ajuste', 'baja_con_acta')
             RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
        .get("id")
    }

    #[tokio::test]
    async fn el_acta_toma_el_costo_fiscal_del_kardex_y_no_de_quien_la_digita() {
        let pool = prueba_apoyo::base().await;
        let (a, _, _) = montar(&pool).await;
        let baja = concepto_baja(&pool).await;
        mover(&pool, prueba_apoyo::COMPRA, a, 100, "2026-08-01", None)
            .await
            .unwrap();
        let mov = mover(&pool, baja, a, 10, "2026-08-05", None).await.unwrap();

        let r = crear_acta(
            &pool,
            &PeticionActa {
                fecha: "2026-08-06".into(),
                metodo: "destruccion".into(),
                representante_legal: "Juan Pérez".into(),
                responsables: vec!["Ana Gómez".into()],
                cubierto_por_seguro: 0,
                renglones: vec![PeticionRenglon {
                    articulo_id: a,
                    cantidad: Cantidad::de_unidades(10).0,
                    justificacion: "Vencido el 2026-07-31".into(),
                    movimiento_id: mov,
                }],
            },
            None,
        )
        .await
        .unwrap();

        assert_eq!(r.costo_total, Centavos::de_pesos(30_000).0, "10 × $3.000 del kardex");
        assert_eq!(r.deducible, Centavos::de_pesos(30_000).0);

        // Y el renglón guardó el costo fiscal UNITARIO, que es lo que pide la
        // norma.
        let (unitario,): (i64,) =
            sqlx::query_as("SELECT costo_fiscal_unitario FROM acta_renglon LIMIT 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(unitario, Centavos::de_pesos(3_000).0);
    }

    /// No se puede levantar un acta sobre una salida que no ocurrió, ni sobre
    /// una que no es una baja.
    #[tokio::test]
    async fn el_acta_exige_un_movimiento_real_y_del_tipo_correcto() {
        let pool = prueba_apoyo::base().await;
        let (a, faltante, _) = montar(&pool).await;
        let baja = concepto_baja(&pool).await;
        mover(&pool, prueba_apoyo::COMPRA, a, 100, "2026-08-01", None)
            .await
            .unwrap();

        let peticion = |mov: i64, cant: i64| PeticionActa {
            fecha: "2026-08-06".into(),
            metodo: "destruccion".into(),
            representante_legal: "Juan Pérez".into(),
            responsables: vec!["Ana Gómez".into()],
            cubierto_por_seguro: 0,
            renglones: vec![PeticionRenglon {
                articulo_id: a,
                cantidad: cant,
                justificacion: "Vencido".into(),
                movimiento_id: mov,
            }],
        };

        // Movimiento inexistente.
        let e = crear_acta(&pool, &peticion(9_999, 1_000), None).await.unwrap_err();
        assert!(e.to_string().contains("no existe"), "{e}");

        // Movimiento que no es una baja (es un faltante del art. 64).
        let m = mover(&pool, faltante, a, 5, "2026-08-05", None).await.unwrap();
        let e = crear_acta(&pool, &peticion(m, 1_000), None).await.unwrap_err();
        assert!(e.to_string().contains("baja_con_acta"), "{e}");

        // Más unidades de las que salieron.
        let m = mover(&pool, baja, a, 5, "2026-08-05", None).await.unwrap();
        let e = crear_acta(&pool, &peticion(m, Cantidad::de_unidades(10).0), None)
            .await
            .unwrap_err();
        assert!(e.to_string().contains("solo sacó"), "{e}");

        // Y con todo bien, pasa.
        assert!(crear_acta(&pool, &peticion(m, Cantidad::de_unidades(5).0), None).await.is_ok());
    }

    #[tokio::test]
    async fn un_metodo_de_destruccion_inventado_se_rechaza_citando_la_norma() {
        let pool = prueba_apoyo::base().await;
        let (a, _, _) = montar(&pool).await;
        let baja = concepto_baja(&pool).await;
        mover(&pool, prueba_apoyo::COMPRA, a, 100, "2026-08-01", None)
            .await
            .unwrap();
        let m = mover(&pool, baja, a, 5, "2026-08-05", None).await.unwrap();

        let e = crear_acta(
            &pool,
            &PeticionActa {
                fecha: "2026-08-06".into(),
                metodo: "se botó a la basura".into(),
                representante_legal: "Juan Pérez".into(),
                responsables: vec!["Ana".into()],
                cubierto_por_seguro: 0,
                renglones: vec![PeticionRenglon {
                    articulo_id: a,
                    cantidad: Cantidad::de_unidades(5).0,
                    justificacion: "Vencido".into(),
                    movimiento_id: m,
                }],
            },
            None,
        )
        .await
        .unwrap_err();
        assert!(e.to_string().contains("64"), "{e}");
        assert!(e.to_string().contains("chatarrización"), "{e}");
    }
}
