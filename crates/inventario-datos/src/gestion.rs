//! Informes de gestión: rotación, clasificación ABC y punto de reorden.
//!
//! No son normativos, pero son la razón por la que alguien mira un sistema de
//! inventario a diario. Todos se calculan **sobre el kardex real**, que es lo
//! que el sistema anterior no podía hacer porque su existencia era ficción.
//!
//! El punto de reorden sustituye al «punto mínimo» digitado a mano: en el
//! corpus heredado, **1 artículo de 148** lo tenía puesto, porque un número que
//! hay que mantener a mano no se mantiene.

use crate::{Error, Pool, Resultado};
use inventario_dominio::dinero::dividir_redondeando;
use sqlx::Row;

// ---------------------------------------------------------------------------
// Rotación
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct FilaRotacion {
    pub articulo_id: i64,
    pub codigo: String,
    pub descripcion: String,
    /// Costo de lo que salió por venta en el periodo.
    pub costo_de_ventas: i64,
    pub inventario_promedio: i64,
    /// Veces que rota en el periodo, **en centésimas** (350 = 3,50 veces). En
    /// enteros para no meter coma flotante en un informe que se compara.
    pub rotacion_centesimas: i64,
    /// Días que tarda en venderse el inventario. `None` cuando no rota: dividir
    /// entre cero daría infinito, y «infinito» no es un dato que nadie pueda
    /// usar. Un artículo que no rotó se informa como tal.
    pub dias_inventario: Option<i64>,
}

/// Rotación del periodo: `costo de ventas / inventario promedio`.
///
/// El inventario promedio se toma como `(inicial + final) / 2`, que es la
/// convención corriente. No es la media de los saldos diarios —eso sería más
/// preciso y menos comparable con lo que publica cualquier otro informe—.
pub async fn rotacion(pool: &Pool, desde: &str, hasta: &str) -> Resultado<Vec<FilaRotacion>> {
    for f in [desde, hasta] {
        chrono::NaiveDate::parse_from_str(f, "%Y-%m-%d")
            .map_err(|_| Error::Regla(format!("fecha inválida: {f:?}")))?;
    }
    let dias = (chrono::NaiveDate::parse_from_str(hasta, "%Y-%m-%d").unwrap()
        - chrono::NaiveDate::parse_from_str(desde, "%Y-%m-%d").unwrap())
    .num_days()
        + 1;
    if dias <= 0 {
        return Err(Error::Regla(format!("el periodo está al revés: {desde} .. {hasta}")));
    }

    let filas = sqlx::query(
        "SELECT a.id, a.codigo, a.descripcion,
                COALESCE((SELECT SUM(m.valor) FROM movimiento m
                            JOIN concepto c ON c.id = m.concepto_id
                           WHERE m.articulo_id = a.id AND c.clase = 'venta'
                             AND m.fecha BETWEEN ?1 AND ?2), 0) AS costo_ventas,
                COALESCE((SELECT m.valor_despues FROM movimiento m
                           WHERE m.articulo_id = a.id AND m.fecha < ?1
                           ORDER BY m.id DESC LIMIT 1), 0) AS valor_inicial,
                COALESCE((SELECT m.valor_despues FROM movimiento m
                           WHERE m.articulo_id = a.id AND m.fecha <= ?2
                           ORDER BY m.id DESC LIMIT 1), 0) AS valor_final
           FROM articulo a
          WHERE a.activo = 1
          ORDER BY a.descripcion",
    )
    .bind(desde)
    .bind(hasta)
    .fetch_all(pool)
    .await?;

    Ok(filas
        .into_iter()
        .map(|f| {
            let costo_de_ventas: i64 = f.get("costo_ventas");
            let inicial: i64 = f.get("valor_inicial");
            let final_: i64 = f.get("valor_final");
            let promedio = (inicial + final_) / 2;
            // INDICADOR, no un importe que alguien cobre: si no cabe, la
            // lectura honesta es «no hay rotación que mostrar», y por eso el
            // cero — que es el mismo valor que ya se usa cuando el promedio no
            // es positivo. Aquí no se rechaza la consulta entera por una fila.
            let rotacion_centesimas = if promedio <= 0 {
                0
            } else {
                dividir_redondeando(costo_de_ventas as i128 * 100, promedio as i128).unwrap_or(0)
            };
            FilaRotacion {
                articulo_id: f.get("id"),
                codigo: f.get("codigo"),
                descripcion: f.get("descripcion"),
                costo_de_ventas,
                inventario_promedio: promedio,
                rotacion_centesimas,
                // `flatten`: el `Option` externo dice «no hay rotación», el
                // interno «no cabe». Los dos significan lo mismo para quien lee
                // el informe —no hay días que mostrar— y por eso se juntan.
                dias_inventario: (rotacion_centesimas > 0)
                    .then(|| dividir_redondeando(dias as i128 * 100, rotacion_centesimas as i128))
                    .flatten(),
            }
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Clasificación ABC
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct FilaAbc {
    pub articulo_id: i64,
    pub codigo: String,
    pub descripcion: String,
    pub valor_consumido: i64,
    /// Participación en el total, en centésimas de punto (1250 = 12,50 %).
    pub participacion: i64,
    /// Participación acumulada, en centésimas de punto.
    pub acumulado: i64,
    pub clase: char,
}

/// Clasificación ABC por valor consumido en el periodo (Pareto).
///
/// Cortes en 80 % y 95 % del acumulado. Son la convención; se dejan como
/// constantes con nombre para que se vean y se puedan discutir, en vez de
/// aparecer como números sueltos dentro de un `if`.
const CORTE_A: i64 = 8_000; // 80,00 %
const CORTE_B: i64 = 9_500; // 95,00 %

pub async fn abc(pool: &Pool, desde: &str, hasta: &str) -> Resultado<Vec<FilaAbc>> {
    for f in [desde, hasta] {
        chrono::NaiveDate::parse_from_str(f, "%Y-%m-%d")
            .map_err(|_| Error::Regla(format!("fecha inválida: {f:?}")))?;
    }
    let filas = sqlx::query(
        "SELECT a.id, a.codigo, a.descripcion,
                COALESCE(SUM(m.valor), 0) AS valor
           FROM articulo a
           LEFT JOIN movimiento m ON m.articulo_id = a.id AND m.fecha BETWEEN ? AND ?
           LEFT JOIN concepto c ON c.id = m.concepto_id AND c.clase = 'venta'
          WHERE a.activo = 1 AND c.id IS NOT NULL
          GROUP BY a.id
          ORDER BY valor DESC",
    )
    .bind(desde)
    .bind(hasta)
    .fetch_all(pool)
    .await?;

    let total: i64 = filas.iter().map(|f| f.get::<i64, _>("valor")).sum();
    if total <= 0 {
        // Sin consumo no hay Pareto que hacer. Se devuelve vacío en vez de
        // clasificarlo todo como «C», que sugeriría que se midió algo.
        return Ok(Vec::new());
    }

    let mut acumulado = 0i64;
    Ok(filas
        .into_iter()
        .map(|f| {
            let valor: i64 = f.get("valor");
            // Participación en diezmilésimas sobre el total: por definición no
            // pasa de 10.000, así que no cabe solo si `total` es absurdo frente
            // a `valor`. Cero es la lectura conservadora para un ABC.
            let participacion =
                dividir_redondeando(valor as i128 * 10_000, total as i128).unwrap_or(0);
            acumulado += participacion;
            FilaAbc {
                articulo_id: f.get("id"),
                codigo: f.get("codigo"),
                descripcion: f.get("descripcion"),
                valor_consumido: valor,
                participacion,
                acumulado,
                clase: if acumulado <= CORTE_A {
                    'A'
                } else if acumulado <= CORTE_B {
                    'B'
                } else {
                    'C'
                },
            }
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Punto de reorden con stock de seguridad
// ---------------------------------------------------------------------------

/// Factor `z` según el nivel de servicio, en centésimas (165 = 1,65).
///
/// Lista **cerrada** a propósito: interpolar la inversa de la normal para un
/// nivel arbitrario daría una precisión falsa sobre una demanda que casi nunca
/// es normal. Se ofrecen los niveles que se usan y se rechaza el resto.
fn z_centesimas(nivel_servicio: i64) -> Option<i64> {
    Some(match nivel_servicio {
        50 => 0,
        80 => 84,
        85 => 104,
        90 => 128,
        95 => 165,
        98 => 205,
        99 => 233,
        _ => None?,
    })
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FilaReorden {
    pub articulo_id: i64,
    pub codigo: String,
    pub descripcion: String,
    pub existencia: i64,
    /// Demanda media diaria, en milésimas de unidad.
    pub demanda_diaria: i64,
    /// Desviación estándar de la demanda diaria, en milésimas.
    pub desviacion: i64,
    pub stock_seguridad: i64,
    pub punto_reorden: i64,
    /// `true` si hay que pedir ya.
    pub reponer: bool,
    pub sugerido: i64,
    /// Cuántos días de historia respaldan el cálculo. Va en el informe porque
    /// un punto de reorden calculado sobre tres días no vale lo mismo que uno
    /// sobre noventa, y quien lo mira tiene derecho a saberlo.
    pub dias_de_historia: i64,
}

/// Punto de reorden estadístico:
///
/// ```text
/// SS  = z × σ_demanda_diaria × √(tiempo de reposición)
/// ROP = demanda_media_diaria × tiempo de reposición + SS
/// ```
///
/// Devuelve error si el nivel de servicio no está en la tabla, en vez de
/// interpolar un `z` inventado.
pub async fn punto_reorden(
    pool: &Pool,
    desde: &str,
    hasta: &str,
    dias_reposicion: i64,
    nivel_servicio: i64,
) -> Resultado<Vec<FilaReorden>> {
    let z = z_centesimas(nivel_servicio).ok_or_else(|| {
        Error::Regla(format!(
            "nivel de servicio {nivel_servicio} % no está tabulado: use 50, 80, 85, 90, 95, 98 o 99"
        ))
    })?;
    if dias_reposicion <= 0 {
        return Err(Error::Regla("el tiempo de reposición debe ser de al menos un día".into()));
    }
    let d0 = chrono::NaiveDate::parse_from_str(desde, "%Y-%m-%d")
        .map_err(|_| Error::Regla(format!("fecha inválida: {desde:?}")))?;
    let d1 = chrono::NaiveDate::parse_from_str(hasta, "%Y-%m-%d")
        .map_err(|_| Error::Regla(format!("fecha inválida: {hasta:?}")))?;
    let dias = (d1 - d0).num_days() + 1;
    if dias <= 0 {
        return Err(Error::Regla(format!("el periodo está al revés: {desde} .. {hasta}")));
    }

    // Salidas por venta agrupadas por artículo y día. La varianza se calcula
    // sobre TODOS los días del periodo, no solo los que tuvieron venta: un día
    // sin venta es demanda cero, no un dato ausente. Contarlo mal subestima la
    // variabilidad y deja el stock de seguridad corto.
    let filas = sqlx::query(
        "SELECT a.id, a.codigo, a.descripcion,
                COALESCE((SELECT m.existencia_despues FROM movimiento m
                           WHERE m.articulo_id = a.id ORDER BY m.id DESC LIMIT 1), 0) AS existencia
           FROM articulo a WHERE a.activo = 1 ORDER BY a.descripcion",
    )
    .fetch_all(pool)
    .await?;

    let mut salida = Vec::new();
    for f in filas {
        let articulo_id: i64 = f.get("id");
        let por_dia: Vec<(String, i64)> = sqlx::query_as(
            "SELECT m.fecha, SUM(m.cantidad)
               FROM movimiento m JOIN concepto c ON c.id = m.concepto_id
              WHERE m.articulo_id = ? AND c.clase = 'venta' AND m.fecha BETWEEN ? AND ?
              GROUP BY m.fecha",
        )
        .bind(articulo_id)
        .bind(desde)
        .bind(hasta)
        .fetch_all(pool)
        .await?;
        if por_dia.is_empty() {
            continue;
        }

        let total: i64 = por_dia.iter().map(|(_, c)| *c).sum();
        let media = total / dias;
        // Varianza poblacional sobre los `dias` del periodo, contando los días
        // sin venta como cero.
        let suma_cuadrados: i128 = por_dia
            .iter()
            .map(|(_, c)| {
                let d = (*c - media) as i128;
                d * d
            })
            .sum::<i128>()
            + (dias - por_dia.len() as i64).max(0) as i128 * (media as i128).pow(2);
        let varianza = suma_cuadrados / dias as i128;
        let desviacion = raiz_entera(varianza);

        // SS = z × σ × √LT. Todo en enteros: z viene en centésimas y la raíz
        // del tiempo de reposición se escala por 1.000 para no perderla.
        let raiz_lt = raiz_entera((dias_reposicion as i128) * 1_000_000); // ×1000
        let ss = (z as i128 * desviacion as i128 * raiz_lt as i128) / (100 * 1_000);
        let stock_seguridad = ss as i64;
        let punto = media * dias_reposicion + stock_seguridad;
        let existencia: i64 = f.get("existencia");

        salida.push(FilaReorden {
            articulo_id,
            codigo: f.get("codigo"),
            descripcion: f.get("descripcion"),
            existencia,
            demanda_diaria: media,
            desviacion,
            stock_seguridad,
            punto_reorden: punto,
            reponer: existencia <= punto,
            // Reponer hasta cubrir el punto de reorden más otro ciclo de
            // demanda: es lo mínimo que evita volver a pedir al día siguiente.
            sugerido: ((punto + media * dias_reposicion) - existencia).max(0),
            dias_de_historia: dias,
        });
    }
    Ok(salida)
}

/// Raíz cuadrada entera por Newton. Sin coma flotante: un punto de reorden es
/// un dato operativo que se compara entre corridas, y `f64` haría que la misma
/// entrada diera resultados distintos en máquinas distintas.
fn raiz_entera(n: i128) -> i64 {
    if n <= 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x as i64
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::prueba_apoyo::{self, COMPRA, VENTA};
    use inventario_dominio::{Cantidad, Centavos};

    async fn articulo(pool: &Pool, codigo: &str) -> i64 {
        sqlx::query("INSERT INTO articulo (codigo, descripcion) VALUES (?, ?) RETURNING id")
            .bind(codigo)
            .bind(format!("Artículo {codigo}"))
            .fetch_one(pool)
            .await
            .unwrap()
            .get("id")
    }

    async fn mover(pool: &Pool, concepto: i64, a: i64, u: i64, pesos: i64, fecha: &str) {
        crate::kardex::registrar(
            pool,
            &crate::kardex::Peticion {
                fecha: fecha.into(),
                concepto_id: concepto,
                articulo_id: a,
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

    #[test]
    fn la_raiz_entera_es_exacta_en_los_cuadrados_y_trunca_en_lo_demas() {
        assert_eq!(raiz_entera(0), 0);
        assert_eq!(raiz_entera(1), 1);
        assert_eq!(raiz_entera(4), 2);
        assert_eq!(raiz_entera(9), 3);
        assert_eq!(raiz_entera(10_000), 100);
        assert_eq!(raiz_entera(2), 1, "trunca");
        assert_eq!(raiz_entera(8), 2);
        assert_eq!(raiz_entera(-5), 0);
        // 1e12 → 1e6, que es el caso del escalado por mil.
        assert_eq!(raiz_entera(1_000_000_000_000), 1_000_000);
    }

    #[tokio::test]
    async fn la_rotacion_sale_del_kardex_y_los_dias_se_derivan_de_ella() {
        let pool = prueba_apoyo::base().await;
        let a = articulo(&pool, "A1").await;
        mover(&pool, COMPRA, a, 100, 1_000, "2026-01-01").await;
        mover(&pool, VENTA, a, 50, 0, "2026-01-15").await;

        let r = rotacion(&pool, "2026-01-01", "2026-12-31").await.unwrap();
        let f = r.iter().find(|x| x.articulo_id == a).unwrap();
        // Costo de ventas 50.000; inventario promedio (0 + 50.000)/2 = 25.000.
        assert_eq!(f.costo_de_ventas, Centavos::de_pesos(50_000).0);
        assert_eq!(f.inventario_promedio, Centavos::de_pesos(25_000).0);
        assert_eq!(f.rotacion_centesimas, 200, "rota 2,00 veces");
        assert_eq!(f.dias_inventario, Some(365 / 2 + 1), "365 días / 2 vueltas");
    }

    /// Un artículo que no rotó se informa como tal. Dividir entre cero daría
    /// «infinito», que no es un dato que nadie pueda usar.
    #[tokio::test]
    async fn un_articulo_que_no_rota_no_produce_infinito() {
        let pool = prueba_apoyo::base().await;
        let a = articulo(&pool, "QUIETO").await;
        mover(&pool, COMPRA, a, 100, 1_000, "2026-01-01").await;

        let r = rotacion(&pool, "2026-01-01", "2026-12-31").await.unwrap();
        let f = r.iter().find(|x| x.articulo_id == a).unwrap();
        assert_eq!(f.rotacion_centesimas, 0);
        assert_eq!(f.dias_inventario, None);
    }

    #[tokio::test]
    async fn el_abc_ordena_por_valor_y_corta_en_ochenta_y_noventa_y_cinco() {
        let pool = prueba_apoyo::base().await;
        let grande = articulo(&pool, "GRANDE").await;
        let medio = articulo(&pool, "MEDIO").await;
        let chico = articulo(&pool, "CHICO").await;
        for (a, u) in [(grande, 800i64), (medio, 150), (chico, 50)] {
            mover(&pool, COMPRA, a, 1_000, 1_000, "2026-01-01").await;
            mover(&pool, VENTA, a, u, 0, "2026-01-10").await;
        }

        let r = abc(&pool, "2026-01-01", "2026-12-31").await.unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].articulo_id, grande, "ordenado por valor descendente");
        assert_eq!(r[0].clase, 'A', "80 % del valor");
        assert_eq!(r[1].clase, 'B');
        assert_eq!(r[2].clase, 'C');
        assert_eq!(r[0].participacion, 8_000, "80,00 %");
        assert_eq!(r[2].acumulado, 10_000, "el acumulado cierra en 100 %");
    }

    /// Sin consumo no hay Pareto. Clasificarlo todo como «C» sugeriría que se
    /// midió algo.
    #[tokio::test]
    async fn sin_consumo_el_abc_devuelve_vacio_en_vez_de_clasificarlo_todo_en_c() {
        let pool = prueba_apoyo::base().await;
        let a = articulo(&pool, "A1").await;
        mover(&pool, COMPRA, a, 100, 1_000, "2026-01-01").await;
        assert!(abc(&pool, "2026-01-01", "2026-12-31").await.unwrap().is_empty());
    }

    /// El stock de seguridad crece con la variabilidad: dos artículos con la
    /// MISMA demanda total pero distinta dispersión no necesitan el mismo
    /// colchón. Es justo lo que un «punto mínimo» digitado no puede expresar.
    #[tokio::test]
    async fn el_stock_de_seguridad_crece_con_la_variabilidad() {
        let pool = prueba_apoyo::base().await;
        let parejo = articulo(&pool, "PAREJO").await;
        let irregular = articulo(&pool, "IRREGULAR").await;
        mover(&pool, COMPRA, parejo, 10_000, 1_000, "2026-01-01").await;
        mover(&pool, COMPRA, irregular, 10_000, 1_000, "2026-01-01").await;

        // Mismo total (100 unidades en 10 días), muy distinta dispersión.
        for d in 2..=11 {
            mover(&pool, VENTA, parejo, 10, 0, &format!("2026-01-{d:02}")).await;
        }
        mover(&pool, VENTA, irregular, 100, 0, "2026-01-05").await;

        let r = punto_reorden(&pool, "2026-01-02", "2026-01-11", 5, 95).await.unwrap();
        let ss = |id: i64| r.iter().find(|x| x.articulo_id == id).unwrap().stock_seguridad;
        assert!(
            ss(irregular) > ss(parejo),
            "el irregular ({}) debería necesitar más colchón que el parejo ({})",
            ss(irregular),
            ss(parejo)
        );
        // Y el parejo, con demanda constante, casi no necesita colchón.
        assert_eq!(ss(parejo), 0, "sin variabilidad no hace falta stock de seguridad");
    }

    /// Un nivel de servicio más alto exige más colchón. Si no, el parámetro no
    /// estaría haciendo nada.
    #[tokio::test]
    async fn subir_el_nivel_de_servicio_sube_el_stock_de_seguridad() {
        let pool = prueba_apoyo::base().await;
        let a = articulo(&pool, "A1").await;
        mover(&pool, COMPRA, a, 10_000, 1_000, "2026-01-01").await;
        for (d, u) in [(2, 5i64), (3, 50), (4, 1), (5, 80), (6, 10)] {
            mover(&pool, VENTA, a, u, 0, &format!("2026-01-{d:02}")).await;
        }
        let ss = |nivel: i64| {
            let pool = pool.clone();
            async move {
                punto_reorden(&pool, "2026-01-02", "2026-01-06", 5, nivel)
                    .await
                    .unwrap()[0]
                    .stock_seguridad
            }
        };
        let s90 = ss(90).await;
        let s95 = ss(95).await;
        let s99 = ss(99).await;
        assert!(s90 < s95 && s95 < s99, "90:{s90} 95:{s95} 99:{s99}");
        assert_eq!(ss(50).await, 0, "al 50 % de servicio, z = 0");
    }

    /// Un nivel de servicio que no está tabulado se rechaza en vez de
    /// interpolar un `z` inventado.
    #[tokio::test]
    async fn un_nivel_de_servicio_no_tabulado_se_rechaza() {
        let pool = prueba_apoyo::base().await;
        let e = punto_reorden(&pool, "2026-01-01", "2026-01-31", 5, 93)
            .await
            .unwrap_err();
        assert!(e.to_string().contains("tabulado"), "{e}");
        assert!(punto_reorden(&pool, "2026-01-01", "2026-01-31", 0, 95).await.is_err());
        assert!(punto_reorden(&pool, "2026-01-31", "2026-01-01", 5, 95).await.is_err());
    }
}
