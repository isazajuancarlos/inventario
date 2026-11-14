//! Notas crédito y débito sobre factura.
//!
//! Son la forma correcta de corregir una factura ya emitida: **una factura
//! electrónica no se edita**. La nota crédito rebaja lo que el cliente debe; la
//! débito lo aumenta.
//!
//! Llevan **CUDE** y no CUFE (anexo técnico 1.9, num. 11.4): el mismo hash con
//! el PIN del software en lugar de la clave técnica del rango.
//!
//! Sin estas dos no se puede certificar: el set de pruebas de habilitación de
//! la DIAN para software propio exige **2 facturas + 1 nota crédito + 1 nota
//! débito**. Ver `docs/DIAN.md`.

use crate::{Error, Pool, Resultado, kardex};
use inventario_dominio::{
    Centavos,
    cufe::{self, Ambiente},
    factura::{self as dom, Linea},
};
use sqlx::Row;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clase {
    Credito,
    Debito,
}

impl Clase {
    pub fn como_texto(self) -> &'static str {
        match self {
            Clase::Credito => "credito",
            Clase::Debito => "debito",
        }
    }
    pub fn desde_texto(s: &str) -> Option<Clase> {
        Some(match s.trim() {
            "credito" => Clase::Credito,
            "debito" => Clase::Debito,
            _ => None?,
        })
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Peticion {
    pub factura_id: i64,
    /// `credito` o `debito`.
    pub clase: String,
    pub fecha: String,
    #[serde(default = "hora_por_defecto")]
    pub hora: String,
    /// Código del concepto de la DIAN. Se guarda tal cual.
    #[serde(default)]
    pub concepto_dian: String,
    /// Obligatorio. Una nota sin motivo no explica nada, y la DIAN lo exige.
    pub motivo: String,
    /// Solo en las de crédito: si la mercancía vuelve al inventario.
    #[serde(default)]
    pub reintegra_inventario: bool,
    pub lineas: Vec<Linea>,
}

fn hora_por_defecto() -> String {
    format!("{}-05:00", chrono::Utc::now().format("%H:%M:%S"))
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Emitida {
    pub id: i64,
    pub clase: String,
    pub prefijo: String,
    pub numero: i64,
    pub subtotal: i64,
    pub iva: i64,
    pub inc: i64,
    pub total: i64,
    pub en_letras: String,
    pub cude: Option<String>,
    /// Por qué no hay CUDE, cuando no lo hay.
    pub cude_falta: Option<String>,
}

/// Emite una nota crédito o débito.
///
/// Reglas que la definen:
///
/// - **No se puede exceder la factura.** Una nota crédito que rebaje más de lo
///   que la factura vale dejaría al cliente con saldo a favor sin documento que
///   lo explique; el sistema lo rechaza y dice cuánto queda disponible.
/// - **La factura anulada no admite notas.** Ya no debe nada.
/// - **La nota crédito con reintegro mueve kardex**, en la misma transacción,
///   por la misma razón que facturar: si el movimiento fuera un paso aparte,
///   alguien lo olvidaría.
pub async fn emitir(pool: &Pool, p: &Peticion, usuario_id: Option<i64>) -> Resultado<Emitida> {
    let clase = Clase::desde_texto(&p.clase)
        .ok_or_else(|| Error::Regla(format!("clase de nota {:?}: «credito» o «debito»", p.clase)))?;
    if p.motivo.trim().is_empty() {
        return Err(Error::Regla("la nota exige un motivo".into()));
    }
    chrono::NaiveDate::parse_from_str(&p.fecha, "%Y-%m-%d")
        .map_err(|_| Error::Regla(format!("fecha inválida: {:?}", p.fecha)))?;
    if clase == Clase::Debito && p.reintegra_inventario {
        return Err(Error::Regla(
            "una nota débito no devuelve mercancía: aumenta lo que se debe".into(),
        ));
    }

    // La aritmética antes de tocar nada, igual que en la factura.
    let totales = dom::calcular(&p.lineas, Centavos::CERO)?;

    let f = sqlx::query(
        "SELECT prefijo, numero, tercero_id, total, estado FROM factura WHERE id = ?",
    )
    .bind(p.factura_id)
    .fetch_optional(pool)
    .await?
    .ok_or(Error::NoExiste {
        que: "factura",
        id: p.factura_id,
    })?;
    if f.get::<String, _>("estado") == "anulada" {
        return Err(Error::Regla(format!(
            "la factura {} está anulada: no admite notas",
            p.factura_id
        )));
    }
    let tercero_id: i64 = f.get("tercero_id");
    let total_factura: i64 = f.get("total");

    // Una nota crédito no puede rebajar más de lo que la factura vale, contando
    // las notas crédito que ya tiene.
    if clase == Clase::Credito {
        let (ya,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(total), 0) FROM nota
              WHERE factura_id = ? AND clase = 'credito' AND estado = 'activa'",
        )
        .bind(p.factura_id)
        .fetch_one(pool)
        .await?;
        let disponible = total_factura - ya;
        if totales.total.0 > disponible {
            return Err(Error::Regla(format!(
                "la nota crédito es de {} y la factura {} solo admite {} más \
                 (vale {} y ya tiene {} en notas crédito)",
                totales.total,
                p.factura_id,
                Centavos(disponible.max(0)),
                Centavos(total_factura),
                Centavos(ya)
            )));
        }
    }

    let concepto_devolucion = if p.reintegra_inventario {
        Some(concepto_de_clase(pool, "devolucion_venta").await?)
    } else {
        None
    };
    // La configuración de costeo se lee ANTES de la transacción: pedir otra
    // conexión del pool con una transacción abierta interbloquea (ver el
    // comentario de `factura::emitir`).
    let mut config = std::collections::HashMap::new();
    for l in &p.lineas {
        if let std::collections::hash_map::Entry::Vacant(e) = config.entry(l.articulo_id) {
            e.insert(kardex::configuracion(pool, l.articulo_id).await?);
        }
    }

    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;

    // Numeración propia por clase. Las notas tienen su consecutivo, separado
    // del de las facturas.
    let numero: i64 = sqlx::query(
        "SELECT COALESCE(MAX(numero), 0) + 1 AS n FROM nota WHERE clase = ? AND prefijo = ?",
    )
    .bind(clase.como_texto())
    .bind(f.get::<String, _>("prefijo"))
    .fetch_one(&mut *tx)
    .await?
    .get("n");
    let prefijo: String = f.get("prefijo");

    let nota_id: i64 = sqlx::query(
        "INSERT INTO nota
            (clase, prefijo, numero, fecha, hora, factura_id, concepto_dian, motivo,
             subtotal, iva, inc, total, reintegra_inventario, usuario_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(clase.como_texto())
    .bind(&prefijo)
    .bind(numero)
    .bind(&p.fecha)
    .bind(&p.hora)
    .bind(p.factura_id)
    .bind(&p.concepto_dian)
    .bind(p.motivo.trim())
    .bind(totales.subtotal.0)
    .bind(totales.iva.0)
    .bind(totales.inc.0)
    .bind(totales.total.0)
    .bind(i64::from(p.reintegra_inventario))
    .bind(usuario_id)
    .fetch_one(&mut *tx)
    .await?
    .get("id");

    for (i, l) in totales.lineas.iter().enumerate() {
        // El reintegro mueve kardex EN LA MISMA TRANSACCIÓN.
        if let Some(concepto) = concepto_devolucion {
            kardex::registrar_en_tx(
                &mut tx,
                &kardex::Peticion {
                    fecha: p.fecha.clone(),
                    concepto_id: concepto,
                    articulo_id: l.articulo_id,
                    tercero_id: Some(tercero_id),
                    documento: format!("NC {prefijo}{numero}"),
                    cantidad: l.cantidad,
                    // Vuelve al precio de la nota; el promedio lo absorbe.
                    costo_unitario: l.precio_unitario,
                    factura_id: Some(p.factura_id),
                    usuario_id,
                    valor_comercial_unitario: None,
                },
                kardex::Configuracion {
                    // Una devolución es una ENTRADA: nunca deja saldo negativo.
                    permite_negativo: true,
                    ..config[&l.articulo_id]
                },
            )
            .await?;
        }

        let unidad: Option<String> = sqlx::query_as::<_, (Option<String>,)>(
            "SELECT unidad_medida FROM articulo WHERE id = ?",
        )
        .bind(l.articulo_id)
        .fetch_optional(&mut *tx)
        .await?
        .and_then(|(u,)| u);

        sqlx::query(
            "INSERT INTO nota_linea
                (nota_id, item, articulo_id, cantidad, precio_unitario, base,
                 iva_porcentaje, iva, inc_porcentaje, inc, total, unidad_medida)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(nota_id)
        .bind(i as i64 + 1)
        .bind(l.articulo_id)
        .bind(l.cantidad.0)
        .bind(l.precio_unitario.0)
        .bind(l.base.0)
        .bind(l.iva_porcentaje)
        .bind(l.iva.0)
        .bind(l.inc_porcentaje)
        .bind(l.inc.0)
        .bind(l.total.0)
        .bind(&unidad)
        .execute(&mut *tx)
        .await?;
    }

    // --- CUDE -------------------------------------------------------------
    let (cude, cude_falta) = calcular_cude(
        &mut tx,
        &format!("{prefijo}{numero}"),
        &p.fecha,
        &p.hora,
        tercero_id,
        &totales,
    )
    .await?;
    if let Some(c) = &cude {
        sqlx::query("UPDATE nota SET cude = ? WHERE id = ?")
            .bind(c)
            .bind(nota_id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;

    Ok(Emitida {
        id: nota_id,
        clase: clase.como_texto().into(),
        prefijo,
        numero,
        subtotal: totales.subtotal.0,
        iva: totales.iva.0,
        inc: totales.inc.0,
        total: totales.total.0,
        en_letras: inventario_dominio::letras::monto(totales.total)
            .unwrap_or_else(|| "(monto fuera de rango)".into()),
        cude,
        cude_falta,
    })
}

async fn calcular_cude(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    numero_completo: &str,
    fecha: &str,
    hora: &str,
    tercero_id: i64,
    totales: &dom::Totales,
) -> Resultado<(Option<String>, Option<String>)> {
    let empresa: Option<(String, String, Option<String>)> =
        sqlx::query_as("SELECT nit, ambiente_dian, pin_software FROM empresa WHERE id = 1")
            .fetch_optional(&mut **tx)
            .await?;
    let Some((nit_empresa, ambiente, pin)) = empresa else {
        return Ok((None, Some("la empresa no está configurada".into())));
    };
    let Some(pin) = pin.filter(|p| !p.trim().is_empty()) else {
        return Ok((
            None,
            Some(
                "falta el PIN del software (lo asigna la DIAN al registrar el software \
                 propio). Sin él no se puede calcular el CUDE de una nota"
                    .into(),
            ),
        ));
    };
    let adquirente: Option<(String,)> = sqlx::query_as("SELECT nit FROM tercero WHERE id = ?")
        .bind(tercero_id)
        .fetch_optional(&mut **tx)
        .await?;
    let Some((nit_adquirente,)) = adquirente else {
        return Ok((None, Some("el adquirente no tiene NIT".into())));
    };

    let datos = cufe::Datos {
        numero_completo,
        fecha,
        hora,
        valor_sin_impuestos: totales.subtotal,
        iva: totales.iva,
        inc: totales.inc,
        ica: Centavos::CERO,
        valor_total: totales.total,
        nit_facturador: &nit_empresa,
        nit_adquirente: &nit_adquirente,
        // Se ignora: el CUDE usa el PIN. Se pasa algo no vacío para que la
        // validación de campo no vacío no confunda el motivo.
        clave_tecnica: "n/a",
        ambiente: if ambiente == "produccion" {
            Ambiente::Produccion
        } else {
            Ambiente::Pruebas
        },
    };
    match cufe::calcular_cude(&datos, &pin) {
        Ok(c) => Ok((Some(c), None)),
        Err(e) => Ok((None, Some(e.to_string()))),
    }
}

async fn concepto_de_clase(pool: &Pool, clase: &str) -> Resultado<i64> {
    sqlx::query("SELECT id FROM concepto WHERE clase = ? AND activo = 1 ORDER BY id LIMIT 1")
        .bind(clase)
        .fetch_optional(pool)
        .await?
        .map(|f| f.get::<i64, _>("id"))
        .ok_or_else(|| {
            Error::Regla(format!(
                "no hay concepto de kardex activo de clase «{clase}»: la instalación \
                 está incompleta"
            ))
        })
}

/// Notas de una factura, para mostrarlas junto a ella.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Vista {
    pub id: i64,
    pub clase: String,
    pub prefijo: String,
    pub numero: i64,
    pub fecha: String,
    pub motivo: String,
    pub total: i64,
    pub cude: Option<String>,
    pub estado: String,
}

pub async fn de_factura(pool: &Pool, factura_id: i64) -> Resultado<Vec<Vista>> {
    let filas = sqlx::query(
        "SELECT id, clase, prefijo, numero, fecha, motivo, total, cude, estado
           FROM nota WHERE factura_id = ? ORDER BY fecha, id",
    )
    .bind(factura_id)
    .fetch_all(pool)
    .await?;
    Ok(filas
        .into_iter()
        .map(|f| Vista {
            id: f.get("id"),
            clase: f.get("clase"),
            prefijo: f.get("prefijo"),
            numero: f.get("numero"),
            fecha: f.get("fecha"),
            motivo: f.get("motivo"),
            total: f.get("total"),
            cude: f.get("cude"),
            estado: f.get("estado"),
        })
        .collect())
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::prueba_apoyo;
    use inventario_dominio::Cantidad;

    async fn con_factura(pool: &Pool) -> (i64, i64, i64) {
        let a: i64 = sqlx::query(
            "INSERT INTO articulo (codigo, descripcion, iva_porcentaje, precio_venta)
             VALUES ('A1', 'Botellón', 19, 500000) RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
        .get("id");
        let t: i64 = sqlx::query(
            "INSERT INTO tercero (nit, dv, nombre, plazo_dias) VALUES ('43627185',1,'Cli',30)
             RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
        .get("id");
        let r: i64 = sqlx::query(
            "INSERT INTO resolucion (nombre, prefijo, desde, hasta) VALUES ('R','FE',1,1000)
             RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
        .get("id");
        kardex::registrar(
            pool,
            &kardex::Peticion {
                fecha: "2026-08-01".into(),
                concepto_id: prueba_apoyo::COMPRA,
                articulo_id: a,
                tercero_id: None,
                documento: String::new(),
                cantidad: Cantidad::de_unidades(1000),
                costo_unitario: Centavos::de_pesos(3000),
                factura_id: None,
                usuario_id: None,
                valor_comercial_unitario: None,
            },
        )
        .await
        .unwrap();
        let f = crate::factura::emitir(
            pool,
            &crate::factura::Peticion {
                resolucion_id: r,
                tercero_id: t,
                vendedor_id: None,
                fecha: "2026-08-02".into(),
                hora: "10:53:10-05:00".into(),
                forma_pago: "credito".into(),
                medio_pago: None,
                retencion: 0,
                observacion: String::new(),
                lineas: vec![linea(a, 10, 10_000)],
            },
        )
        .await
        .unwrap();
        (f.id, a, t)
    }

    fn linea(a: i64, u: i64, pesos: i64) -> Linea {
        Linea {
            articulo_id: a,
            cantidad: Cantidad::de_unidades(u),
            precio_unitario: Centavos::de_pesos(pesos),
            descuento: Centavos::CERO,
            iva_porcentaje: 19,
            inc_porcentaje: 0,
        }
    }

    fn peticion(factura_id: i64, clase: &str, a: i64, u: i64, reintegra: bool) -> Peticion {
        Peticion {
            factura_id,
            clase: clase.into(),
            fecha: "2026-08-05".into(),
            hora: "09:00:00-05:00".into(),
            concepto_dian: "2".into(),
            motivo: "Devolución de mercancía".into(),
            reintegra_inventario: reintegra,
            lineas: vec![linea(a, u, 10_000)],
        }
    }

    #[tokio::test]
    async fn la_nota_credito_con_reintegro_devuelve_el_inventario_en_la_misma_transaccion() {
        let pool = prueba_apoyo::base().await;
        let (f, a, t) = con_factura(&pool).await;
        let antes = kardex::saldo(&pool, a).await.unwrap().existencia;

        let n = emitir(&pool, &peticion(f, "credito", a, 4, true), None)
            .await
            .unwrap();
        assert_eq!(n.clase, "credito");
        assert_eq!(n.total, Centavos::de_pesos(47_600).0, "40.000 + 19 %");

        let despues = kardex::saldo(&pool, a).await.unwrap().existencia;
        assert_eq!(despues - antes, 4_000, "no volvieron las 4 unidades");

        // Y la cartera bajó.
        let c = crate::factura::por_cobrar(&pool, t).await.unwrap();
        assert_eq!(c[0].notas, Centavos::de_pesos(47_600));
    }

    /// Una nota crédito sin reintegro es un ajuste de precio: **no toca el
    /// inventario**. Confundirlo infla la existencia con mercancía que nunca
    /// volvió.
    #[tokio::test]
    async fn la_nota_credito_sin_reintegro_no_toca_el_inventario() {
        let pool = prueba_apoyo::base().await;
        let (f, a, _) = con_factura(&pool).await;
        let antes = kardex::saldo(&pool, a).await.unwrap().existencia;
        emitir(&pool, &peticion(f, "credito", a, 4, false), None)
            .await
            .unwrap();
        assert_eq!(kardex::saldo(&pool, a).await.unwrap().existencia, antes);
    }

    /// La nota débito AUMENTA lo que se debe, y no devuelve nada.
    #[tokio::test]
    async fn la_nota_debito_no_devuelve_mercancia() {
        let pool = prueba_apoyo::base().await;
        let (f, a, _) = con_factura(&pool).await;
        let antes = kardex::saldo(&pool, a).await.unwrap().existencia;

        let n = emitir(&pool, &peticion(f, "debito", a, 2, false), None)
            .await
            .unwrap();
        assert_eq!(n.clase, "debito");
        assert_eq!(kardex::saldo(&pool, a).await.unwrap().existencia, antes);

        // Pedir reintegro en una débito se rechaza.
        assert!(
            emitir(&pool, &peticion(f, "debito", a, 2, true), None)
                .await
                .is_err()
        );
    }

    /// Una nota crédito no puede rebajar más de lo que la factura vale.
    #[tokio::test]
    async fn la_nota_credito_no_puede_exceder_la_factura() {
        let pool = prueba_apoyo::base().await;
        let (f, a, _) = con_factura(&pool).await; // factura de 10 unidades

        // Primera nota por 8: cabe.
        emitir(&pool, &peticion(f, "credito", a, 8, false), None)
            .await
            .unwrap();
        // Segunda por 5: ya no.
        let e = emitir(&pool, &peticion(f, "credito", a, 5, false), None)
            .await
            .unwrap_err();
        assert!(e.to_string().contains("solo admite"), "{e}");
        // Y por 2 sí.
        assert!(
            emitir(&pool, &peticion(f, "credito", a, 2, false), None)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn el_cude_se_calcula_con_el_pin_o_se_explica_por_que_no() {
        let pool = prueba_apoyo::base().await;
        let (f, a, _) = con_factura(&pool).await;

        // Sin PIN del software: no hay CUDE, y el motivo lo dice.
        let n = emitir(&pool, &peticion(f, "credito", a, 1, false), None)
            .await
            .unwrap();
        assert!(n.cude.is_none());
        assert!(n.cude_falta.as_ref().unwrap().contains("PIN"), "{:?}", n.cude_falta);

        sqlx::query("UPDATE empresa SET pin_software = '12301'")
            .execute(&pool)
            .await
            .unwrap();
        let n = emitir(&pool, &peticion(f, "credito", a, 1, false), None)
            .await
            .unwrap();
        let c = n.cude.expect("tenía que salir el CUDE");
        assert_eq!(c.len(), 96);
        assert!(c.chars().all(|x| x.is_ascii_hexdigit()));

        // Y quedó guardado.
        let (guardado,): (Option<String>,) =
            sqlx::query_as("SELECT cude FROM nota WHERE id = ?")
                .bind(n.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(guardado.as_deref(), Some(c.as_str()));
    }

    #[tokio::test]
    async fn una_nota_sin_motivo_o_sobre_una_factura_anulada_se_rechaza() {
        let pool = prueba_apoyo::base().await;
        let (f, a, _) = con_factura(&pool).await;

        let mut p = peticion(f, "credito", a, 1, false);
        p.motivo = "   ".into();
        assert!(emitir(&pool, &p, None).await.is_err());

        let mut p = peticion(f, "credito", a, 1, false);
        p.clase = "rara".into();
        assert!(emitir(&pool, &p, None).await.is_err());

        crate::factura::anular(&pool, f, "prueba", None).await.unwrap();
        let e = emitir(&pool, &peticion(f, "credito", a, 1, false), None)
            .await
            .unwrap_err();
        assert!(e.to_string().contains("anulada"), "{e}");
    }

    #[tokio::test]
    async fn las_notas_llevan_numeracion_propia_por_clase() {
        let pool = prueba_apoyo::base().await;
        let (f, a, _) = con_factura(&pool).await;
        let c1 = emitir(&pool, &peticion(f, "credito", a, 1, false), None).await.unwrap();
        let c2 = emitir(&pool, &peticion(f, "credito", a, 1, false), None).await.unwrap();
        let d1 = emitir(&pool, &peticion(f, "debito", a, 1, false), None).await.unwrap();
        assert_eq!((c1.numero, c2.numero), (1, 2));
        assert_eq!(d1.numero, 1, "la débito tiene su propio consecutivo");

        let v = de_factura(&pool, f).await.unwrap();
        assert_eq!(v.len(), 3);
    }
}
