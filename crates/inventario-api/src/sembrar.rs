//! `inventario-api instalar`: deja la base lista para operar.
//!
//! Vive en el binario y no en un SQL suelto por la misma razón que `hashear`:
//! el hash del administrador tiene que salir de los MISMOS parámetros de Argon2
//! que verifica el acceso. Un hash de otra herramienta deja al dueño del
//! sistema fuera de su propio sistema, y el fallo aparece lejos de su causa.
//!
//! Es **idempotente**: se puede correr en cada arranque sin duplicar nada.

use crate::claves;
use std::process::ExitCode;

/// Los conceptos sin los cuales el sistema no funciona. Los demás los crea el
/// cliente o los trae la migración.
/// `(código, nombre, signo, clase, efecto tributario)`.
///
/// El efecto tributario va explícito en cada fila, no deducido de la clase: es
/// lo que decide si un movimiento liquida IVA, consume el tope del art. 64 o
/// exige acta. Los tres últimos cubren los casos que el sistema anterior tenía
/// sueltos —obsequios, faltantes y bajas— y que le costaron 161 movimientos sin
/// impuesto.
const CONCEPTOS: [(&str, &str, &str, &str, &str); 9] = [
    ("VTA", "Ventas", "salida", "venta", "venta"),
    ("DEV", "Devolución de venta", "entrada", "devolucion_venta", "venta"),
    ("COM", "Compras a proveedores", "entrada", "compra", "no_afecta_impuestos"),
    ("INI", "Saldo inicial", "entrada", "inicial", "no_afecta_impuestos"),
    ("AJE", "Entrada por ajuste de inventario", "entrada", "ajuste", "no_afecta_impuestos"),
    ("AJS", "Salida por ajuste de inventario", "salida", "ajuste", "no_afecta_impuestos"),
    // ET art. 421 lit. b): causa IVA sobre el valor comercial.
    ("RET", "Retiro para uso propio, obsequios o socios", "salida", "consumo", "retiro_gravado"),
    // ET art. 64: consume el tope del 3 %.
    ("FAL", "Faltante de fácil destrucción o pérdida", "salida", "ajuste", "faltante_art64"),
    // ET art. 64: exige acta de destrucción.
    ("BAJ", "Baja por obsolescencia (con acta)", "salida", "ajuste", "baja_con_acta"),
];

pub async fn instalar() -> ExitCode {
    let url = std::env::var("INVENTARIO_BD").unwrap_or_else(|_| "sqlite://inventario.db".into());
    let clave = std::env::var("INVENTARIO_CLAVE_ADMIN").ok();

    let pool = match inventario_datos::abrir(&url).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("no se pudo abrir la base ({url}): {e}");
            return ExitCode::FAILURE;
        }
    };

    match sembrar(&pool, clave.as_deref()).await {
        Ok(mensajes) => {
            for m in mensajes {
                println!("{m}");
            }
            println!("Listo. Base: {url}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("instalación: {e}");
            ExitCode::FAILURE
        }
    }
}

pub async fn sembrar(
    pool: &inventario_datos::Pool,
    clave_admin: Option<&str>,
) -> Result<Vec<String>, String> {
    let mut msgs = Vec::new();

    for (codigo, nombre, signo, clase, efecto) in CONCEPTOS {
        sqlx::query(
            "INSERT OR IGNORE INTO concepto (codigo, nombre, signo, clase, efecto_tributario)
             VALUES (?,?,?,?,?)",
        )
        .bind(codigo)
        .bind(nombre)
        .bind(signo)
        .bind(clase)
        .bind(efecto)
        .execute(pool)
        .await
        .map_err(|e| format!("conceptos: {e}"))?;
    }
    msgs.push(format!("{} conceptos de kardex asegurados", CONCEPTOS.len()));

    let (hay_admin,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM usuario WHERE rol = 'admin'")
        .fetch_one(pool)
        .await
        .map_err(|e| format!("usuarios: {e}"))?;

    if hay_admin == 0 {
        // Sin clave NO se crea un administrador con una por defecto: un
        // «admin/admin» que nadie cambia es la puerta abierta que sobrevive a
        // todo lo demás. Se falla ruidosamente (directiva 20).
        let Some(clave) = clave_admin.filter(|c| c.len() >= 8) else {
            return Err(
                "no hay administrador y no se dio INVENTARIO_CLAVE_ADMIN (mínimo 8 \
                 caracteres). No se crea un usuario con clave por defecto."
                    .into(),
            );
        };
        let hash = claves::hashear(clave).ok_or("no se pudo hashear la clave")?;
        sqlx::query("INSERT INTO usuario (codigo, nombre, hash_clave, rol) VALUES (?,?,?,'admin')")
            .bind("admin")
            .bind("Administrador")
            .bind(hash)
            .execute(pool)
            .await
            .map_err(|e| format!("crear admin: {e}"))?;
        msgs.push("usuario «admin» creado".into());
    } else {
        msgs.push(format!("ya había {hay_admin} administrador(es): no se toca"));
    }

    let (hay_empresa,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM empresa")
        .fetch_one(pool)
        .await
        .map_err(|e| format!("empresa: {e}"))?;
    if hay_empresa == 0 {
        msgs.push(
            "AVISO: falta configurar la empresa (nombre y NIT). Hasta entonces la \
             facturación no debe usarse."
                .into(),
        );
    }

    Ok(msgs)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[tokio::test]
    async fn no_se_crea_un_admin_con_clave_por_defecto() {
        let pool = inventario_datos::abrir("sqlite::memory:").await.unwrap();
        assert!(sembrar(&pool, None).await.is_err(), "creó admin sin clave");
        assert!(sembrar(&pool, Some("corta")).await.is_err(), "aceptó una clave de 5");

        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM usuario")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn sembrar_dos_veces_no_duplica_nada() {
        let pool = inventario_datos::abrir("sqlite::memory:").await.unwrap();
        sembrar(&pool, Some("una clave larga")).await.unwrap();
        let contar = |t: &'static str| {
            let pool = pool.clone();
            async move {
                let sql = format!("SELECT COUNT(*) FROM {t}");
                sqlx::query_as::<_, (i64,)>(&sql).fetch_one(&pool).await.unwrap().0
            }
        };
        let (c1, u1) = (contar("concepto").await, contar("usuario").await);
        sembrar(&pool, Some("otra clave larga")).await.unwrap();
        assert_eq!(contar("concepto").await, c1);
        assert_eq!(contar("usuario").await, u1, "se creó un segundo admin");
    }

    /// El admin creado tiene que poder entrar con la clave que se dio: es el
    /// cruce entre el sembrador y el verificador, que es donde se rompe si cada
    /// uno usa parámetros distintos de Argon2.
    #[tokio::test]
    async fn el_admin_sembrado_entra_con_su_clave() {
        let pool = inventario_datos::abrir("sqlite::memory:").await.unwrap();
        sembrar(&pool, Some("clave de instalación")).await.unwrap();
        let (_, hash) = inventario_datos::usuario::por_codigo(&pool, "admin")
            .await
            .unwrap()
            .expect("el admin existe");
        assert!(claves::verificar("clave de instalación", &hash));
        assert!(!claves::verificar("otra", &hash));
    }

    #[tokio::test]
    async fn se_asegura_un_concepto_de_venta_sin_el_cual_no_se_factura() {
        let pool = inventario_datos::abrir("sqlite::memory:").await.unwrap();
        sembrar(&pool, Some("clave de instalación")).await.unwrap();
        for clase in ["venta", "devolucion_venta", "compra", "inicial"] {
            let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM concepto WHERE clase = ?")
                .bind(clase)
                .fetch_one(&pool)
                .await
                .unwrap();
            assert!(n >= 1, "falta un concepto de clase {clase}");
        }
    }

    /// La instalación tiene que traer los tres conceptos que el cumplimiento
    /// exige: sin ellos, el cliente que quiere regalar una caja o dar de baja
    /// mercancía vencida no tiene por dónde hacerlo bien.
    #[tokio::test]
    async fn se_siembran_los_conceptos_de_cumplimiento_tributario() {
        let pool = inventario_datos::abrir("sqlite::memory:").await.unwrap();
        sembrar(&pool, Some("clave de instalación")).await.unwrap();
        for efecto in ["retiro_gravado", "faltante_art64", "baja_con_acta"] {
            let (n,): (i64,) =
                sqlx::query_as("SELECT COUNT(*) FROM concepto WHERE efecto_tributario = ?")
                    .bind(efecto)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert!(n >= 1, "falta un concepto de efecto {efecto}");
        }
    }
}
