//! El monto en letras que va impreso en la factura.
//!
//! El sistema viejo lo resolvía con la tabla `MONTOESC.DBF` (108 filas) más el
//! procedimiento `montoesc` de `procesos.prg`, que iba concatenando trozos
//! buscados por índice. Si la tabla faltaba, la factura salía sin el monto y el
//! programa avisaba con un `messagebox`.
//!
//! Un dato que no cambia nunca no es un dato: es código (directiva 6). Aquí no
//! hay tabla que falte, ni instalación que pueda quedarse sin ella.
//!
//! Cubre hasta 999.999.999.999.999 pesos. Por encima devuelve `None` en vez de
//! imprimir algo truncado.

use crate::dinero::Centavos;

const UNIDADES: [&str; 30] = [
    "CERO", "UNO", "DOS", "TRES", "CUATRO", "CINCO", "SEIS", "SIETE", "OCHO", "NUEVE", "DIEZ",
    "ONCE", "DOCE", "TRECE", "CATORCE", "QUINCE", "DIECISÉIS", "DIECISIETE", "DIECIOCHO",
    "DIECINUEVE", "VEINTE", "VEINTIUNO", "VEINTIDÓS", "VEINTITRÉS", "VEINTICUATRO",
    "VEINTICINCO", "VEINTISÉIS", "VEINTISIETE", "VEINTIOCHO", "VEINTINUEVE",
];

const DECENAS: [&str; 10] = [
    "", "", "VEINTE", "TREINTA", "CUARENTA", "CINCUENTA", "SESENTA", "SETENTA", "OCHENTA",
    "NOVENTA",
];

const CENTENAS: [&str; 10] = [
    "", "CIENTO", "DOSCIENTOS", "TRESCIENTOS", "CUATROCIENTOS", "QUINIENTOS", "SEISCIENTOS",
    "SETECIENTOS", "OCHOCIENTOS", "NOVECIENTOS",
];

/// El límite: tres grupos de millón (billones) por encima de los miles.
const TOPE: i64 = 1_000_000_000_000_000;

/// Escribe un entero de 0 a 999 en letras.
fn menos_de_mil(n: i64) -> String {
    debug_assert!((0..1000).contains(&n));
    if n < 30 {
        return UNIDADES[n as usize].to_string();
    }
    if n < 100 {
        let (d, u) = (n / 10, n % 10);
        return if u == 0 {
            DECENAS[d as usize].to_string()
        } else {
            format!("{} Y {}", DECENAS[d as usize], UNIDADES[u as usize])
        };
    }
    // «CIEN» a secas solo cuando es exactamente cien; 101 es «CIENTO UNO».
    if n == 100 {
        return "CIEN".to_string();
    }
    let (c, r) = (n / 100, n % 100);
    if r == 0 {
        CENTENAS[c as usize].to_string()
    } else {
        format!("{} {}", CENTENAS[c as usize], menos_de_mil(r))
    }
}

/// Apócope: delante de un sustantivo masculino, «uno» es «un» y «veintiuno»
/// es «veintiún». Se dice «VEINTIÚN MIL PESOS», no «VEINTIUNO MIL PESOS».
fn apocopar(s: String) -> String {
    if s == "UNO" {
        return "UN".to_string();
    }
    if s == "VEINTIUNO" {
        return "VEINTIÚN".to_string();
    }
    if let Some(resto) = s.strip_suffix(" UNO") {
        return format!("{resto} UN");
    }
    if let Some(resto) = s.strip_suffix(" VEINTIUNO") {
        return format!("{resto} VEINTIÚN");
    }
    s
}

/// Escribe un entero no negativo en letras. `None` si no cabe.
pub fn entero(n: i64) -> Option<String> {
    if !(0..TOPE).contains(&n) {
        return None;
    }
    if n == 0 {
        return Some("CERO".to_string());
    }

    let mut partes: Vec<String> = Vec::new();
    // (divisor, singular, plural)
    let escalas: [(i64, &str, &str); 3] = [
        (1_000_000_000_000, "BILLÓN", "BILLONES"),
        (1_000_000, "MILLÓN", "MILLONES"),
        (1_000, "MIL", "MIL"),
    ];

    let mut resto = n;
    for (divisor, singular, plural) in escalas {
        let grupo = resto / divisor;
        if grupo > 0 {
            // «MIL» no lleva «UN» delante: es «MIL PESOS», no «UN MIL PESOS».
            // «MILLÓN» sí: «UN MILLÓN».
            if grupo == 1 && divisor == 1_000 {
                partes.push("MIL".to_string());
            } else {
                partes.push(format!(
                    "{} {}",
                    apocopar(escribir_grupo(grupo)),
                    if grupo == 1 { singular } else { plural }
                ));
            }
            resto %= divisor;
        }
    }
    if resto > 0 {
        partes.push(menos_de_mil(resto));
    }
    Some(partes.join(" "))
}

/// Un grupo de hasta seis cifras (lo que puede preceder a MILLONES o BILLONES),
/// porque «doscientos treinta y cuatro mil millones» es un grupo válido.
fn escribir_grupo(n: i64) -> String {
    if n < 1000 {
        return menos_de_mil(n);
    }
    let (miles, resto) = (n / 1000, n % 1000);
    let cabeza = if miles == 1 {
        "MIL".to_string()
    } else {
        format!("{} MIL", apocopar(menos_de_mil(miles)))
    };
    if resto == 0 {
        cabeza
    } else {
        format!("{cabeza} {}", menos_de_mil(resto))
    }
}

/// El monto tal como va impreso: `«… PESOS M/CTE»`, con los centavos solo si
/// los hay.
///
/// `None` si el valor es negativo o no cabe: una factura en letras que dice
/// algo distinto del número es peor que una factura sin letras.
pub fn monto(v: Centavos) -> Option<String> {
    if v.is_negativo_para_letras() {
        return None;
    }
    let (pesos, centavos) = (v.0 / 100, v.0 % 100);
    let texto = entero(pesos)?;
    Some(if centavos == 0 {
        format!("{texto} PESOS M/CTE")
    } else {
        format!("{texto} PESOS CON {} CENTAVOS M/CTE", entero(centavos)?)
    })
}

impl Centavos {
    fn is_negativo_para_letras(self) -> bool {
        self.0 < 0
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn los_casos_que_la_gente_escribe_mal() {
        assert_eq!(entero(0).unwrap(), "CERO");
        assert_eq!(entero(16).unwrap(), "DIECISÉIS");
        assert_eq!(entero(21).unwrap(), "VEINTIUNO");
        assert_eq!(entero(30).unwrap(), "TREINTA");
        assert_eq!(entero(31).unwrap(), "TREINTA Y UNO");
        // «CIEN» solo cuando es cien exacto.
        assert_eq!(entero(100).unwrap(), "CIEN");
        assert_eq!(entero(101).unwrap(), "CIENTO UNO");
        assert_eq!(entero(200).unwrap(), "DOSCIENTOS");
        assert_eq!(entero(500).unwrap(), "QUINIENTOS");
        assert_eq!(entero(999).unwrap(), "NOVECIENTOS NOVENTA Y NUEVE");
    }

    #[test]
    fn mil_no_lleva_un_delante_y_millon_si() {
        assert_eq!(entero(1_000).unwrap(), "MIL");
        assert_eq!(entero(1_001).unwrap(), "MIL UNO");
        assert_eq!(entero(2_000).unwrap(), "DOS MIL");
        assert_eq!(entero(1_000_000).unwrap(), "UN MILLÓN");
        assert_eq!(entero(2_000_000).unwrap(), "DOS MILLONES");
        assert_eq!(entero(1_000_000_000_000).unwrap(), "UN BILLÓN");
    }

    #[test]
    fn el_apocope_de_veintiuno_y_uno_delante_de_mil() {
        assert_eq!(entero(21_000).unwrap(), "VEINTIÚN MIL");
        assert_eq!(entero(31_000).unwrap(), "TREINTA Y UN MIL");
        assert_eq!(entero(21_000_000).unwrap(), "VEINTIÚN MILLONES");
        assert_eq!(entero(121_000).unwrap(), "CIENTO VEINTIÚN MIL");
    }

    #[test]
    fn los_valores_reales_del_corpus() {
        // La factura más alta del corpus: $2.565.030.
        assert_eq!(
            monto(Centavos::de_pesos(2_565_030)).unwrap(),
            "DOS MILLONES QUINIENTOS SESENTA Y CINCO MIL TREINTA PESOS M/CTE"
        );
        // La suma facturada: $114.557.096,66.
        assert_eq!(
            monto(Centavos(11_455_709_666)).unwrap(),
            "CIENTO CATORCE MILLONES QUINIENTOS CINCUENTA Y SIETE MIL NOVENTA Y SEIS \
             PESOS CON SESENTA Y SEIS CENTAVOS M/CTE"
        );
        assert_eq!(monto(Centavos::CERO).unwrap(), "CERO PESOS M/CTE");
    }

    #[test]
    fn los_millones_con_miles_dentro() {
        assert_eq!(
            entero(234_567_890).unwrap(),
            "DOSCIENTOS TREINTA Y CUATRO MILLONES QUINIENTOS SESENTA Y SIETE MIL \
             OCHOCIENTOS NOVENTA"
        );
        assert_eq!(entero(1_000_500).unwrap(), "UN MILLÓN QUINIENTOS");
        assert_eq!(entero(100_000_000).unwrap(), "CIEN MILLONES");
    }

    #[test]
    fn lo_que_no_cabe_devuelve_none_en_vez_de_truncar() {
        assert_eq!(entero(-1), None);
        assert_eq!(entero(TOPE), None);
        assert_eq!(monto(Centavos(-1)), None);
        assert!(entero(TOPE - 1).is_some());
    }

    /// Directiva 8: que discrimine. Un millón de valores distintos no se puede
    /// revisar a ojo, pero sí se puede exigir que **ninguno colisione** —dos
    /// números distintos con las mismas letras serían un monto ambiguo en una
    /// factura— y que ninguno se quede sin texto.
    #[test]
    fn mil_montos_distintos_dan_mil_textos_distintos() {
        let mut vistos = std::collections::HashSet::new();
        for i in 0..1000i64 {
            let n = i * 1_234_567 + i;
            let t = entero(n).unwrap_or_else(|| panic!("sin texto para {n}"));
            assert!(!t.is_empty());
            assert!(!t.contains("  "), "espacios dobles en «{t}» (n={n})");
            assert!(vistos.insert(t.clone()), "«{t}» salió dos veces (n={n})");
        }
    }
}
