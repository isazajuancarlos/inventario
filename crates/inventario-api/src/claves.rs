//! Contraseñas con Argon2id.
//!
//! `USUARIOS.DBF` guardaba `CLAVE_USU C(10)`: la contraseña en claro, en un
//! archivo que cualquiera con acceso a la carpeta abría con Excel. Diez
//! caracteres, además, era el techo.
//!
//! Los parámetros viven aquí y **solo aquí**: el subcomando `hashear` usa esta
//! misma función. Un hash producido con otra configuración deja al usuario
//! fuera, y el fallo aparece lejos de su causa.

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use password_hash::{SaltString, rand_core::OsRng};

pub fn hashear(clave: &str) -> Option<String> {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(clave.as_bytes(), &sal)
        .ok()
        .map(|h| h.to_string())
}

/// Verifica. Devuelve `false` tanto si la clave está mal como si el hash
/// guardado es ilegible: por fuera son el mismo «no entra», y distinguirlos le
/// diría a quien prueba claves que ese usuario existe.
pub fn verificar(clave: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(h) => Argon2::default().verify_password(clave.as_bytes(), &h).is_ok(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn lo_que_hashea_es_lo_que_verifica() {
        let h = hashear("clave larga y con espacios").unwrap();
        assert!(verificar("clave larga y con espacios", &h));
        assert!(!verificar("clave larga y con espacio", &h));
        assert!(!verificar("", &h));
    }

    /// La misma clave da hashes distintos: la sal es aleatoria. Sin esto, dos
    /// usuarios con la misma contraseña se delatarían mutuamente.
    #[test]
    fn dos_hashes_de_la_misma_clave_son_distintos() {
        let a = hashear("igual").unwrap();
        let b = hashear("igual").unwrap();
        assert_ne!(a, b);
        assert!(verificar("igual", &a) && verificar("igual", &b));
    }

    #[test]
    fn un_hash_ilegible_no_deja_entrar_a_nadie() {
        // Lo que habría en la base si alguien migrara CLAVE_USU tal cual.
        assert!(!verificar("1234", "1234"));
        assert!(!verificar("1234", ""));
        assert!(!verificar("", ""));
        assert!(!verificar("x", "$argon2id$v=19$m=1,t=1,p=1$roto"));
    }

    #[test]
    fn no_hay_techo_de_diez_caracteres_como_en_el_sistema_viejo() {
        let larga = "a".repeat(200);
        let h = hashear(&larga).unwrap();
        assert!(verificar(&larga, &h));
        assert!(!verificar(&"a".repeat(199), &h));
    }
}
