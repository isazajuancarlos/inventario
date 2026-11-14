//! Deterioro del inventario: medición al **menor entre costo y valor neto
//! realizable**.
//!
//! NIC 2 párrafos 9 y 28–33, y Sección 13.19–13.20 de NIIF para PYMES; ambas
//! incorporadas al derecho colombiano por el **Decreto 2420 de 2015**.
//!
//! > «Los inventarios se medirán al costo o al valor neto realizable, según cuál
//! > sea menor.»
//!
//! El **valor neto realizable (VNR)** es el precio estimado de venta **menos**
//! los costos estimados de terminación y los necesarios para llevar a cabo la
//! venta. No es el precio de lista, y no es el valor razonable.
//!
//! ## Las tres reglas que se rompen sin darse cuenta
//!
//! 1. **La rebaja se reconoce como gasto del periodo**, no se esconde en el
//!    costo. Un inventario que se lleva siempre al costo está sobrevaluado en
//!    cuanto el mercado se cae, y el balance dice algo que no es.
//! 2. **Se reevalúa en cada periodo posterior** (NIC 2.33). Un deterioro que se
//!    calcula una vez y se olvida es peor que no calcularlo: da la apariencia de
//!    que alguien lo está mirando.
//! 3. **La reversión está LIMITADA al costo original.** Si el VNR sube por
//!    encima del costo, el inventario vuelve al costo y ni un peso más: la NIC 2
//!    no admite revaluar inventarios por encima de lo que costaron.
//!
//! El módulo compara **por partida**, no sobre el total. Compensar el deterioro
//! de un artículo con la ganancia potencial de otro es exactamente lo que la
//! NIC 2.29 prohíbe: «los inventarios se rebajarán, habitualmente, partida por
//! partida».

use crate::dinero::{Cantidad, Centavos, multiplicar};

/// Lo que se sabe de una partida para medirla.
#[derive(Debug, Clone, Copy)]
pub struct Partida {
    pub articulo_id: i64,
    pub existencia: Cantidad,
    /// Valor en libros **al costo**, tal como sale del kardex.
    pub costo: Centavos,
    /// Precio de venta estimado, por unidad.
    pub precio_venta_estimado: Centavos,
    /// Costos por unidad necesarios para terminar y vender: comisión,
    /// transporte al cliente, reacondicionamiento.
    pub costos_para_vender: Centavos,
}

/// El resultado de medir una partida.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Medicion {
    pub articulo_id: i64,
    pub costo: Centavos,
    /// `(precio estimado − costos de venta) × existencia`. Nunca negativo: un
    /// VNR negativo significa que vender cuesta más de lo que se recibe, y en
    /// ese caso el inventario vale cero, no menos que cero.
    pub vnr: Centavos,
    /// `min(costo, vnr)`. Es el importe en libros que exige la NIC 2.9.
    pub valor_en_libros: Centavos,
    /// Cuánto hay que rebajar **ahora**. Positivo = gasto del periodo.
    pub deterioro: Centavos,
}

impl Medicion {
    pub fn hay_deterioro(&self) -> bool {
        self.deterioro.0 > 0
    }
}

/// El movimiento que hay que registrar respecto de lo ya reconocido.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Ajuste {
    pub articulo_id: i64,
    /// Deterioro que debería estar reconocido a esta fecha.
    pub deterioro_objetivo: Centavos,
    /// El que ya está reconocido de antes.
    pub deterioro_previo: Centavos,
    /// Lo que hay que registrar: positivo = **nueva rebaja** (gasto);
    /// negativo = **reversión** (menor gasto del periodo).
    pub movimiento: Centavos,
}

impl Ajuste {
    pub fn es_reversion(&self) -> bool {
        self.movimiento.0 < 0
    }
    pub fn es_nueva_rebaja(&self) -> bool {
        self.movimiento.0 > 0
    }
    pub fn sin_cambio(&self) -> bool {
        self.movimiento.0 == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErrorVnr {
    #[error("el costo en libros de la partida {0} es negativo: el kardex está descuadrado")]
    CostoNegativo(i64),
    #[error("el precio de venta estimado de la partida {0} no puede ser negativo")]
    PrecioNegativo(i64),
    #[error("los costos para vender de la partida {0} no pueden ser negativos")]
    CostosVentaNegativos(i64),
    /// El valor neto realizable de la partida no cabe en `Centavos`.
    #[error("el valor neto realizable de la partida {0} no cabe en centavos")]
    ImporteFueraDeRango(i64),
}

/// Mide una partida al menor entre costo y VNR.
pub fn medir(p: &Partida) -> Result<Medicion, ErrorVnr> {
    if p.costo.es_negativo() {
        return Err(ErrorVnr::CostoNegativo(p.articulo_id));
    }
    if p.precio_venta_estimado.es_negativo() {
        return Err(ErrorVnr::PrecioNegativo(p.articulo_id));
    }
    if p.costos_para_vender.es_negativo() {
        return Err(ErrorVnr::CostosVentaNegativos(p.articulo_id));
    }

    // Sin existencia no hay nada que medir: el VNR es cero y el costo también
    // debería serlo. No se fuerza: si el costo no es cero, el deterioro lo
    // llevará a cero y el descuadre saldrá a la luz en vez de esconderse.
    let neto_unitario = p.precio_venta_estimado - p.costos_para_vender;
    let vnr = if neto_unitario.0 <= 0 || p.existencia.0 <= 0 {
        Centavos::CERO
    } else {
        multiplicar(p.existencia, neto_unitario).ok_or(ErrorVnr::ImporteFueraDeRango(p.articulo_id))?
    };

    let valor_en_libros = if vnr < p.costo { vnr } else { p.costo };
    Ok(Medicion {
        articulo_id: p.articulo_id,
        costo: p.costo,
        vnr,
        valor_en_libros,
        deterioro: p.costo - valor_en_libros,
    })
}

/// Calcula el ajuste contra lo ya reconocido.
///
/// **La reversión queda limitada al costo original** por construcción: el
/// objetivo nunca pasa de `costo − valor_en_libros`, así que revertir a lo sumo
/// devuelve el deterioro previo y nunca sube el inventario por encima del costo.
pub fn ajustar(p: &Partida, deterioro_previo: Centavos) -> Result<Ajuste, ErrorVnr> {
    let m = medir(p)?;
    Ok(Ajuste {
        articulo_id: p.articulo_id,
        deterioro_objetivo: m.deterioro,
        deterioro_previo,
        movimiento: m.deterioro - deterioro_previo,
    })
}

/// Total a revelar (NIC 2.36): importe en libros, deterioro del periodo y
/// reversiones del periodo, **sin compensarse entre sí**.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct Revelacion {
    pub partidas: i64,
    pub costo_total: Centavos,
    pub valor_en_libros_total: Centavos,
    /// Suma de las rebajas nuevas del periodo. Se informa **aparte** de las
    /// reversiones porque la NIC 2.36 pide las dos cifras, no su diferencia.
    pub rebajas_del_periodo: Centavos,
    pub reversiones_del_periodo: Centavos,
}

pub fn revelar(ajustes: &[(Partida, Ajuste)]) -> Result<Revelacion, ErrorVnr> {
    let mut r = Revelacion::default();
    for (p, a) in ajustes {
        let m = medir(p)?;
        r.partidas += 1;
        r.costo_total = r.costo_total + m.costo;
        r.valor_en_libros_total = r.valor_en_libros_total + m.valor_en_libros;
        if a.movimiento.0 > 0 {
            r.rebajas_del_periodo = r.rebajas_del_periodo + a.movimiento;
        } else {
            r.reversiones_del_periodo = r.reversiones_del_periodo - a.movimiento;
        }
    }
    Ok(r)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn partida(costo: i64, existencia: i64, precio: i64, gastos: i64) -> Partida {
        Partida {
            articulo_id: 1,
            existencia: Cantidad::de_unidades(existencia),
            costo: Centavos::de_pesos(costo),
            precio_venta_estimado: Centavos::de_pesos(precio),
            costos_para_vender: Centavos::de_pesos(gastos),
        }
    }

    /// El caso normal: el inventario vale menos de lo que costó.
    #[test]
    fn cuando_el_vnr_es_menor_el_inventario_se_rebaja() {
        // 10 unidades que costaron $1.000 c/u; hoy se venden a $700 y vender
        // cuesta $50 por unidad → VNR = 10 × 650 = 6.500.
        let m = medir(&partida(10_000, 10, 700, 50)).unwrap();
        assert_eq!(m.costo, Centavos::de_pesos(10_000));
        assert_eq!(m.vnr, Centavos::de_pesos(6_500));
        assert_eq!(m.valor_en_libros, Centavos::de_pesos(6_500));
        assert_eq!(m.deterioro, Centavos::de_pesos(3_500));
        assert!(m.hay_deterioro());
    }

    /// Y el caso que la gente olvida: si el VNR es MAYOR, **no se revalúa**.
    #[test]
    fn cuando_el_vnr_es_mayor_el_inventario_se_queda_al_costo() {
        let m = medir(&partida(10_000, 10, 5_000, 100)).unwrap();
        assert_eq!(m.vnr, Centavos::de_pesos(49_000));
        assert_eq!(
            m.valor_en_libros,
            Centavos::de_pesos(10_000),
            "el inventario NO se revalúa por encima del costo"
        );
        assert_eq!(m.deterioro, Centavos::CERO);
        assert!(!m.hay_deterioro());
    }

    /// El VNR resta los costos de VENTA. Usar el precio de lista a secas
    /// sobrevalúa el inventario, que es el error corriente.
    #[test]
    fn el_vnr_no_es_el_precio_de_lista() {
        let con_gastos = medir(&partida(10_000, 10, 1_050, 100)).unwrap();
        let sin_gastos = medir(&partida(10_000, 10, 1_050, 0)).unwrap();
        assert_eq!(con_gastos.vnr, Centavos::de_pesos(9_500));
        assert_eq!(sin_gastos.vnr, Centavos::de_pesos(10_500));
        assert!(con_gastos.hay_deterioro(), "con los costos de venta SÍ hay deterioro");
        assert!(!sin_gastos.hay_deterioro(), "sin ellos parecería que no");
    }

    /// Si vender cuesta más de lo que se cobra, el inventario vale cero — no
    /// menos que cero. Un pasivo no se crea aquí.
    #[test]
    fn un_vnr_negativo_lleva_el_valor_a_cero_y_no_por_debajo() {
        let m = medir(&partida(10_000, 10, 100, 500)).unwrap();
        assert_eq!(m.vnr, Centavos::CERO);
        assert_eq!(m.valor_en_libros, Centavos::CERO);
        assert_eq!(m.deterioro, Centavos::de_pesos(10_000));
    }

    /// NIC 2.33: la reversión existe, y está LIMITADA al costo original.
    #[test]
    fn la_reversion_devuelve_al_costo_y_nunca_lo_supera() {
        // Periodo 1: el precio se cae, se reconoce deterioro de 3.500.
        let p1 = partida(10_000, 10, 700, 50);
        let a1 = ajustar(&p1, Centavos::CERO).unwrap();
        assert_eq!(a1.movimiento, Centavos::de_pesos(3_500));
        assert!(a1.es_nueva_rebaja());

        // Periodo 2: el precio se recupera parcialmente (a $900).
        let p2 = partida(10_000, 10, 900, 50);
        let a2 = ajustar(&p2, Centavos::de_pesos(3_500)).unwrap();
        assert_eq!(a2.deterioro_objetivo, Centavos::de_pesos(1_500));
        assert_eq!(a2.movimiento, Centavos::de_pesos(-2_000), "reversión parcial");
        assert!(a2.es_reversion());

        // Periodo 3: el precio se dispara. Se revierte TODO lo que quedaba, y
        // ni un peso más.
        let p3 = partida(10_000, 10, 9_000, 50);
        let a3 = ajustar(&p3, Centavos::de_pesos(1_500)).unwrap();
        assert_eq!(a3.deterioro_objetivo, Centavos::CERO);
        assert_eq!(
            a3.movimiento,
            Centavos::de_pesos(-1_500),
            "la reversión no puede superar lo reconocido: sería revaluar"
        );
        // Y el valor en libros vuelve al costo, exactamente.
        assert_eq!(medir(&p3).unwrap().valor_en_libros, Centavos::de_pesos(10_000));
    }

    /// Reevaluar sin que nada cambie no produce asiento. Un sistema que
    /// registrara un movimiento de cero en cada corte llenaría el libro de ruido.
    #[test]
    fn reevaluar_sin_cambios_no_mueve_nada() {
        let p = partida(10_000, 10, 700, 50);
        let a = ajustar(&p, Centavos::de_pesos(3_500)).unwrap();
        assert!(a.sin_cambio());
        assert_eq!(a.movimiento, Centavos::CERO);
    }

    /// NIC 2.29: partida por partida. Compensar el deterioro de una con la
    /// ganancia potencial de otra está prohibido, y aquí es imposible por
    /// construcción — pero la prueba lo fija para que nadie «optimice» sumando.
    #[test]
    fn el_deterioro_no_se_compensa_entre_partidas() {
        let mala = Partida { articulo_id: 1, ..partida(10_000, 10, 500, 0) };
        let buena = Partida { articulo_id: 2, ..partida(10_000, 10, 9_000, 0) };

        let m1 = medir(&mala).unwrap();
        let m2 = medir(&buena).unwrap();
        assert_eq!(m1.deterioro, Centavos::de_pesos(5_000));
        assert_eq!(m2.deterioro, Centavos::CERO);

        // Si se hubiera medido sobre el total, el VNR conjunto (5.000 + 90.000)
        // superaría el costo conjunto (20.000) y NO habría deterioro ninguno.
        let vnr_conjunto = m1.vnr + m2.vnr;
        let costo_conjunto = m1.costo + m2.costo;
        assert!(
            vnr_conjunto > costo_conjunto,
            "el caso dejó de discriminar: medir en conjunto tiene que ocultar el deterioro"
        );
        assert!(m1.deterioro.0 > 0, "medido partida por partida SÍ aparece");
    }

    #[test]
    fn la_revelacion_separa_rebajas_de_reversiones() {
        let p1 = partida(10_000, 10, 700, 50); // deterioro 3.500
        let a1 = ajustar(&p1, Centavos::CERO).unwrap();
        let p2 = Partida { articulo_id: 2, ..partida(10_000, 10, 900, 50) }; // objetivo 1.500
        let a2 = ajustar(&p2, Centavos::de_pesos(4_000)).unwrap(); // reversión 2.500

        let r = revelar(&[(p1, a1), (p2, a2)]).unwrap();
        assert_eq!(r.partidas, 2);
        assert_eq!(r.costo_total, Centavos::de_pesos(20_000));
        assert_eq!(r.valor_en_libros_total, Centavos::de_pesos(6_500 + 8_500));
        // Las dos cifras van SEPARADAS, no su diferencia.
        assert_eq!(r.rebajas_del_periodo, Centavos::de_pesos(3_500));
        assert_eq!(r.reversiones_del_periodo, Centavos::de_pesos(2_500));
        assert_ne!(
            r.rebajas_del_periodo, r.reversiones_del_periodo,
            "si se compensaran, la revelación diría 1.000 y perdería las dos cifras"
        );
    }

    #[test]
    fn sin_existencia_el_vnr_es_cero() {
        let m = medir(&partida(0, 0, 1_000, 0)).unwrap();
        assert_eq!(m.vnr, Centavos::CERO);
        assert_eq!(m.deterioro, Centavos::CERO);
        // Y si quedó costo sin existencia, el deterioro lo saca a la luz.
        let m = medir(&partida(500, 0, 1_000, 0)).unwrap();
        assert_eq!(m.deterioro, Centavos::de_pesos(500));
    }

    #[test]
    fn se_rechaza_lo_que_no_puede_ser() {
        let mut p = partida(10_000, 10, 700, 50);
        p.costo = Centavos(-1);
        assert_eq!(medir(&p), Err(ErrorVnr::CostoNegativo(1)));
        let mut p = partida(10_000, 10, 700, 50);
        p.precio_venta_estimado = Centavos(-1);
        assert_eq!(medir(&p), Err(ErrorVnr::PrecioNegativo(1)));
        let mut p = partida(10_000, 10, 700, 50);
        p.costos_para_vender = Centavos(-1);
        assert_eq!(medir(&p), Err(ErrorVnr::CostosVentaNegativos(1)));
    }
}
