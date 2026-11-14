//! NIT y dígito de verificación.
//!
//! El corpus guardaba el NIT como texto libre en `C(20)`, unas veces con
//! guion (`890934562-5`) y otras sin él (`43627185`), y de ahí salían 47
//! duplicados y un `IDCLIENTE` que era el NIT concatenado con un consecutivo.
//! Aquí el NIT se normaliza a solo dígitos y el DV se **calcula**, nunca se
//! digita: un dígito de verificación tecleado a mano no verifica nada.

/// Pesos del algoritmo del DV que publica la DIAN, de la cifra menos
/// significativa hacia la más significativa.
const PESOS: [u32; 15] = [3, 7, 13, 17, 19, 23, 29, 37, 41, 43, 47, 53, 59, 67, 71];

/// Quita todo lo que no sea dígito. Devuelve `None` si no queda ninguno o si
/// hay más de 15: por encima de eso no es un NIT, y truncarlo lo convertiría en
/// el NIT de otro.
pub fn normalizar(bruto: &str) -> Option<String> {
    // Si trae guion, lo de después es el DV que alguien tecleó: se descarta y
    // se recalcula. Conservarlo permitiría guardar un NIT con DV equivocado.
    let cuerpo = bruto.split('-').next().unwrap_or(bruto);
    let solo: String = cuerpo.chars().filter(|c| c.is_ascii_digit()).collect();
    if solo.is_empty() || solo.len() > 15 {
        return None;
    }
    Some(solo)
}

/// Dígito de verificación. `None` si el número no es un NIT normalizable.
pub fn digito_verificacion(nit: &str) -> Option<u32> {
    let n = normalizar(nit)?;
    let suma: u32 = n
        .chars()
        .rev()
        .zip(PESOS)
        .map(|(c, p)| c.to_digit(10).unwrap_or(0) * p)
        .sum();
    let resto = suma % 11;
    Some(if resto > 1 { 11 - resto } else { resto })
}

/// `890934562-5`, listo para imprimir.
pub fn con_dv(nit: &str) -> Option<String> {
    let n = normalizar(nit)?;
    let dv = digito_verificacion(&n)?;
    Some(format!("{n}-{dv}"))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// NIT reales del corpus, cuyo DV venía escrito en el propio dato: si el
    /// algoritmo estuviera mal, estos no cuadrarían.
    #[test]
    fn reproduce_el_dv_que_traia_el_corpus() {
        // La empresa dueña del sistema: 811033579-6.
        assert_eq!(digito_verificacion("811033579"), Some(6));
        // Clientes: 890934562-5, 860040576-1, 811008668-8.
        assert_eq!(digito_verificacion("890934562"), Some(5));
        assert_eq!(digito_verificacion("860040576"), Some(1));
        assert_eq!(digito_verificacion("811008668"), Some(8));
    }

    #[test]
    fn el_dv_tecleado_se_descarta_y_se_recalcula() {
        // Aunque venga con un DV equivocado, sale el correcto.
        assert_eq!(con_dv("811033579-9").unwrap(), "811033579-6");
        assert_eq!(con_dv("811.033.579").unwrap(), "811033579-6");
        assert_eq!(con_dv(" 811033579 ").unwrap(), "811033579-6");
    }

    #[test]
    fn lo_que_no_es_un_nit_no_se_convierte_en_uno() {
        assert_eq!(normalizar(""), None);
        assert_eq!(normalizar("   "), None);
        assert_eq!(normalizar("ABC"), None);
        assert_eq!(normalizar("-5"), None, "solo el DV no es un NIT");
        assert_eq!(normalizar(&"9".repeat(16)), None, "no se trunca al NIT de otro");
        assert!(normalizar(&"9".repeat(15)).is_some());
    }

    #[test]
    fn una_cedula_corta_tambien_tiene_dv() {
        // Cédula del corpus: 43627185.
        assert!(digito_verificacion("43627185").is_some());
        assert!((0..=9).contains(&digito_verificacion("43627185").unwrap()));
    }

    /// El DV siempre está en 0..=9 — nunca 10, que es el error clásico de
    /// escribir `11 - resto` sin el caso `resto <= 1`.
    #[test]
    fn el_dv_nunca_se_sale_de_un_digito() {
        for n in 1..20_000u64 {
            let dv = digito_verificacion(&n.to_string()).unwrap();
            assert!(dv <= 9, "el NIT {n} produjo DV {dv}");
        }
    }
}
