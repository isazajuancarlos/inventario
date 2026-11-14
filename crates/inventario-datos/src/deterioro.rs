//! Deterioro del inventario sobre la base — NIC 2 párrafos 28 a 33.
//!
//! El corte se corre a una fecha y **mide todos los artículos con existencia**.
//! No es opcional por artículo: la norma manda medir el inventario, no los
//! artículos que a alguien se le ocurra revisar.

use crate::{Error, Pool, Resultado};
use inventario_dominio::{
    Cantidad, Centavos,
    vnr::{self, Partida},
};
use sqlx::Row;

#[derive(Debug, Clone, serde::Serialize)]
pub struct FilaCorte {
    pub articulo_id: i64,
    pub codigo: String,
    pub descripcion: String,
    pub existencia: i64,
    pub costo: i64,
    pub vnr: i64,
    pub valor_en_libros: i64,
    pub deterioro_acumulado: i64,
    /// Positivo = rebaja del periodo; negativo = reversión.
    pub movimiento: i64,
    /// `None` cuando el artículo no tiene precio de venta estimado. **No se
    /// mide con un cero**: se dice que falta el dato.
    pub medido: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Corte {
    pub fecha: String,
    pub filas: Vec<FilaCorte>,
    pub revelacion: Revelacion,
    /// Artículos con existencia que **no se pudieron medir** por falta de
    /// precio de venta estimado. Salen aparte y por nombre: un corte que los
    /// omitiera en silencio diría que el inventario está medido cuando no lo
    /// está (directiva 20).
    pub sin_estimacion: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct Revelacion {
    pub partidas_medidas: i64,
    pub costo_total: i64,
    pub valor_en_libros_total: i64,
    pub rebajas_del_periodo: i64,
    pub reversiones_del_periodo: i64,
    pub deterioro_acumulado_total: i64,
}

/// Corre el corte de deterioro a una fecha y **lo persiste**.
///
/// Es idempotente por fecha: volver a correrlo el mismo día reemplaza la
/// medición de ese día en vez de duplicarla. Reevaluar es lo que manda la NIC
/// 2.33, y tiene que poder hacerse dos veces si alguien corrigió un precio.
pub async fn correr_corte(pool: &Pool, fecha: &str, usuario_id: Option<i64>) -> Resultado<Corte> {
    chrono::NaiveDate::parse_from_str(fecha, "%Y-%m-%d")
        .map_err(|_| Error::Regla(format!("fecha inválida: {fecha:?} (se espera AAAA-MM-DD)")))?;

    let filas = sqlx::query(
        "SELECT a.id, a.codigo, a.descripcion, a.precio_venta_estimado, a.costos_para_vender,
                COALESCE(m.existencia_despues, 0) AS existencia,
                COALESCE(m.valor_despues, 0)      AS costo
           FROM articulo a
           LEFT JOIN movimiento m ON m.id = (
                SELECT id FROM movimiento WHERE articulo_id = a.id ORDER BY id DESC LIMIT 1)
          WHERE a.activo = 1
          ORDER BY a.descripcion",
    )
    .fetch_all(pool)
    .await?;

    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let mut salida = Vec::new();
    let mut sin_estimacion = Vec::new();
    let mut rev = Revelacion::default();

    for f in filas {
        let articulo_id: i64 = f.get("id");
        let existencia: i64 = f.get("existencia");
        let costo: i64 = f.get("costo");
        let descripcion: String = f.get("descripcion");

        // Un artículo sin existencia y sin costo no tiene nada que medir.
        if existencia == 0 && costo == 0 {
            continue;
        }

        let Some(precio) = f.get::<Option<i64>, _>("precio_venta_estimado") else {
            sin_estimacion.push(format!("{} — {}", f.get::<String, _>("codigo"), descripcion));
            continue;
        };
        let gastos = f.get::<Option<i64>, _>("costos_para_vender").unwrap_or(0);

        let partida = Partida {
            articulo_id,
            existencia: Cantidad(existencia),
            costo: Centavos(costo.max(0)),
            precio_venta_estimado: Centavos(precio),
            costos_para_vender: Centavos(gastos),
        };

        // Lo ya reconocido: la última medición ANTERIOR a esta fecha. Si se
        // vuelve a correr el mismo día, la de hoy no cuenta como previa — si no,
        // la segunda corrida revertiría la primera contra sí misma.
        let previo: Option<(i64,)> = sqlx::query_as(
            "SELECT deterioro_acumulado FROM deterioro
              WHERE articulo_id = ? AND fecha < ?
              ORDER BY fecha DESC LIMIT 1",
        )
        .bind(articulo_id)
        .bind(fecha)
        .fetch_optional(&mut *tx)
        .await?;
        let previo = Centavos(previo.map(|(v,)| v).unwrap_or(0));

        let ajuste = vnr::ajustar(&partida, previo).map_err(|e| Error::Regla(e.to_string()))?;
        let medicion = vnr::medir(&partida).map_err(|e| Error::Regla(e.to_string()))?;

        sqlx::query(
            "INSERT INTO deterioro
                (fecha, articulo_id, existencia, costo, vnr, valor_en_libros,
                 deterioro_acumulado, movimiento, usuario_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(articulo_id, fecha) DO UPDATE SET
                existencia = excluded.existencia, costo = excluded.costo, vnr = excluded.vnr,
                valor_en_libros = excluded.valor_en_libros,
                deterioro_acumulado = excluded.deterioro_acumulado,
                movimiento = excluded.movimiento",
        )
        .bind(fecha)
        .bind(articulo_id)
        .bind(existencia)
        .bind(medicion.costo.0)
        .bind(medicion.vnr.0)
        .bind(medicion.valor_en_libros.0)
        .bind(medicion.deterioro.0)
        .bind(ajuste.movimiento.0)
        .bind(usuario_id)
        .execute(&mut *tx)
        .await?;

        rev.partidas_medidas += 1;
        rev.costo_total += medicion.costo.0;
        rev.valor_en_libros_total += medicion.valor_en_libros.0;
        rev.deterioro_acumulado_total += medicion.deterioro.0;
        // Las dos cifras van SEPARADAS: la NIC 2.36 pide las rebajas y las
        // reversiones del periodo, no su diferencia.
        if ajuste.movimiento.0 > 0 {
            rev.rebajas_del_periodo += ajuste.movimiento.0;
        } else {
            rev.reversiones_del_periodo -= ajuste.movimiento.0;
        }

        salida.push(FilaCorte {
            articulo_id,
            codigo: f.get("codigo"),
            descripcion,
            existencia,
            costo: medicion.costo.0,
            vnr: medicion.vnr.0,
            valor_en_libros: medicion.valor_en_libros.0,
            deterioro_acumulado: medicion.deterioro.0,
            movimiento: ajuste.movimiento.0,
            medido: true,
        });
    }

    tx.commit().await?;
    Ok(Corte {
        fecha: fecha.to_string(),
        filas: salida,
        revelacion: rev,
        sin_estimacion,
    })
}

/// Lee un corte ya guardado, sin recalcular. Es lo que se consulta años después.
pub async fn leer_corte(pool: &Pool, fecha: &str) -> Resultado<Vec<FilaCorte>> {
    let filas = sqlx::query(
        "SELECT d.articulo_id, a.codigo, a.descripcion, d.existencia, d.costo, d.vnr,
                d.valor_en_libros, d.deterioro_acumulado, d.movimiento
           FROM deterioro d JOIN articulo a ON a.id = d.articulo_id
          WHERE d.fecha = ?
          ORDER BY a.descripcion",
    )
    .bind(fecha)
    .fetch_all(pool)
    .await?;
    Ok(filas
        .into_iter()
        .map(|f| FilaCorte {
            articulo_id: f.get("articulo_id"),
            codigo: f.get("codigo"),
            descripcion: f.get("descripcion"),
            existencia: f.get("existencia"),
            costo: f.get("costo"),
            vnr: f.get("vnr"),
            valor_en_libros: f.get("valor_en_libros"),
            deterioro_acumulado: f.get("deterioro_acumulado"),
            movimiento: f.get("movimiento"),
            medido: true,
        })
        .collect())
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::prueba_apoyo::{self, COMPRA};

    async fn articulo(pool: &Pool, codigo: &str, precio_est: Option<i64>) -> i64 {
        let id: i64 = sqlx::query(
            "INSERT INTO articulo (codigo, descripcion, precio_venta_estimado, costos_para_vender)
             VALUES (?, ?, ?, 0) RETURNING id",
        )
        .bind(codigo)
        .bind(format!("Artículo {codigo}"))
        .bind(precio_est)
        .fetch_one(pool)
        .await
        .unwrap()
        .get("id");
        id
    }

    async fn comprar(pool: &Pool, articulo_id: i64, u: i64, pesos: i64) {
        crate::kardex::registrar(
            pool,
            &crate::kardex::Peticion {
                fecha: "2026-08-01".into(),
                concepto_id: COMPRA,
                articulo_id,
                tercero_id: None,
                documento: String::new(),
                cantidad: Cantidad::de_unidades(u),
                costo_unitario: Centavos::de_pesos(pesos),
                factura_id: None,
                usuario_id: None,
                valor_comercial_unitario: None,
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn el_corte_rebaja_lo_que_vale_menos_y_deja_al_costo_lo_que_vale_mas() {
        let pool = prueba_apoyo::base().await;
        // Costó $1.000 y hoy se vende a $700 → se rebaja.
        let malo = articulo(&pool, "MALO", Some(Centavos::de_pesos(700).0)).await;
        comprar(&pool, malo, 10, 1_000).await;
        // Costó $1.000 y hoy se vende a $5.000 → se queda al costo.
        let bueno = articulo(&pool, "BUENO", Some(Centavos::de_pesos(5_000).0)).await;
        comprar(&pool, bueno, 10, 1_000).await;

        let c = correr_corte(&pool, "2026-08-31", None).await.unwrap();
        let f = |cod: &str| c.filas.iter().find(|x| x.codigo == cod).unwrap().clone();

        assert_eq!(f("MALO").deterioro_acumulado, Centavos::de_pesos(3_000).0);
        assert_eq!(f("MALO").valor_en_libros, Centavos::de_pesos(7_000).0);
        assert_eq!(f("BUENO").deterioro_acumulado, 0);
        assert_eq!(
            f("BUENO").valor_en_libros,
            Centavos::de_pesos(10_000).0,
            "el inventario NO se revalúa por encima del costo"
        );
        assert_eq!(c.revelacion.rebajas_del_periodo, Centavos::de_pesos(3_000).0);
        assert_eq!(c.revelacion.reversiones_del_periodo, 0);
    }

    /// NIC 2.33: reevaluar en el periodo siguiente y **revertir** si las causas
    /// dejaron de existir. Es lo que separa un deterioro que se calcula una vez
    /// de uno que se lleva de verdad.
    #[tokio::test]
    async fn el_corte_siguiente_revierte_cuando_el_precio_se_recupera() {
        let pool = prueba_apoyo::base().await;
        let a = articulo(&pool, "A1", Some(Centavos::de_pesos(700).0)).await;
        comprar(&pool, a, 10, 1_000).await;

        let c1 = correr_corte(&pool, "2026-08-31", None).await.unwrap();
        assert_eq!(c1.revelacion.rebajas_del_periodo, Centavos::de_pesos(3_000).0);

        // El precio se recupera a $900.
        sqlx::query("UPDATE articulo SET precio_venta_estimado = ? WHERE id = ?")
            .bind(Centavos::de_pesos(900).0)
            .bind(a)
            .execute(&pool)
            .await
            .unwrap();
        let c2 = correr_corte(&pool, "2026-09-30", None).await.unwrap();
        assert_eq!(c2.revelacion.reversiones_del_periodo, Centavos::de_pesos(2_000).0);
        assert_eq!(c2.revelacion.rebajas_del_periodo, 0);
        assert_eq!(c2.filas[0].deterioro_acumulado, Centavos::de_pesos(1_000).0);

        // Y por encima del costo NO sube.
        sqlx::query("UPDATE articulo SET precio_venta_estimado = ? WHERE id = ?")
            .bind(Centavos::de_pesos(9_000).0)
            .bind(a)
            .execute(&pool)
            .await
            .unwrap();
        let c3 = correr_corte(&pool, "2026-10-31", None).await.unwrap();
        assert_eq!(c3.filas[0].deterioro_acumulado, 0);
        assert_eq!(c3.filas[0].valor_en_libros, Centavos::de_pesos(10_000).0);
        assert_eq!(
            c3.revelacion.reversiones_del_periodo,
            Centavos::de_pesos(1_000).0,
            "solo se revierte lo que quedaba, no más"
        );
    }

    /// Un artículo sin precio estimado NO se mide con cero: se reporta aparte.
    /// Medirlo con cero lo deterioraría al 100 % y el balance diría una
    /// barbaridad con toda la apariencia de estar calculada.
    #[tokio::test]
    async fn lo_que_no_se_puede_medir_se_dice_en_vez_de_medirse_con_cero() {
        let pool = prueba_apoyo::base().await;
        let a = articulo(&pool, "SINPRECIO", None).await;
        comprar(&pool, a, 10, 1_000).await;

        let c = correr_corte(&pool, "2026-08-31", None).await.unwrap();
        assert!(c.filas.is_empty(), "no debería haberse medido");
        assert_eq!(c.sin_estimacion.len(), 1);
        assert!(c.sin_estimacion[0].contains("SINPRECIO"));
        assert_eq!(c.revelacion.partidas_medidas, 0);
    }

    #[tokio::test]
    async fn correr_el_mismo_corte_dos_veces_no_lo_duplica_ni_lo_revierte_contra_si_mismo() {
        let pool = prueba_apoyo::base().await;
        let a = articulo(&pool, "A1", Some(Centavos::de_pesos(700).0)).await;
        comprar(&pool, a, 10, 1_000).await;

        let c1 = correr_corte(&pool, "2026-08-31", None).await.unwrap();
        let c2 = correr_corte(&pool, "2026-08-31", None).await.unwrap();
        assert_eq!(c1.revelacion.rebajas_del_periodo, c2.revelacion.rebajas_del_periodo);
        assert_eq!(c1.filas[0].movimiento, c2.filas[0].movimiento);

        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM deterioro")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 1, "se duplicó la medición del mismo día");
    }

    #[tokio::test]
    async fn el_corte_guardado_se_lee_sin_recalcular() {
        let pool = prueba_apoyo::base().await;
        let a = articulo(&pool, "A1", Some(Centavos::de_pesos(700).0)).await;
        comprar(&pool, a, 10, 1_000).await;
        correr_corte(&pool, "2026-08-31", None).await.unwrap();

        // Aunque el precio cambie después, el corte guardado dice lo que dijo.
        sqlx::query("UPDATE articulo SET precio_venta_estimado = 999999 WHERE id = ?")
            .bind(a)
            .execute(&pool)
            .await
            .unwrap();
        let leido = leer_corte(&pool, "2026-08-31").await.unwrap();
        assert_eq!(leido.len(), 1);
        assert_eq!(leido[0].deterioro_acumulado, Centavos::de_pesos(3_000).0);
        assert_eq!(leido[0].vnr, Centavos::de_pesos(7_000).0);
    }

    #[tokio::test]
    async fn una_fecha_invalida_no_corre_el_corte() {
        let pool = prueba_apoyo::base().await;
        assert!(correr_corte(&pool, "31/08/2026", None).await.is_err());
        assert!(correr_corte(&pool, "", None).await.is_err());
    }
}
