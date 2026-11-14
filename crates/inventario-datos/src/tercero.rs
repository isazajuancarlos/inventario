//! Clientes y proveedores.
//!
//! Una sola tabla para los dos: en el corpus estaban en `CLIENTES` y
//! `PROVEEDOR` por separado, y el mismo negocio aparecía dos veces con la
//! dirección escrita distinta.

use crate::{Error, Pool, Resultado};
use inventario_dominio::nit;
use sqlx::Row;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Tercero {
    pub id: i64,
    pub nit: String,
    pub dv: i64,
    pub nit_completo: String,
    pub nombre: String,
    pub es_cliente: bool,
    pub es_proveedor: bool,
    pub direccion: String,
    pub telefono: String,
    pub ciudad_id: Option<i64>,
    pub zona_id: Option<i64>,
    pub barrio_id: Option<i64>,
    pub vendedor_id: Option<i64>,
    pub convenio_id: Option<i64>,
    pub plazo_dias: i64,
    pub activo: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Nuevo {
    /// Tal como lo teclean: con puntos, con guion o pelado. Se normaliza aquí.
    pub nit: String,
    pub nombre: String,
    #[serde(default = "si")]
    pub es_cliente: bool,
    #[serde(default)]
    pub es_proveedor: bool,
    #[serde(default)]
    pub direccion: String,
    #[serde(default)]
    pub telefono: String,
    #[serde(default)]
    pub telefono2: String,
    #[serde(default)]
    pub correo: String,
    #[serde(default)]
    pub ciudad_id: Option<i64>,
    #[serde(default)]
    pub zona_id: Option<i64>,
    #[serde(default)]
    pub barrio_id: Option<i64>,
    #[serde(default)]
    pub vendedor_id: Option<i64>,
    #[serde(default)]
    pub convenio_id: Option<i64>,
    #[serde(default)]
    pub plazo_dias: i64,
}

fn si() -> bool {
    true
}

fn preparar(n: &Nuevo) -> Resultado<(String, u32)> {
    if n.nombre.trim().is_empty() {
        return Err(Error::Regla("el tercero necesita un nombre".into()));
    }
    if !n.es_cliente && !n.es_proveedor {
        return Err(Error::Regla("un tercero es cliente, proveedor o las dos cosas".into()));
    }
    if n.plazo_dias < 0 {
        return Err(Error::Regla("el plazo no puede ser negativo".into()));
    }
    // El plazo del corpus venía en C(3) con valores como '0.' y ''. Aquí es un
    // entero o no entra.
    let limpio = nit::normalizar(&n.nit)
        .ok_or_else(|| Error::Regla(format!("«{}» no es un NIT o cédula", n.nit)))?;
    let dv = nit::digito_verificacion(&limpio).expect("normalizado ya");
    Ok((limpio, dv))
}

pub async fn crear(pool: &Pool, n: &Nuevo) -> Resultado<i64> {
    let (limpio, dv) = preparar(n)?;
    let id = sqlx::query(
        "INSERT INTO tercero
            (nit, dv, nombre, es_cliente, es_proveedor, direccion, telefono, telefono2,
             correo, ciudad_id, zona_id, barrio_id, vendedor_id, convenio_id, plazo_dias)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(&limpio)
    .bind(dv as i64)
    .bind(n.nombre.trim())
    .bind(i64::from(n.es_cliente))
    .bind(i64::from(n.es_proveedor))
    .bind(n.direccion.trim())
    .bind(n.telefono.trim())
    .bind(n.telefono2.trim())
    .bind(n.correo.trim())
    .bind(n.ciudad_id)
    .bind(n.zona_id)
    .bind(n.barrio_id)
    .bind(n.vendedor_id)
    .bind(n.convenio_id)
    .bind(n.plazo_dias)
    .fetch_one(pool)
    .await?
    .get("id");
    Ok(id)
}

pub async fn actualizar(pool: &Pool, id: i64, n: &Nuevo) -> Resultado<()> {
    let (limpio, dv) = preparar(n)?;
    let r = sqlx::query(
        "UPDATE tercero SET nit = ?, dv = ?, nombre = ?, es_cliente = ?, es_proveedor = ?,
                direccion = ?, telefono = ?, telefono2 = ?, correo = ?, ciudad_id = ?,
                zona_id = ?, barrio_id = ?, vendedor_id = ?, convenio_id = ?, plazo_dias = ?
          WHERE id = ?",
    )
    .bind(&limpio)
    .bind(dv as i64)
    .bind(n.nombre.trim())
    .bind(i64::from(n.es_cliente))
    .bind(i64::from(n.es_proveedor))
    .bind(n.direccion.trim())
    .bind(n.telefono.trim())
    .bind(n.telefono2.trim())
    .bind(n.correo.trim())
    .bind(n.ciudad_id)
    .bind(n.zona_id)
    .bind(n.barrio_id)
    .bind(n.vendedor_id)
    .bind(n.convenio_id)
    .bind(n.plazo_dias)
    .bind(id)
    .execute(pool)
    .await?;
    if r.rows_affected() == 0 {
        return Err(Error::NoExiste { que: "tercero", id });
    }
    Ok(())
}

fn leer(f: sqlx::sqlite::SqliteRow) -> Tercero {
    let nit_: String = f.get("nit");
    let dv: i64 = f.get("dv");
    Tercero {
        nit_completo: format!("{nit_}-{dv}"),
        id: f.get("id"),
        nit: nit_,
        dv,
        nombre: f.get("nombre"),
        es_cliente: f.get::<i64, _>("es_cliente") == 1,
        es_proveedor: f.get::<i64, _>("es_proveedor") == 1,
        direccion: f.get("direccion"),
        telefono: f.get("telefono"),
        ciudad_id: f.get("ciudad_id"),
        zona_id: f.get("zona_id"),
        barrio_id: f.get("barrio_id"),
        vendedor_id: f.get("vendedor_id"),
        convenio_id: f.get("convenio_id"),
        plazo_dias: f.get("plazo_dias"),
        activo: f.get::<i64, _>("activo") == 1,
    }
}

const CAMPOS: &str = "id, nit, dv, nombre, es_cliente, es_proveedor, direccion, telefono,
                      ciudad_id, zona_id, barrio_id, vendedor_id, convenio_id, plazo_dias, activo";

pub async fn listar(pool: &Pool, busqueda: &str, solo: Option<&str>) -> Resultado<Vec<Tercero>> {
    let patron = format!("%{}%", busqueda.trim());
    // El NIT se busca ya normalizado: si escriben «890.934.562-5» tiene que
    // encontrar al mismo que si escriben «890934562».
    let nit_busqueda = nit::normalizar(busqueda).unwrap_or_default();
    let filtro = match solo {
        Some("cliente") => "AND es_cliente = 1",
        Some("proveedor") => "AND es_proveedor = 1",
        _ => "",
    };
    let sql = format!(
        "SELECT {CAMPOS} FROM tercero
          WHERE activo = 1 {filtro}
            AND (?1 = '' OR nombre LIKE ?1 OR nit = ?2)
          ORDER BY nombre LIMIT 500"
    );
    let filas = sqlx::query(&sql)
        .bind(if busqueda.trim().is_empty() { String::new() } else { patron })
        .bind(nit_busqueda)
        .fetch_all(pool)
        .await?;
    Ok(filas.into_iter().map(leer).collect())
}

pub async fn uno(pool: &Pool, id: i64) -> Resultado<Tercero> {
    let sql = format!("SELECT {CAMPOS} FROM tercero WHERE id = ?");
    sqlx::query(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .map(leer)
        .ok_or(Error::NoExiste { que: "tercero", id })
}

/// Terceros que comparten NIT. No es un error —dos sedes de la misma empresa lo
/// hacen— pero el corpus tenía 47 y conviene poder verlos.
#[derive(Debug, serde::Serialize)]
pub struct NitRepetido {
    pub nit: String,
    pub cuantos: i64,
    pub nombres: String,
}

pub async fn nits_repetidos(pool: &Pool) -> Resultado<Vec<NitRepetido>> {
    let filas = sqlx::query(
        "SELECT nit, COUNT(*) AS cuantos, GROUP_CONCAT(nombre, ' | ') AS nombres
           FROM tercero GROUP BY nit HAVING COUNT(*) > 1 ORDER BY cuantos DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(filas
        .into_iter()
        .map(|f| NitRepetido {
            nit: f.get("nit"),
            cuantos: f.get("cuantos"),
            nombres: f.get("nombres"),
        })
        .collect())
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::prueba_apoyo;

    fn nuevo(nit: &str, nombre: &str) -> Nuevo {
        Nuevo {
            nit: nit.into(),
            nombre: nombre.into(),
            es_cliente: true,
            es_proveedor: false,
            direccion: String::new(),
            telefono: String::new(),
            telefono2: String::new(),
            correo: String::new(),
            ciudad_id: None,
            zona_id: None,
            barrio_id: None,
            vendedor_id: None,
            convenio_id: None,
            plazo_dias: 0,
        }
    }

    #[tokio::test]
    async fn el_nit_se_normaliza_y_el_dv_se_calcula() {
        let pool = prueba_apoyo::base().await;
        let id = crear(&pool, &nuevo("890.934.562-9", "Asistir Ltda.")).await.unwrap();
        let t = uno(&pool, id).await.unwrap();
        assert_eq!(t.nit, "890934562");
        assert_eq!(t.dv, 5, "el DV se recalcula, no se cree el que tecleen");
        assert_eq!(t.nit_completo, "890934562-5");
    }

    #[tokio::test]
    async fn se_encuentra_igual_con_puntos_que_sin_ellos() {
        let pool = prueba_apoyo::base().await;
        crear(&pool, &nuevo("890934562", "Asistir Ltda.")).await.unwrap();
        assert_eq!(listar(&pool, "890.934.562-5", None).await.unwrap().len(), 1);
        assert_eq!(listar(&pool, "890934562", None).await.unwrap().len(), 1);
        assert_eq!(listar(&pool, "Asistir", None).await.unwrap().len(), 1);
    }

    /// El corpus tenía 47 NIT repetidos. No se prohíben —dos sedes existen—
    /// pero se pueden listar.
    #[tokio::test]
    async fn los_nit_repetidos_se_admiten_y_se_pueden_ver() {
        let pool = prueba_apoyo::base().await;
        crear(&pool, &nuevo("52581553", "Sede Norte")).await.unwrap();
        crear(&pool, &nuevo("52581553", "Sede Sur")).await.unwrap();
        crear(&pool, &nuevo("70087299", "Otro")).await.unwrap();
        let r = nits_repetidos(&pool).await.unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].cuantos, 2);
        assert!(r[0].nombres.contains("Sede Norte") && r[0].nombres.contains("Sede Sur"));
    }

    #[tokio::test]
    async fn lo_que_no_es_un_nit_no_entra() {
        let pool = prueba_apoyo::base().await;
        assert!(crear(&pool, &nuevo("", "Sin NIT")).await.is_err());
        assert!(crear(&pool, &nuevo("ABC", "Letras")).await.is_err());
        assert!(crear(&pool, &nuevo("1", "")).await.is_err(), "sin nombre");

        let mut n = nuevo("1", "Ni cliente ni proveedor");
        n.es_cliente = false;
        assert!(crear(&pool, &n).await.is_err());
    }

    #[tokio::test]
    async fn el_filtro_separa_clientes_de_proveedores() {
        let pool = prueba_apoyo::base().await;
        crear(&pool, &nuevo("1", "Cliente")).await.unwrap();
        let mut p = nuevo("2", "Proveedor");
        p.es_cliente = false;
        p.es_proveedor = true;
        crear(&pool, &p).await.unwrap();
        let mut ambos = nuevo("3", "Ambos");
        ambos.es_proveedor = true;
        crear(&pool, &ambos).await.unwrap();

        assert_eq!(listar(&pool, "", Some("cliente")).await.unwrap().len(), 2);
        assert_eq!(listar(&pool, "", Some("proveedor")).await.unwrap().len(), 2);
        assert_eq!(listar(&pool, "", None).await.unwrap().len(), 3);
    }
}
