//! CUFE — Código Único de Factura Electrónica.
//!
//! Especificación: **Anexo Técnico de la Factura Electrónica de Venta v1.9**,
//! numeral 11.2, adoptado por la Resolución DIAN **000165 del 1 de noviembre de
//! 2023**.
//!
//! ```text
//! CUFE = SHA-384( NumFac + FecFac + HorFac + ValFac
//!               + "01" + ValImp1(IVA)
//!               + "04" + ValImp2(INC)
//!               + "03" + ValImp3(ICA)
//!               + ValTot + NitFE + NumAdq + ClTec + TipoAmbiente )
//! ```
//!
//! donde `+` es concatenación **sin separador**.
//!
//! ## Lo que hay que saber para no equivocarse
//!
//! - **No use la especificación de 2016.** El Anexo Técnico 003 de la
//!   Resolución 000019 de 2016 usaba **SHA-1**, separaba los campos con `;` y
//!   el segundo impuesto era `02` en vez de `04`. Está derogado, y sigue siendo
//!   el primer resultado en muchas búsquedas. Implementar aquel produce un CUFE
//!   que la DIAN rechaza — y como el hash *se calcula igual de bien*, el fallo
//!   no se ve hasta que se transmite.
//! - Los valores van con **dos decimales TRUNCADOS**, no redondeados. Lo dice el
//!   anexo con esa palabra. Redondear cambia el hash en las facturas cuyo
//!   tercer decimal es ≥ 5, que no son pocas.
//! - Un impuesto que no aplica se escribe `0.00`; **no se omite**.
//! - El NIT va sin puntos, sin guiones y **sin dígito de verificación**.
//!
//! ## CUDE: el mismo cálculo con otra llave
//!
//! Las **notas crédito y débito** —y los documentos de transmisión— llevan
//! **CUDE**, no CUFE (anexo 1.9, num. 11.3 y 11.4). La composición es idéntica
//! salvo en el penúltimo campo: donde el CUFE pone la **clave técnica** del
//! rango, el CUDE pone el **PIN del software**.
//!
//! Es una diferencia de un solo campo y produce hashes completamente distintos,
//! así que confundirlos no se nota leyendo: se nota cuando la DIAN rechaza el
//! documento. Por eso aquí son dos funciones con nombres distintos sobre un
//! mismo motor, y cada una tiene su ejemplo oficial como prueba.
//!
//! Este módulo calcula el CUFE y el CUDE. **No firma ni transmite**: eso exige
//! certificado digital y habilitación ante la DIAN. Ver `docs/DIAN.md`.

use crate::dinero::Centavos;

/// Ambiente de la DIAN. El número entra en el hash, así que confundirlos
/// produce un CUFE que no valida — y son dos, no «un booleano de pruebas».
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ambiente {
    Produccion,
    Pruebas,
}

impl Ambiente {
    pub fn codigo(self) -> &'static str {
        match self {
            Ambiente::Produccion => "1",
            Ambiente::Pruebas => "2",
        }
    }
}

/// Los datos que entran al CUFE. Todos, y en el orden del anexo.
#[derive(Debug, Clone)]
pub struct Datos<'a> {
    /// Prefijo **concatenado** con el número. `FE` + `1` → `FE1`.
    pub numero_completo: &'a str,
    /// `AAAA-MM-DD`.
    pub fecha: &'a str,
    /// `HH:MM:SS-05:00` — con desplazamiento horario.
    pub hora: &'a str,
    /// Valor de la factura **sin impuestos**.
    pub valor_sin_impuestos: Centavos,
    pub iva: Centavos,
    /// Impuesto nacional al consumo.
    pub inc: Centavos,
    pub ica: Centavos,
    pub valor_total: Centavos,
    /// NIT del facturador, solo dígitos, sin DV.
    pub nit_facturador: &'a str,
    /// Identificación del adquirente, solo dígitos, sin DV.
    pub nit_adquirente: &'a str,
    /// Clave técnica del rango de facturación, la que entrega la DIAN.
    pub clave_tecnica: &'a str,
    pub ambiente: Ambiente,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErrorCufe {
    #[error("el campo «{campo}» del CUFE está vacío y la DIAN lo exige")]
    CampoVacio { campo: &'static str },
    #[error("la fecha «{0}» no tiene el formato AAAA-MM-DD que exige el anexo")]
    FechaInvalida(String),
    #[error(
        "la hora «{0}» no tiene el formato HH:MM:SS±HH:MM. El desplazamiento horario \
         NO es opcional: entra en el hash"
    )]
    HoraInvalida(String),
    #[error("«{campo}» = «{valor}» debe contener solo dígitos, sin puntos ni guiones ni DV")]
    NoEsSoloDigitos { campo: &'static str, valor: String },
    #[error("un valor del CUFE es negativo ({campo} = {valor}), y una factura no puede serlo")]
    ValorNegativo { campo: &'static str, valor: i64 },
}

/// Formatea un valor como lo exige el anexo: punto decimal, **dos decimales
/// truncados**, sin separador de miles ni símbolo.
///
/// Trunca, no redondea. Como el dinero ya vive en centavos, truncar a dos
/// decimales es exacto y no hay nada que perder — pero la función existe
/// separada y probada porque el día que alguien guarde milésimas de peso, aquí
/// es donde se decide.
fn formato(v: Centavos) -> String {
    let neg = v.0 < 0;
    let abs = v.0.unsigned_abs();
    format!("{}{}.{:02}", if neg { "-" } else { "" }, abs / 100, abs % 100)
}

fn solo_digitos(campo: &'static str, v: &str) -> Result<(), ErrorCufe> {
    if v.is_empty() {
        return Err(ErrorCufe::CampoVacio { campo });
    }
    if !v.chars().all(|c| c.is_ascii_digit()) {
        return Err(ErrorCufe::NoEsSoloDigitos {
            campo,
            valor: v.to_string(),
        });
    }
    Ok(())
}

/// La cadena que se va a hashear. Se expone porque cuando la DIAN rechaza un
/// CUFE, lo primero que se compara es esta cadena — no el hash.
pub fn cadena(d: &Datos<'_>) -> Result<String, ErrorCufe> {
    if d.numero_completo.trim().is_empty() {
        return Err(ErrorCufe::CampoVacio { campo: "NumFac" });
    }
    if d.clave_tecnica.trim().is_empty() {
        return Err(ErrorCufe::CampoVacio { campo: "ClTec" });
    }
    // AAAA-MM-DD: 10 caracteres con guiones en 4 y 7.
    let f = d.fecha.as_bytes();
    if f.len() != 10 || f[4] != b'-' || f[7] != b'-' {
        return Err(ErrorCufe::FechaInvalida(d.fecha.to_string()));
    }
    // HH:MM:SS±HH:MM: 14 caracteres. Se comprueba la forma, no el huso: un
    // desplazamiento distinto de -05:00 es legítimo si el facturador no está
    // en Colombia continental.
    let h = d.hora.as_bytes();
    if h.len() != 14 || h[2] != b':' || h[5] != b':' || !(h[8] == b'+' || h[8] == b'-') || h[11] != b':'
    {
        return Err(ErrorCufe::HoraInvalida(d.hora.to_string()));
    }
    solo_digitos("NitFE", d.nit_facturador)?;
    solo_digitos("NumAdq", d.nit_adquirente)?;

    for (campo, v) in [
        ("ValFac", d.valor_sin_impuestos),
        ("IVA", d.iva),
        ("INC", d.inc),
        ("ICA", d.ica),
        ("ValTot", d.valor_total),
    ] {
        if v.0 < 0 {
            return Err(ErrorCufe::ValorNegativo { campo, valor: v.0 });
        }
    }

    Ok(format!(
        "{}{}{}{}01{}04{}03{}{}{}{}{}{}",
        d.numero_completo,
        d.fecha,
        d.hora,
        formato(d.valor_sin_impuestos),
        formato(d.iva),
        formato(d.inc),
        formato(d.ica),
        formato(d.valor_total),
        d.nit_facturador,
        d.nit_adquirente,
        d.clave_tecnica,
        d.ambiente.codigo(),
    ))
}

/// El CUFE en minúsculas hexadecimales, 96 caracteres.
pub fn calcular(d: &Datos<'_>) -> Result<String, ErrorCufe> {
    Ok(sha384_hex(cadena(d)?.as_bytes()))
}

/// **CUDE** — Código Único de Documento Electrónico (anexo 1.9, num. 11.4).
///
/// Lo llevan las notas crédito, las notas débito y los documentos de
/// transmisión. Misma composición que el CUFE, pero con el **PIN del software**
/// en lugar de la clave técnica del rango.
///
/// `datos.clave_tecnica` **se ignora**: la firma pide el PIN aparte para que sea
/// imposible pasar una clave técnica por descuido y obtener un código que
/// parece bueno.
pub fn calcular_cude(d: &Datos<'_>, pin_software: &str) -> Result<String, ErrorCufe> {
    if pin_software.trim().is_empty() {
        return Err(ErrorCufe::CampoVacio {
            campo: "Software-PIN",
        });
    }
    let con_pin = Datos {
        clave_tecnica: pin_software,
        ..d.clone()
    };
    Ok(sha384_hex(cadena(&con_pin)?.as_bytes()))
}

// ---------------------------------------------------------------------------
// SHA-384
// ---------------------------------------------------------------------------
//
// Implementado aquí, sin dependencia. La razón no es el peso: es que el CUFE es
// el mecanismo de control fiscal de un documento con efectos legales, y su
// cálculo tiene que poder auditarse leyendo este archivo y contrastándolo con
// FIPS 180-4 y con el ejemplo oficial del anexo — sin seguir una cadena de
// versiones de terceros. La prueba `el_ejemplo_oficial_del_anexo_1_9` es la que
// lo respalda, y hay vectores de FIPS abajo.

const K: [u64; 80] = [
    0x428a2f98d728ae22, 0x7137449123ef65cd, 0xb5c0fbcfec4d3b2f, 0xe9b5dba58189dbbc,
    0x3956c25bf348b538, 0x59f111f1b605d019, 0x923f82a4af194f9b, 0xab1c5ed5da6d8118,
    0xd807aa98a3030242, 0x12835b0145706fbe, 0x243185be4ee4b28c, 0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f, 0x80deb1fe3b1696b1, 0x9bdc06a725c71235, 0xc19bf174cf692694,
    0xe49b69c19ef14ad2, 0xefbe4786384f25e3, 0x0fc19dc68b8cd5b5, 0x240ca1cc77ac9c65,
    0x2de92c6f592b0275, 0x4a7484aa6ea6e483, 0x5cb0a9dcbd41fbd4, 0x76f988da831153b5,
    0x983e5152ee66dfab, 0xa831c66d2db43210, 0xb00327c898fb213f, 0xbf597fc7beef0ee4,
    0xc6e00bf33da88fc2, 0xd5a79147930aa725, 0x06ca6351e003826f, 0x142929670a0e6e70,
    0x27b70a8546d22ffc, 0x2e1b21385c26c926, 0x4d2c6dfc5ac42aed, 0x53380d139d95b3df,
    0x650a73548baf63de, 0x766a0abb3c77b2a8, 0x81c2c92e47edaee6, 0x92722c851482353b,
    0xa2bfe8a14cf10364, 0xa81a664bbc423001, 0xc24b8b70d0f89791, 0xc76c51a30654be30,
    0xd192e819d6ef5218, 0xd69906245565a910, 0xf40e35855771202a, 0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8, 0x1e376c085141ab53, 0x2748774cdf8eeb99, 0x34b0bcb5e19b48a8,
    0x391c0cb3c5c95a63, 0x4ed8aa4ae3418acb, 0x5b9cca4f7763e373, 0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc, 0x78a5636f43172f60, 0x84c87814a1f0ab72, 0x8cc702081a6439ec,
    0x90befffa23631e28, 0xa4506cebde82bde9, 0xbef9a3f7b2c67915, 0xc67178f2e372532b,
    0xca273eceea26619c, 0xd186b8c721c0c207, 0xeada7dd6cde0eb1e, 0xf57d4f7fee6ed178,
    0x06f067aa72176fba, 0x0a637dc5a2c898a6, 0x113f9804bef90dae, 0x1b710b35131c471b,
    0x28db77f523047d84, 0x32caab7b40c72493, 0x3c9ebe0a15c9bebc, 0x431d67c49c100d4c,
    0x4cc5d4becb3e42b6, 0x597f299cfc657e2a, 0x5fcb6fab3ad6faec, 0x6c44198c4a475817,
];

/// Estado inicial de SHA-384 (FIPS 180-4 §5.3.4). Es lo ÚNICO que lo distingue
/// de SHA-512, junto con truncar a 48 bytes.
const H0: [u64; 8] = [
    0xcbbb9d5dc1059ed8, 0x629a292a367cd507, 0x9159015a3070dd17, 0x152fecd8f70e5939,
    0x67332667ffc00b31, 0x8eb44a8768581511, 0xdb0c2e0d64f98fa7, 0x47b5481dbefa4fa4,
];

fn sha384(mensaje: &[u8]) -> [u8; 48] {
    let mut h = H0;

    // Relleno: 0x80, ceros, y la longitud en bits en 128 bits big-endian.
    let mut datos = mensaje.to_vec();
    let bits = (mensaje.len() as u128) * 8;
    datos.push(0x80);
    while datos.len() % 128 != 112 {
        datos.push(0);
    }
    datos.extend_from_slice(&bits.to_be_bytes());

    for bloque in datos.chunks_exact(128) {
        let mut w = [0u64; 80];
        for (i, trozo) in bloque.chunks_exact(8).enumerate() {
            w[i] = u64::from_be_bytes(trozo.try_into().expect("8 bytes"));
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..80 {
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (dst, src) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *dst = dst.wrapping_add(src);
        }
    }

    // SHA-384 = SHA-512 truncado a los primeros 6 registros.
    let mut salida = [0u8; 48];
    for (i, v) in h.iter().take(6).enumerate() {
        salida[i * 8..i * 8 + 8].copy_from_slice(&v.to_be_bytes());
    }
    salida
}

fn sha384_hex(mensaje: &[u8]) -> String {
    sha384(mensaje).iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Vectores de FIPS 180-4 / NIST CAVP. Si el SHA-384 estuviera mal, todo lo
    /// demás daría un hash consistente y equivocado — que es la peor forma de
    /// estar mal.
    #[test]
    fn el_sha384_reproduce_los_vectores_de_nist() {
        assert_eq!(
            sha384_hex(b""),
            "38b060a751ac96384cd9327eb1b1e36a21fdb71114be07434c0cc7bf63f6e1da274edebfe76f65fbd51ad2f14898b95b"
        );
        assert_eq!(
            sha384_hex(b"abc"),
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
        );
        assert_eq!(
            sha384_hex(b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu"),
            "09330c33f71147e83d192fc782cd1b4753111b173b3b05d22fa08086e3b0f712fcc7c71a557e2db966c3e9fa91746039"
        );
        // Un bloque de 112 bytes: el caso justo en la frontera del relleno, que
        // es donde se rompen las implementaciones a mano.
        assert_eq!(
            sha384_hex(&[b'a'; 111]).len(),
            96
        );
        assert_ne!(sha384_hex(&[b'a'; 111]), sha384_hex(&[b'a'; 112]));
        assert_ne!(sha384_hex(&[b'a'; 127]), sha384_hex(&[b'a'; 128]));
    }

    fn ejemplo() -> Datos<'static> {
        Datos {
            numero_completo: "323200000129",
            fecha: "2019-01-16",
            hora: "10:53:10-05:00",
            valor_sin_impuestos: Centavos::de_pesos(1_500_000),
            iva: Centavos::de_pesos(285_000),
            inc: Centavos::CERO,
            ica: Centavos::CERO,
            valor_total: Centavos::de_pesos(1_785_000),
            nit_facturador: "700085371",
            nit_adquirente: "800199436",
            clave_tecnica: "693ff6f2a553c3646a063436fd4dd9ded0311471",
            ambiente: Ambiente::Produccion,
        }
    }

    /// **La prueba de aceptación del módulo.** Es el ejemplo textual del Anexo
    /// Técnico 1.9, numeral 11.2.1, con su resultado publicado. Si esto pasa, el
    /// CUFE que produce este sistema es el que espera la DIAN.
    #[test]
    fn el_ejemplo_oficial_del_anexo_1_9() {
        assert_eq!(
            cadena(&ejemplo()).unwrap(),
            "3232000001292019-01-1610:53:10-05:001500000.0001285000.00040.00030.00\
             1785000.00700085371800199436693ff6f2a553c3646a063436fd4dd9ded03114711"
                .replace(['\n', ' '], "")
        );
        assert_eq!(
            calcular(&ejemplo()).unwrap(),
            "8bb918b19ba22a694f1da11c643b5e9de39adf60311cf179179e9b33381030bcd4c3c3f156c506ed5908f9276f5bd9b4"
        );
    }

    /// Directiva 8: que DISCRIMINE. Cambiar **cualquier** campo tiene que
    /// cambiar el CUFE. Si un campo no entrara al hash, esto lo caza.
    #[test]
    fn cambiar_cualquier_campo_cambia_el_cufe() {
        let base = calcular(&ejemplo()).unwrap();
        let variantes: Vec<Datos<'static>> = vec![
            Datos { numero_completo: "323200000130", ..ejemplo() },
            Datos { fecha: "2019-01-17", ..ejemplo() },
            Datos { hora: "10:53:11-05:00", ..ejemplo() },
            Datos { hora: "10:53:10-06:00", ..ejemplo() },
            Datos { valor_sin_impuestos: Centavos::de_pesos(1_500_001), ..ejemplo() },
            Datos { iva: Centavos::de_pesos(285_001), ..ejemplo() },
            Datos { inc: Centavos(1), ..ejemplo() },
            Datos { ica: Centavos(1), ..ejemplo() },
            Datos { valor_total: Centavos::de_pesos(1_785_001), ..ejemplo() },
            Datos { nit_facturador: "700085372", ..ejemplo() },
            Datos { nit_adquirente: "800199437", ..ejemplo() },
            Datos { clave_tecnica: "693ff6f2a553c3646a063436fd4dd9ded0311472", ..ejemplo() },
            Datos { ambiente: Ambiente::Pruebas, ..ejemplo() },
        ];
        let mut vistos = std::collections::HashSet::new();
        vistos.insert(base.clone());
        for (i, v) in variantes.iter().enumerate() {
            let c = calcular(v).unwrap();
            assert_ne!(c, base, "la variante {i} no cambió el CUFE: ese campo no entra al hash");
            assert!(vistos.insert(c), "la variante {i} colisionó con otra");
        }
    }

    /// El error del que avisa la documentación del módulo: la especificación de
    /// 2016 (SHA-1, separador `;`, impuesto `02`) da otra cosa. Se comprueba
    /// que NO coincide, para que nadie «arregle» este módulo hacia atrás.
    #[test]
    fn la_especificacion_derogada_de_2016_da_otro_resultado() {
        let d = ejemplo();
        let vieja = format!(
            "{};{};{};01;{};02;{};03;{};{};{};{};{};{}",
            d.numero_completo,
            "20190116105310",
            formato(d.valor_sin_impuestos),
            formato(Centavos::CERO),
            formato(d.iva),
            formato(d.ica),
            formato(d.valor_total),
            d.nit_facturador,
            "31",
            d.nit_adquirente,
            d.clave_tecnica
        );
        assert_ne!(
            sha384_hex(vieja.as_bytes()),
            calcular(&d).unwrap(),
            "la especificación de 2016 y la de 2023 no pueden dar lo mismo"
        );
    }

    #[test]
    fn el_impuesto_que_no_aplica_se_escribe_cero_no_se_omite() {
        let c = cadena(&ejemplo()).unwrap();
        assert!(c.contains("040.00030.00"), "INC e ICA en cero tienen que aparecer: {c}");
    }

    #[test]
    fn el_valor_va_con_dos_decimales_truncados_sin_separador_de_miles() {
        assert_eq!(formato(Centavos::de_pesos(1_500_000)), "1500000.00");
        assert_eq!(formato(Centavos(1)), "0.01");
        assert_eq!(formato(Centavos::CERO), "0.00");
        assert_eq!(formato(Centavos(123_456_789)), "1234567.89");
        // Nada de puntos de miles ni de símbolo: eso es para la pantalla.
        assert!(!formato(Centavos::de_pesos(1_000_000)).contains('$'));
        assert_eq!(formato(Centavos::de_pesos(1_000_000)).matches('.').count(), 1);
    }

    #[test]
    fn lo_que_la_dian_rechazaria_se_rechaza_aqui_antes() {
        let malo = |d: Datos<'_>| calcular(&d).unwrap_err();

        assert!(matches!(
            malo(Datos { clave_tecnica: "", ..ejemplo() }),
            ErrorCufe::CampoVacio { campo: "ClTec" }
        ));
        assert!(matches!(
            malo(Datos { numero_completo: "  ", ..ejemplo() }),
            ErrorCufe::CampoVacio { campo: "NumFac" }
        ));
        assert!(matches!(
            malo(Datos { fecha: "16/01/2019", ..ejemplo() }),
            ErrorCufe::FechaInvalida(_)
        ));
        // Hora sin desplazamiento horario: es el olvido típico.
        assert!(matches!(
            malo(Datos { hora: "10:53:10", ..ejemplo() }),
            ErrorCufe::HoraInvalida(_)
        ));
        // NIT con guion y DV: el otro olvido típico.
        assert!(matches!(
            malo(Datos { nit_facturador: "700085371-1", ..ejemplo() }),
            ErrorCufe::NoEsSoloDigitos { campo: "NitFE", .. }
        ));
        assert!(matches!(
            malo(Datos { nit_adquirente: "800.199.436", ..ejemplo() }),
            ErrorCufe::NoEsSoloDigitos { campo: "NumAdq", .. }
        ));
        assert!(matches!(
            malo(Datos { iva: Centavos(-1), ..ejemplo() }),
            ErrorCufe::ValorNegativo { campo: "IVA", .. }
        ));
        // Un desplazamiento distinto de -05:00 es legítimo y NO se rechaza.
        assert!(calcular(&Datos { hora: "10:53:10+02:00", ..ejemplo() }).is_ok());
    }

    /// **Prueba de aceptación del CUDE**: el ejemplo de NOTA CRÉDITO del anexo
    /// 1.9, numeral 11.4.3, con su resultado publicado.
    #[test]
    fn el_ejemplo_oficial_de_cude_para_nota_credito() {
        let d = Datos {
            numero_completo: "8110007871",
            fecha: "2019-01-12",
            hora: "07:00:00-05:00",
            valor_sin_impuestos: Centavos::de_pesos(5_000),
            iva: Centavos::de_pesos(950),
            inc: Centavos::CERO,
            ica: Centavos::CERO,
            valor_total: Centavos::de_pesos(5_950),
            nit_facturador: "900373076",
            nit_adquirente: "8355990",
            // Se pone una clave técnica cualquiera a propósito: tiene que
            // IGNORARSE. Si el CUDE la usara, el hash no coincidiría.
            clave_tecnica: "no-se-debe-usar",
            ambiente: Ambiente::Produccion,
        };
        assert_eq!(
            calcular_cude(&d, "12301").unwrap(),
            "907e4444decc9e59c160a2fb3b6659b33dc5b632a5008922b9a62f83f757b1c448e47f5867f2b50dbdb96f48c7681168"
        );
    }

    /// El otro ejemplo del anexo (num. 11.4.1, documento de transmisión), que
    /// además usa ambiente de PRUEBAS, INC en cero e ICA distinto de cero — la
    /// combinación que más se equivoca.
    #[test]
    fn el_ejemplo_oficial_de_cude_para_documento_de_transmision() {
        let d = Datos {
            numero_completo: "8110007871",
            fecha: "2019-02-20",
            hora: "16:46:55-05:00",
            valor_sin_impuestos: Centavos(23_528),
            iva: Centavos(1_900),
            inc: Centavos::CERO,
            ica: Centavos(828),
            valor_total: Centavos(26_256),
            nit_facturador: "900373076",
            nit_adquirente: "8355990",
            clave_tecnica: "ignorada",
            ambiente: Ambiente::Pruebas,
        };
        assert_eq!(
            calcular_cude(&d, "12345").unwrap(),
            "955327eb55f8bdf16d069358a063d87e1577a292cb088ec186ed60bbc38e750b7b3980659b278ead789b95f9c51a9ef7"
        );
    }

    /// CUFE y CUDE **no pueden coincidir** con los mismos datos: se distinguen
    /// justo en el campo que la DIAN valida distinto. Si esta prueba pasara a
    /// dar lo mismo, es que uno de los dos dejó de leer su campo.
    #[test]
    fn el_cufe_y_el_cude_no_dan_lo_mismo() {
        let d = ejemplo();
        let cufe = calcular(&d).unwrap();
        let cude = calcular_cude(&d, "12301").unwrap();
        assert_ne!(cufe, cude);
        // Y el CUDE sí cambia con el PIN, que es su llave.
        assert_ne!(cude, calcular_cude(&d, "12302").unwrap());
        // Pero NO cambia con la clave técnica, que ignora.
        let otra = Datos { clave_tecnica: "otra cosa", ..d };
        assert_eq!(cude, calcular_cude(&otra, "12301").unwrap());
    }

    #[test]
    fn el_cude_sin_pin_de_software_se_rechaza() {
        assert!(matches!(
            calcular_cude(&ejemplo(), ""),
            Err(ErrorCufe::CampoVacio { campo: "Software-PIN" })
        ));
        assert!(calcular_cude(&ejemplo(), "  ").is_err());
    }

    #[test]
    fn el_cufe_mide_noventa_y_seis_caracteres_hexadecimales() {
        let c = calcular(&ejemplo()).unwrap();
        assert_eq!(c.len(), 96);
        assert!(c.chars().all(|x| x.is_ascii_hexdigit() && !x.is_ascii_uppercase()));
    }
}
