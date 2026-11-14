//! Configuración desde el entorno.
//!
//! Regla del módulo: **ante la ausencia de un dato, fallar ruidosamente**
//! (directiva 20). Lo único que trae valor por defecto es lo que no puede
//! causar daño silencioso: la ruta de la base y el puerto. La clave de firma
//! de sesión NO: generar una al vuelo haría que cada reinicio cerrara todas las
//! sesiones —y que dos procesos no se entendieran— sin que nadie supiera por qué.

use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub url_bd: String,
    pub direccion: String,
    pub clave_firma: Vec<u8>,
    /// Marca `Secure` en la cookie. Se enciende detrás de HTTPS.
    pub cookie_segura: bool,
    pub horas_sesion: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum ErrorConfig {
    #[error(
        "falta INVENTARIO_CLAVE_FIRMA (mínimo 32 caracteres). \
         Genere una con: inventario-api clave-firma"
    )]
    SinClave,
    #[error("INVENTARIO_CLAVE_FIRMA tiene {0} caracteres; hacen falta 32 o más")]
    ClaveCorta(usize),
    #[error("INVENTARIO_HORAS_SESION debe ser un entero positivo (llegó {0:?})")]
    HorasInvalidas(String),
}

impl Config {
    pub fn desde_entorno() -> Result<Config, ErrorConfig> {
        let clave = env::var("INVENTARIO_CLAVE_FIRMA").map_err(|_| ErrorConfig::SinClave)?;
        if clave.len() < 32 {
            return Err(ErrorConfig::ClaveCorta(clave.len()));
        }
        let horas = match env::var("INVENTARIO_HORAS_SESION") {
            Err(_) => 12,
            Ok(v) => v
                .parse::<i64>()
                .ok()
                .filter(|h| *h > 0)
                .ok_or(ErrorConfig::HorasInvalidas(v))?,
        };
        Ok(Config {
            url_bd: env::var("INVENTARIO_BD").unwrap_or_else(|_| "sqlite://inventario.db".into()),
            direccion: env::var("INVENTARIO_DIRECCION").unwrap_or_else(|_| "127.0.0.1:8080".into()),
            clave_firma: clave.into_bytes(),
            cookie_segura: env::var("INVENTARIO_COOKIE_SEGURA").as_deref() == Ok("1"),
            horas_sesion: horas,
        })
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Las variables de entorno son globales al proceso y las pruebas corren en
    /// hilos: se serializan con un mutex para que no se pisen. Sin esto la
    /// prueba falla una vez de cada tantas y se le echa la culpa a otra cosa.
    static CANDADO: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn con_entorno<T>(vars: &[(&str, Option<&str>)], f: impl FnOnce() -> T) -> T {
        let _g = CANDADO.lock().unwrap_or_else(|e| e.into_inner());
        let previas: Vec<(String, Option<String>)> = vars
            .iter()
            .map(|(k, _)| (k.to_string(), env::var(k).ok()))
            .collect();
        for (k, v) in vars {
            match v {
                Some(v) => unsafe { env::set_var(k, v) },
                None => unsafe { env::remove_var(k) },
            }
        }
        let r = f();
        for (k, v) in previas {
            match v {
                Some(v) => unsafe { env::set_var(&k, v) },
                None => unsafe { env::remove_var(&k) },
            }
        }
        r
    }

    #[test]
    fn sin_clave_de_firma_no_arranca() {
        con_entorno(&[("INVENTARIO_CLAVE_FIRMA", None)], || {
            assert!(matches!(Config::desde_entorno(), Err(ErrorConfig::SinClave)));
        });
    }

    /// Una clave corta se rechaza con su longitud, en vez de rellenarse hasta
    /// 32 — que dejaría un sistema que *parece* seguro.
    #[test]
    fn una_clave_corta_se_rechaza_diciendo_cuanto_falta() {
        con_entorno(&[("INVENTARIO_CLAVE_FIRMA", Some("corta"))], || {
            match Config::desde_entorno() {
                Err(ErrorConfig::ClaveCorta(n)) => assert_eq!(n, 5),
                otro => panic!("salió {otro:?}"),
            }
        });
    }

    #[test]
    fn los_defectos_son_solo_los_inocuos() {
        con_entorno(
            &[
                ("INVENTARIO_CLAVE_FIRMA", Some(&"k".repeat(32))),
                ("INVENTARIO_BD", None),
                ("INVENTARIO_DIRECCION", None),
                ("INVENTARIO_HORAS_SESION", None),
                ("INVENTARIO_COOKIE_SEGURA", None),
            ],
            || {
                let c = Config::desde_entorno().unwrap();
                assert_eq!(c.url_bd, "sqlite://inventario.db");
                assert_eq!(c.direccion, "127.0.0.1:8080");
                assert_eq!(c.horas_sesion, 12);
                assert!(!c.cookie_segura);
            },
        );
    }

    #[test]
    fn unas_horas_que_no_son_un_numero_positivo_paran_el_arranque() {
        for malo in ["cero", "0", "-3", ""] {
            con_entorno(
                &[
                    ("INVENTARIO_CLAVE_FIRMA", Some(&"k".repeat(32))),
                    ("INVENTARIO_HORAS_SESION", Some(malo)),
                ],
                || {
                    assert!(
                        matches!(Config::desde_entorno(), Err(ErrorConfig::HorasInvalidas(_))),
                        "se aceptó {malo:?}"
                    );
                },
            );
        }
    }
}
