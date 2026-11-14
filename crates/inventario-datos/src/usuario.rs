//! Usuarios y roles.
//!
//! `USUARIOS.DBF` guardaba `CLAVE_USU C(10)`: la contraseña **en claro**, en un
//! archivo que cualquiera con acceso a la carpeta podía abrir. Aquí solo se
//! guarda el hash, y la verificación vive en la capa de API junto al algoritmo
//! que lo produce.

use crate::{Error, Pool, Resultado};
use sqlx::Row;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Usuario {
    pub id: i64,
    pub codigo: String,
    pub nombre: String,
    pub rol: String,
    pub activo: bool,
}

/// Lo que puede hacer cada rol. Es una tabla explícita y no una escala de
/// niveles: «bodega» puede mover inventario y no puede facturar, lo que en una
/// escala numérica no se puede expresar sin darle también lo del cajero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permiso {
    Facturar,
    Recaudar,
    MoverInventario,
    Anular,
    Configurar,
    Consultar,
}

pub const ROLES: [&str; 4] = ["admin", "cajero", "bodega", "consulta"];

/// `None` cuando el rol no se reconoce. Ante un rol desconocido no se cae al
/// más permisivo **ni** al más restrictivo en silencio: se responde «no sé» y
/// quien llama decide (directiva 20).
pub fn puede(rol: &str, p: Permiso) -> Option<bool> {
    let permisos: &[Permiso] = match rol {
        "admin" => &[
            Permiso::Facturar,
            Permiso::Recaudar,
            Permiso::MoverInventario,
            Permiso::Anular,
            Permiso::Configurar,
            Permiso::Consultar,
        ],
        "cajero" => &[Permiso::Facturar, Permiso::Recaudar, Permiso::Consultar],
        "bodega" => &[Permiso::MoverInventario, Permiso::Consultar],
        "consulta" => &[Permiso::Consultar],
        _ => return None,
    };
    Some(permisos.contains(&p))
}

pub async fn crear(pool: &Pool, codigo: &str, nombre: &str, hash: &str, rol: &str) -> Resultado<i64> {
    if !ROLES.contains(&rol) {
        return Err(Error::Regla(format!(
            "rol {rol:?} desconocido: uno de {}",
            ROLES.join(", ")
        )));
    }
    if codigo.trim().is_empty() || nombre.trim().is_empty() {
        return Err(Error::Regla("el usuario necesita código y nombre".into()));
    }
    if hash.is_empty() {
        return Err(Error::Regla("no se puede crear un usuario sin clave".into()));
    }
    Ok(sqlx::query(
        "INSERT INTO usuario (codigo, nombre, hash_clave, rol) VALUES (?, ?, ?, ?) RETURNING id",
    )
    .bind(codigo.trim())
    .bind(nombre.trim())
    .bind(hash)
    .bind(rol)
    .fetch_one(pool)
    .await?
    .get("id"))
}

/// Devuelve `(usuario, hash)` para que la API compare la clave. El hash no sale
/// de aquí en ninguna otra función ni entra en `Usuario`, que es lo que se
/// serializa hacia el navegador.
pub async fn por_codigo(pool: &Pool, codigo: &str) -> Resultado<Option<(Usuario, String)>> {
    let f = sqlx::query(
        "SELECT id, codigo, nombre, rol, activo, hash_clave FROM usuario
          WHERE codigo = ? AND activo = 1",
    )
    .bind(codigo.trim())
    .fetch_optional(pool)
    .await?;
    Ok(f.map(|f| {
        (
            Usuario {
                id: f.get("id"),
                codigo: f.get("codigo"),
                nombre: f.get("nombre"),
                rol: f.get("rol"),
                activo: f.get::<i64, _>("activo") == 1,
            },
            f.get("hash_clave"),
        )
    }))
}

pub async fn listar(pool: &Pool) -> Resultado<Vec<Usuario>> {
    let filas = sqlx::query("SELECT id, codigo, nombre, rol, activo FROM usuario ORDER BY nombre")
        .fetch_all(pool)
        .await?;
    Ok(filas
        .into_iter()
        .map(|f| Usuario {
            id: f.get("id"),
            codigo: f.get("codigo"),
            nombre: f.get("nombre"),
            rol: f.get("rol"),
            activo: f.get::<i64, _>("activo") == 1,
        })
        .collect())
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::prueba_apoyo;

    #[test]
    fn los_roles_no_son_una_escala() {
        // Bodega mueve inventario y NO factura; cajero al revés. Una escala
        // numérica no podría expresarlo.
        assert_eq!(puede("bodega", Permiso::MoverInventario), Some(true));
        assert_eq!(puede("bodega", Permiso::Facturar), Some(false));
        assert_eq!(puede("cajero", Permiso::Facturar), Some(true));
        assert_eq!(puede("cajero", Permiso::MoverInventario), Some(false));
        assert_eq!(puede("cajero", Permiso::Anular), Some(false));
        assert_eq!(puede("admin", Permiso::Anular), Some(true));
        assert_eq!(puede("consulta", Permiso::Consultar), Some(true));
        assert_eq!(puede("consulta", Permiso::Recaudar), Some(false));
    }

    /// Un rol desconocido responde «no sé», no «sí» ni «no».
    #[test]
    fn un_rol_desconocido_no_se_resuelve_a_ninguna_respuesta() {
        assert_eq!(puede("gerente", Permiso::Facturar), None);
        assert_eq!(puede("", Permiso::Consultar), None);
        assert_eq!(puede("ADMIN", Permiso::Configurar), None, "no se normaliza a la brava");
    }

    #[test]
    fn todos_los_roles_declarados_tienen_permisos_definidos() {
        for r in ROLES {
            assert!(puede(r, Permiso::Consultar).is_some(), "el rol {r} no tiene tabla");
        }
    }

    #[tokio::test]
    async fn el_hash_no_viaja_en_el_usuario_que_se_serializa() {
        let pool = prueba_apoyo::base().await;
        crear(&pool, "caj", "Cajera", "$argon2id$...", "cajero").await.unwrap();
        let (u, hash) = por_codigo(&pool, "caj").await.unwrap().unwrap();
        assert_eq!(hash, "$argon2id$...");
        let json = serde_json::to_string(&u).unwrap();
        assert!(!json.contains("argon2"), "el hash se escapó al JSON: {json}");
        assert!(!json.contains("hash"));
    }

    #[tokio::test]
    async fn no_se_crea_un_usuario_sin_clave_ni_con_un_rol_inventado() {
        let pool = prueba_apoyo::base().await;
        assert!(crear(&pool, "x", "X", "", "cajero").await.is_err());
        assert!(crear(&pool, "x", "X", "h", "gerente").await.is_err());
        assert!(crear(&pool, "", "X", "h", "cajero").await.is_err());
        // Código repetido tampoco.
        crear(&pool, "x", "X", "h", "cajero").await.unwrap();
        assert!(crear(&pool, "x", "Y", "h", "cajero").await.is_err());
    }

    #[tokio::test]
    async fn un_usuario_inactivo_no_entra() {
        let pool = prueba_apoyo::base().await;
        let id = crear(&pool, "x", "X", "h", "cajero").await.unwrap();
        sqlx::query("UPDATE usuario SET activo = 0 WHERE id = ?")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(por_codigo(&pool, "x").await.unwrap().is_none());
    }
}
