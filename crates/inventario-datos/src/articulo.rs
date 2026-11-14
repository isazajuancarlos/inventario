//! Catálogo de artículos y precios por convenio.

use crate::{Error, Pool, Resultado};
use inventario_dominio::{Cantidad, Centavos, dinero::MIL, kardex::Saldo};
use sqlx::Row;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Articulo {
    pub id: i64,
    pub codigo: String,
    pub codigo_barras: Option<String>,
    pub descripcion: String,
    pub presentacion: String,
    pub categoria_id: Option<i64>,
    pub iva_porcentaje: i64,
    pub precio_venta: i64,
    pub punto_minimo: i64,
    pub punto_maximo: i64,
    pub inc_porcentaje: i64,
    pub unidad_medida: Option<String>,
    pub codigo_estandar: Option<String>,
    pub tipo_estandar: Option<String>,
    pub precio_venta_estimado: Option<i64>,
    pub costos_para_vender: Option<i64>,
    pub activo: bool,
    /// Existencia derivada del kardex. No es una columna: es una consulta.
    pub existencia: i64,
    pub costo_promedio: Option<i64>,
    /// Qué le falta a este artículo para poder ir en una factura electrónica.
    /// Vacío = está completo. Se calcula al vuelo y **se muestra**: descubrirlo
    /// al emitir es descubrirlo tarde.
    pub falta_para_factura_electronica: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Nuevo {
    pub codigo: String,
    #[serde(default)]
    pub codigo_barras: Option<String>,
    pub descripcion: String,
    #[serde(default)]
    pub presentacion: String,
    #[serde(default)]
    pub categoria_id: Option<i64>,
    #[serde(default)]
    pub iva_porcentaje: i64,
    #[serde(default)]
    pub precio_venta: i64,
    #[serde(default)]
    pub punto_minimo: i64,
    #[serde(default)]
    pub punto_maximo: i64,
    #[serde(default)]
    pub inc_porcentaje: i64,
    /// Unidad de medida del ítem, que la factura electrónica exige.
    #[serde(default)]
    pub unidad_medida: Option<String>,
    /// Precio de venta estimado y costos para vender: los dos datos con los que
    /// se mide el deterioro (NIC 2.28-33). Sin ellos el artículo no se puede
    /// medir, y el corte lo dice en vez de suponer.
    #[serde(default)]
    pub precio_venta_estimado: Option<i64>,
    #[serde(default)]
    pub costos_para_vender: Option<i64>,
}

/// El código de barras vacío se guarda como NULL, nunca como `''`.
///
/// No es cosmética: 85 de los 148 artículos del corpus lo tienen vacío, y en
/// SQLite un índice único admite muchos NULL pero solo una cadena vacía. Con
/// `''` el segundo artículo sin código de barras no se podría crear.
fn barras(v: Option<&String>) -> Option<String> {
    v.map(|s| s.trim()).filter(|s| !s.is_empty()).map(str::to_string)
}

fn validar(n: &Nuevo) -> Resultado<()> {
    if n.codigo.trim().is_empty() {
        return Err(Error::Regla("el artículo necesita un código".into()));
    }
    if n.descripcion.trim().is_empty() {
        return Err(Error::Regla("el artículo necesita una descripción".into()));
    }
    if !(0..=100).contains(&n.iva_porcentaje) {
        return Err(Error::Regla(format!(
            "tarifa de IVA imposible: {} %",
            n.iva_porcentaje
        )));
    }
    if n.precio_venta < 0 || n.punto_minimo < 0 || n.punto_maximo < 0 {
        return Err(Error::Regla("precio y puntos de reposición no pueden ser negativos".into()));
    }
    if n.punto_maximo > 0 && n.punto_maximo < n.punto_minimo {
        return Err(Error::Regla(format!(
            "el punto máximo ({}) es menor que el mínimo ({})",
            n.punto_maximo, n.punto_minimo
        )));
    }
    if !(0..=100).contains(&n.inc_porcentaje) {
        return Err(Error::Regla(format!(
            "tarifa de INC imposible: {} %",
            n.inc_porcentaje
        )));
    }
    if n.precio_venta_estimado.is_some_and(|v| v < 0)
        || n.costos_para_vender.is_some_and(|v| v < 0)
    {
        return Err(Error::Regla(
            "el precio estimado y los costos para vender no pueden ser negativos".into(),
        ));
    }
    Ok(())
}

/// Clasifica el código de barras bajo su estándar REAL.
///
/// Un código que no pasa el dígito de control de GS1 **no es un GTIN**, y
/// declararlo como tal ante la DIAN es informar un identificador internacional
/// que no existe. Se guarda como código interno (`@schemeID = 999`), que es la
/// verdad y además es válido.
fn clasificar_barras(bruto: Option<&String>) -> (Option<String>, Option<&'static str>) {
    match barras(bruto) {
        None => (None, None),
        Some(c) => {
            let e = match inventario_dominio::gtin::clasificar(&c) {
                inventario_dominio::gtin::Estandar::Gtin => "gtin",
                inventario_dominio::gtin::Estandar::Unspsc => "unspsc",
                inventario_dominio::gtin::Estandar::Interno => "interno",
            };
            (Some(c), Some(e))
        }
    }
}

/// Qué le falta al artículo para poder ir en una factura electrónica
/// (Res. DIAN 000165 de 2023, art. 11).
fn falta_fe(unidad: &Option<String>, tipo_estandar: &Option<String>) -> Vec<String> {
    let mut f = Vec::new();
    if unidad.as_deref().unwrap_or("").trim().is_empty() {
        f.push("unidad de medida".into());
    }
    if tipo_estandar.is_none() {
        f.push("código de producto con estándar declarado".into());
    }
    f
}

pub async fn crear(pool: &Pool, n: &Nuevo) -> Resultado<i64> {
    validar(n)?;
    let (cod_est, tipo_est) = clasificar_barras(n.codigo_barras.as_ref());
    let id = sqlx::query(
        "INSERT INTO articulo
            (codigo, codigo_barras, descripcion, presentacion, categoria_id,
             iva_porcentaje, inc_porcentaje, precio_venta, punto_minimo, punto_maximo,
             unidad_medida, codigo_estandar, tipo_estandar,
             precio_venta_estimado, costos_para_vender)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(n.codigo.trim())
    .bind(barras(n.codigo_barras.as_ref()))
    .bind(n.descripcion.trim())
    .bind(n.presentacion.trim())
    .bind(n.categoria_id)
    .bind(n.iva_porcentaje)
    .bind(n.inc_porcentaje)
    .bind(n.precio_venta)
    .bind(n.punto_minimo)
    .bind(n.punto_maximo)
    .bind(n.unidad_medida.as_deref().map(str::trim).filter(|u| !u.is_empty()))
    .bind(&cod_est)
    .bind(tipo_est)
    .bind(n.precio_venta_estimado)
    .bind(n.costos_para_vender)
    .fetch_one(pool)
    .await?
    .get("id");
    Ok(id)
}

pub async fn actualizar(pool: &Pool, id: i64, n: &Nuevo) -> Resultado<()> {
    validar(n)?;
    let (cod_est, tipo_est) = clasificar_barras(n.codigo_barras.as_ref());
    let r = sqlx::query(
        "UPDATE articulo SET codigo = ?, codigo_barras = ?, descripcion = ?, presentacion = ?,
                categoria_id = ?, iva_porcentaje = ?, inc_porcentaje = ?, precio_venta = ?,
                punto_minimo = ?, punto_maximo = ?, unidad_medida = ?,
                codigo_estandar = ?, tipo_estandar = ?,
                precio_venta_estimado = ?, costos_para_vender = ?
          WHERE id = ?",
    )
    .bind(n.codigo.trim())
    .bind(barras(n.codigo_barras.as_ref()))
    .bind(n.descripcion.trim())
    .bind(n.presentacion.trim())
    .bind(n.categoria_id)
    .bind(n.iva_porcentaje)
    .bind(n.inc_porcentaje)
    .bind(n.precio_venta)
    .bind(n.punto_minimo)
    .bind(n.punto_maximo)
    .bind(n.unidad_medida.as_deref().map(str::trim).filter(|u| !u.is_empty()))
    .bind(&cod_est)
    .bind(tipo_est)
    .bind(n.precio_venta_estimado)
    .bind(n.costos_para_vender)
    .bind(id)
    .execute(pool)
    .await?;
    if r.rows_affected() == 0 {
        return Err(Error::NoExiste { que: "artículo", id });
    }
    Ok(())
}

/// Desactiva un artículo. **No lo borra**: tiene kardex e historia de ventas
/// detrás, y la clave foránea con `ON DELETE RESTRICT` lo impediría de todos
/// modos. Un artículo inactivo no se puede facturar pero su historia se lee.
pub async fn desactivar(pool: &Pool, id: i64) -> Resultado<()> {
    let r = sqlx::query("UPDATE articulo SET activo = 0 WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if r.rows_affected() == 0 {
        return Err(Error::NoExiste { que: "artículo", id });
    }
    Ok(())
}

const SELECT: &str = "
    SELECT a.id, a.codigo, a.codigo_barras, a.descripcion, a.presentacion, a.categoria_id,
           a.iva_porcentaje, a.inc_porcentaje, a.precio_venta, a.punto_minimo, a.punto_maximo,
           a.unidad_medida, a.codigo_estandar, a.tipo_estandar,
           a.precio_venta_estimado, a.costos_para_vender, a.activo,
           COALESCE(m.existencia_despues, 0) AS existencia,
           COALESCE(m.valor_despues, 0)      AS valor
      FROM articulo a
      LEFT JOIN movimiento m ON m.id = (
           SELECT id FROM movimiento WHERE articulo_id = a.id ORDER BY id DESC LIMIT 1)";

fn leer(f: sqlx::sqlite::SqliteRow) -> Articulo {
    let existencia: i64 = f.get("existencia");
    let valor: i64 = f.get("valor");
    let unidad_medida: Option<String> = f.get("unidad_medida");
    let tipo_estandar: Option<String> = f.get("tipo_estandar");
    Articulo {
        falta_para_factura_electronica: falta_fe(&unidad_medida, &tipo_estandar),
        inc_porcentaje: f.get("inc_porcentaje"),
        unidad_medida,
        codigo_estandar: f.get("codigo_estandar"),
        tipo_estandar,
        precio_venta_estimado: f.get("precio_venta_estimado"),
        costos_para_vender: f.get("costos_para_vender"),
        id: f.get("id"),
        codigo: f.get("codigo"),
        codigo_barras: f.get("codigo_barras"),
        descripcion: f.get("descripcion"),
        presentacion: f.get("presentacion"),
        categoria_id: f.get("categoria_id"),
        iva_porcentaje: f.get("iva_porcentaje"),
        precio_venta: f.get("precio_venta"),
        punto_minimo: f.get::<i64, _>("punto_minimo") * MIL,
        punto_maximo: f.get::<i64, _>("punto_maximo") * MIL,
        activo: f.get::<i64, _>("activo") == 1,
        existencia,
        costo_promedio: Saldo {
            existencia: Cantidad(existencia),
            valor: Centavos(valor),
        }
        // `.ok()`: en un LISTADO, «no hay existencia» y «el saldo es imposible»
        // se dibujan igual —una celda vacía— porque no hay promedio que enseñar
        // en ninguno de los dos. Lo que no puede es colapsarse donde se COSTEA,
        // que es de donde vino el defecto (ver `Saldo::costo_promedio`).
        .costo_promedio()
        .ok()
        .map(|c| c.0),
    }
}

/// Busca por código, código de barras o parte de la descripción.
pub async fn listar(pool: &Pool, busqueda: &str, incluir_inactivos: bool) -> Resultado<Vec<Articulo>> {
    let patron = format!("%{}%", busqueda.trim());
    let sql = format!(
        "{SELECT} WHERE (? = 1 OR a.activo = 1)
           AND (?2 = '' OR a.codigo = ?2 OR a.codigo_barras = ?2 OR a.descripcion LIKE ?3)
         ORDER BY a.descripcion LIMIT 500"
    );
    let filas = sqlx::query(&sql)
        .bind(i64::from(incluir_inactivos))
        .bind(busqueda.trim())
        .bind(&patron)
        .fetch_all(pool)
        .await?;
    Ok(filas.into_iter().map(leer).collect())
}

pub async fn uno(pool: &Pool, id: i64) -> Resultado<Articulo> {
    let sql = format!("{SELECT} WHERE a.id = ?");
    sqlx::query(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .map(leer)
        .ok_or(Error::NoExiste { que: "artículo", id })
}

/// El precio que le toca a un cliente: el de su convenio si lo tiene, y si no
/// el precio de lista.
///
/// La cascada es explícita a propósito (directiva 19): ningún cliente tiene que
/// configurar convenios si no los usa, y ninguno se queda sin precio.
pub async fn precio_para(pool: &Pool, articulo_id: i64, tercero_id: Option<i64>) -> Resultado<i64> {
    if let Some(t) = tercero_id {
        let p: Option<(i64,)> = sqlx::query_as(
            "SELECT pc.precio
               FROM tercero t
               JOIN precio_convenio pc ON pc.convenio_id = t.convenio_id
              WHERE t.id = ? AND pc.articulo_id = ?",
        )
        .bind(t)
        .bind(articulo_id)
        .fetch_optional(pool)
        .await?;
        if let Some((precio,)) = p {
            return Ok(precio);
        }
    }
    let base: Option<(i64,)> = sqlx::query_as("SELECT precio_venta FROM articulo WHERE id = ?")
        .bind(articulo_id)
        .fetch_optional(pool)
        .await?;
    base.map(|(p,)| p).ok_or(Error::NoExiste {
        que: "artículo",
        id: articulo_id,
    })
}

#[cfg(test)]
mod pruebas {

    /// **LA UNIDAD DE `precio_venta` ES CENTAVOS, y esto lo FIJA.**
    ///
    /// No existía, y ésa era la causa de fondo: hasta el 2026-08-12 la columna
    /// tenía DOS convenciones a la vez —el migrador de VFP y la demo escribían
    /// centavos, el panel escribía pesos— y **ninguna prueba las distinguía**.
    /// Las que hay comparan un número consigo mismo (round-trip), así que pasan
    /// en verde con las dos: es el positivo de control que faltaba.
    ///
    /// El daño no era teórico. En una lectura, un artículo creado desde el panel
    /// a 12.000 valía $120; en la otra, todo lo migrado de un cliente real estaba
    /// 100 veces alto. No había lectura sin un error de 100×.
    ///
    /// La comparación es contra `Centavos::de_pesos`, no contra un literal: si
    /// alguien cambiara la escala del tipo, esto se movería con ella.
    #[tokio::test]
    async fn el_precio_de_venta_se_guarda_en_centavos() {
        let pool = crate::prueba_apoyo::base().await;
        let mut n = nuevo("UNI-1");
        n.precio_venta = Centavos::de_pesos(12_000).0;
        let id = crear(&pool, &n).await.unwrap();
        let a = uno(&pool, id).await.unwrap();
        assert_eq!(
            a.precio_venta,
            Centavos::de_pesos(12_000).0,
            "la columna dejó de estar en centavos: el panel y el migrador vuelven a discrepar"
        );
        // Y la pareja, que es la que da sentido a la de arriba: 12.000 PESOS no
        // son 12.000 en la columna. Sin esto, la aserción pasaría igual con la
        // convención equivocada.
        assert_ne!(a.precio_venta, 12_000, "se guardaron pesos donde van centavos");
    }

    use super::*;
    use crate::prueba_apoyo;

    fn nuevo(codigo: &str) -> Nuevo {
        Nuevo {
            codigo: codigo.into(),
            codigo_barras: None,
            descripcion: format!("Artículo {codigo}"),
            presentacion: String::new(),
            categoria_id: None,
            iva_porcentaje: 19,
            precio_venta: 500_000,
            punto_minimo: 0,
            punto_maximo: 0,
            inc_porcentaje: 0,
            unidad_medida: None,
            precio_venta_estimado: None,
            costos_para_vender: None,
        }
    }

    /// El caso exacto del corpus: 85 artículos sin código de barras. Con `''`
    /// en vez de NULL, el segundo no se podría crear.
    #[tokio::test]
    async fn muchos_articulos_sin_codigo_de_barras_conviven() {
        let pool = prueba_apoyo::base().await;
        for i in 0..85 {
            let mut n = nuevo(&format!("A{i}"));
            n.codigo_barras = Some("   ".into()); // vacío con espacios
            crear(&pool, &n).await.unwrap();
        }
        assert_eq!(listar(&pool, "", false).await.unwrap().len(), 85);

        // Pero dos artículos CON el mismo código de barras sí chocan.
        let mut a = nuevo("X1");
        a.codigo_barras = Some("41251501".into());
        crear(&pool, &a).await.unwrap();
        let mut b = nuevo("X2");
        b.codigo_barras = Some("41251501".into());
        assert!(crear(&pool, &b).await.is_err());
    }

    /// El código de barras se clasifica bajo su estándar REAL. Uno que no pasa
    /// el dígito de control de GS1 no es un GTIN, y declararlo como tal le
    /// informaría a la DIAN un identificador internacional inexistente.
    #[tokio::test]
    async fn el_codigo_de_barras_se_clasifica_por_su_digito_de_control() {
        let pool = prueba_apoyo::base().await;

        let mut bueno = nuevo("BUENO");
        bueno.codigo_barras = Some("4006381333931".into()); // GTIN-13 válido
        let id = crear(&pool, &bueno).await.unwrap();
        assert_eq!(uno(&pool, id).await.unwrap().tipo_estandar.as_deref(), Some("gtin"));

        // El del corpus heredado: parece GTIN-8 y no lo es.
        let mut malo = nuevo("MALO");
        malo.codigo_barras = Some("41251501".into());
        let id = crear(&pool, &malo).await.unwrap();
        assert_eq!(
            uno(&pool, id).await.unwrap().tipo_estandar.as_deref(),
            Some("interno"),
            "un código que falla su dígito de control NO puede declararse GTIN"
        );
    }

    /// La lista de artículos DICE qué le falta a cada uno para poder ir en una
    /// factura electrónica. Descubrirlo al emitir es descubrirlo tarde.
    #[tokio::test]
    async fn el_articulo_dice_que_le_falta_para_la_factura_electronica() {
        let pool = prueba_apoyo::base().await;
        let id = crear(&pool, &nuevo("PELADO")).await.unwrap();
        let a = uno(&pool, id).await.unwrap();
        assert_eq!(a.falta_para_factura_electronica.len(), 2);
        assert!(a.falta_para_factura_electronica.iter().any(|f| f.contains("unidad")));

        let mut completo = nuevo("COMPLETO");
        completo.unidad_medida = Some("94".into());
        completo.codigo_barras = Some("4006381333931".into());
        let id = crear(&pool, &completo).await.unwrap();
        assert!(
            uno(&pool, id).await.unwrap().falta_para_factura_electronica.is_empty(),
            "con unidad y estándar no debería faltar nada"
        );
    }

    #[tokio::test]
    async fn el_codigo_no_se_repite() {
        let pool = prueba_apoyo::base().await;
        crear(&pool, &nuevo("A1")).await.unwrap();
        assert!(crear(&pool, &nuevo("A1")).await.is_err());
    }

    #[tokio::test]
    async fn se_rechaza_lo_imposible_pero_no_lo_inusual() {
        let pool = prueba_apoyo::base().await;
        let mut n = nuevo("A1");
        n.iva_porcentaje = 101;
        assert!(crear(&pool, &n).await.is_err());
        n.iva_porcentaje = 16; // el del corpus de 2004: inusual, no imposible
        assert!(crear(&pool, &n).await.is_ok());

        let mut n = nuevo("A2");
        n.punto_minimo = 50;
        n.punto_maximo = 10;
        assert!(crear(&pool, &n).await.is_err(), "máximo por debajo del mínimo");

        let mut n = nuevo("A3");
        n.descripcion = "  ".into();
        assert!(crear(&pool, &n).await.is_err());
    }

    #[tokio::test]
    async fn la_busqueda_encuentra_por_codigo_barras_y_descripcion() {
        let pool = prueba_apoyo::base().await;
        let mut n = nuevo("1102001");
        n.descripcion = "Pet De 335 C.c. Unidad".into();
        n.codigo_barras = Some("41251501".into());
        crear(&pool, &n).await.unwrap();
        crear(&pool, &nuevo("OTRO")).await.unwrap();

        assert_eq!(listar(&pool, "1102001", false).await.unwrap().len(), 1);
        assert_eq!(listar(&pool, "41251501", false).await.unwrap().len(), 1);
        assert_eq!(listar(&pool, "Pet De", false).await.unwrap().len(), 1);
        assert_eq!(listar(&pool, "", false).await.unwrap().len(), 2);
        assert_eq!(listar(&pool, "no existe", false).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn desactivar_no_borra_y_lo_saca_de_la_lista() {
        let pool = prueba_apoyo::base().await;
        let id = crear(&pool, &nuevo("A1")).await.unwrap();
        desactivar(&pool, id).await.unwrap();
        assert!(listar(&pool, "", false).await.unwrap().is_empty());
        assert_eq!(listar(&pool, "", true).await.unwrap().len(), 1);
        assert!(uno(&pool, id).await.is_ok(), "la historia sigue accesible");
    }

    #[tokio::test]
    async fn el_precio_baja_por_la_cascada_convenio_y_luego_lista() {
        let pool = prueba_apoyo::base().await;
        let a = crear(&pool, &nuevo("A1")).await.unwrap();
        let c: i64 = sqlx::query("INSERT INTO convenio (codigo, nombre) VALUES ('01','Distribuidor') RETURNING id")
            .fetch_one(&pool).await.unwrap().get("id");
        let t: i64 = sqlx::query(
            "INSERT INTO tercero (nit, dv, nombre, convenio_id) VALUES ('1', 0, 'Cli', ?) RETURNING id")
            .bind(c).fetch_one(&pool).await.unwrap().get("id");
        let sin: i64 = sqlx::query("INSERT INTO tercero (nit, dv, nombre) VALUES ('2', 0, 'Otro') RETURNING id")
            .fetch_one(&pool).await.unwrap().get("id");

        // Sin precio de convenio todavía: cae al de lista.
        assert_eq!(precio_para(&pool, a, Some(t)).await.unwrap(), 500_000);

        sqlx::query("INSERT INTO precio_convenio (convenio_id, articulo_id, precio) VALUES (?,?,?)")
            .bind(c).bind(a).bind(400_000i64).execute(&pool).await.unwrap();
        assert_eq!(precio_para(&pool, a, Some(t)).await.unwrap(), 400_000);
        // El cliente sin convenio sigue en el precio de lista.
        assert_eq!(precio_para(&pool, a, Some(sin)).await.unwrap(), 500_000);
        assert_eq!(precio_para(&pool, a, None).await.unwrap(), 500_000);
        // Y un artículo que no existe falla, no devuelve cero.
        assert!(precio_para(&pool, 9999, None).await.is_err());
    }
}
