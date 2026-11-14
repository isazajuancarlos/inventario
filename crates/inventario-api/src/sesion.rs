//! Sesión: una cookie firmada con HMAC-SHA256.
//!
//! La interfaz se sirve desde el **mismo origen** que la API (viaja dentro del
//! binario), así que la sesión puede ser una cookie `HttpOnly` +
//! `SameSite=Strict`: no hay token en `localStorage` que un XSS pueda
//! exfiltrar, ni CORS que resolver.
//!
//! **Antes esta línea decía «ni CSRF», y se quedó corta al llegar el panel.**
//! Se escribió cuando toda escritura iba por `/api` con cuerpo JSON, que un
//! formulario de otro sitio no puede emitir: la protección era doble sin que
//! nadie la hubiera pedido. El panel escribe con formularios normales, así que
//! queda solo `SameSite=Strict` — que es un control real y suficiente contra
//! otro SITIO, pero es por sitio y no por ORIGEN: `chasqui.xiliux.com` y
//! `inventario.xiliux.com` son el mismo sitio y la cookie viaja entre ellos.
//! Ese resto lo cierra `rutas::origen_ajeno`, en el middleware.
//!
//! La cookie lleva el contenido en claro y su firma. Es deliberado: el
//! contenido no es secreto —id, rol y vencimiento—, lo que hace falta es que
//! **no se pueda alterar**. Cifrarla escondería que el rol viaja al cliente sin
//! impedir nada más.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub const COOKIE: &str = "inventario_sesion";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sesion {
    pub usuario_id: i64,
    pub rol: String,
    /// Segundos desde la época en que deja de valer.
    pub vence: i64,
}

fn firmar(clave: &[u8], carga: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(clave).expect("HMAC acepta cualquier largo");
    mac.update(carga.as_bytes());
    B64.encode(mac.finalize().into_bytes())
}

/// `base64(id|rol|vence).firma`
pub fn emitir(clave: &[u8], s: &Sesion) -> String {
    let carga = B64.encode(format!("{}|{}|{}", s.usuario_id, s.rol, s.vence));
    let firma = firmar(clave, &carga);
    format!("{carga}.{firma}")
}

/// Lee y valida. `None` si la firma no cuadra, si el formato no es el esperado
/// o si ya venció. Un `None` no distingue entre esos casos a propósito.
pub fn leer(clave: &[u8], cookie: &str, ahora: i64) -> Option<Sesion> {
    let (carga, firma) = cookie.split_once('.')?;

    // Comparación en tiempo constante: `==` sobre cadenas sale en el primer
    // byte distinto, y ese tiempo se puede medir para adivinar la firma byte a
    // byte. `Mac::verify_slice` ya lo hace, así que se usa esa.
    let mut mac = Hmac::<Sha256>::new_from_slice(clave).ok()?;
    mac.update(carga.as_bytes());
    mac.verify_slice(&B64.decode(firma).ok()?).ok()?;

    let texto = String::from_utf8(B64.decode(carga).ok()?).ok()?;
    let mut partes = texto.split('|');
    let usuario_id: i64 = partes.next()?.parse().ok()?;
    let rol = partes.next()?.to_string();
    let vence: i64 = partes.next()?.parse().ok()?;
    if partes.next().is_some() {
        return None;
    }
    if ahora >= vence {
        return None;
    }
    Some(Sesion {
        usuario_id,
        rol,
        vence,
    })
}

/// La cabecera `Set-Cookie`. `Secure` se omite a propósito cuando la
/// instalación es local por HTTP: un binario que corre en el mostrador del
/// cliente no tiene certificado, y con `Secure` el navegador tiraría la cookie
/// y nadie podría entrar. Quien despliegue detrás de HTTPS lo enciende.
pub fn cabecera(valor: &str, segundos: i64, seguro: bool) -> String {
    format!(
        "{COOKIE}={valor}; Path=/; HttpOnly; SameSite=Strict; Max-Age={segundos}{}",
        if seguro { "; Secure" } else { "" }
    )
}

pub fn cabecera_de_salida(seguro: bool) -> String {
    format!(
        "{COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{}",
        if seguro { "; Secure" } else { "" }
    )
}

/// Extrae la cookie de una cabecera `Cookie:` que puede traer varias.
pub fn de_cabecera(cabecera: &str) -> Option<&str> {
    cabecera.split(';').find_map(|par| {
        let (k, v) = par.split_once('=')?;
        (k.trim() == COOKIE).then_some(v.trim())
    })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    const CLAVE: &[u8] = b"una clave de firma de prueba, larga";

    fn sesion() -> Sesion {
        Sesion {
            usuario_id: 7,
            rol: "cajero".into(),
            vence: 2_000_000_000,
        }
    }

    #[test]
    fn ida_y_vuelta() {
        let c = emitir(CLAVE, &sesion());
        assert_eq!(leer(CLAVE, &c, 1_000_000_000), Some(sesion()));
    }

    /// Lo que impide que un cajero se ascienda a admin editando su cookie.
    #[test]
    fn una_cookie_manipulada_no_pasa() {
        let c = emitir(CLAVE, &sesion());
        let (carga, firma) = c.split_once('.').unwrap();

        // Cambiar el rol a admin en la carga, conservando la firma vieja.
        let falsa = B64.encode("7|admin|2000000000");
        assert!(leer(CLAVE, &format!("{falsa}.{firma}"), 0).is_none());

        // Y con la firma tocada tampoco.
        assert!(leer(CLAVE, &format!("{carga}.{}", B64.encode([0u8; 32])), 0).is_none());
        // Ni con otra clave.
        assert!(leer(b"otra clave distinta", &c, 0).is_none());
    }

    #[test]
    fn una_sesion_vencida_no_vale() {
        let c = emitir(CLAVE, &sesion());
        assert!(leer(CLAVE, &c, 2_000_000_000).is_none(), "justo al vencer");
        assert!(leer(CLAVE, &c, 2_000_000_001).is_none());
        assert!(leer(CLAVE, &c, 1_999_999_999).is_some());
    }

    #[test]
    fn la_basura_no_se_interpreta() {
        for basura in ["", ".", "a.b", "sin punto", "....", &"x".repeat(500)] {
            assert!(leer(CLAVE, basura, 0).is_none(), "pasó: {basura:?}");
        }
        // Carga con campos de más: se rechaza en vez de quedarse con los tres
        // primeros, que es como se cuela un rol extra.
        let carga = B64.encode("7|cajero|2000000000|admin");
        let firma = firmar(CLAVE, &carga);
        assert!(leer(CLAVE, &format!("{carga}.{firma}"), 0).is_none());
    }

    #[test]
    fn la_cookie_es_httponly_y_samesite_strict() {
        let c = cabecera("valor", 3600, false);
        assert!(c.contains("HttpOnly"), "{c}");
        assert!(c.contains("SameSite=Strict"), "{c}");
        assert!(!c.contains("Secure"), "en instalación local no va Secure: {c}");
        assert!(cabecera("v", 1, true).contains("Secure"));
        assert!(cabecera_de_salida(false).contains("Max-Age=0"));
    }

    #[test]
    fn se_encuentra_entre_varias_cookies() {
        let h = format!("otra=1; {COOKIE}=elvalor; tercera=3");
        assert_eq!(de_cabecera(&h), Some("elvalor"));
        assert_eq!(de_cabecera("otra=1"), None);
        assert_eq!(de_cabecera(""), None);
        // Y no se confunde con una que solo empieza igual.
        assert_eq!(de_cabecera(&format!("{COOKIE}_falsa=x")), None);
    }
}
