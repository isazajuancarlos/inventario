//! Identificación de artículos: GTIN (GS1) y el estándar que la DIAN exige
//! declarar.
//!
//! El Anexo Técnico 1.9 de la Resolución DIAN 000165 de 2023 (cap. 14.2) adopta
//! GTIN, y en el ítem de la factura obliga a declarar **qué estándar** es el
//! código del producto: `@schemeID = 001` para UNSPSC, `010` para GTIN (con
//! `@schemeAgencyID = 9`, que es GS1).
//!
//! ## Por qué esto importa aquí y no es adorno
//!
//! El sistema heredado guardaba `41251501` en 63 artículos como «código de
//! barras». Ese número **no es un GTIN**: tiene 8 dígitos pero no cumple el
//! dígito de control, y además lo comparten artículos distintos. Era una
//! clasificación interna guardada en el campo equivocado. Declararlo como GTIN
//! en una factura electrónica es informarle a la DIAN un identificador
//! internacional que no existe.
//!
//! Aquí un código solo se acepta como GTIN si **cumple su dígito de control**;
//! si no, se guarda como código interno y se dice.

/// Estándar bajo el que se informa el código de un producto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Estandar {
    /// `@schemeID = 001`. Es una **clasificación**, no un identificador: dos
    /// artículos distintos pueden compartir código UNSPSC legítimamente.
    Unspsc,
    /// `@schemeID = 010`, `@schemeAgencyID = 9` (GS1).
    Gtin,
    /// `@schemeID = 999`. El código propio del vendedor. Es lo que corresponde
    /// cuando no hay GTIN — y es honesto, no un defecto.
    Interno,
}

impl Estandar {
    /// El `@schemeID` del anexo técnico.
    pub fn scheme_id(self) -> &'static str {
        match self {
            Estandar::Unspsc => "001",
            Estandar::Gtin => "010",
            Estandar::Interno => "999",
        }
    }
    /// El `@schemeName`, que el anexo exige que concuerde con el `@schemeID`.
    pub fn scheme_name(self) -> &'static str {
        match self {
            Estandar::Unspsc => "UNSPSC",
            Estandar::Gtin => "GTIN",
            Estandar::Interno => "Estándar de adopción del contribuyente",
        }
    }
    /// `@schemeAgencyID`. El anexo rechaza la factura si `schemeID = 010` y la
    /// agencia no es `9`.
    pub fn scheme_agency_id(self) -> Option<&'static str> {
        matches!(self, Estandar::Gtin).then_some("9")
    }

    pub fn desde_texto(s: &str) -> Option<Estandar> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "unspsc" | "001" => Estandar::Unspsc,
            "gtin" | "010" => Estandar::Gtin,
            "interno" | "999" => Estandar::Interno,
            _ => None?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErrorGtin {
    #[error("un GTIN solo tiene dígitos; «{0}» trae otra cosa")]
    NoEsNumerico(String),
    #[error("un GTIN tiene 8, 12, 13 o 14 dígitos; «{codigo}» tiene {largo}")]
    LargoInvalido { codigo: String, largo: usize },
    #[error(
        "el dígito de control de «{codigo}» debería ser {esperado} y es {tiene}: \
         no es un GTIN válido. Guárdelo como código interno (estándar 999)."
    )]
    DigitoIncorrecto {
        codigo: String,
        esperado: u32,
        tiene: u32,
    },
}

/// Largos válidos de GTIN: GTIN-8, GTIN-12 (UPC-A), GTIN-13 (EAN-13) y GTIN-14.
const LARGOS: [usize; 4] = [8, 12, 13, 14];

/// Dígito de control de GS1: módulo 10 con pesos alternos 3 y 1 **desde la
/// derecha**, sin contar el propio dígito de control.
///
/// Que los pesos se cuenten desde la derecha y no desde la izquierda es lo que
/// hace que la misma función sirva para GTIN-8, 12, 13 y 14 sin cambiar nada.
/// Contarlos desde la izquierda funciona para los largos pares y falla para los
/// impares — un error que pasa las pruebas si solo se prueba con EAN-13.
pub fn digito_control(cuerpo: &str) -> Result<u32, ErrorGtin> {
    if cuerpo.is_empty() || !cuerpo.chars().all(|c| c.is_ascii_digit()) {
        return Err(ErrorGtin::NoEsNumerico(cuerpo.to_string()));
    }
    let suma: u32 = cuerpo
        .chars()
        .rev()
        .enumerate()
        .map(|(i, c)| {
            let d = c.to_digit(10).expect("ya se comprobó que son dígitos");
            d * if i % 2 == 0 { 3 } else { 1 }
        })
        .sum();
    Ok((10 - (suma % 10)) % 10)
}

/// Valida un GTIN completo (con su dígito de control incluido).
pub fn validar(codigo: &str) -> Result<(), ErrorGtin> {
    let c = codigo.trim();
    if !c.chars().all(|x| x.is_ascii_digit()) || c.is_empty() {
        return Err(ErrorGtin::NoEsNumerico(c.to_string()));
    }
    if !LARGOS.contains(&c.len()) {
        return Err(ErrorGtin::LargoInvalido {
            codigo: c.to_string(),
            largo: c.len(),
        });
    }
    let (cuerpo, ultimo) = c.split_at(c.len() - 1);
    let esperado = digito_control(cuerpo)?;
    let tiene = ultimo.chars().next().and_then(|x| x.to_digit(10)).unwrap_or(99);
    if esperado != tiene {
        return Err(ErrorGtin::DigitoIncorrecto {
            codigo: c.to_string(),
            esperado,
            tiene,
        });
    }
    Ok(())
}

pub fn es_valido(codigo: &str) -> bool {
    validar(codigo).is_ok()
}

/// Decide bajo qué estándar informar un código.
///
/// **No adivina hacia arriba**: si el código no es un GTIN válido, devuelve
/// `Interno`, que es la verdad. Informar como GTIN algo que no lo es le mete a
/// la DIAN un identificador internacional inexistente.
pub fn clasificar(codigo: &str) -> Estandar {
    if es_valido(codigo) {
        Estandar::Gtin
    } else {
        Estandar::Interno
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// GTIN reales publicados. Si el algoritmo estuviera mal, ninguno cuadraría.
    #[test]
    fn valida_gtin_reales_de_los_cuatro_largos() {
        // GTIN-13 (EAN-13) del ejemplo canónico de GS1.
        assert!(es_valido("4006381333931"));
        // GTIN-12 (UPC-A).
        assert!(es_valido("012345678905"));
        // GTIN-8.
        assert!(es_valido("96385074"));
        // GTIN-14: el mismo GTIN-13 con indicador de agrupación delante.
        assert!(es_valido("14006381333938"));
    }

    /// El caso exacto del corpus heredado: `41251501` parece un GTIN-8 y no lo
    /// es. Es lo que separa un identificador de un número cualquiera.
    #[test]
    fn el_codigo_de_barras_del_corpus_heredado_no_es_un_gtin() {
        assert!(!es_valido("41251501"), "41251501 no cumple su dígito de control");
        match validar("41251501") {
            Err(ErrorGtin::DigitoIncorrecto { esperado, tiene, .. }) => {
                assert_ne!(esperado, tiene);
            }
            otro => panic!("se esperaba dígito incorrecto, salió {otro:?}"),
        }
        // Y por eso se clasifica como interno, no como GTIN.
        assert_eq!(clasificar("41251501"), Estandar::Interno);
        assert_eq!(clasificar("4006381333931"), Estandar::Gtin);
    }

    /// Los pesos van desde la DERECHA. Contarlos desde la izquierda funciona
    /// con largos pares y falla con los impares: esta prueba usa GTIN-13
    /// (impar sin el dígito) para que ese error no pase.
    #[test]
    fn los_pesos_se_cuentan_desde_la_derecha() {
        // 400638133393 → dígito 1.
        assert_eq!(digito_control("400638133393").unwrap(), 1);
        // 9638507 (GTIN-8, cuerpo de 7 = impar) → dígito 4.
        assert_eq!(digito_control("9638507").unwrap(), 4);
        // 01234567890 (GTIN-12, cuerpo de 11 = impar) → dígito 5.
        assert_eq!(digito_control("01234567890").unwrap(), 5);
    }

    #[test]
    fn el_digito_de_control_nunca_se_sale_de_un_digito() {
        // El error clásico: escribir `10 - resto` sin el segundo módulo, que
        // produce 10 cuando el resto es 0.
        for n in 0..5000u64 {
            let cuerpo = format!("{n:012}");
            let d = digito_control(&cuerpo).unwrap();
            assert!(d <= 9, "el cuerpo {cuerpo} produjo dígito {d}");
        }
        // El caso que lo dispara: suma múltiplo de 10.
        assert_eq!(digito_control("000000000000").unwrap(), 0);
    }

    #[test]
    fn un_gtin_al_que_le_cambian_un_digito_deja_de_validar() {
        // La razón de ser del dígito de control: detectar el error de tecleo.
        let bueno = "4006381333931";
        assert!(es_valido(bueno));
        let mut fallos = 0;
        for i in 0..bueno.len() - 1 {
            for d in b'0'..=b'9' {
                let mut roto: Vec<u8> = bueno.bytes().collect();
                if roto[i] == d {
                    continue;
                }
                roto[i] = d;
                let roto = String::from_utf8(roto).unwrap();
                if !es_valido(&roto) {
                    fallos += 1;
                }
            }
        }
        // GS1 detecta TODOS los errores de un solo dígito.
        assert_eq!(fallos, (bueno.len() - 1) * 9, "no detectó todos los cambios de un dígito");
    }

    #[test]
    fn lo_que_no_es_un_gtin_se_rechaza_con_el_motivo() {
        assert!(matches!(validar(""), Err(ErrorGtin::NoEsNumerico(_))));
        assert!(matches!(validar("ABC12345"), Err(ErrorGtin::NoEsNumerico(_))));
        // 9 dígitos no es un largo de GTIN.
        assert!(matches!(
            validar("123456789"),
            Err(ErrorGtin::LargoInvalido { largo: 9, .. })
        ));
        assert!(matches!(
            validar("4006381333930"),
            Err(ErrorGtin::DigitoIncorrecto { .. })
        ));
    }

    #[test]
    fn el_estandar_lleva_los_atributos_que_exige_el_anexo() {
        assert_eq!(Estandar::Gtin.scheme_id(), "010");
        assert_eq!(Estandar::Gtin.scheme_name(), "GTIN");
        // El anexo rechaza la factura si schemeID = 010 y la agencia no es 9.
        assert_eq!(Estandar::Gtin.scheme_agency_id(), Some("9"));
        assert_eq!(Estandar::Unspsc.scheme_id(), "001");
        assert_eq!(Estandar::Unspsc.scheme_name(), "UNSPSC");
        assert_eq!(Estandar::Unspsc.scheme_agency_id(), None);
        assert_eq!(Estandar::Interno.scheme_id(), "999");
        assert_eq!(Estandar::desde_texto("010"), Some(Estandar::Gtin));
        assert_eq!(Estandar::desde_texto("GTIN"), Some(Estandar::Gtin));
        assert_eq!(Estandar::desde_texto("magia"), None);
    }
}
