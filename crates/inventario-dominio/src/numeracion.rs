//! Numeración de facturas contra la resolución de la DIAN.
//!
//! `CONFIFAC` del sistema viejo guardaba grupo, prefijo, rango
//! `[NROMINFAC..NROMAXFAC]`, número de resolución, fecha y `ULT_FACTU`. Dos de
//! sus seis grupos ya estaban agotados —el `08` con último 5650 sobre un tope
//! de 5650— y nada avisaba.
//!
//! Aquí agotarse es un error con nombre, no un número que sigue subiendo. El
//! aviso de proximidad (`quedan`) existe para que el cliente pida la resolución
//! nueva antes de quedarse sin facturar un lunes por la mañana.

use chrono::NaiveDate;

/// Una resolución de facturación tal como la autoriza la DIAN.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Resolucion {
    pub id: i64,
    /// Prefijo autorizado. Puede ir vacío: el corpus lo tenía vacío en los seis
    /// grupos, y una resolución sin prefijo es legítima.
    pub prefijo: String,
    pub desde: i64,
    pub hasta: i64,
    /// Último número **efectivamente emitido**. `None` si no se ha emitido
    /// ninguno: es distinto de «emitió el 0», que en un rango que empieza en 1
    /// sería un número fuera del rango.
    pub ultimo_emitido: Option<i64>,
    /// Fecha hasta la que la resolución habilita. `None` = sin vencimiento.
    pub vence: Option<NaiveDate>,
    pub activa: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErrorNumeracion {
    #[error("el rango de la resolución está al revés o vacío: [{desde}..{hasta}]")]
    RangoInvalido { desde: i64, hasta: i64 },
    #[error("la resolución {0} está marcada como inactiva")]
    Inactiva(i64),
    #[error("la resolución {id} venció el {vence} y hoy es {hoy}")]
    Vencida {
        id: i64,
        vence: NaiveDate,
        hoy: NaiveDate,
    },
    #[error(
        "la resolución {id} se agotó: su rango llega hasta {hasta} y ya se emitió ese número. \
         Hace falta una resolución nueva antes de seguir facturando."
    )]
    Agotada { id: i64, hasta: i64 },
    #[error(
        "el último emitido ({ultimo}) está fuera del rango [{desde}..{hasta}] de la resolución {id}: \
         los datos no son consistentes y no se emite nada hasta arreglarlo"
    )]
    UltimoFueraDeRango {
        id: i64,
        ultimo: i64,
        desde: i64,
        hasta: i64,
    },
}

/// El número que sigue, o el motivo exacto por el que no hay siguiente.
///
/// No escribe nada: quien la llama tiene que persistir `ultimo_emitido` **en la
/// misma transacción** que la factura. Si se hiciera en dos pasos, dos cajeros
/// simultáneos sacarían el mismo número — que es exactamente el defecto que
/// dejó 15 números duplicados en el corpus.
pub fn siguiente(r: &Resolucion, hoy: NaiveDate) -> Result<i64, ErrorNumeracion> {
    if r.desde > r.hasta {
        return Err(ErrorNumeracion::RangoInvalido {
            desde: r.desde,
            hasta: r.hasta,
        });
    }
    if !r.activa {
        return Err(ErrorNumeracion::Inactiva(r.id));
    }
    if let Some(v) = r.vence
        && hoy > v
    {
        return Err(ErrorNumeracion::Vencida {
            id: r.id,
            vence: v,
            hoy,
        });
    }
    match r.ultimo_emitido {
        None => Ok(r.desde),
        Some(u) if u < r.desde || u > r.hasta => Err(ErrorNumeracion::UltimoFueraDeRango {
            id: r.id,
            ultimo: u,
            desde: r.desde,
            hasta: r.hasta,
        }),
        Some(u) if u == r.hasta => Err(ErrorNumeracion::Agotada {
            id: r.id,
            hasta: r.hasta,
        }),
        Some(u) => Ok(u + 1),
    }
}

/// Cuántos números quedan. Para avisar antes, no para autorizar.
pub fn quedan(r: &Resolucion) -> i64 {
    match r.ultimo_emitido {
        None => (r.hasta - r.desde + 1).max(0),
        Some(u) => (r.hasta - u).max(0),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn hoy() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, 2).unwrap()
    }

    fn res() -> Resolucion {
        Resolucion {
            id: 1,
            prefijo: String::new(),
            desde: 5601,
            hasta: 5650,
            ultimo_emitido: None,
            vence: None,
            activa: true,
        }
    }

    #[test]
    fn la_primera_factura_toma_el_inicio_del_rango() {
        assert_eq!(siguiente(&res(), hoy()), Ok(5601));
        assert_eq!(quedan(&res()), 50);
    }

    #[test]
    fn la_siguiente_es_la_siguiente() {
        let r = Resolucion { ultimo_emitido: Some(5601), ..res() };
        assert_eq!(siguiente(&r, hoy()), Ok(5602));
        assert_eq!(quedan(&r), 49);
    }

    /// El caso real del grupo `08` del corpus: 5650 de 5650.
    #[test]
    fn agotarse_es_un_error_con_nombre_no_un_numero_fuera_de_rango() {
        let r = Resolucion { ultimo_emitido: Some(5650), ..res() };
        assert_eq!(
            siguiente(&r, hoy()),
            Err(ErrorNumeracion::Agotada { id: 1, hasta: 5650 })
        );
        assert_eq!(quedan(&r), 0);
    }

    #[test]
    fn una_resolucion_vencida_no_emite() {
        let r = Resolucion {
            vence: Some(NaiveDate::from_ymd_opt(2026, 8, 1).unwrap()),
            ..res()
        };
        assert!(matches!(siguiente(&r, hoy()), Err(ErrorNumeracion::Vencida { .. })));
        // El último día sí emite: vence ese día, no la víspera.
        let r = Resolucion { vence: Some(hoy()), ..res() };
        assert_eq!(siguiente(&r, hoy()), Ok(5601));
    }

    #[test]
    fn un_ultimo_emitido_incoherente_para_la_emision_en_vez_de_seguir_contando() {
        let r = Resolucion { ultimo_emitido: Some(1), ..res() };
        assert!(matches!(
            siguiente(&r, hoy()),
            Err(ErrorNumeracion::UltimoFueraDeRango { .. })
        ));
        let r = Resolucion { ultimo_emitido: Some(9999), ..res() };
        assert!(matches!(
            siguiente(&r, hoy()),
            Err(ErrorNumeracion::UltimoFueraDeRango { .. })
        ));
    }

    #[test]
    fn recorrer_el_rango_entero_da_numeros_distintos_y_luego_se_agota() {
        let mut r = res();
        let mut vistos = std::collections::BTreeSet::new();
        while let Ok(n) = siguiente(&r, hoy()) {
            assert!(vistos.insert(n), "el número {n} salió dos veces");
            r.ultimo_emitido = Some(n);
        }
        assert_eq!(vistos.len(), 50);
        assert_eq!(*vistos.first().unwrap(), 5601);
        assert_eq!(*vistos.last().unwrap(), 5650);
        assert!(matches!(siguiente(&r, hoy()), Err(ErrorNumeracion::Agotada { .. })));
    }

    #[test]
    fn un_rango_al_reves_se_rechaza() {
        let r = Resolucion { desde: 100, hasta: 1, ..res() };
        assert_eq!(
            siguiente(&r, hoy()),
            Err(ErrorNumeracion::RangoInvalido { desde: 100, hasta: 1 })
        );
    }
}
