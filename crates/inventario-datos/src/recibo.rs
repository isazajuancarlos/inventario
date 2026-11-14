//! Recibos de caja: el recaudo de la cartera.
//!
//! El detalle **no existe sin cabecera**. En el corpus, 313 de 323 detalles de
//! recibo sobrevivían a la cabecera borrada: los abonos existían y el
//! documento que los explicaba, no. Aquí lo garantiza el esquema
//! (`ON DELETE CASCADE` con las claves foráneas encendidas) y lo comprueba
//! `borrar_el_recibo_se_lleva_sus_aplicaciones`.

use crate::{Error, Pool, Resultado, factura};
use inventario_dominio::{Centavos, cartera};
use sqlx::Row;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Pago {
    pub medio: String,
    #[serde(default)]
    pub banco_id: Option<i64>,
    #[serde(default)]
    pub referencia: String,
    #[serde(default)]
    pub vence: Option<String>,
    pub valor: i64,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Peticion {
    pub tercero_id: i64,
    pub fecha: String,
    #[serde(default)]
    pub observacion: String,
    /// Cómo llega el dinero. La suma tiene que dar el total del recibo.
    pub pagos: Vec<Pago>,
    /// A qué facturas se aplica. Vacío = repartir de la más vieja a la más
    /// nueva. Que el reparto automático sea opcional es deliberado: el cajero
    /// tiene que poder decir «estos $50.000 van a la factura 3214».
    #[serde(default)]
    pub aplicaciones: Vec<(i64, i64)>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Recibido {
    pub id: i64,
    pub numero: i64,
    pub total: i64,
    pub aplicado: i64,
    /// Lo que no se pudo aplicar porque el cliente no debía tanto. **No se
    /// encaja en ninguna factura**: queda dicho, y quien opera decide.
    pub sobrante: i64,
    pub en_letras: String,
}


pub async fn registrar(pool: &Pool, p: &Peticion, usuario_id: Option<i64>) -> Resultado<Recibido> {
    if p.pagos.is_empty() {
        return Err(Error::Regla("un recibo sin pagos no es un recibo".into()));
    }
    let mut total = Centavos::CERO;
    for pago in &p.pagos {
        if !inventario_dominio::factura::medio_de_pago_conocido(&pago.medio) {
            return Err(Error::Regla(format!(
                "medio de pago desconocido: {:?} (uno de {})",
                pago.medio,
                inventario_dominio::factura::medios_de_pago_admitidos()
            )));
        }
        if pago.valor <= 0 {
            return Err(Error::Regla(format!(
                "el pago en {} es de {}, y un pago tiene que ser positivo",
                pago.medio,
                Centavos(pago.valor)
            )));
        }
        // `sumar` y no `+`: el `+` desborda —pánico en los dos perfiles con
        // `overflow-checks = true`— y aquí se
        // guardaba un recibo cuyo total no era la suma de sus pagos. Ver el
        // doc-comment de `Centavos::sumar` — está medido por HTTP, no supuesto.
        let Some(nuevo) = total.sumar(Centavos(pago.valor)) else {
            return Err(Error::Regla(
                "la suma de los pagos no cabe en el importe de un recibo".into(),
            ));
        };
        total = nuevo;
    }

    let pendientes = factura::por_cobrar(pool, p.tercero_id).await?;

    // Reparto: el que pidieron o el automático.
    let aplicaciones: Vec<(i64, Centavos)> = if p.aplicaciones.is_empty() {
        cartera::repartir(&pendientes, total)?.aplicaciones
    } else {
        let a: Vec<(i64, Centavos)> = p
            .aplicaciones
            .iter()
            .map(|(id, v)| (*id, Centavos(*v)))
            .collect();
        // SUMA COMPROBADA, no `.sum()`. El `Sum` de `Centavos` es un `+` sin
        // comprobar, y esto entra de FUERA: son los pares (factura, importe) que
        // manda el formulario de recaudo.
        //
        // Aquí escribí primero «en release da la vuelta», y en este árbol es
        // FALSO: `Cargo.toml` lleva `[profile.release] overflow-checks = true`
        // desde el 2026-08-12, así que el desbordamiento es PÁNICO en los dos
        // perfiles. Lo midió la revisión de seguridad del 2026-08-13 sobre el
        // artefacto. El arreglo sigue siendo el correcto —el propio `Cargo.toml`
        // dice que un rechazo limpio es mejor que un pánico— pero lo que evita
        // es un 500, no un dato corrupto.
        //
        // Y esto llega de fuera: `p.aplicaciones` son los pares (factura,
        // importe) que manda el formulario de recaudo. Es el hermano del
        // desbordamiento de `multiplicar`, a diecinueve líneas del que se
        // arregló primero.
        let mut suma = Centavos::CERO;
        for (_, v) in &a {
            suma = suma.sumar(*v).ok_or_else(|| {
                Error::Regla(
                    "las aplicaciones suman un importe que no cabe en centavos; \
                     revise las cifras"
                        .into(),
                )
            })?;
        }
        // Un reparto a mano puede aplicar MENOS que el recibo (el resto queda a
        // favor); lo que no puede es aplicar de más ni tocar lo que no debe.
        if suma > total {
            return Err(Error::Regla(format!(
                "las aplicaciones suman {suma} y el recibo es de {total}"
            )));
        }
        cartera::validar_reparto(&pendientes, suma, &a)?;
        a
    };

    // SUMA COMPROBADA también aquí, y no por simetría: éste es un `.sum()` crudo
    // que quedó DOS LÍNEAS por debajo del comentario que explica por qué el
    // `.sum()` crudo es peligroso. Lo señaló la revisión de seguridad, y tenía
    // razón: hoy es seguro solo por invariantes que viven en OTRO archivo
    // —`cartera::validar_reparto` exige que la suma sea la del recibo, y
    // `cartera::repartir` la acota por construcción—, así que su seguridad
    // depende de que nadie toque `cartera.rs` sin acordarse de esto.
    //
    // **ES DEFENSA EN PROFUNDIDAD, Y HOY NO SE PUEDE ALCANZAR. Escrito así
    // porque lo intenté probar y no pude.**
    //
    // Escribí una prueba para esta guarda y la mutación la dejó VERDE: con el
    // `.sum()` crudo de vuelta no se pone roja, porque el camino manual ya
    // rechaza arriba (`suma > total`) y el automático viene acotado a `total`
    // por `cartera::repartir`. O sea que la prueba medía la guarda ANTERIOR con
    // otro nombre, que es peor que no tenerla. Se retiró (directiva 33).
    //
    // La comprobación se queda: su valor es que la seguridad de esta línea no
    // dependa de dos invariantes que viven en `cartera.rs` y que nadie recuerda
    // al editarlas. Pero queda dicho que **no está medida**, en vez de dejar un
    // `ok_or` que aparente cobertura.
    let mut aplicado = Centavos::CERO;
    for (_, v) in &aplicaciones {
        aplicado = aplicado.sumar(*v).ok_or_else(|| {
            Error::Regla(
                "el total aplicado no cabe en centavos; revise el reparto del recibo".into(),
            )
        })?;
    }

    // `IMMEDIATE`: el consecutivo del recibo se LEE y luego se escribe. Con
    // una transacción diferida, dos cajeros recaudando a la vez toman el mismo
    // número y el segundo muere con «database is locked».
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let numero: i64 = sqlx::query("SELECT COALESCE(MAX(numero), 0) + 1 AS n FROM recibo")
        .fetch_one(&mut *tx)
        .await?
        .get("n");

    let id: i64 = sqlx::query(
        "INSERT INTO recibo (numero, fecha, tercero_id, total, observacion, usuario_id)
         VALUES (?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(numero)
    .bind(&p.fecha)
    .bind(p.tercero_id)
    .bind(total.0)
    .bind(&p.observacion)
    .bind(usuario_id)
    .fetch_one(&mut *tx)
    .await?
    .get("id");

    for pago in &p.pagos {
        sqlx::query(
            "INSERT INTO recibo_pago (recibo_id, medio, banco_id, referencia, vence, valor)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(&pago.medio)
        .bind(pago.banco_id)
        .bind(&pago.referencia)
        .bind(&pago.vence)
        .bind(pago.valor)
        .execute(&mut *tx)
        .await?;
    }

    for (factura_id, valor) in &aplicaciones {
        sqlx::query(
            "INSERT INTO recibo_aplicacion (recibo_id, factura_id, valor) VALUES (?, ?, ?)",
        )
        .bind(id)
        .bind(factura_id)
        .bind(valor.0)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    Ok(Recibido {
        id,
        numero,
        total: total.0,
        aplicado: aplicado.0,
        sobrante: (total - aplicado).0,
        en_letras: inventario_dominio::letras::monto(total)
            .unwrap_or_else(|| "(monto fuera de rango)".into()),
    })
}

/// Anula un recibo: las facturas vuelven a deber. No borra nada.
pub async fn anular(pool: &Pool, recibo_id: i64) -> Resultado<()> {
    let r = sqlx::query("UPDATE recibo SET estado = 'anulado' WHERE id = ? AND estado = 'activo'")
        .bind(recibo_id)
        .execute(pool)
        .await?;
    if r.rows_affected() == 0 {
        return Err(Error::Regla(format!(
            "el recibo {recibo_id} no existe o ya estaba anulado"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::prueba_apoyo;
    use inventario_dominio::{Cantidad, factura::Linea};

    async fn cliente_con_cartera(pool: &Pool) -> (i64, Vec<i64>) {
        let a: i64 = sqlx::query(
            "INSERT INTO articulo (codigo, descripcion, precio_venta) VALUES ('A1','Bot',1000)
             RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
        .get("id");
        let t: i64 = sqlx::query(
            "INSERT INTO tercero (nit, dv, nombre, plazo_dias) VALUES ('1',0,'Cli',30) RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
        .get("id");
        let r: i64 = sqlx::query(
            "INSERT INTO resolucion (nombre, prefijo, desde, hasta) VALUES ('R','',1,1000)
             RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
        .get("id");
        crate::kardex::registrar(
            pool,
            &crate::kardex::Peticion {
                fecha: "2026-08-01".into(),
                concepto_id: prueba_apoyo::COMPRA,
                articulo_id: a,
                tercero_id: None,
                documento: String::new(),
                cantidad: Cantidad::de_unidades(10_000),
                costo_unitario: Centavos::de_pesos(500),
                factura_id: None,
                usuario_id: None,
                valor_comercial_unitario: None,
            },
        )
        .await
        .unwrap();

        let mut ids = Vec::new();
        for u in [10i64, 20, 30] {
            let f = factura::emitir(
                pool,
                &factura::Peticion {
                    resolucion_id: r,
                    tercero_id: t,
                    vendedor_id: None,
                    fecha: "2026-08-02".into(),
                    hora: "10:53:10-05:00".into(),
                    forma_pago: "credito".into(),
                    medio_pago: None,
                    retencion: 0,
                    observacion: String::new(),
                    lineas: vec![Linea {
                        articulo_id: a,
                        cantidad: Cantidad::de_unidades(u),
                        precio_unitario: Centavos::de_pesos(1000),
                        descuento: Centavos::CERO,
                        iva_porcentaje: 0,
                        inc_porcentaje: 0,
                    }],
                },
            )
            .await
            .unwrap();
            ids.push(f.id);
        }
        (t, ids)
    }

    fn efectivo(v: i64) -> Pago {
        Pago {
            medio: "efectivo".into(),
            banco_id: None,
            referencia: String::new(),
            vence: None,
            valor: Centavos::de_pesos(v).0,
        }
    }

    /// **UN RECIBO CUYO TOTAL NO SEA LA SUMA DE SUS PAGOS NO SE GUARDA.**
    ///
    /// Los números son los del caso REAL, medido el 2026-08-12 por HTTP contra el
    /// binario de release: tres pagos individualmente válidos —dos de
    /// 9 223 372 036 854 775 700 y uno de 300— dejaban persistido un recibo con
    /// `total = 84` mientras la suma real de sus `recibo_pago.valor` era
    /// 18 446 744 073 709 551 700. La cartera bajó exactamente 84 centavos y
    /// quien lo tecleó vio «Recibo 1 registrado por $84.» con un 200.
    ///
    /// **Ninguna de las tres redes lo veía**: `intentar_de_pesos` mira CADA pago
    /// por separado y los tres eran válidos, `cartera::repartir` solo rechaza
    /// `<= 0`, y los dos `CHECK` del esquema los pasa un 84 positivo.
    ///
    /// Se prueba aquí, en `registrar`, y no solo sobre la aritmética: el defecto
    /// no estaba en sumar dos números sino en que ESTE camino los sumara sin
    /// mirar. Y vale en los DOS perfiles porque `sumar` no depende de
    /// `overflow-checks` — escrita sobre `+`, esta prueba sería un pánico en
    /// debug y un dato corrupto en release, o sea que mediría cosas distintas
    /// según cómo se compile, y el binario que se instala es el que nadie medía.
    #[tokio::test]
    async fn un_recibo_cuya_suma_de_pagos_desborda_se_rechaza() {
        let pool = prueba_apoyo::base().await;
        let (t, _) = cliente_con_cartera(&pool).await;
        let casi = 9_223_372_036_854_775_700i64;
        let pago = |v: i64| Pago {
            medio: "efectivo".into(),
            banco_id: None,
            referencia: String::new(),
            vence: None,
            valor: v,
        };
        let e = registrar(
            &pool,
            &Peticion {
                tercero_id: t,
                fecha: "2026-08-03".into(),
                observacion: String::new(),
                pagos: vec![pago(casi), pago(casi), pago(300)],
                aplicaciones: vec![],
            },
            None,
        )
        .await;
        assert!(matches!(e, Err(Error::Regla(_))), "se aceptó un recibo imposible: {e:?}");

        // Y NO QUEDÓ NADA ESCRITO: un rechazo que dejara el recibo a medias sería
        // peor que el defecto original.
        let recibos: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recibo")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(recibos, 0, "el rechazo dejó un recibo escrito");
    }

    /// PAREJA: tres pagos NORMALES sí se guardan, y el total es su suma. Sin
    /// esto, un `registrar` que rechazara todo pasaría la de arriba — y el
    /// recaudo entero quedaría muerto con el banco en verde.
    #[tokio::test]
    async fn tres_pagos_normales_suman_y_se_guardan() {
        let pool = prueba_apoyo::base().await;
        let (t, _) = cliente_con_cartera(&pool).await;
        let r = registrar(
            &pool,
            &Peticion {
                tercero_id: t,
                fecha: "2026-08-03".into(),
                observacion: String::new(),
                pagos: vec![efectivo(10_000), efectivo(5_000), efectivo(2_500)],
                aplicaciones: vec![],
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(r.total, Centavos::de_pesos(17_500).0);

        // Y el total guardado coincide con la suma de las filas, que es
        // exactamente la coherencia que el defecto rompía.
        let suma: i64 = sqlx::query_scalar("SELECT SUM(valor) FROM recibo_pago WHERE recibo_id = ?")
            .bind(r.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(suma, r.total, "el total guardado no es la suma de sus pagos");
    }

    /// **El hermano del anterior, y vivía a diecinueve líneas de él.**
    ///
    /// La suma de PAGOS ya estaba comprobada; la de APLICACIONES —el reparto a
    /// mano que manda el formulario de recaudo— seguía con un `.sum()` crudo.
    ///
    /// **CORRECCIÓN del 2026-08-13**: aquí puse que «en release da la vuelta», y
    /// en este árbol es falso — `Cargo.toml` lleva `overflow-checks = true` en
    /// release desde el 2026-08-12. El desbordamiento es PÁNICO en los dos
    /// perfiles, así que lo que esta prueba fija no es «no se corrompe el dato»
    /// sino «no se cae el servidor»: un reparto imposible se rechaza con un
    /// mensaje en vez de tumbar la petición con un 500.
    #[tokio::test]
    async fn unas_aplicaciones_cuya_suma_desborda_se_rechazan() {
        let pool = prueba_apoyo::base().await;
        let (t, facturas) = cliente_con_cartera(&pool).await;
        let casi = 9_223_372_036_854_775_700i64;
        let e = registrar(
            &pool,
            &Peticion {
                tercero_id: t,
                fecha: "2026-08-03".into(),
                observacion: String::new(),
                pagos: vec![efectivo(10_000)],
                // Dos aplicaciones enormes sobre la misma factura: su suma no
                // cabe, y antes daba la vuelta a un número negativo.
                aplicaciones: vec![(facturas[0], casi), (facturas[0], casi)],
            },
            None,
        )
        .await;
        assert!(
            matches!(e, Err(Error::Regla(_))),
            "se aceptó un reparto cuya suma no cabe: {e:?}"
        );

        // Y NO QUEDÓ NADA ESCRITO.
        let recibos: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recibo")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(recibos, 0, "el rechazo dejó un recibo escrito");
    }

    #[tokio::test]
    async fn el_recibo_paga_de_la_mas_vieja_a_la_mas_nueva() {
        let pool = prueba_apoyo::base().await;
        let (t, ids) = cliente_con_cartera(&pool).await; // 10.000, 20.000, 30.000
        let r = registrar(
            &pool,
            &Peticion {
                tercero_id: t,
                fecha: "2026-08-03".into(),
                observacion: String::new(),
                pagos: vec![efectivo(25_000)],
                aplicaciones: vec![],
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(r.aplicado, Centavos::de_pesos(25_000).0);
        assert_eq!(r.sobrante, 0);

        let c = factura::por_cobrar(&pool, t).await.unwrap();
        assert_eq!(c[0].saldo(), Centavos::CERO, "la 1ª quedó paga");
        assert_eq!(c[1].saldo(), Centavos::de_pesos(5_000), "la 2ª parcialmente");
        assert_eq!(c[2].saldo(), Centavos::de_pesos(30_000));
        assert_eq!(ids.len(), 3);
    }

    #[tokio::test]
    async fn lo_que_sobra_se_dice_y_no_se_encaja_en_ninguna_factura() {
        let pool = prueba_apoyo::base().await;
        let (t, _) = cliente_con_cartera(&pool).await; // debe 60.000
        let r = registrar(
            &pool,
            &Peticion {
                tercero_id: t,
                fecha: "2026-08-03".into(),
                observacion: String::new(),
                pagos: vec![efectivo(100_000)],
                aplicaciones: vec![],
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(r.aplicado, Centavos::de_pesos(60_000).0);
        assert_eq!(r.sobrante, Centavos::de_pesos(40_000).0);
        // Ninguna factura quedó con saldo negativo.
        for f in factura::por_cobrar(&pool, t).await.unwrap() {
            assert!(f.saldo().0 >= 0);
        }
    }

    #[tokio::test]
    async fn el_cajero_puede_decir_a_que_factura_va_el_dinero() {
        let pool = prueba_apoyo::base().await;
        let (t, ids) = cliente_con_cartera(&pool).await;
        registrar(
            &pool,
            &Peticion {
                tercero_id: t,
                fecha: "2026-08-03".into(),
                observacion: String::new(),
                pagos: vec![efectivo(30_000)],
                aplicaciones: vec![(ids[2], Centavos::de_pesos(30_000).0)],
            },
            None,
        )
        .await
        .unwrap();
        let c = factura::por_cobrar(&pool, t).await.unwrap();
        assert_eq!(c[0].saldo(), Centavos::de_pesos(10_000), "la vieja sigue debiendo");
        assert_eq!(c[2].saldo(), Centavos::CERO, "se pagó la que dijo el cajero");
    }

    #[tokio::test]
    async fn no_se_puede_aplicar_mas_de_lo_que_se_debe() {
        let pool = prueba_apoyo::base().await;
        let (t, ids) = cliente_con_cartera(&pool).await;
        let e = registrar(
            &pool,
            &Peticion {
                tercero_id: t,
                fecha: "2026-08-03".into(),
                observacion: String::new(),
                pagos: vec![efectivo(50_000)],
                aplicaciones: vec![(ids[0], Centavos::de_pesos(50_000).0)],
            },
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(e, Error::Cartera(_)), "salió {e}");
        // Y no quedó recibo a medias.
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM recibo")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn un_medio_de_pago_inventado_se_rechaza() {
        let pool = prueba_apoyo::base().await;
        let (t, _) = cliente_con_cartera(&pool).await;
        let mut p = efectivo(1000);
        p.medio = "bitcoin".into();
        let e = registrar(
            &pool,
            &Peticion {
                tercero_id: t,
                fecha: "2026-08-03".into(),
                observacion: String::new(),
                pagos: vec![p],
                aplicaciones: vec![],
            },
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(e, Error::Regla(_)));
    }

    /// El defecto exacto del corpus: 313 abonos cuya cabecera fue borrada.
    #[tokio::test]
    async fn borrar_el_recibo_se_lleva_sus_aplicaciones() {
        let pool = prueba_apoyo::base().await;
        let (t, _) = cliente_con_cartera(&pool).await;
        let r = registrar(
            &pool,
            &Peticion {
                tercero_id: t,
                fecha: "2026-08-03".into(),
                observacion: String::new(),
                pagos: vec![efectivo(25_000)],
                aplicaciones: vec![],
            },
            None,
        )
        .await
        .unwrap();

        let (antes,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM recibo_aplicacion")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(antes > 0);

        sqlx::query("DELETE FROM recibo WHERE id = ?")
            .bind(r.id)
            .execute(&pool)
            .await
            .unwrap();

        let (despues,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM recibo_aplicacion")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(despues, 0, "quedaron abonos sin recibo que los explique");
        let (pagos,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM recibo_pago")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(pagos, 0);
    }

    #[tokio::test]
    async fn anular_el_recibo_devuelve_la_deuda() {
        let pool = prueba_apoyo::base().await;
        let (t, _) = cliente_con_cartera(&pool).await;
        let r = registrar(
            &pool,
            &Peticion {
                tercero_id: t,
                fecha: "2026-08-03".into(),
                observacion: String::new(),
                pagos: vec![efectivo(60_000)],
                aplicaciones: vec![],
            },
            None,
        )
        .await
        .unwrap();
        assert!(factura::por_cobrar(&pool, t).await.unwrap().iter().all(|f| f.esta_paga()));

        anular(&pool, r.id).await.unwrap();
        let c = factura::por_cobrar(&pool, t).await.unwrap();
        let deuda: Centavos = c.iter().map(|f| f.saldo()).sum();
        assert_eq!(deuda, Centavos::de_pesos(60_000));
        assert!(anular(&pool, r.id).await.is_err(), "no se anula dos veces");
    }

    /// Directiva 8: 120 recibos seguidos sobre una cartera viva.
    #[tokio::test]
    async fn ciento_veinte_recibos_no_pierden_ni_inventan_dinero() {
        let pool = prueba_apoyo::base().await;
        let (t, _) = cliente_con_cartera(&pool).await;
        let deuda_inicial: Centavos = factura::por_cobrar(&pool, t)
            .await
            .unwrap()
            .iter()
            .map(|f| f.saldo())
            .sum();

        let mut recaudado = Centavos::CERO;
        for k in 1..=120i64 {
            let valor = Centavos::de_pesos(k % 700 + 1);
            let r = registrar(
                &pool,
                &Peticion {
                    tercero_id: t,
                    fecha: "2026-08-03".into(),
                    observacion: String::new(),
                    pagos: vec![Pago { valor: valor.0, ..efectivo(0) }],
                    aplicaciones: vec![],
                },
                None,
            )
            .await
            .unwrap();
            assert_eq!(r.aplicado + r.sobrante, valor.0, "recibo {k} descuadra");
            recaudado = recaudado + Centavos(r.aplicado);
        }
        let deuda_final: Centavos = factura::por_cobrar(&pool, t)
            .await
            .unwrap()
            .iter()
            .map(|f| f.saldo())
            .sum();
        assert_eq!(
            deuda_inicial - recaudado,
            deuda_final,
            "el dinero recaudado no coincide con lo que bajó la cartera"
        );
    }
}
