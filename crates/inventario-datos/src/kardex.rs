//! El kardex sobre la base.
//!
//! Regla de oro del módulo: **el saldo de un artículo se lee de la foto del
//! último movimiento, y esa foto tiene que poder reconstruirse recorriendo la
//! historia entera**. Las dos cosas se comprueban en `el_kardex_y_las_fotos_dicen_lo_mismo`.
//! Si algún día divergen, es que alguien escribió un movimiento por fuera de
//! `registrar`, y esa prueba lo caza.

use crate::{Error, Pool, Resultado};
use inventario_dominio::{
    Cantidad, Centavos,
    costeo::{self, Capa, Formula},
    kardex::{self, Movimiento, Saldo, Signo},
    retiro::{self, EfectoTributario},
};
use sqlx::{Row, Sqlite, Transaction};

/// Un movimiento tal como lo pide quien lo registra.
#[derive(Debug, Clone)]
pub struct Peticion {
    pub fecha: String,
    pub concepto_id: i64,
    pub articulo_id: i64,
    pub tercero_id: Option<i64>,
    pub documento: String,
    pub cantidad: Cantidad,
    /// Solo lo usan las entradas. En una salida el costo lo fija el promedio.
    pub costo_unitario: Centavos,
    pub factura_id: Option<i64>,
    pub usuario_id: Option<i64>,
    /// Valor comercial **unitario** del retiro (ET art. 421 lit. b). Solo lo
    /// necesitan los conceptos de efecto `retiro_gravado`, y ahí es
    /// **obligatorio**: sin él el movimiento se rechaza en vez de liquidar el
    /// IVA sobre el costo, que sería subdeclararlo.
    pub valor_comercial_unitario: Option<Centavos>,
}

/// Una fila del kardex, como se muestra.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FilaKardex {
    pub id: i64,
    pub fecha: String,
    pub concepto: String,
    pub signo: String,
    pub documento: String,
    pub cantidad: i64,
    pub valor: i64,
    pub existencia_despues: i64,
    pub valor_despues: i64,
    pub costo_promedio_despues: Option<i64>,
    pub anulado: bool,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct SaldoArticulo {
    pub articulo_id: i64,
    pub existencia: i64,
    pub valor: i64,
    pub costo_promedio: Option<i64>,
}

/// Lee el saldo **dentro de una transacción**, bloqueando la fila del último
/// movimiento del artículo. Es lo que hace que dos ventas simultáneas del mismo
/// artículo no partan del mismo saldo.
pub async fn saldo_en_tx(tx: &mut Transaction<'_, Sqlite>, articulo_id: i64) -> Resultado<Saldo> {
    let fila = sqlx::query(
        "SELECT existencia_despues, valor_despues
           FROM movimiento
          WHERE articulo_id = ?
          ORDER BY id DESC
          LIMIT 1",
    )
    .bind(articulo_id)
    .fetch_optional(&mut **tx)
    .await?;

    Ok(match fila {
        None => Saldo::VACIO,
        Some(f) => Saldo {
            existencia: Cantidad(f.get::<i64, _>("existencia_despues")),
            valor: Centavos(f.get::<i64, _>("valor_despues")),
        },
    })
}

pub async fn saldo(pool: &Pool, articulo_id: i64) -> Resultado<SaldoArticulo> {
    let mut tx = pool.begin().await?;
    let s = saldo_en_tx(&mut tx, articulo_id).await?;
    tx.rollback().await?;
    Ok(SaldoArticulo {
        articulo_id,
        existencia: s.existencia.0,
        valor: s.valor.0,
        // Lectura: los dos casos se enseñan como «sin promedio». Ver el porqué
        // en `articulo::listar`.
        costo_promedio: s.costo_promedio().ok().map(|c| c.0),
    })
}

/// El último costo promedio conocido de un artículo, aunque hoy no haya
/// existencia. Lo necesita el camino permisivo del kardex para costear una
/// salida contra saldo cero.
async fn ultimo_costo(tx: &mut Transaction<'_, Sqlite>, articulo_id: i64) -> Resultado<Option<Centavos>> {
    let fila = sqlx::query(
        "SELECT valor_despues, existencia_despues
           FROM movimiento
          WHERE articulo_id = ? AND existencia_despues > 0
          ORDER BY id DESC
          LIMIT 1",
    )
    .bind(articulo_id)
    .fetch_optional(&mut **tx)
    .await?;
    // AQUÍ NO SE COLAPSAN LOS DOS CASOS, y ésa es la razón de todo el cambio.
    //
    // La consulta de arriba filtra `existencia_despues > 0`, así que si hay
    // fila, `SinExistencia` es IMPOSIBLE: el único error que puede llegar es
    // `NoCabe`, o sea un saldo imposible. Y quien recibe este `None` cuesta la
    // salida a CERO (`aplicar_permitiendo_negativo`), así que colapsarlos
    // convertía un saldo imposible en un movimiento de costo cero, en silencio y
    // con la transacción confirmada.
    //
    // Ahora `NoCabe` PROPAGA. `SinExistencia` no puede ocurrir, y si ocurriera
    // sería un fallo de la consulta, no un dato: también propaga, en vez de
    // hacerse pasar por «no hay costo anterior».
    let Some(f) = fila else { return Ok(None) };
    let saldo = Saldo {
        existencia: Cantidad(f.get::<i64, _>("existencia_despues")),
        valor: Centavos(f.get::<i64, _>("valor_despues")),
    };
    match saldo.costo_promedio() {
        Ok(c) => Ok(Some(c)),
        Err(e) => Err(Error::Regla(format!(
            "el último costo promedio del artículo {articulo_id} no se puede usar: {e}. \
             Una salida no se cuesta a cero por un saldo que no cuadra"
        ))),
    }
}

/// Registra un movimiento dentro de una transacción que ya está abierta.
///
/// Devuelve `(id_del_movimiento, valor_del_movimiento)`. El valor es lo que la
/// facturación anota como **costo de la línea**: no se recalcula después con un
/// promedio que ya cambió, que es lo que hacía `recalsalidas.prg`.
pub async fn registrar_en_tx(
    tx: &mut Transaction<'_, Sqlite>,
    p: &Peticion,
    cfg: Configuracion,
) -> Resultado<(i64, Centavos)> {
    let (signo, efecto_trib) = signo_del_concepto(tx, p.concepto_id).await?;
    let saldo = saldo_en_tx(tx, p.articulo_id).await?;

    let mov = Movimiento {
        signo,
        cantidad: p.cantidad,
        costo_unitario: p.costo_unitario,
    };

    // El saldo (existencia y valor) es el MISMO con las dos fórmulas: la
    // invariante del kardex no cambia. Lo que cambia es **cómo se cuesta la
    // salida**, y por eso PEPS solo interviene ahí.
    let mut efecto = if cfg.permite_negativo {
        // EL ÚLTIMO COSTO SE PIDE SOLO SI LA RAMA PERMISIVA LO VA A CONSUMIR, y
        // esa condición es exactamente la de `aplicar_permitiendo_negativo`:
        // salida contra existencia no positiva. En cualquier otro caso —toda
        // ENTRADA, y toda salida con existencia positiva— el valor se descarta.
        //
        // Pedirlo siempre era inocuo mientras devolvía `Option`. Al hacerlo
        // propagar `Err`, pasó a rechazar operaciones sobre un valor que nadie
        // mira: con `permite_negativo` puesto —que es ajuste de EMPRESA, no de
        // artículo—, un artículo con un saldo imposible quedaba inmovilizado
        // para todo, compras incluidas. Y como la única forma de arreglar ese
        // saldo es una entrada correctiva, el guardián se llevaba por delante la
        // reparación: quitaba la salida en vez de protegerla.
        //
        // Lo encontró la revisión de seguridad del 2026-08-13 y lo midió con su
        // pareja: la misma entrada sobre el mismo artículo corrupto pasa con
        // `permite_negativo=false` y se rechazaba con `true`. O sea que lo que
        // bloqueaba no era el dato, era la consulta que sobraba (directiva 21).
        let uc = if mov.signo == Signo::Salida && saldo.existencia.0 <= 0 {
            ultimo_costo(tx, p.articulo_id).await?
        } else {
            None
        };
        kardex::aplicar_permitiendo_negativo(saldo, mov, uc)?
    } else {
        kardex::aplicar(saldo, mov)?
    };

    let consumos = if cfg.formula == Formula::Peps && signo == Signo::Salida {
        let capas = capas_pendientes(tx, p.articulo_id).await?;
        // Si el negocio admite saldo negativo, puede pedirse más de lo que hay
        // en capas. Ahí se cae al promedio, y se dice en el código por qué: no
        // hay capa de la que sacar el costo, así que el único dato disponible
        // es el acumulado.
        match costeo::consumir(&capas, p.cantidad) {
            Ok((consumos, total)) => {
                let diferencia = total - efecto.valor_movimiento;
                efecto.valor_movimiento = total;
                efecto.saldo.valor = efecto.saldo.valor - diferencia;
                // EL CIERRE DEL BLANQUEO, POR ABSORCIÓN Y NO POR RECHAZO.
                //
                // Con PEPS este reajuste sobre un saldo CORRUPTO aterrizaba en
                // (0, basura), y una entrada posterior lo BLANQUEABA —el promedio
                // volvía a caber y la guarda `SaldoNoRepresentable` de `calcular`
                // callaba para siempre—.
                //
                // La regla es una identidad contable, no un juicio sobre el
                // origen: **cero unidades implica cero valor, siempre.** Cualquier
                // valor con existencia cero es un artefacto —da igual que sea
                // basura de un saldo corrupto o el residuo de ±unos centavos que
                // el redondeo de `multiplicar` deja al costear ventas parciales
                // fraccionarias—. Se ajusta a cero, y con eso no sobrevive nada
                // que una entrada posterior pueda blanquear.
                //
                // POR QUÉ ABSORBER Y NO RECHAZAR, que fue mi error del 2026-08-14
                // que cazó la revisión (confianza 8): `valor_despues` y la suma de
                // valores de las capas DERIVAN uno del otro por el redondeo del
                // costeo por capas, así que una venta de CIERRE perfectamente
                // legítima —1500 milésimas a 1 centavo, vendidas en dos tandas—
                // deja `(0, 1)` y mi guarda la rechazaba, bloqueando al cliente
                // para vender su propio stock. Absorber el residuo no bloquea a
                // nadie y cierra el blanqueo igual.
                //
                // El saldo corrupto con existencia POSITIVA no llega aquí: lo caza
                // antes la rama proporcional de `calcular` (`costo_promedio` no
                // cabe → `SaldoNoRepresentable`). Aquí solo llega la salida que se
                // lleva TODO, que es la única segura de absorber.
                //
                // Detectar la CORRUPCIÓN no es trabajo de aquí: eso lo hace la
                // auditoría del kardex (`la_auditoria_caza_un_movimiento_escrito_
                // _por_fuera`), que es donde un saldo escrito saltándose
                // `registrar` sale a la luz. Aquí solo se aplica la identidad.
                if efecto.saldo.existencia.0 == 0 {
                    efecto.saldo.valor = Centavos::CERO;
                }
                consumos
            }
            // LA MANGA NOMBRA LO QUE ABSORBE, desde el 2026-08-13. Era
            // `Err(_)`, escrita cuando `ErrorPeps` solo tenía dos variantes; al
            // añadir `ImporteFueraDeRango` pasó a tragárselo también, y en ese
            // camino el movimiento se persistiría al costo promedio con las
            // capas SIN consumir. Hoy no es alcanzable —una `Capa` se construye
            // de un movimiento cuyo `multiplicar` ya salió bien, así que
            // `capa.valor()` cabe por construcción— pero un comodín que crece
            // solo con cada variante nueva es la puerta que se abre sin que
            // nadie la abra. Lo señaló la revisión de seguridad.
            Err(costeo::ErrorPeps::CapasInsuficientes { .. }) if cfg.permite_negativo => {
                Vec::new()
            }
            Err(e) => return Err(Error::Regla(format!("PEPS: {e}"))),
        }
    } else {
        Vec::new()
    };

    // ET art. 421 lit. b): el retiro para uso propio es venta para efectos de
    // IVA, y **la base es el valor comercial, no el costo**. Sin valor comercial
    // el movimiento NO se guarda: liquidar sobre el costo subdeclara el
    // impuesto, y en silencio.
    let (valor_comercial, iva_generado) = if efecto_trib.causa_iva() {
        let r = retiro::iva_por_retiro(
            p.cantidad,
            p.valor_comercial_unitario,
            cfg.tarifa_iva_articulo,
            efecto.valor_movimiento,
        )
        .map_err(|e| Error::Regla(e.to_string()))?;
        (Some(r.base.0), r.iva.0)
    } else {
        (None, 0)
    };

    let id = sqlx::query(
        "INSERT INTO movimiento
            (fecha, concepto_id, articulo_id, tercero_id, documento, cantidad, valor,
             existencia_despues, valor_despues, valor_comercial, iva_generado,
             factura_id, usuario_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(&p.fecha)
    .bind(p.concepto_id)
    .bind(p.articulo_id)
    .bind(p.tercero_id)
    .bind(&p.documento)
    .bind(p.cantidad.0)
    .bind(efecto.valor_movimiento.0)
    .bind(efecto.saldo.existencia.0)
    .bind(efecto.saldo.valor.0)
    .bind(valor_comercial)
    .bind(iva_generado)
    .bind(p.factura_id)
    .bind(p.usuario_id)
    .fetch_one(&mut **tx)
    .await?
    .get::<i64, _>("id");

    // Las capas se llevan SIEMPRE que la fórmula sea PEPS, entradas y salidas.
    if cfg.formula == Formula::Peps {
        match signo {
            Signo::Entrada => {
                sqlx::query(
                    "INSERT INTO capa_costo
                        (movimiento_id, articulo_id, cantidad_restante, cantidad_inicial, costo_unitario)
                     VALUES (?, ?, ?, ?, ?)",
                )
                .bind(id)
                .bind(p.articulo_id)
                .bind(p.cantidad.0)
                .bind(p.cantidad.0)
                .bind(p.costo_unitario.0)
                .execute(&mut **tx)
                .await?;
            }
            Signo::Salida => {
                for c in &consumos {
                    // `movimiento_id` de la capa es su clave: `consumir`
                    // devuelve el del movimiento que la creó.
                    sqlx::query(
                        "UPDATE capa_costo SET cantidad_restante = cantidad_restante - ?
                          WHERE movimiento_id = ?",
                    )
                    .bind(c.cantidad.0)
                    .bind(c.movimiento_id)
                    .execute(&mut **tx)
                    .await?;
                    sqlx::query(
                        "INSERT INTO consumo_capa (movimiento_id, capa_id, cantidad, costo)
                         SELECT ?, id, ?, ? FROM capa_costo WHERE movimiento_id = ?",
                    )
                    .bind(id)
                    .bind(c.cantidad.0)
                    .bind(c.costo.0)
                    .bind(c.movimiento_id)
                    .execute(&mut **tx)
                    .await?;
                }
            }
        }
    }

    Ok((id, efecto.valor_movimiento))
}

/// Lo que el kardex necesita saber de la instalación para registrar un
/// movimiento. Va junto porque son tres lecturas que siempre se hacen a la vez,
/// y separarlas invita a olvidar una.
#[derive(Debug, Clone, Copy)]
pub struct Configuracion {
    pub permite_negativo: bool,
    pub formula: Formula,
    /// Tarifa de IVA del artículo que se mueve. Solo la usan los retiros
    /// gravados; en lo demás se ignora.
    pub tarifa_iva_articulo: i64,
}

/// Capas pendientes de un artículo, **de la más vieja a la más nueva**.
async fn capas_pendientes(
    tx: &mut Transaction<'_, Sqlite>,
    articulo_id: i64,
) -> Resultado<Vec<Capa>> {
    let filas = sqlx::query(
        "SELECT movimiento_id, cantidad_restante, costo_unitario
           FROM capa_costo
          WHERE articulo_id = ? AND cantidad_restante > 0
          ORDER BY id",
    )
    .bind(articulo_id)
    .fetch_all(&mut **tx)
    .await?;
    Ok(filas
        .into_iter()
        .map(|f| Capa {
            movimiento_id: f.get("movimiento_id"),
            cantidad: Cantidad(f.get::<i64, _>("cantidad_restante")),
            costo_unitario: Centavos(f.get::<i64, _>("costo_unitario")),
        })
        .collect())
}

/// Registra un movimiento suelto (compra, ajuste, saldo inicial, retiro).
pub async fn registrar(pool: &Pool, p: &Peticion) -> Resultado<i64> {
    let cfg = configuracion(pool, p.articulo_id).await?;
    // `IMMEDIATE` por lo mismo que en `factura::emitir`: se lee el saldo y
    // luego se escribe, y una transacción diferida que hace eso recibe
    // `SQLITE_BUSY` sin reintento en cuanto hay dos a la vez.
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let (id, _) = registrar_en_tx(&mut tx, p, cfg).await?;
    tx.commit().await?;
    Ok(id)
}

/// Lee la configuración de costeo y la tarifa del artículo.
pub async fn configuracion(pool: &Pool, articulo_id: i64) -> Resultado<Configuracion> {
    let e: Option<(i64, String)> =
        sqlx::query_as("SELECT permite_negativo, formula_costeo FROM empresa WHERE id = 1")
            .fetch_optional(pool)
            .await?;
    // Sin fila de empresa la instalación no está terminada. Se responde el
    // camino ESTRICTO y la fórmula por defecto de la NIC 2 más conservadora:
    // ante la ausencia del dato, lo que no puede causar daño silencioso.
    let (permite, formula) = match e {
        Some((p, f)) => (
            p == 1,
            Formula::desde_texto(&f).map_err(|x| Error::Regla(x.to_string()))?,
        ),
        None => (false, Formula::PromedioPonderado),
    };
    let tarifa: Option<(i64,)> = sqlx::query_as("SELECT iva_porcentaje FROM articulo WHERE id = ?")
        .bind(articulo_id)
        .fetch_optional(pool)
        .await?;
    Ok(Configuracion {
        permite_negativo: permite,
        formula,
        tarifa_iva_articulo: tarifa.map(|(t,)| t).unwrap_or(0),
    })
}

pub(crate) async fn signo_del_concepto(
    tx: &mut Transaction<'_, Sqlite>,
    concepto_id: i64,
) -> Resultado<(Signo, EfectoTributario)> {
    let fila = sqlx::query("SELECT signo, efecto_tributario FROM concepto WHERE id = ? AND activo = 1")
        .bind(concepto_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(Error::NoExiste {
            que: "concepto",
            id: concepto_id,
        })?;
    let s: String = fila.get("signo");
    let e: String = fila.get("efecto_tributario");
    // Los CHECK del esquema ya limitan los valores; si aun así llegara otro, se
    // falla ruidosamente en vez de suponer una dirección o un efecto fiscal.
    let signo = Signo::desde_texto(&s)
        .ok_or_else(|| Error::Regla(format!("signo de concepto ilegible: {s:?}")))?;
    let efecto = EfectoTributario::desde_texto(&e)
        .ok_or_else(|| Error::Regla(format!("efecto tributario ilegible: {e:?}")))?;
    Ok((signo, efecto))
}

/// El kardex de un artículo, del más viejo al más nuevo.
pub async fn historia(pool: &Pool, articulo_id: i64, limite: i64) -> Resultado<Vec<FilaKardex>> {
    let filas = sqlx::query(
        "SELECT m.id, m.fecha, c.nombre AS concepto, c.signo, m.documento, m.cantidad,
                m.valor, m.existencia_despues, m.valor_despues,
                (SELECT COUNT(*) FROM movimiento r WHERE r.reversa_de = m.id) AS reversas
           FROM movimiento m
           JOIN concepto c ON c.id = m.concepto_id
          WHERE m.articulo_id = ?
          ORDER BY m.id
          LIMIT ?",
    )
    .bind(articulo_id)
    .bind(limite)
    .fetch_all(pool)
    .await?;

    Ok(filas
        .into_iter()
        .map(|f| {
            let ex = f.get::<i64, _>("existencia_despues");
            let va = f.get::<i64, _>("valor_despues");
            FilaKardex {
                id: f.get("id"),
                fecha: f.get("fecha"),
                concepto: f.get("concepto"),
                signo: f.get("signo"),
                documento: f.get("documento"),
                cantidad: f.get("cantidad"),
                valor: f.get("valor"),
                existencia_despues: ex,
                valor_despues: va,
                costo_promedio_despues: Saldo {
                    existencia: Cantidad(ex),
                    valor: Centavos(va),
                }
                .costo_promedio()
                .ok()
                .map(|c| c.0),
                anulado: f.get::<i64, _>("reversas") > 0,
            }
        })
        .collect())
}

/// Artículos por debajo de su punto mínimo. Es el informe que el sistema viejo
/// no podía dar porque su existencia era ficción.
#[derive(Debug, serde::Serialize)]
pub struct Escasez {
    pub articulo_id: i64,
    pub codigo: String,
    pub descripcion: String,
    pub existencia: i64,
    pub punto_minimo: i64,
    pub punto_maximo: i64,
    pub sugerido: i64,
}

pub async fn bajo_minimo(pool: &Pool) -> Resultado<Vec<Escasez>> {
    let filas = sqlx::query(
        "SELECT a.id, a.codigo, a.descripcion, a.punto_minimo, a.punto_maximo,
                COALESCE((SELECT m.existencia_despues FROM movimiento m
                           WHERE m.articulo_id = a.id ORDER BY m.id DESC LIMIT 1), 0) AS existencia
           FROM articulo a
          WHERE a.activo = 1 AND a.punto_minimo > 0
          ORDER BY a.descripcion",
    )
    .fetch_all(pool)
    .await?;

    Ok(filas
        .into_iter()
        .filter_map(|f| {
            let existencia: i64 = f.get("existencia");
            let minimo: i64 = f.get::<i64, _>("punto_minimo") * inventario_dominio::dinero::MIL;
            if existencia >= minimo {
                return None;
            }
            let maximo: i64 = f.get::<i64, _>("punto_maximo") * inventario_dominio::dinero::MIL;
            Some(Escasez {
                articulo_id: f.get("id"),
                codigo: f.get("codigo"),
                descripcion: f.get("descripcion"),
                existencia,
                punto_minimo: minimo,
                punto_maximo: maximo,
                // Reponer hasta el máximo si lo hay; si no, hasta el mínimo.
                sugerido: (if maximo > 0 { maximo } else { minimo }) - existencia,
            })
        })
        .collect())
}

/// Recorre la historia entera de un artículo y devuelve el saldo que resulta.
///
/// **Es la auditoría del módulo**, no una comodidad: contrasta la foto contra
/// el recorrido. En el sistema viejo nadie contrastaba `INV_ACTUAL` con nada.
pub async fn recomputar(pool: &Pool, articulo_id: i64) -> Resultado<Saldo> {
    let filas = sqlx::query(
        "SELECT c.signo, m.cantidad, m.valor
           FROM movimiento m JOIN concepto c ON c.id = m.concepto_id
          WHERE m.articulo_id = ?
          ORDER BY m.id",
    )
    .bind(articulo_id)
    .fetch_all(pool)
    .await?;

    let mut s = Saldo::VACIO;
    for f in filas {
        let signo: String = f.get("signo");
        let cantidad = Cantidad(f.get::<i64, _>("cantidad"));
        let valor = Centavos(f.get::<i64, _>("valor"));
        match Signo::desde_texto(&signo) {
            Some(Signo::Entrada) => {
                s.existencia = s.existencia + cantidad;
                s.valor = s.valor + valor;
            }
            Some(Signo::Salida) => {
                s.existencia = s.existencia - cantidad;
                s.valor = s.valor - valor;
            }
            None => return Err(Error::Regla(format!("signo ilegible en el kardex: {signo:?}"))),
        }
    }
    Ok(s)
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::prueba_apoyo::{self, COMPRA};

    async fn articulo(pool: &Pool, codigo: &str) -> i64 {
        sqlx::query("INSERT INTO articulo (codigo, descripcion) VALUES (?, ?) RETURNING id")
            .bind(codigo)
            .bind(format!("Artículo {codigo}"))
            .fetch_one(pool)
            .await
            .unwrap()
            .get("id")
    }

    fn pet(articulo_id: i64, concepto_id: i64, u: i64, pesos: i64) -> Peticion {
        Peticion {
            fecha: "2026-08-02".into(),
            concepto_id,
            articulo_id,
            tercero_id: None,
            documento: String::new(),
            cantidad: Cantidad::de_unidades(u),
            costo_unitario: Centavos::de_pesos(pesos),
            factura_id: None,
            usuario_id: Some(1),
            valor_comercial_unitario: None,
        }
    }

    /// **UN SALDO IMPOSIBLE YA NO CUESTA LA SALIDA A CERO.**
    ///
    /// Es el defecto que motiva todo el cambio, y solo se puede provocar
    /// escribiendo un movimiento SALTÁNDOSE `registrar` —que es justo el estado
    /// que el defecto original de `multiplicar` dejaba persistido—: una
    /// existencia de 1 milésima con un valor enorme. El promedio por unidad es
    /// valor × 1000 y no cabe.
    ///
    /// Antes: `costo_promedio()` devolvía `None`, `ultimo_costo` lo colapsaba
    /// con «no hay existencia», y `aplicar_permitiendo_negativo` costeaba la
    /// salida a CERO — en silencio y con la transacción confirmada.
    #[tokio::test]
    async fn un_saldo_imposible_no_cuesta_la_salida_a_cero() {
        let pool = prueba_apoyo::base().await;
        let a = articulo(&pool, "IMP").await;

        // DOS FOTOS, y el orden es lo que hace que la prueba ejerza el camino
        // real. `ultimo_costo` solo se consulta desde el camino PERMISIVO, que
        // se toma cuando la existencia actual NO es positiva; y esa consulta
        // filtra `existencia_despues > 0`, o sea que mira hacia atrás.
        //
        // La primera versión de esta prueba dejaba una sola foto con
        // existencia 1 —positiva—, así que ni siquiera entraba por ahí: el
        // mutante salía rojo por OTRA guarda, la de `multiplicar`. Un caso rojo
        // que falla por la vía equivocada no mide lo que dice (directiva 33).
        //
        // 1) saldo imposible CON existencia positiva: es el que `ultimo_costo`
        //    encontrará al mirar atrás.
        // 2) el artículo se agota: la existencia actual queda en cero, que es
        //    lo que fuerza el camino permisivo.
        for (existencia, valor) in [(1i64, i64::MAX / 500), (0, 0)] {
            sqlx::query(
                "INSERT INTO movimiento
                   (fecha, articulo_id, concepto_id, cantidad, valor,
                    existencia_despues, valor_despues)
                 VALUES ('2026-08-01', ?, ?, 1, 0, ?, ?)",
            )
            .bind(a)
            .bind(COMPRA)
            .bind(existencia)
            .bind(valor)
            .execute(&pool)
            .await
            .unwrap();
        }

        // Una salida por el camino permisivo, que es el que consulta
        // `ultimo_costo`. Tiene que RECHAZARSE, no costearse a cero.
        let mut tx = pool.begin().await.unwrap();
        let e = registrar_en_tx(
            &mut tx,
            &pet(a, prueba_apoyo::VENTA, 5, 0),
            Configuracion {
                permite_negativo: true,
                formula: Formula::PromedioPonderado,
                tarifa_iva_articulo: 0,
            },
        )
        .await;
        assert!(
            matches!(e, Err(Error::Regla(ref m)) if m.contains("no se puede usar")),
            "un saldo imposible se costeó como si nada: {e:?}"
        );
    }

    /// **LA PAREJA DEL ANTERIOR, y sin ella el arreglo quitaba la reparación.**
    ///
    /// El mismo artículo con el mismo saldo imposible, pero una ENTRADA. Ahí el
    /// último costo ni se mira —`aplicar_permitiendo_negativo` solo lo usa en
    /// una salida contra existencia no positiva—, así que rechazarla no protege
    /// ningún costeo y sí inmoviliza el artículo.
    ///
    /// Y lo inmoviliza para siempre: la única forma de arreglar un saldo
    /// imposible es justamente una entrada correctiva. Lo encontró la revisión
    /// de seguridad del 2026-08-13; era el sesgo a cerrar de la directiva 21.
    #[tokio::test]
    async fn una_entrada_correctiva_sigue_pasando_sobre_un_saldo_imposible() {
        let pool = prueba_apoyo::base().await;
        let a = articulo(&pool, "IMP2").await;
        for (existencia, valor) in [(1i64, i64::MAX / 500), (0, 0)] {
            sqlx::query(
                "INSERT INTO movimiento
                   (fecha, articulo_id, concepto_id, cantidad, valor,
                    existencia_despues, valor_despues)
                 VALUES ('2026-08-01', ?, ?, 1, 0, ?, ?)",
            )
            .bind(a)
            .bind(COMPRA)
            .bind(existencia)
            .bind(valor)
            .execute(&pool)
            .await
            .unwrap();
        }

        let mut tx = pool.begin().await.unwrap();
        let r = registrar_en_tx(
            &mut tx,
            &pet(a, COMPRA, 10, 1_000),
            Configuracion {
                permite_negativo: true,
                formula: Formula::PromedioPonderado,
                tarifa_iva_articulo: 0,
            },
        )
        .await;
        assert!(
            r.is_ok(),
            "una COMPRA se rechazó por un último costo que esa rama ni mira, \
             y es la única forma de reparar el saldo: {r:?}"
        );

        // Y QUE DE VERDAD REPARE, no solo que no la rechacen. El comentario de
        // arriba invoca la reparación como motivo, así que la prueba tiene que
        // medirla: tras la entrada, el saldo vuelve a tener un promedio dable.
        // Lo señaló la revisión del 2026-08-13 — afirmaba menos de lo que su
        // propio enunciado prometía.
        let saldo = saldo_en_tx(&mut tx, a).await.unwrap();
        assert!(
            saldo.costo_promedio().is_ok(),
            "la entrada correctiva pasó pero el saldo sigue sin promedio dable: {saldo:?}"
        );
    }

    /// **BAJO PEPS, LLEVÁRSELO TODO *NO* REPARA — y esto lo mide porque el
    /// comentario que lo afirmaba estaba fuera del alcance de su propia
    /// prueba.**
    ///
    /// La exención de `calcular` —no bloquear la salida que se lleva toda la
    /// existencia— se justificaba diciendo que el saldo queda en (0, 0) exacto.
    /// Es cierto con promedio ponderado y FALSO con `Formula::Peps`, que
    /// reescribe el saldo aquí, después de `aplicar`:
    ///
    /// ```text
    /// efecto.saldo.valor -= total - efecto.valor_movimiento
    /// ```
    ///
    /// Sobre un saldo corrupto, `valor_movimiento` era la basura y `total` el
    /// costo real de las capas, así que aterriza en (0, basura). Lo midió la
    /// revisión de seguridad del 2026-08-13; la prueba del dominio no podía
    /// verlo porque allí PEPS no existe.
    ///
    /// **Esta prueba NO afirma que esté bien: afirma cuál es el estado.** Es un
    /// límite conocido y medido en vez de una frase falsa. Cerrarlo sería exigir
    /// que existencia cero implique valor cero tras el ajuste, y eso puede
    /// rechazar datos heredados donde las capas y el saldo discrepan por
    /// redondeo — decisión de alcance, con su propia medición.
    #[tokio::test]
    async fn peps_absorbe_la_basura_al_cerrar_en_vez_de_blanquearla() {
        let pool = prueba_apoyo::base().await;
        let a = articulo(&pool, "PEPS").await;
        let basura = i64::MAX / 250;

        // UNA COMPRA REAL PRIMERO: sin ella no hay `capa_costo` y PEPS falla por
        // «las capas suman 0», que es otra cosa. El fixture tiene que llegar al
        // camino que se quiere medir, no a un error cualquiera (directiva 33).
        {
            let mut tx = pool.begin().await.unwrap();
            registrar_en_tx(
                &mut tx,
                &Peticion {
                    cantidad: Cantidad(2),
                    ..pet(a, COMPRA, 0, 100)
                },
                Configuracion {
                    permite_negativo: false,
                    formula: Formula::Peps,
                    tarifa_iva_articulo: 0,
                },
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
        }

        // Y AHORA la foto corrupta, escrita por fuera de `registrar`.
        sqlx::query(
            "INSERT INTO movimiento
               (fecha, articulo_id, concepto_id, cantidad, valor,
                existencia_despues, valor_despues)
             VALUES ('2026-08-01', ?, ?, 2, 0, 2, ?)",
        )
        .bind(a)
        .bind(COMPRA)
        .bind(basura)
        .execute(&pool)
        .await
        .unwrap();

        // Salida que se lleva TODA la existencia (2 milésimas), con PEPS.
        let mut tx = pool.begin().await.unwrap();
        let r = registrar_en_tx(
            &mut tx,
            &Peticion {
                cantidad: Cantidad(2),
                ..pet(a, prueba_apoyo::VENTA, 0, 0)
            },
            Configuracion {
                permite_negativo: false,
                formula: Formula::Peps,
                tarifa_iva_articulo: 0,
            },
        )
        .await;

        // EL BLANQUEO SE CIERRA POR ABSORCIÓN, no por rechazo (corregido el
        // 2026-08-14 tras una revisión de confianza 8: rechazar bloqueaba
        // ventas de cierre LEGÍTIMAS, ver la prueba de al lado). La salida que
        // se lleva toda la existencia SÍ se acepta —cero unidades es un cierre
        // válido—, y el residuo se absorbe a cero. Con eso no sobrevive nada que
        // una entrada posterior pueda blanquear.
        r.expect("la salida de cierre se acepta y absorbe el residuo");
        tx.commit().await.unwrap();

        let mut tx2 = pool.begin().await.unwrap();
        let saldo = saldo_en_tx(&mut tx2, a).await.unwrap();
        assert_eq!(saldo.existencia, Cantidad::CERO, "el cierre deja existencia cero");
        // LO QUE CIERRA EL BLANQUEO: la basura NO sobrevive. Antes quedaba
        // `(0, basura)`; ahora `(0, 0)`, así que la entrada siguiente arranca
        // limpia y no hay nada que blanquear.
        assert_eq!(
            saldo.valor, Centavos::CERO,
            "la basura del saldo corrupto debe absorberse a cero, no persistir"
        );
    }

    /// EL LADO BENIGNO, que es la pareja que a la prueba de arriba le FALTABA
    /// (directiva 33), y sin la cual mi guarda rechazaba ventas legítimas.
    ///
    /// Un saldo perfectamente SANO —1500 milésimas a 1 centavo la unidad—
    /// vendido en dos tandas. `valor_despues` y la suma de valores de las capas
    /// DERIVAN uno del otro por el redondeo de `multiplicar`, así que la venta
    /// de cierre deja `(0, 1)` — un residuo de un centavo, no un saldo corrupto.
    /// Debe ACEPTARSE y absorberse, no rechazarse. Mi primera versión la
    /// rechazaba, y lo cazó la revisión.
    #[tokio::test]
    async fn una_venta_de_cierre_con_residuo_de_redondeo_no_se_bloquea() {
        let pool = prueba_apoyo::base().await;
        let a = articulo(&pool, "CIERRE").await;
        let cfg = Configuracion {
            permite_negativo: false,
            formula: Formula::Peps,
            tarifa_iva_articulo: 0,
        };
        {
            let mut tx = pool.begin().await.unwrap();
            registrar_en_tx(&mut tx, &Peticion { cantidad: Cantidad(1500), ..pet(a, COMPRA, 0, 0) }, cfg)
                .await
                .unwrap();
            tx.commit().await.unwrap();
        }
        // Costo unitario de 1 centavo exacto: es donde el redondeo del costeo por
        // capas deja residuo.
        sqlx::query("UPDATE capa_costo SET costo_unitario = 1 WHERE articulo_id = ?")
            .bind(a).execute(&pool).await.unwrap();
        sqlx::query("UPDATE movimiento SET valor = 2, valor_despues = 2 WHERE articulo_id = ?")
            .bind(a).execute(&pool).await.unwrap();

        // Venta parcial de 400 milésimas: introduce la deriva de ±1 centavo.
        {
            let mut tx = pool.begin().await.unwrap();
            registrar_en_tx(&mut tx, &Peticion { cantidad: Cantidad(400), ..pet(a, prueba_apoyo::VENTA, 0, 0) }, cfg)
                .await
                .expect("la venta parcial de un saldo sano no se bloquea");
            tx.commit().await.unwrap();
        }
        // Venta de CIERRE de las 1100 restantes: la que mi guarda rechazaba.
        let mut tx = pool.begin().await.unwrap();
        registrar_en_tx(&mut tx, &Peticion { cantidad: Cantidad(1100), ..pet(a, prueba_apoyo::VENTA, 0, 0) }, cfg)
            .await
            .expect("VENTA DE CIERRE LEGÍTIMA: un saldo sano se puede vender entero");
        tx.commit().await.unwrap();

        let mut tx2 = pool.begin().await.unwrap();
        let saldo = saldo_en_tx(&mut tx2, a).await.unwrap();
        assert_eq!(saldo.existencia, Cantidad::CERO);
        assert_eq!(saldo.valor, Centavos::CERO, "el residuo de redondeo se absorbe a cero");
    }

    #[tokio::test]
    async fn el_saldo_sale_del_kardex_y_no_de_ningun_campo() {
        let pool = prueba_apoyo::base().await;
        let a = articulo(&pool, "A1").await;

        assert_eq!(saldo(&pool, a).await.unwrap().existencia, 0);
        registrar(&pool, &pet(a, COMPRA, 10, 100)).await.unwrap();
        registrar(&pool, &pet(a, COMPRA, 10, 200)).await.unwrap();

        let s = saldo(&pool, a).await.unwrap();
        assert_eq!(s.existencia, 20_000);
        assert_eq!(s.costo_promedio, Some(15_000)); // $150,00

        // No existe ninguna columna de existencia en `articulo`: si alguien la
        // añadiera, tendría dos fuentes de verdad. Esta prueba lo impide.
        let cols: Vec<String> = sqlx::query("PRAGMA table_info(articulo)")
            .fetch_all(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(|f| f.get::<String, _>("name"))
            .collect();
        for prohibida in ["existencia", "inv_actual", "saldo", "stock"] {
            assert!(
                !cols.iter().any(|c| c == prohibida),
                "`articulo` ganó una columna `{prohibida}`: son dos verdades y una va a mentir"
            );
        }
    }

    /// La auditoría: la foto y el recorrido tienen que coincidir tras muchos
    /// movimientos. Directiva 8: 120 operaciones sobre tres artículos.
    #[tokio::test]
    async fn el_kardex_y_las_fotos_dicen_lo_mismo() {
        let pool = prueba_apoyo::base().await;
        let arts = [
            articulo(&pool, "A1").await,
            articulo(&pool, "A2").await,
            articulo(&pool, "A3").await,
        ];
        for i in 1..=120i64 {
            let a = arts[(i % 3) as usize];
            if i % 4 == 0 {
                let s = saldo(&pool, a).await.unwrap();
                let piden = (i % 3 + 1) * 1000;
                if s.existencia >= piden {
                    let mut p = pet(a, prueba_apoyo::VENTA, 0, 0);
                    p.cantidad = Cantidad(piden);
                    registrar(&pool, &p).await.unwrap();
                }
            } else {
                registrar(&pool, &pet(a, COMPRA, i % 7 + 1, i % 500 + 10))
                    .await
                    .unwrap();
            }
        }
        for a in arts {
            let foto = saldo(&pool, a).await.unwrap();
            let recorrido = recomputar(&pool, a).await.unwrap();
            assert_eq!(
                (foto.existencia, foto.valor),
                (recorrido.existencia.0, recorrido.valor.0),
                "el artículo {a} tiene una foto que no cuadra con su historia"
            );
            assert!(foto.existencia >= 0);
        }
    }

    /// Directiva 8: que DISCRIMINE. Se escribe un movimiento a mano saltándose
    /// `registrar` —igual que hacía el sistema viejo con su campo mutable— y la
    /// auditoría tiene que cazarlo.
    #[tokio::test]
    async fn la_auditoria_caza_un_movimiento_escrito_por_fuera() {
        let pool = prueba_apoyo::base().await;
        let a = articulo(&pool, "A1").await;
        registrar(&pool, &pet(a, COMPRA, 10, 100)).await.unwrap();

        sqlx::query(
            "INSERT INTO movimiento
                (fecha, concepto_id, articulo_id, cantidad, valor, existencia_despues, valor_despues)
             VALUES ('2026-08-02', ?, ?, 5000, 50000, 99999, 12345)",
        )
        .bind(COMPRA)
        .bind(a)
        .execute(&pool)
        .await
        .unwrap();

        let foto = saldo(&pool, a).await.unwrap();
        let recorrido = recomputar(&pool, a).await.unwrap();
        assert_ne!(
            (foto.existencia, foto.valor),
            (recorrido.existencia.0, recorrido.valor.0),
            "la auditoría dejó de discriminar: no distingue una foto falsa"
        );
    }

    #[tokio::test]
    async fn no_se_puede_vender_lo_que_no_hay() {
        let pool = prueba_apoyo::base().await;
        let a = articulo(&pool, "A1").await;
        registrar(&pool, &pet(a, COMPRA, 3, 100)).await.unwrap();

        let mut p = pet(a, prueba_apoyo::VENTA, 4, 0);
        p.cantidad = Cantidad::de_unidades(4);
        let e = registrar(&pool, &p).await.unwrap_err();
        assert!(matches!(e, Error::Kardex(_)), "salió {e}");
        // Y el saldo no se movió.
        assert_eq!(saldo(&pool, a).await.unwrap().existencia, 3000);
    }

    #[tokio::test]
    async fn el_saldo_negativo_solo_si_la_empresa_lo_permite() {
        let pool = prueba_apoyo::base().await;
        let a = articulo(&pool, "A1").await;
        let mut p = pet(a, prueba_apoyo::VENTA, 5, 0);
        p.cantidad = Cantidad::de_unidades(5);

        assert!(registrar(&pool, &p).await.is_err());

        sqlx::query("UPDATE empresa SET permite_negativo = 1")
            .execute(&pool)
            .await
            .unwrap();
        registrar(&pool, &p).await.unwrap();
        assert_eq!(saldo(&pool, a).await.unwrap().existencia, -5000);
    }

    #[tokio::test]
    async fn un_concepto_que_no_existe_no_mueve_nada() {
        let pool = prueba_apoyo::base().await;
        let a = articulo(&pool, "A1").await;
        let e = registrar(&pool, &pet(a, 9_999, 1, 100)).await.unwrap_err();
        assert!(matches!(e, Error::NoExiste { que: "concepto", .. }), "salió {e}");
    }

    #[tokio::test]
    async fn el_informe_de_bajo_minimo_usa_la_existencia_real() {
        let pool = prueba_apoyo::base().await;
        let a = articulo(&pool, "A1").await;
        sqlx::query("UPDATE articulo SET punto_minimo = 10, punto_maximo = 40 WHERE id = ?")
            .bind(a)
            .execute(&pool)
            .await
            .unwrap();

        // Sin movimientos: existencia 0, está bajo mínimo.
        let r = bajo_minimo(&pool).await.unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].sugerido, 40_000);

        // Con 25 unidades y mínimo 10 ya NO está bajo mínimo: sale del informe.
        // El máximo no lo mete de vuelta — «bajo mínimo» y «por debajo del
        // máximo» son cosas distintas, y confundirlas llenaría el informe de
        // artículos que no hay que reponer (una alarma que a la tercera nadie
        // mira, directiva 23).
        registrar(&pool, &pet(a, COMPRA, 25, 100)).await.unwrap();
        assert!(bajo_minimo(&pool).await.unwrap().is_empty(), "25 ≥ 10");

        // Si el mínimo sube a 30, sí entra, y se repone hasta el máximo.
        sqlx::query("UPDATE articulo SET punto_minimo = 30 WHERE id = ?")
            .bind(a)
            .execute(&pool)
            .await
            .unwrap();
        let r = bajo_minimo(&pool).await.unwrap();
        assert_eq!(r[0].existencia, 25_000);
        assert_eq!(r[0].sugerido, 15_000, "repone hasta el máximo (40), no hasta el mínimo");
    }
}
