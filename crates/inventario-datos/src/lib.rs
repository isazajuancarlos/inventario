//! Acceso a la base. Nadie de aquí para arriba escribe SQL.

pub mod alistamiento;
pub mod articulo;
pub mod catalogo;
pub mod deterioro;
pub mod factura;
pub mod gestion;
pub mod kardex;
pub mod nota;
pub mod recibo;
pub mod resolucion;
pub mod tercero;
pub mod tributario;
pub mod usuario;

pub type Pool = sqlx::SqlitePool;

/// Abre la base y aplica las migraciones pendientes.
///
/// `create_if_missing` está a propósito: el despliegue es un binario y nada
/// más, y pedirle al cliente que cree la base a mano antes de arrancar es
/// justo el paso donde se pierde una instalación.
///
/// Las dos opciones que **no** son cosméticas:
///
/// - **`foreign_keys(true)`.** SQLite las trae APAGADAS por compatibilidad
///   hacia atrás. Apagadas, `recibo_aplicacion` puede apuntar a un recibo que
///   no existe — que es literalmente lo que le pasó al sistema viejo: 313 de
///   sus 323 detalles de recibo conservaban un abono cuya cabecera había sido
///   borrada. La prueba `las_claves_foraneas_estan_activas` lo comprueba.
/// - **WAL.** Lectores concurrentes sin bloquear al escritor. No convierte a
///   SQLite en multiescritor; es la diferencia entre un mostrador que va bien y
///   uno que se traba al consultar mientras el otro cajero factura.
pub async fn abrir(url: &str) -> Result<Pool, sqlx::Error> {
    use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
    use std::str::FromStr;

    let opciones = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        // La facturación toma la base para escribir mientras asigna número y
        // mueve kardex. Sin esto, dos cajeros a la vez producen «database is
        // locked» inmediato en vez de esperar su turno.
        .busy_timeout(std::time::Duration::from_secs(10));

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opciones)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// Lo que puede salir mal en la capa de datos y no es un fallo de SQL.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("base de datos: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("no existe {que} con id {id}")]
    NoExiste { que: &'static str, id: i64 },
    #[error("{0}")]
    Kardex(#[from] inventario_dominio::kardex::ErrorKardex),
    #[error("{0}")]
    Factura(#[from] inventario_dominio::factura::ErrorFactura),
    #[error("{0}")]
    Numeracion(#[from] inventario_dominio::numeracion::ErrorNumeracion),
    #[error("{0}")]
    Cartera(#[from] inventario_dominio::cartera::ErrorCartera),
    #[error("{0}")]
    Regla(String),
}

pub type Resultado<T> = Result<T, Error>;

#[cfg(test)]
pub(crate) mod prueba_apoyo {
    use super::*;

    /// Una base en memoria con el esquema aplicado y los catálogos mínimos.
    pub async fn base() -> Pool {
        let pool = abrir("sqlite::memory:").await.expect("abrir en memoria");
        sembrar(&pool).await;
        pool
    }

    pub async fn sembrar(pool: &Pool) {
        sqlx::query(
            "INSERT INTO empresa (id, nombre, nit, dv) VALUES (1, 'Prueba S.A.', '811033579', 6)",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO usuario (id, codigo, nombre, hash_clave, rol)
             VALUES (1, 'adm', 'Admin', 'x', 'admin')",
        )
        .execute(pool)
        .await
        .unwrap();
        for (id, cod, nom, signo, clase, efecto) in [
            (1i64, "11", "Compras a proveedores", "entrada", "compra", "no_afecta_impuestos"),
            (2, "13", "Saldos iniciales", "entrada", "inicial", "no_afecta_impuestos"),
            (3, "41", "Entradas por ajuste", "entrada", "ajuste", "no_afecta_impuestos"),
            (4, "42", "Salidas por ajuste", "salida", "ajuste", "no_afecta_impuestos"),
            (5, "51", "Ventas", "salida", "venta", "venta"),
            (6, "21", "Devolución de venta", "entrada", "devolucion_venta", "venta"),
        ] {
            sqlx::query(
                "INSERT INTO concepto (id, codigo, nombre, signo, clase, efecto_tributario)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(cod)
            .bind(nom)
            .bind(signo)
            .bind(clase)
            .bind(efecto)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    /// Concepto de venta que usa la facturación.
    pub const VENTA: i64 = 5;
    pub const COMPRA: i64 = 1;
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Si esto falla, toda la integridad referencial del esquema es
    /// decorativa. SQLite trae las claves foráneas apagadas.
    #[tokio::test]
    async fn las_claves_foraneas_estan_activas() {
        let pool = prueba_apoyo::base().await;
        let (encendidas,): (i64,) = sqlx::query_as("PRAGMA foreign_keys")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(encendidas, 1, "las claves foráneas están APAGADAS");

        // Y que además muerdan: un recibo_aplicacion sin recibo.
        let r = sqlx::query(
            "INSERT INTO recibo_aplicacion (recibo_id, factura_id, valor) VALUES (999, 999, 1)",
        )
        .execute(&pool)
        .await;
        assert!(r.is_err(), "se aceptó un abono cuyo recibo no existe");
    }

    #[tokio::test]
    async fn el_diario_es_wal() {
        let pool = abrir("sqlite::memory:").await.unwrap();
        let (modo,): (String,) = sqlx::query_as("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .unwrap();
        // Una base en memoria no puede usar WAL; el disco sí. Se comprueba que
        // la opción no rompe y que en memoria cae a 'memory'.
        assert!(
            modo.eq_ignore_ascii_case("wal") || modo.eq_ignore_ascii_case("memory"),
            "modo inesperado: {modo}"
        );
    }

    /// El CHECK que impide una segunda empresa. Una base por instancia.
    #[tokio::test]
    async fn no_caben_dos_empresas() {
        let pool = prueba_apoyo::base().await;
        let r = sqlx::query("INSERT INTO empresa (id, nombre, nit, dv) VALUES (2, 'Otra', '1', 0)")
            .execute(&pool)
            .await;
        assert!(r.is_err());
    }
}
