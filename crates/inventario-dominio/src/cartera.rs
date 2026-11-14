//! Cartera: qué debe cada factura y cómo se le aplica un recibo.
//!
//! En el sistema viejo el saldo era `VALORFAC − Σ DETRECIBO.VALOR − Σ NOTAFACT.VALOR`,
//! con el detalle del recibo apuntando a la factura por `(nrofac, prefijo)`.
//! Funcionaba, pero la cabecera del recibo se podía borrar dejando el detalle
//! vivo: 313 de 323 abonos del corpus explican un dinero cuyo recibo ya no
//! existe. Aquí el detalle no existe sin cabecera (lo garantiza el esquema con
//! `ON DELETE CASCADE` y las claves foráneas encendidas), y este módulo se
//! ocupa solo de la aritmética.

use crate::dinero::Centavos;

/// Lo que se sabe de una factura para efectos de cobro.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FacturaPorCobrar {
    pub factura_id: i64,
    pub total: Centavos,
    /// Suma de abonos ya aplicados.
    pub abonado: Centavos,
    /// Suma de notas crédito ya aplicadas.
    pub notas: Centavos,
    pub anulada: bool,
}

impl FacturaPorCobrar {
    /// Lo que falta por cobrar. Una factura anulada no debe nada — pero se
    /// conserva, no se borra.
    pub fn saldo(&self) -> Centavos {
        if self.anulada {
            return Centavos::CERO;
        }
        self.total - self.abonado - self.notas
    }

    pub fn esta_paga(&self) -> bool {
        self.saldo().0 <= 0
    }
}

/// Cómo queda un recibo repartido entre facturas.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Reparto {
    pub aplicaciones: Vec<(i64, Centavos)>,
    /// Lo que sobró después de pagar todo lo pendiente. **No se aplica a
    /// ninguna factura**: queda como saldo a favor del cliente, y quien llama
    /// decide qué hacer con él. Repartirlo «por si acaso» sobre la factura más
    /// nueva es justo el relleno que la directiva 20 prohíbe.
    pub sobrante: Centavos,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErrorCartera {
    #[error("el valor de un recibo debe ser mayor que cero (llegó {0})")]
    ValorNoPositivo(i64),
    #[error("la factura {factura_id} debe {saldo} y se le quieren aplicar {aplicado}")]
    AplicacionMayorQueSaldo {
        factura_id: i64,
        saldo: i64,
        aplicado: i64,
    },
    #[error("la factura {0} está anulada: no admite abonos")]
    FacturaAnulada(i64),
    #[error("la factura {0} no está entre las pendientes del cliente")]
    FacturaAjena(i64),
    #[error("las aplicaciones suman {suma} y el recibo es de {recibo}")]
    NoCuadra { suma: i64, recibo: i64 },
}

/// Reparte un recibo sobre las facturas pendientes, **de la más vieja a la más
/// nueva**.
///
/// El orden lo fija quien llama pasando la lista ya ordenada: esta función no
/// ordena por fecha porque no la conoce, y adivinarla por el `id` sería suponer
/// que los identificadores crecen con el tiempo. Aquí se respeta el orden
/// recibido, que es una regla que el llamante puede ver.
pub fn repartir(pendientes: &[FacturaPorCobrar], recibo: Centavos) -> Result<Reparto, ErrorCartera> {
    if recibo.0 <= 0 {
        return Err(ErrorCartera::ValorNoPositivo(recibo.0));
    }
    let mut restante = recibo;
    let mut aplicaciones = Vec::new();
    for f in pendientes {
        if restante.0 <= 0 {
            break;
        }
        let saldo = f.saldo();
        if saldo.0 <= 0 {
            continue;
        }
        let aplicar = if saldo < restante { saldo } else { restante };
        aplicaciones.push((f.factura_id, aplicar));
        restante = restante - aplicar;
    }
    Ok(Reparto {
        aplicaciones,
        sobrante: restante,
    })
}

/// Comprueba un reparto **hecho a mano** antes de guardarlo.
///
/// Existe porque el reparto automático es una comodidad, no la regla: el cajero
/// tiene que poder decir «estos $50.000 van a la factura 3214». Lo que no puede
/// es aplicar más de lo que se debe, tocar una factura anulada, tocar una que
/// no es del cliente, o que la suma no dé el recibo.
pub fn validar_reparto(
    pendientes: &[FacturaPorCobrar],
    recibo: Centavos,
    aplicaciones: &[(i64, Centavos)],
) -> Result<(), ErrorCartera> {
    if recibo.0 <= 0 {
        return Err(ErrorCartera::ValorNoPositivo(recibo.0));
    }
    let mut suma = Centavos::CERO;
    for (id, valor) in aplicaciones {
        let f = pendientes
            .iter()
            .find(|f| f.factura_id == *id)
            .ok_or(ErrorCartera::FacturaAjena(*id))?;
        if f.anulada {
            return Err(ErrorCartera::FacturaAnulada(*id));
        }
        if valor.0 <= 0 {
            return Err(ErrorCartera::ValorNoPositivo(valor.0));
        }
        if *valor > f.saldo() {
            return Err(ErrorCartera::AplicacionMayorQueSaldo {
                factura_id: *id,
                saldo: f.saldo().0,
                aplicado: valor.0,
            });
        }
        suma = suma + *valor;
    }
    if suma != recibo {
        return Err(ErrorCartera::NoCuadra {
            suma: suma.0,
            recibo: recibo.0,
        });
    }
    Ok(())
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn f(id: i64, total: i64, abonado: i64) -> FacturaPorCobrar {
        FacturaPorCobrar {
            factura_id: id,
            total: Centavos::de_pesos(total),
            abonado: Centavos::de_pesos(abonado),
            notas: Centavos::CERO,
            anulada: false,
        }
    }

    #[test]
    fn el_saldo_descuenta_abonos_y_notas() {
        let mut x = f(1, 100_000, 30_000);
        assert_eq!(x.saldo(), Centavos::de_pesos(70_000));
        x.notas = Centavos::de_pesos(20_000);
        assert_eq!(x.saldo(), Centavos::de_pesos(50_000));
        assert!(!x.esta_paga());
        x.abonado = Centavos::de_pesos(80_000);
        assert!(x.esta_paga());
    }

    #[test]
    fn una_factura_anulada_no_debe_nada_pero_sigue_existiendo() {
        let x = FacturaPorCobrar { anulada: true, ..f(1, 100_000, 0) };
        assert_eq!(x.saldo(), Centavos::CERO);
        assert_eq!(x.total, Centavos::de_pesos(100_000), "el histórico se conserva");
    }

    #[test]
    fn el_recibo_paga_de_la_mas_vieja_a_la_mas_nueva() {
        let p = [f(1, 50_000, 0), f(2, 30_000, 0), f(3, 40_000, 0)];
        let r = repartir(&p, Centavos::de_pesos(70_000)).unwrap();
        assert_eq!(
            r.aplicaciones,
            vec![
                (1, Centavos::de_pesos(50_000)),
                (2, Centavos::de_pesos(20_000))
            ]
        );
        assert_eq!(r.sobrante, Centavos::CERO);
    }

    /// Lo que sobra NO se le encaja a la última factura.
    #[test]
    fn el_sobrante_se_devuelve_y_no_se_reparte_por_si_acaso() {
        let p = [f(1, 10_000, 0)];
        let r = repartir(&p, Centavos::de_pesos(25_000)).unwrap();
        assert_eq!(r.aplicaciones, vec![(1, Centavos::de_pesos(10_000))]);
        assert_eq!(r.sobrante, Centavos::de_pesos(15_000));
    }

    #[test]
    fn las_facturas_ya_pagas_y_las_anuladas_se_saltan() {
        let p = [
            f(1, 10_000, 10_000),
            FacturaPorCobrar { anulada: true, ..f(2, 99_000, 0) },
            f(3, 5_000, 0),
        ];
        let r = repartir(&p, Centavos::de_pesos(5_000)).unwrap();
        assert_eq!(r.aplicaciones, vec![(3, Centavos::de_pesos(5_000))]);
    }

    #[test]
    fn el_reparto_a_mano_no_puede_pagar_de_mas() {
        let p = [f(1, 10_000, 0)];
        assert!(matches!(
            validar_reparto(&p, Centavos::de_pesos(20_000), &[(1, Centavos::de_pesos(20_000))]),
            Err(ErrorCartera::AplicacionMayorQueSaldo { .. })
        ));
    }

    #[test]
    fn el_reparto_a_mano_tiene_que_cuadrar_con_el_recibo() {
        let p = [f(1, 10_000, 0), f(2, 10_000, 0)];
        assert!(matches!(
            validar_reparto(&p, Centavos::de_pesos(10_000), &[(1, Centavos::de_pesos(4_000))]),
            Err(ErrorCartera::NoCuadra { .. })
        ));
        assert!(validar_reparto(
            &p,
            Centavos::de_pesos(10_000),
            &[(1, Centavos::de_pesos(4_000)), (2, Centavos::de_pesos(6_000))]
        )
        .is_ok());
    }

    #[test]
    fn no_se_puede_abonar_a_una_factura_que_no_es_del_cliente_ni_a_una_anulada() {
        let p = [f(1, 10_000, 0), FacturaPorCobrar { anulada: true, ..f(2, 10_000, 0) }];
        assert_eq!(
            validar_reparto(&p, Centavos::de_pesos(1_000), &[(99, Centavos::de_pesos(1_000))]),
            Err(ErrorCartera::FacturaAjena(99))
        );
        assert_eq!(
            validar_reparto(&p, Centavos::de_pesos(1_000), &[(2, Centavos::de_pesos(1_000))]),
            Err(ErrorCartera::FacturaAnulada(2))
        );
    }

    /// 120 recibos sobre una cartera que se va pagando: el dinero aplicado más
    /// el sobrante tiene que ser siempre el recibo, y la cartera nunca negativa.
    #[test]
    fn ciento_veinte_recibos_conservan_el_dinero() {
        let mut cartera: Vec<FacturaPorCobrar> =
            (1..=40).map(|i| f(i, 10_000 + i * 137, 0)).collect();
        for k in 1..=120i64 {
            let valor = Centavos::de_pesos(k * 733 % 20_000 + 1);
            let r = repartir(&cartera, valor).unwrap();
            let aplicado: Centavos = r.aplicaciones.iter().map(|(_, v)| *v).sum();
            assert_eq!(aplicado + r.sobrante, valor, "se perdió dinero en el recibo {k}");
            validar_reparto(&cartera, aplicado, &r.aplicaciones)
                .or_else(|e| if aplicado.0 == 0 { Ok(()) } else { Err(e) })
                .expect("el reparto automático tiene que pasar su propia validación");
            for (id, v) in r.aplicaciones {
                let f = cartera.iter_mut().find(|f| f.factura_id == id).unwrap();
                f.abonado = f.abonado + v;
                assert!(f.saldo().0 >= 0, "la factura {id} quedó con saldo negativo");
            }
        }
        assert!(cartera.iter().all(|f| f.esta_paga()), "quedó cartera sin cobrar");
    }
}
