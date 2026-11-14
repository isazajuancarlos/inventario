//! Facturación.
//!
//! **La regla que define este módulo**: emitir una factura escribe la cabecera,
//! el detalle, el consecutivo de la resolución y los movimientos de kardex
//! **en la misma transacción**. No hay forma de hacer una cosa sin la otra.
//!
//! No es una preferencia de estilo. El sistema que se reemplaza facturó desde
//! 2004 hasta 2020 con el kardex parado en septiembre de 2004 —medido: 3.154
//! facturas, ningún movimiento con concepto de venta— porque mover inventario
//! era un paso aparte que alguien tenía que acordarse de dar. Aquí no hay paso
//! aparte que olvidar.

use crate::{Error, Pool, Resultado, kardex};
use chrono::NaiveDate;
use inventario_dominio::{
    Cantidad, Centavos,
    cartera::{self, FacturaPorCobrar},
    cufe::{self, Ambiente},
    factura::{self as dom, Linea},
    numeracion::{self, Resolucion},
};
use sqlx::Row;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Peticion {
    pub resolucion_id: i64,
    pub tercero_id: i64,
    #[serde(default)]
    pub vendedor_id: Option<i64>,
    pub fecha: String,
    /// Hora de generación con desplazamiento horario ('10:53:10-05:00'), que
    /// exige el art. 11 de la Res. 000165/2023 y entra en el CUFE.
    #[serde(default = "hora_por_defecto")]
    pub hora: String,
    pub forma_pago: String,
    /// Obligatorio cuando la forma de pago es contado (art. 11).
    #[serde(default)]
    pub medio_pago: Option<String>,
    #[serde(default)]
    pub retencion: i64,
    #[serde(default)]
    pub observacion: String,
    pub lineas: Vec<Linea>,
}

/// Sin hora explícita se toma la del reloj, con el huso de Colombia. NO es un
/// relleno silencioso: la hora de generación es un dato que el sistema conoce,
/// a diferencia de un precio o un NIT.
fn hora_por_defecto() -> String {
    format!("{}-05:00", chrono::Utc::now().format("%H:%M:%S"))
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Emitida {
    pub id: i64,
    pub prefijo: String,
    pub numero: i64,
    pub total: i64,
    pub iva: i64,
    pub inc: i64,
    pub en_letras: String,
    pub vence: String,
    /// CUFE del anexo técnico 1.9. `None` cuando la resolución no tiene clave
    /// técnica: se dice que falta, no se emite un código inventado.
    pub cufe: Option<String>,
    /// Por qué no hay CUFE, cuando no lo hay.
    pub cufe_falta: Option<String>,
}

/// El concepto de kardex con el que salen las ventas.
///
/// Se busca por `clase`, no por un código fijo: los códigos son del cliente
/// (el corpus usaba 51..55 para cinco puntos de venta distintos) y clavar uno
/// aquí ataría el programa a una instalación.
async fn concepto_de_clase(pool: &Pool, clase: &str) -> Resultado<i64> {
    let f = sqlx::query("SELECT id FROM concepto WHERE clase = ? AND activo = 1 ORDER BY id LIMIT 1")
        .bind(clase)
        .fetch_optional(pool)
        .await?;
    f.map(|f| f.get::<i64, _>("id")).ok_or_else(|| {
        Error::Regla(format!(
            "no hay ningún concepto de kardex activo de clase «{clase}»; \
             la instalación está incompleta y no se puede facturar"
        ))
    })
}

/// Emite una factura. Todo o nada.
pub async fn emitir(pool: &Pool, p: &Peticion) -> Resultado<Emitida> {
    if p.forma_pago != "contado" && p.forma_pago != "credito" {
        return Err(Error::Regla(format!(
            "forma de pago desconocida: {:?} (contado o credito)",
            p.forma_pago
        )));
    }
    let fecha = NaiveDate::parse_from_str(&p.fecha, "%Y-%m-%d")
        .map_err(|_| Error::Regla(format!("fecha inválida: {:?} (se espera AAAA-MM-DD)", p.fecha)))?;
    // Art. 11 de la Res. 000165/2023: el medio de pago es obligatorio cuando la
    // operación es de contado.
    if p.forma_pago == "contado" {
        match p.medio_pago.as_deref() {
            None => {
                return Err(Error::Regla(
                    "una factura de contado exige el medio de pago (Res. DIAN 000165 de 2023, \
                     art. 11)"
                        .into(),
                ));
            }
            Some(m) if !inventario_dominio::factura::medio_de_pago_conocido(m) => {
                return Err(Error::Regla(format!(
                    "medio de pago {m:?} desconocido: uno de {}",
                    inventario_dominio::factura::medios_de_pago_admitidos()
                )));
            }
            Some(_) => {}
        }
    }

    // La aritmética se decide ANTES de tocar la base: si la factura no cuadra,
    // no se ha consumido un número de resolución. Un número quemado por una
    // factura que no llegó a existir es un hueco que hay que explicarle a la
    // DIAN.
    let totales = dom::calcular(&p.lineas, Centavos(p.retencion))?;
    let concepto_venta = concepto_de_clase(pool, "venta").await?;

    // La configuración de costeo se lee ANTES de abrir la transacción, una vez
    // por artículo.
    //
    // No es una optimización: pedirle otra conexión al pool mientras se tiene
    // una transacción abierta es un INTERBLOQUEO por agotamiento. Con 8
    // conexiones y 12 cajeros simultáneos, los 8 primeros retienen su
    // transacción esperando una novena conexión que nadie va a soltar, y el
    // servidor responde 500 a todos. Lo cazó
    // `doce_cajeros_a_la_vez_no_repiten_numero`.
    let mut config: std::collections::HashMap<i64, kardex::Configuracion> =
        std::collections::HashMap::new();
    for l in &p.lineas {
        if let std::collections::hash_map::Entry::Vacant(e) = config.entry(l.articulo_id) {
            e.insert(kardex::configuracion(pool, l.articulo_id).await?);
        }
    }

    // **`BEGIN IMMEDIATE`, y no el `BEGIN` de siempre.**
    //
    // Esto no es afinación: sin ello el sistema se cae con dos cajeros. Un
    // `BEGIN` normal abre la transacción en modo DIFERIDO, así que la lectura
    // de `ultimo_emitido` no reserva nada y la escritura llega después. Cuando
    // dos transacciones diferidas leen y luego escriben, SQLite le devuelve
    // `SQLITE_BUSY` a la segunda **de inmediato y sin reintentar** —el
    // `busy_timeout` NO ayuda aquí: reintentar rompería la instantánea que esa
    // transacción ya leyó, así que SQLite ni lo intenta—. El resultado es un
    // 500 en la cara del cajero.
    //
    // Con `IMMEDIATE` la escritura se reserva desde el principio, el
    // `busy_timeout` sí aplica, y el segundo cajero **espera su turno** y saca
    // el número siguiente. Lo midió `doce_cajeros_a_la_vez_no_repiten_numero`:
    // con `BEGIN` a secas, seis de doce facturas morían con «database is
    // locked».
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;

    let r = sqlx::query(
        "SELECT id, prefijo, desde, hasta, ultimo_emitido, rige_desde, vence, activa,
                clave_tecnica
           FROM resolucion WHERE id = ?",
    )
    .bind(p.resolucion_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(Error::NoExiste {
        que: "resolución",
        id: p.resolucion_id,
    })?;

    let resolucion = Resolucion {
        id: r.get("id"),
        prefijo: r.get("prefijo"),
        desde: r.get("desde"),
        hasta: r.get("hasta"),
        ultimo_emitido: r.get("ultimo_emitido"),
        vence: r
            .get::<Option<String>, _>("vence")
            .and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()),
        activa: r.get::<i64, _>("activa") == 1,
    };
    let numero = numeracion::siguiente(&resolucion, fecha)?;
    // El rango numérico y la VIGENCIA se agotan por separado: una autorización
    // puede tener números y estar fuera de fecha. `numeracion::siguiente` mira
    // el vencimiento; el inicio se mira aquí porque una factura anterior a la
    // vigencia tampoco es válida.
    if let Some(rd) = r.get::<Option<String>, _>("rige_desde")
        && p.fecha.as_str() < rd.as_str()
    {
        return Err(Error::Regla(format!(
            "la resolución rige desde {rd} y la factura es del {}: la autorización de \
             numeración todavía no había empezado",
            p.fecha
        )));
    }

    let plazo: i64 = if p.forma_pago == "credito" {
        sqlx::query("SELECT plazo_dias FROM tercero WHERE id = ? AND activo = 1")
            .bind(p.tercero_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::NoExiste {
                que: "tercero",
                id: p.tercero_id,
            })?
            .get("plazo_dias")
    } else {
        0
    };
    let vence = (fecha + chrono::Duration::days(plazo)).to_string();

    let factura_id: i64 = sqlx::query(
        "INSERT INTO factura
            (resolucion_id, prefijo, numero, tercero_id, vendedor_id, fecha, hora, vence,
             forma_pago, medio_pago, subtotal, descuento, iva, inc, retencion, total,
             observacion, usuario_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL)
         RETURNING id",
    )
    .bind(resolucion.id)
    .bind(&resolucion.prefijo)
    .bind(numero)
    .bind(p.tercero_id)
    .bind(p.vendedor_id)
    .bind(&p.fecha)
    .bind(&p.hora)
    .bind(&vence)
    .bind(&p.forma_pago)
    .bind(&p.medio_pago)
    .bind(totales.subtotal.0)
    .bind(totales.descuento.0)
    .bind(totales.iva.0)
    .bind(totales.inc.0)
    .bind(totales.retencion.0)
    .bind(totales.total.0)
    .bind(&p.observacion)
    .fetch_one(&mut *tx)
    .await?
    .get("id");

    for (i, l) in totales.lineas.iter().enumerate() {
        // El movimiento de kardex va PRIMERO: si no hay existencia, la
        // transacción entera se cae aquí y la factura no llega a existir. El
        // costo que devuelve es el que se anota en la línea.
        let cfg = config[&l.articulo_id];
        let (_, costo) = kardex::registrar_en_tx(
            &mut tx,
            &kardex::Peticion {
                fecha: p.fecha.clone(),
                concepto_id: concepto_venta,
                articulo_id: l.articulo_id,
                tercero_id: Some(p.tercero_id),
                documento: format!("{}{}", resolucion.prefijo, numero),
                cantidad: l.cantidad,
                costo_unitario: Centavos::CERO,
                factura_id: Some(factura_id),
                usuario_id: None,
                valor_comercial_unitario: None,
            },
            cfg,
        )
        .await?;

        // La unidad de medida se CONGELA en la línea: si mañana alguien cambia
        // la del artículo, la factura ya emitida tiene que seguir diciendo lo
        // que dijo.
        let unidad: Option<String> = sqlx::query_as::<_, (Option<String>,)>(
            "SELECT unidad_medida FROM articulo WHERE id = ?",
        )
        .bind(l.articulo_id)
        .fetch_optional(&mut *tx)
        .await?
        .and_then(|(u,)| u);

        sqlx::query(
            "INSERT INTO factura_linea
                (factura_id, item, articulo_id, cantidad, precio_unitario, bruto,
                 descuento, base, iva_porcentaje, iva, inc_porcentaje, inc,
                 unidad_medida, total, costo)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(factura_id)
        .bind(i as i64 + 1)
        .bind(l.articulo_id)
        .bind(l.cantidad.0)
        .bind(l.precio_unitario.0)
        .bind(l.bruto.0)
        .bind(l.descuento.0)
        .bind(l.base.0)
        .bind(l.iva_porcentaje)
        .bind(l.iva.0)
        .bind(l.inc_porcentaje)
        .bind(l.inc.0)
        .bind(&unidad)
        .bind(l.total.0)
        .bind(costo.0)
        .execute(&mut *tx)
        .await?;
    }

    // El consecutivo se guarda en la MISMA transacción. Si esto se hiciera
    // fuera, un fallo entre medias dejaría dos facturas con el mismo número.
    sqlx::query("UPDATE resolucion SET ultimo_emitido = ? WHERE id = ?")
        .bind(numero)
        .bind(resolucion.id)
        .execute(&mut *tx)
        .await?;

    // --- CUFE (anexo técnico 1.9, num. 11.2) -----------------------------
    //
    // Se calcula DESPUÉS de tener número y totales, y solo si la instalación
    // tiene los datos que exige. Cuando falta alguno se guarda NULL y se DICE
    // cuál: un CUFE inventado pasa por bueno hasta que la DIAN lo rechaza, y
    // para entonces hay un talonario entero mal emitido.
    let (cufe_valor, cufe_falta) = calcular_cufe(
        &mut tx,
        &resolucion,
        numero,
        &p.fecha,
        &p.hora,
        p.tercero_id,
        &totales,
        r.get::<Option<String>, _>("clave_tecnica"),
    )
    .await?;
    if let Some(c) = &cufe_valor {
        sqlx::query("UPDATE factura SET cufe = ? WHERE id = ?")
            .bind(c)
            .bind(factura_id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;

    Ok(Emitida {
        id: factura_id,
        prefijo: resolucion.prefijo,
        numero,
        total: totales.total.0,
        iva: totales.iva.0,
        inc: totales.inc.0,
        en_letras: inventario_dominio::letras::monto(totales.total)
            .unwrap_or_else(|| "(monto fuera de rango)".into()),
        vence,
        cufe: cufe_valor,
        cufe_falta,
    })
}

/// Calcula el CUFE, o dice exactamente qué falta para poder calcularlo.
#[allow(clippy::too_many_arguments)]
async fn calcular_cufe(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    resolucion: &Resolucion,
    numero: i64,
    fecha: &str,
    hora: &str,
    tercero_id: i64,
    totales: &dom::Totales,
    clave_tecnica: Option<String>,
) -> Resultado<(Option<String>, Option<String>)> {
    let Some(clave) = clave_tecnica.filter(|c| !c.trim().is_empty()) else {
        return Ok((
            None,
            Some(format!(
                "la resolución «{}» no tiene clave técnica; pídala en el portal de la DIAN \
                 y regístrela para poder emitir factura electrónica",
                resolucion.id
            )),
        ));
    };
    let empresa: Option<(String, String)> =
        sqlx::query_as("SELECT nit, ambiente_dian FROM empresa WHERE id = 1")
            .fetch_optional(&mut **tx)
            .await?;
    let Some((nit_empresa, ambiente)) = empresa else {
        return Ok((None, Some("la empresa no está configurada".into())));
    };
    let adquirente: Option<(String,)> = sqlx::query_as("SELECT nit FROM tercero WHERE id = ?")
        .bind(tercero_id)
        .fetch_optional(&mut **tx)
        .await?;
    let Some((nit_adquirente,)) = adquirente else {
        return Ok((None, Some("el adquirente no tiene NIT".into())));
    };

    let datos = cufe::Datos {
        numero_completo: &format!("{}{}", resolucion.prefijo, numero),
        fecha,
        hora,
        // ValFac es el valor SIN impuestos: la base, no el total.
        valor_sin_impuestos: totales.subtotal,
        iva: totales.iva,
        inc: totales.inc,
        // El ICA no se liquida en este sistema todavía; va en cero, que es lo
        // que manda el anexo para un impuesto no referenciado.
        ica: Centavos::CERO,
        valor_total: totales.total,
        nit_facturador: &nit_empresa,
        nit_adquirente: &nit_adquirente,
        clave_tecnica: &clave,
        ambiente: if ambiente == "produccion" {
            Ambiente::Produccion
        } else {
            Ambiente::Pruebas
        },
    };
    match cufe::calcular(&datos) {
        Ok(c) => Ok((Some(c), None)),
        // El CUFE no se puede calcular por un dato mal formado. No se emite
        // uno a medias: se guarda el motivo.
        Err(e) => Ok((None, Some(e.to_string()))),
    }
}

/// Anula una factura: **no la borra**.
///
/// Escribe un movimiento de reverso por cada línea, para que el inventario
/// vuelva a donde estaba. Se niega si la factura ya tiene abonos: primero hay
/// que deshacer el recaudo, porque anular una factura pagada dejaría el dinero
/// aplicado a un documento que ya no debe nada.
pub async fn anular(pool: &Pool, factura_id: i64, motivo: &str, usuario_id: Option<i64>) -> Resultado<()> {
    if motivo.trim().is_empty() {
        return Err(Error::Regla("anular exige un motivo".into()));
    }
    let concepto_devolucion = concepto_de_clase(pool, "devolucion_venta").await?;

    // Igual que en `emitir`: la configuración se lee ANTES de la transacción,
    // porque pedir otra conexión del pool con una transacción abierta
    // interbloquea bajo concurrencia.
    let articulos: Vec<(i64,)> =
        sqlx::query_as("SELECT DISTINCT articulo_id FROM factura_linea WHERE factura_id = ?")
            .bind(factura_id)
            .fetch_all(pool)
            .await?;
    let mut config: std::collections::HashMap<i64, kardex::Configuracion> =
        std::collections::HashMap::new();
    for (a,) in articulos {
        config.insert(a, kardex::configuracion(pool, a).await?);
    }

    // `IMMEDIATE`: lee la factura y sus líneas, y luego escribe el reverso.
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;

    let f = sqlx::query("SELECT estado, fecha FROM factura WHERE id = ?")
        .bind(factura_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error::NoExiste {
            que: "factura",
            id: factura_id,
        })?;
    if f.get::<String, _>("estado") == "anulada" {
        return Err(Error::Regla(format!("la factura {factura_id} ya está anulada")));
    }

    let (abonado,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(a.valor), 0)
           FROM recibo_aplicacion a JOIN recibo r ON r.id = a.recibo_id
          WHERE a.factura_id = ? AND r.estado = 'activo'",
    )
    .bind(factura_id)
    .fetch_one(&mut *tx)
    .await?;
    if abonado > 0 {
        return Err(Error::Regla(format!(
            "la factura {factura_id} tiene {} en abonos aplicados; \
             anule o desaplique el recaudo antes de anularla",
            Centavos(abonado)
        )));
    }

    let hoy = chrono::Utc::now().date_naive().to_string();
    let lineas = sqlx::query("SELECT articulo_id, cantidad FROM factura_linea WHERE factura_id = ?")
        .bind(factura_id)
        .fetch_all(&mut *tx)
        .await?;

    for l in lineas {
        // El reverso es una ENTRADA al costo con el que salió, para que el
        // promedio del artículo vuelva exactamente a donde estaba. Se toma del
        // movimiento original, no del promedio de hoy —que ya pudo cambiar—.
        let articulo_id: i64 = l.get("articulo_id");
        let cantidad = Cantidad(l.get::<i64, _>("cantidad"));
        let original = sqlx::query(
            "SELECT id, valor FROM movimiento
              WHERE factura_id = ? AND articulo_id = ? AND reversa_de IS NULL
              LIMIT 1",
        )
        .bind(factura_id)
        .bind(articulo_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            Error::Regla(format!(
                "la factura {factura_id} no tiene movimiento de kardex para el artículo \
                 {articulo_id}: no se puede reversar lo que no se registró"
            ))
        })?;
        let valor_original = Centavos(original.get::<i64, _>("valor"));
        let costo_unitario = if cantidad.0 == 0 {
            Centavos::CERO
        } else {
            // LA JUSTIFICACIÓN QUE HABÍA AQUÍ ERA FALSA, y el respaldo también.
            //
            // Decía «`cantidad ≥ 1000` (milésimas), así que no crece por encima
            // de `valor`». `Cantidad` admite FRACCIONES —`Cantidad(500)` es
            // media unidad—, o sea que la premisa no se sostiene. Lo señaló la
            // revisión de seguridad del 2026-08-13.
            //
            // Lo que sí cabe por construcción es otra cosa: `valor` se guardó
            // como `cantidad × unitario / 1000`, luego `valor × 1000 / cantidad`
            // devuelve aproximadamente `unitario`, que ya era un `i64`.
            //
            // Y el `unwrap_or(0)` se va. Su caso alcanzable es justo el que este
            // mismo cambio arregla: si `movimiento.valor` quedó truncado por el
            // `as i64` de antes, la reversa entraba con costo unitario CERO —el
            // artículo vuelve al inventario sin valor, el promedio del kardex
            // queda descuadrado y la transacción se confirma tan tranquila—.
            // Todo el resto de este trabajo eligió `Err` ante un importe que no
            // cabe; aquí un relleno sería la excepción sin motivo (directiva 20).
            Centavos(
                inventario_dominio::dinero::dividir_redondeando(
                    valor_original.0 as i128 * inventario_dominio::dinero::MIL as i128,
                    cantidad.0 as i128,
                )
                .ok_or_else(|| {
                    Error::Regla(format!(
                        "el costo unitario del movimiento de la factura {factura_id} \
                         (artículo {articulo_id}) no cabe en centavos: el valor guardado \
                         es {} para una cantidad de {}. No se reversa a ciegas",
                        valor_original.0, cantidad.0
                    ))
                })?,
            )
        };

        let cfg = kardex::Configuracion {
            // Un reverso es una ENTRADA: nunca deja saldo negativo, así que el
            // camino permisivo aquí no relaja nada.
            permite_negativo: true,
            ..config[&articulo_id]
        };
        let (id_reverso, _) = kardex::registrar_en_tx(
            &mut tx,
            &kardex::Peticion {
                fecha: hoy.clone(),
                concepto_id: concepto_devolucion,
                articulo_id,
                tercero_id: None,
                documento: format!("anulación factura {factura_id}"),
                cantidad,
                costo_unitario,
                factura_id: Some(factura_id),
                usuario_id,
                valor_comercial_unitario: None,
            },
            cfg,
        )
        .await?;
        sqlx::query("UPDATE movimiento SET reversa_de = ? WHERE id = ?")
            .bind(original.get::<i64, _>("id"))
            .bind(id_reverso)
            .execute(&mut *tx)
            .await?;
    }

    sqlx::query(
        "UPDATE factura SET estado = 'anulada', anulada_en = ?, anulada_por = ?, motivo_anulacion = ?
          WHERE id = ?",
    )
    .bind(&hoy)
    .bind(usuario_id)
    .bind(motivo)
    .bind(factura_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// La cartera de un cliente, de la más vieja a la más nueva.
pub async fn por_cobrar(pool: &Pool, tercero_id: i64) -> Resultado<Vec<FacturaPorCobrar>> {
    let filas = sqlx::query(
        "SELECT f.id, f.total, f.estado,
                COALESCE((SELECT SUM(a.valor) FROM recibo_aplicacion a
                            JOIN recibo r ON r.id = a.recibo_id
                           WHERE a.factura_id = f.id AND r.estado = 'activo'), 0) AS abonado,
                COALESCE((SELECT SUM(n.total) FROM nota n
                           WHERE n.factura_id = f.id AND n.clase = 'credito'
                             AND n.estado = 'activa'), 0) AS notas
           FROM factura f
          WHERE f.tercero_id = ? AND f.forma_pago = 'credito'
          ORDER BY f.fecha, f.id",
    )
    .bind(tercero_id)
    .fetch_all(pool)
    .await?;

    Ok(filas
        .into_iter()
        .map(|f| FacturaPorCobrar {
            factura_id: f.get("id"),
            total: Centavos(f.get::<i64, _>("total")),
            abonado: Centavos(f.get::<i64, _>("abonado")),
            notas: Centavos(f.get::<i64, _>("notas")),
            anulada: f.get::<String, _>("estado") == "anulada",
        })
        .collect())
}

/// Cartera pendiente de todos los clientes. El informe que se mira cada lunes.
#[derive(Debug, serde::Serialize)]
pub struct SaldoCliente {
    pub tercero_id: i64,
    pub nombre: String,
    pub facturas: i64,
    pub saldo: i64,
    pub vencido: i64,
}

pub async fn cartera(pool: &Pool, hoy: &str) -> Resultado<Vec<SaldoCliente>> {
    let filas = sqlx::query(
        "SELECT t.id, t.nombre, f.id AS factura_id, f.total, f.vence, f.estado,
                COALESCE((SELECT SUM(a.valor) FROM recibo_aplicacion a
                            JOIN recibo r ON r.id = a.recibo_id
                           WHERE a.factura_id = f.id AND r.estado = 'activo'), 0) AS abonado,
                COALESCE((SELECT SUM(n.total) FROM nota n
                           WHERE n.factura_id = f.id AND n.clase = 'credito'
                             AND n.estado = 'activa'), 0) AS notas
           FROM factura f JOIN tercero t ON t.id = f.tercero_id
          WHERE f.forma_pago = 'credito' AND f.estado = 'activa'
          ORDER BY t.nombre, f.fecha",
    )
    .fetch_all(pool)
    .await?;

    let mut acumulado: Vec<SaldoCliente> = Vec::new();
    for f in filas {
        let pc = FacturaPorCobrar {
            factura_id: f.get("factura_id"),
            total: Centavos(f.get::<i64, _>("total")),
            abonado: Centavos(f.get::<i64, _>("abonado")),
            notas: Centavos(f.get::<i64, _>("notas")),
            anulada: false,
        };
        let saldo = pc.saldo();
        if saldo.0 <= 0 {
            continue;
        }
        let vencida = f.get::<String, _>("vence").as_str() < hoy;
        let id: i64 = f.get("id");
        match acumulado.iter_mut().find(|c| c.tercero_id == id) {
            Some(c) => {
                c.facturas += 1;
                c.saldo += saldo.0;
                if vencida {
                    c.vencido += saldo.0;
                }
            }
            None => acumulado.push(SaldoCliente {
                tercero_id: id,
                nombre: f.get("nombre"),
                facturas: 1,
                saldo: saldo.0,
                vencido: if vencida { saldo.0 } else { 0 },
            }),
        }
    }
    Ok(acumulado)
}

/// Valida un reparto contra la cartera real del cliente.
pub async fn validar_abono(
    pool: &Pool,
    tercero_id: i64,
    total: Centavos,
    aplicaciones: &[(i64, Centavos)],
) -> Resultado<()> {
    let pendientes = por_cobrar(pool, tercero_id).await?;
    cartera::validar_reparto(&pendientes, total, aplicaciones)?;
    Ok(())
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::prueba_apoyo;

    async fn preparar(pool: &Pool) -> (i64, i64, i64) {
        let a: i64 = sqlx::query(
            "INSERT INTO articulo (codigo, descripcion, iva_porcentaje, precio_venta)
             VALUES ('A1', 'Botellón 20 L', 19, 500000) RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
        .get("id");
        let t: i64 = sqlx::query(
            "INSERT INTO tercero (nit, dv, nombre, plazo_dias) VALUES ('43627185', 1, 'Cliente', 30)
             RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
        .get("id");
        let r: i64 = sqlx::query(
            "INSERT INTO resolucion (nombre, prefijo, desde, hasta)
             VALUES ('Principal', 'FE', 1, 100) RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
        .get("id");
        // Existencia inicial.
        crate::kardex::registrar(
            pool,
            &crate::kardex::Peticion {
                fecha: "2026-08-01".into(),
                concepto_id: prueba_apoyo::COMPRA,
                articulo_id: a,
                tercero_id: None,
                documento: "compra".into(),
                cantidad: Cantidad::de_unidades(100),
                costo_unitario: Centavos::de_pesos(3000),
                factura_id: None,
                usuario_id: Some(1),
                valor_comercial_unitario: None,
            },
        )
        .await
        .unwrap();
        (a, t, r)
    }

    fn linea(a: i64, u: i64, pesos: i64, iva: i64) -> Linea {
        Linea {
            articulo_id: a,
            cantidad: Cantidad::de_unidades(u),
            precio_unitario: Centavos::de_pesos(pesos),
            descuento: Centavos::CERO,
            iva_porcentaje: iva,
            inc_porcentaje: 0,
        }
    }

    fn peticion(r: i64, t: i64, lineas: Vec<Linea>) -> Peticion {
        Peticion {
            resolucion_id: r,
            tercero_id: t,
            vendedor_id: None,
            fecha: "2026-08-02".into(),
            hora: "10:53:10-05:00".into(),
            forma_pago: "credito".into(),
            medio_pago: None,
            retencion: 0,
            observacion: String::new(),
            lineas,
        }
    }

    /// LA prueba del módulo: facturar mueve el inventario. Sin ella, todo lo
    /// demás puede estar bien y el sistema repetir el fallo del que se viene.
    #[tokio::test]
    async fn facturar_mueve_el_kardex_en_la_misma_transaccion() {
        let pool = prueba_apoyo::base().await;
        let (a, t, r) = preparar(&pool).await;

        let antes = crate::kardex::saldo(&pool, a).await.unwrap().existencia;
        let f = emitir(&pool, &peticion(r, t, vec![linea(a, 10, 5000, 19)]))
            .await
            .unwrap();
        let despues = crate::kardex::saldo(&pool, a).await.unwrap().existencia;

        assert_eq!(antes - despues, 10_000, "facturar no movió el inventario");
        assert_eq!(f.numero, 1);
        assert_eq!(f.total, Centavos::de_pesos(59_500).0); // 50.000 + 19 %
        assert!(f.en_letras.starts_with("CINCUENTA Y NUEVE MIL QUINIENTOS PESOS"));
        assert_eq!(f.vence, "2026-09-01", "30 días de plazo del cliente");

        // Y el movimiento quedó ligado a la factura, no suelto.
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM movimiento WHERE factura_id = ?")
            .bind(f.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 1);
    }

    /// Si el inventario no alcanza, NO queda una factura a medias ni se quema
    /// un número de resolución.
    #[tokio::test]
    async fn sin_existencia_no_queda_ni_factura_ni_numero_quemado() {
        let pool = prueba_apoyo::base().await;
        let (a, t, r) = preparar(&pool).await;

        let e = emitir(&pool, &peticion(r, t, vec![linea(a, 500, 5000, 19)]))
            .await
            .unwrap_err();
        assert!(matches!(e, Error::Kardex(_)), "salió {e}");

        let (facturas,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM factura")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(facturas, 0, "quedó una factura huérfana");
        let ultimo: (Option<i64>,) =
            sqlx::query_as("SELECT ultimo_emitido FROM resolucion WHERE id = ?")
                .bind(r)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(ultimo.0, None, "se quemó un número de resolución");
        assert_eq!(crate::kardex::saldo(&pool, a).await.unwrap().existencia, 100_000);
    }

    #[tokio::test]
    async fn el_numero_no_se_repite_ni_bajo_cien_facturas() {
        let pool = prueba_apoyo::base().await;
        let (a, t, r) = preparar(&pool).await;
        let mut vistos = std::collections::BTreeSet::new();
        for _ in 0..100 {
            let f = emitir(&pool, &peticion(r, t, vec![linea(a, 1, 1000, 19)]))
                .await
                .unwrap();
            assert!(vistos.insert(f.numero), "el número {} salió dos veces", f.numero);
        }
        assert_eq!(vistos.len(), 100);
        // Y el rango se agota con nombre propio.
        let e = emitir(&pool, &peticion(r, t, vec![linea(a, 1, 1000, 19)]))
            .await
            .unwrap_err();
        assert!(matches!(e, Error::Numeracion(_)), "salió {e}");
    }

    #[tokio::test]
    async fn anular_devuelve_el_inventario_y_conserva_la_factura() {
        let pool = prueba_apoyo::base().await;
        let (a, t, r) = preparar(&pool).await;
        let antes = crate::kardex::saldo(&pool, a).await.unwrap();

        let f = emitir(&pool, &peticion(r, t, vec![linea(a, 10, 5000, 19)]))
            .await
            .unwrap();
        anular(&pool, f.id, "cliente devolvió la mercancía", Some(1))
            .await
            .unwrap();

        let despues = crate::kardex::saldo(&pool, a).await.unwrap();
        assert_eq!(despues.existencia, antes.existencia, "el inventario no volvió");
        assert_eq!(despues.valor, antes.valor, "el valor del inventario no volvió");
        assert_eq!(despues.costo_promedio, antes.costo_promedio);

        // La factura SIGUE ahí: se anula, no se borra.
        let (estado, total): (String, i64) =
            sqlx::query_as("SELECT estado, total FROM factura WHERE id = ?")
                .bind(f.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(estado, "anulada");
        assert_eq!(total, f.total, "el histórico se conserva íntegro");
        // Y no debe nada.
        let c = por_cobrar(&pool, t).await.unwrap();
        assert_eq!(c[0].saldo(), Centavos::CERO);
    }

    #[tokio::test]
    async fn anular_exige_motivo_y_no_se_hace_dos_veces() {
        let pool = prueba_apoyo::base().await;
        let (a, t, r) = preparar(&pool).await;
        let f = emitir(&pool, &peticion(r, t, vec![linea(a, 1, 1000, 0)]))
            .await
            .unwrap();
        assert!(anular(&pool, f.id, "   ", None).await.is_err());
        anular(&pool, f.id, "error de digitación", None).await.unwrap();
        assert!(anular(&pool, f.id, "otra vez", None).await.is_err());
        assert!(anular(&pool, 9999, "no existe", None).await.is_err());
    }

    #[tokio::test]
    async fn la_factura_de_contado_vence_el_mismo_dia() {
        let pool = prueba_apoyo::base().await;
        let (a, t, r) = preparar(&pool).await;
        let mut p = peticion(r, t, vec![linea(a, 1, 1000, 0)]);
        p.forma_pago = "contado".into();
        p.medio_pago = Some("efectivo".into());
        let f = emitir(&pool, &p).await.unwrap();
        assert_eq!(f.vence, "2026-08-02");
        // Y no entra en cartera.
        assert!(por_cobrar(&pool, t).await.unwrap().is_empty());
    }

    /// Res. DIAN 000165 de 2023, art. 11: el medio de pago es OBLIGATORIO
    /// cuando la operación es de contado. Sin él la factura no se emite.
    #[tokio::test]
    async fn una_factura_de_contado_exige_el_medio_de_pago() {
        let pool = prueba_apoyo::base().await;
        let (a, t, r) = preparar(&pool).await;
        let mut p = peticion(r, t, vec![linea(a, 1, 1000, 0)]);
        p.forma_pago = "contado".into();

        let e = emitir(&pool, &p).await.unwrap_err();
        assert!(e.to_string().contains("000165"), "el error no cita la norma: {e}");

        // Un medio inventado tampoco pasa.
        p.medio_pago = Some("trueque".into());
        assert!(emitir(&pool, &p).await.is_err());

        // Y a crédito NO se exige: el medio se conocerá al recaudar.
        let p = peticion(r, t, vec![linea(a, 1, 1000, 0)]);
        assert!(emitir(&pool, &p).await.is_ok());
    }

    /// El CUFE se calcula al emitir cuando están los datos, y cuando no, se
    /// DICE qué falta en vez de guardar un código inventado.
    #[tokio::test]
    async fn el_cufe_se_calcula_o_se_explica_por_que_no() {
        let pool = prueba_apoyo::base().await;
        let (a, t, r) = preparar(&pool).await;

        // Sin clave técnica: no hay CUFE, y el motivo lo dice.
        let f = emitir(&pool, &peticion(r, t, vec![linea(a, 1, 1000, 19)]))
            .await
            .unwrap();
        assert!(f.cufe.is_none());
        assert!(
            f.cufe_falta.as_ref().unwrap().contains("clave técnica"),
            "{:?}",
            f.cufe_falta
        );

        // Con clave técnica: sale el CUFE, de 96 hexadecimales.
        sqlx::query("UPDATE resolucion SET clave_tecnica = ? WHERE id = ?")
            .bind("693ff6f2a553c3646a063436fd4dd9ded0311471")
            .bind(r)
            .execute(&pool)
            .await
            .unwrap();
        let f = emitir(&pool, &peticion(r, t, vec![linea(a, 1, 1000, 19)]))
            .await
            .unwrap();
        let c = f.cufe.expect("tenía que salir el CUFE");
        assert_eq!(c.len(), 96);
        assert!(c.chars().all(|x| x.is_ascii_hexdigit()));
        assert!(f.cufe_falta.is_none());

        // Y quedó guardado en la factura, no solo devuelto.
        let (guardado,): (Option<String>,) =
            sqlx::query_as("SELECT cufe FROM factura WHERE id = ?")
                .bind(f.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(guardado.as_deref(), Some(c.as_str()));
    }

    /// El rango numérico y la VIGENCIA se agotan por separado.
    #[tokio::test]
    async fn una_factura_anterior_al_inicio_de_la_vigencia_no_se_emite() {
        let pool = prueba_apoyo::base().await;
        let (a, t, r) = preparar(&pool).await;
        sqlx::query("UPDATE resolucion SET rige_desde = '2026-09-01' WHERE id = ?")
            .bind(r)
            .execute(&pool)
            .await
            .unwrap();

        let e = emitir(&pool, &peticion(r, t, vec![linea(a, 1, 1000, 0)]))
            .await
            .unwrap_err();
        assert!(e.to_string().contains("rige desde"), "{e}");

        // A partir de la fecha de vigencia, sí.
        let mut p = peticion(r, t, vec![linea(a, 1, 1000, 0)]);
        p.fecha = "2026-09-15".into();
        assert!(emitir(&pool, &p).await.is_ok());
    }

    #[tokio::test]
    async fn una_forma_de_pago_o_una_fecha_que_no_existen_se_rechazan() {
        let pool = prueba_apoyo::base().await;
        let (a, t, r) = preparar(&pool).await;
        let mut p = peticion(r, t, vec![linea(a, 1, 1000, 0)]);
        p.forma_pago = "trueque".into();
        assert!(matches!(emitir(&pool, &p).await, Err(Error::Regla(_))));

        let mut p = peticion(r, t, vec![linea(a, 1, 1000, 0)]);
        p.fecha = "32/13/2026".into();
        assert!(matches!(emitir(&pool, &p).await, Err(Error::Regla(_))));
    }

    #[tokio::test]
    async fn la_cartera_separa_lo_vencido_de_lo_corriente() {
        let pool = prueba_apoyo::base().await;
        let (a, t, r) = preparar(&pool).await;
        emitir(&pool, &peticion(r, t, vec![linea(a, 1, 10_000, 0)]))
            .await
            .unwrap();
        // Vence el 2026-09-01 (30 días).
        let c = cartera(&pool, "2026-08-15").await.unwrap();
        assert_eq!(c[0].saldo, Centavos::de_pesos(10_000).0);
        assert_eq!(c[0].vencido, 0);

        let c = cartera(&pool, "2026-10-01").await.unwrap();
        assert_eq!(c[0].vencido, Centavos::de_pesos(10_000).0);
    }
}
