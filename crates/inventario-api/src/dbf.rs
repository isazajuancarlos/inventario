//! Lector de DBF (dBase III / FoxPro 2.x / Visual FoxPro).
//!
//! Solo lectura, y solo lo que el corpus usa: campos `C`, `N`, `D`, `L`, `I`.
//! Un tipo que no se reconozca **no se adivina**: se devuelve como crudo y el
//! migrador lo rechaza con motivo (directiva 20).
//!
//! Dos detalles que cuestan una tarde si no se saben:
//!
//! - El byte 0 de cada registro es `*` cuando la fila está **borrada**. En el
//!   corpus hay 2.596 filas borradas en `valorconvenio` y 332 en `ENCRECIBO`;
//!   leerlas como vivas duplicaría datos que alguien quitó a propósito.
//! - El texto viene en **CP850/latin-1**, no en UTF-8. Sin convertir, cada
//!   «ñ» y cada tilde salen como un byte suelto que rompe el JSON después.

use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub enum Valor {
    Texto(String),
    Numero(f64),
    /// Fecha ISO. `None` cuando el campo venía vacío o con una fecha imposible
    /// —el corpus tiene fechas en cero— y eso se distingue de «no hay campo».
    Fecha(Option<String>),
    Logico(Option<bool>),
    /// Un tipo que este lector no interpreta. No se convierte a texto ni a
    /// cero: quien lo reciba tiene que decidir a la vista de que es crudo.
    Crudo(Vec<u8>),
}

impl Valor {
    pub fn texto(&self) -> &str {
        match self {
            Valor::Texto(s) => s,
            _ => "",
        }
    }
    pub fn numero(&self) -> Option<f64> {
        match self {
            Valor::Numero(n) => Some(*n),
            _ => None,
        }
    }
    pub fn fecha(&self) -> Option<&str> {
        match self {
            Valor::Fecha(f) => f.as_deref(),
            _ => None,
        }
    }
}

/// La definición de un campo. `decimales` no se usa al convertir —un numérico
/// de DBF lleva el punto decimal DENTRO del texto— pero se conserva porque es
/// lo que permite auditar una tabla desconocida antes de mapearla.
#[derive(Debug, Clone)]
pub struct Campo {
    pub nombre: String,
    pub tipo: char,
    pub largo: usize,
    #[allow(dead_code)]
    pub decimales: u8,
}

#[derive(Debug)]
pub struct Tabla {
    /// Las columnas del DBF. El migrador no las recorre —mapea por nombre— pero
    /// son lo que se mira para entender una tabla que nadie documentó.
    #[allow(dead_code)]
    pub campos: Vec<Campo>,
    pub filas: Vec<Fila>,
    /// Filas marcadas como borradas. Se cuentan pero no se devuelven, y el
    /// informe de migración las publica: 2.596 filas de `valorconvenio` y 332
    /// de `ENCRECIBO` están borradas en el corpus, y quien reciba el sistema
    /// nuevo tiene derecho a saber que no se perdieron ahí.
    pub borradas: usize,
}

pub type Fila = HashMap<String, Valor>;

#[derive(Debug, thiserror::Error)]
pub enum ErrorDbf {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{ruta}: cabecera truncada ({bytes} bytes; hacen falta 32)")]
    CabeceraCorta { ruta: String, bytes: usize },
    #[error("{ruta}: la cabecera declara {declarado} registros de {largo} bytes, y el archivo mide {real}")]
    TamañoIncoherente {
        ruta: String,
        declarado: u32,
        largo: u16,
        real: usize,
    },
}

/// CP850 y latin-1 coinciden en las letras que importan aquí (vocales con
/// tilde, ñ, ü) en el rango 0xA0..0xFF; por debajo, CP850 usa los caracteres
/// de dibujo de caja, que en un dato de negocio no aparecen. Se decodifica como
/// latin-1, que es lo que el corpus produce al abrirlo.
fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

pub fn leer(ruta: &Path) -> Result<Tabla, ErrorDbf> {
    let datos = std::fs::read(ruta)?;
    let nombre = ruta.display().to_string();
    if datos.len() < 32 {
        return Err(ErrorDbf::CabeceraCorta {
            ruta: nombre,
            bytes: datos.len(),
        });
    }

    let n_registros = u32::from_le_bytes([datos[4], datos[5], datos[6], datos[7]]);
    let largo_cabecera = u16::from_le_bytes([datos[8], datos[9]]) as usize;
    let largo_registro = u16::from_le_bytes([datos[10], datos[11]]) as usize;

    let mut campos = Vec::new();
    let mut off = 32;
    while off + 32 <= datos.len() {
        if datos[off] == 0x0D || datos[off] == 0x00 {
            break;
        }
        let d = &datos[off..off + 32];
        let nombre_campo = latin1(&d[0..11])
            .trim_end_matches('\0')
            .trim()
            .to_ascii_uppercase();
        campos.push(Campo {
            nombre: nombre_campo,
            tipo: d[11] as char,
            largo: d[16] as usize,
            decimales: d[17],
        });
        off += 32;
    }

    // El tamaño declarado tiene que caber. Si no, se falla: leer «lo que haya»
    // produciría filas a medias que parecen datos.
    let necesario = largo_cabecera + n_registros as usize * largo_registro;
    if largo_registro == 0 || necesario > datos.len() + largo_registro {
        return Err(ErrorDbf::TamañoIncoherente {
            ruta: nombre,
            declarado: n_registros,
            largo: largo_registro as u16,
            real: datos.len(),
        });
    }

    let mut filas = Vec::new();
    let mut borradas = 0usize;
    for i in 0..n_registros as usize {
        let inicio = largo_cabecera + i * largo_registro;
        if inicio + largo_registro > datos.len() {
            break;
        }
        let reg = &datos[inicio..inicio + largo_registro];
        if reg[0] == b'*' {
            borradas += 1;
            continue;
        }
        let mut fila = HashMap::new();
        let mut p = 1usize;
        for c in &campos {
            if p + c.largo > reg.len() {
                break;
            }
            fila.insert(c.nombre.clone(), convertir(c, &reg[p..p + c.largo]));
            p += c.largo;
        }
        filas.push(fila);
    }

    Ok(Tabla {
        campos,
        filas,
        borradas,
    })
}

fn convertir(c: &Campo, crudo: &[u8]) -> Valor {
    match c.tipo {
        'C' => Valor::Texto(latin1(crudo).trim_end().to_string()),
        'N' | 'F' => {
            let t = latin1(crudo);
            let t = t.trim();
            if t.is_empty() {
                // Un numérico en blanco NO es cero: es «no se digitó». Se
                // devuelve como texto vacío para que el migrador lo distinga.
                Valor::Texto(String::new())
            } else {
                match t.parse::<f64>() {
                    Ok(n) => Valor::Numero(n),
                    // Un numérico ilegible se conserva tal cual para que salga
                    // en el informe de rechazos, en vez de convertirse en 0.
                    Err(_) => Valor::Texto(t.to_string()),
                }
            }
        }
        'D' => {
            let t = latin1(crudo);
            let t = t.trim();
            if t.len() != 8 || t.chars().any(|c| !c.is_ascii_digit()) || t == "00000000" {
                return Valor::Fecha(None);
            }
            let (a, m, d) = (&t[0..4], &t[4..6], &t[6..8]);
            match (a.parse::<i32>(), m.parse::<u32>(), d.parse::<u32>()) {
                (Ok(a), Ok(m), Ok(d))
                    if chrono::NaiveDate::from_ymd_opt(a, m, d).is_some() =>
                {
                    Valor::Fecha(Some(format!("{a:04}-{m:02}-{d:02}")))
                }
                // Una fecha imposible —un 31 de febrero— no se «arregla» al 28.
                _ => Valor::Fecha(None),
            }
        }
        'L' => Valor::Logico(match latin1(crudo).trim().to_ascii_uppercase().as_str() {
            "T" | "Y" => Some(true),
            "F" | "N" => Some(false),
            _ => None,
        }),
        'I' if crudo.len() == 4 => Valor::Numero(i32::from_le_bytes([
            crudo[0], crudo[1], crudo[2], crudo[3],
        ]) as f64),
        _ => Valor::Crudo(crudo.to_vec()),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use std::io::Write;

    /// Construye un DBF mínimo en memoria para no depender del corpus real en
    /// las pruebas unitarias.
    fn dbf(campos: &[(&str, char, u8, u8)], filas: &[Vec<&str>], borradas: &[usize]) -> Vec<u8> {
        let largo_cabecera = 32 + campos.len() * 32 + 1;
        let largo_registro: usize = 1 + campos.iter().map(|c| c.2 as usize).sum::<usize>();
        let mut v = vec![0u8; 32];
        v[0] = 0x03;
        v[4..8].copy_from_slice(&(filas.len() as u32).to_le_bytes());
        v[8..10].copy_from_slice(&(largo_cabecera as u16).to_le_bytes());
        v[10..12].copy_from_slice(&(largo_registro as u16).to_le_bytes());
        for (nombre, tipo, largo, dec) in campos {
            let mut d = vec![0u8; 32];
            for (i, b) in nombre.bytes().take(10).enumerate() {
                d[i] = b;
            }
            d[11] = *tipo as u8;
            d[16] = *largo;
            d[17] = *dec;
            v.extend_from_slice(&d);
        }
        v.push(0x0D);
        for (i, fila) in filas.iter().enumerate() {
            v.push(if borradas.contains(&i) { b'*' } else { b' ' });
            for (c, valor) in campos.iter().zip(fila) {
                let mut celda = vec![b' '; c.2 as usize];
                for (j, b) in valor.bytes().take(c.2 as usize).enumerate() {
                    celda[j] = b;
                }
                v.extend_from_slice(&celda);
            }
        }
        v
    }

    fn en_disco(bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dbf-prueba-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("t{}.dbf", bytes.len()));
        std::fs::File::create(&p).unwrap().write_all(bytes).unwrap();
        p
    }

    #[test]
    fn lee_texto_numeros_y_fechas() {
        let b = dbf(
            &[("COD", 'C', 10, 0), ("VALOR", 'N', 11, 2), ("FECHA", 'D', 8, 0)],
            &[vec!["1102001", "409.00", "20040503"]],
            &[],
        );
        let t = leer(&en_disco(&b)).unwrap();
        assert_eq!(t.filas.len(), 1);
        assert_eq!(t.filas[0]["COD"].texto(), "1102001");
        assert_eq!(t.filas[0]["VALOR"].numero(), Some(409.0));
        assert_eq!(t.filas[0]["FECHA"].fecha(), Some("2004-05-03"));
    }

    /// 2.596 filas de `valorconvenio` están borradas. Leerlas como vivas
    /// resucitaría precios que alguien quitó.
    #[test]
    fn las_filas_borradas_no_salen_pero_se_cuentan() {
        let b = dbf(
            &[("COD", 'C', 5, 0)],
            &[vec!["viva"], vec!["muerta"], vec!["viva2"]],
            &[1],
        );
        let t = leer(&en_disco(&b)).unwrap();
        assert_eq!(t.filas.len(), 2);
        assert_eq!(t.borradas, 1);
        assert!(!t.filas.iter().any(|f| f["COD"].texto() == "muert"));
    }

    /// Un numérico en blanco es «no se digitó», no cero. En el corpus, 59
    /// facturas tienen `VALORFAC` vacío y confundirlo con cero las daría por
    /// facturas de cero pesos.
    #[test]
    fn un_numerico_en_blanco_no_es_cero() {
        let b = dbf(&[("V", 'N', 11, 2)], &[vec![""], vec!["0.00"]], &[]);
        let t = leer(&en_disco(&b)).unwrap();
        assert_eq!(t.filas[0]["V"].numero(), None, "el vacío no es un número");
        assert_eq!(t.filas[1]["V"].numero(), Some(0.0), "el cero explícito sí");
    }

    #[test]
    fn una_fecha_imposible_no_se_arregla() {
        let b = dbf(
            &[("F", 'D', 8, 0)],
            &[vec!["20040231"], vec!["00000000"], vec!["        "], vec!["2004aa03"]],
            &[],
        );
        let t = leer(&en_disco(&b)).unwrap();
        for i in 0..4 {
            assert_eq!(t.filas[i]["F"].fecha(), None, "fila {i}");
        }
    }

    #[test]
    fn el_texto_llega_con_tildes_y_eñes() {
        // «Botellón» en latin-1: la ó es 0xF3.
        let mut b = dbf(&[("D", 'C', 10, 0)], &[vec!["Botell"]], &[]);
        let pos = b.len() - 4;
        b[pos] = 0xF3; // ó
        b[pos + 1] = b'n';
        let t = leer(&en_disco(&b)).unwrap();
        assert_eq!(t.filas[0]["D"].texto(), "Botellón");
    }

    #[test]
    fn un_archivo_que_no_es_un_dbf_falla_en_vez_de_devolver_filas_vacias() {
        let p = en_disco(b"hola");
        assert!(matches!(leer(&p), Err(ErrorDbf::CabeceraCorta { .. })));
    }

    /// El corpus real. Se salta si no está: la prueba tiene que poder correr en
    /// una máquina sin los datos del cliente.
    #[test]
    fn lee_el_corpus_real_si_esta_disponible() {
        let base = std::path::Path::new("/mnt/data/inventario_2004");
        if !base.exists() {
            eprintln!("corpus no disponible: se omite");
            return;
        }
        let t = leer(&base.join("ARTICULOS.DBF")).unwrap();
        assert_eq!(t.filas.len(), 148, "el corpus tiene 148 artículos vivos");
        assert!(t.campos.iter().any(|c| c.nombre == "COD_ARTI"));

        let c = leer(&base.join("CLIENTES.DBF")).unwrap();
        assert_eq!(c.filas.len(), 794);

        let v = leer(&base.join("valorconvenio.dbf")).unwrap();
        assert_eq!(v.filas.len(), 261, "vivos");
        assert_eq!(v.borradas, 2596, "borrados");
    }
}
