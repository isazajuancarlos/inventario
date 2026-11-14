//! Aritmética de la factura.
//!
//! La regla se **recuperó del corpus heredado** y se verificó contra sus 3.154
//! facturas reales (ver `ANALISIS.md` §3):
//!
//! ```text
//! total = Σ(cantidad × precio − descuento_línea) + IVA + INC − retención
//! ```
//!
//! El **impuesto nacional al consumo (INC)** va aparte del IVA, no sumado a él:
//! la Resolución DIAN 000165 de 2023 (art. 11) exige discriminarlos, tienen
//! bases y declaraciones distintas, y el CUFE los recibe como campos separados
//! (`01` para IVA, `04` para INC).
//!
//! Se cumplía en 3.065 de las 3.094 facturas con detalle. Las 29 restantes no
//! eran un fallo de la fórmula sino de la clave: eran los 15 números de factura
//! duplicados, cuyo detalle se le sumaba a dos facturas a la vez. Por eso aquí
//! la aritmética vive en el dominio y la unicidad del número vive en el
//! esquema: son dos problemas distintos y se arreglan en dos sitios distintos.
//!
//! El IVA se calcula **por línea** y se suma. Calcularlo sobre el total sería
//! más simple y estaría mal en cuanto una factura mezcle tarifas —el corpus
//! mezcla 0 % y 16 %—, y el error solo aparecería en las facturas mixtas.

use crate::dinero::{Cantidad, Centavos, multiplicar, porcentaje};

/// Una línea tal como la digita quien factura.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Linea {
    pub articulo_id: i64,
    pub cantidad: Cantidad,
    /// Precio unitario **sin IVA**.
    pub precio_unitario: Centavos,
    /// Descuento sobre la línea, en centavos y ya calculado. Cero si no hay.
    #[serde(default)]
    pub descuento: Centavos,
    /// Tarifa de IVA de la línea, entera (19 = 19 %).
    #[serde(default)]
    pub iva_porcentaje: i64,
    /// Tarifa del impuesto nacional al consumo, entera (8 = 8 %). Se calcula
    /// sobre la misma base que el IVA.
    #[serde(default)]
    pub inc_porcentaje: i64,
}

/// Una línea ya calculada: lo que se persiste y lo que se imprime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LineaCalculada {
    pub articulo_id: i64,
    pub cantidad: Cantidad,
    pub precio_unitario: Centavos,
    /// `cantidad × precio`, antes de descuento.
    pub bruto: Centavos,
    pub descuento: Centavos,
    /// `bruto − descuento`. Es la base del IVA.
    pub base: Centavos,
    pub iva_porcentaje: i64,
    pub iva: Centavos,
    pub inc_porcentaje: i64,
    pub inc: Centavos,
    /// `base + iva + inc`.
    pub total: Centavos,
}

/// La factura cuadrada.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Totales {
    pub lineas: Vec<LineaCalculada>,
    /// Suma de bases (bruto − descuento). Lo que la DIAN llama base gravable
    /// más la exenta.
    pub subtotal: Centavos,
    pub descuento: Centavos,
    pub iva: Centavos,
    pub inc: Centavos,
    pub retencion: Centavos,
    pub total: Centavos,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErrorFactura {
    #[error("una factura sin líneas no es una factura")]
    SinLineas,
    #[error("línea {indice}: la cantidad debe ser mayor que cero (llegó {cantidad})")]
    CantidadNoPositiva { indice: usize, cantidad: i64 },
    #[error("línea {indice}: el precio unitario no puede ser negativo (llegó {precio})")]
    PrecioNegativo { indice: usize, precio: i64 },
    #[error("línea {indice}: el descuento ({descuento}) supera el bruto de la línea ({bruto})")]
    DescuentoMayorQueLinea {
        indice: usize,
        descuento: i64,
        bruto: i64,
    },
    #[error("línea {indice}: la tarifa de {impuesto} {tarifa} % está fuera de [0, 100]")]
    TarifaImposible {
        indice: usize,
        tarifa: i64,
        impuesto: &'static str,
    },
    #[error("la retención ({retencion}) no puede superar el subtotal ({subtotal})")]
    RetencionMayorQueSubtotal { retencion: i64, subtotal: i64 },
    #[error("el artículo {0} aparece repetido; únalo en una sola línea")]
    ArticuloRepetido(i64),
    /// El importe no cabe en `Centavos` (`i64`).
    ///
    /// Es la variante que faltaba y por la que se colaba el peor desenlace
    /// posible: hasta el 2026-08-13, `multiplicar` terminaba en `as i64`, que
    /// **trunca en silencio**, así que una línea con cantidad y precio enormes
    /// se persistía con un total que no era cantidad × precio —medido:
    /// 1.553.255.926.290.448.384 donde el valor real era
    /// 20.000.000.000.000.000.000— y la factura salía «bien».
    ///
    /// Rechazar la factura entera es lo correcto: un importe que no cabe no
    /// tiene una respuesta razonable que dar, y saturar sería inventar un
    /// número que alguien cobraría.
    #[error(
        "línea {indice}: el importe de {que} no cabe en centavos          (cantidad {cantidad}, precio {precio}); revise las cifras"
    )]
    ImporteFueraDeRango {
        indice: usize,
        que: &'static str,
        cantidad: i64,
        precio: i64,
    },
}

/// Calcula la factura o dice exactamente por qué no puede.
///
/// `retencion` es la retefuente, que se descuenta del total y **no** de la base
/// del IVA. En el corpus el campo existía y estaba siempre en cero; se
/// implementa porque un cliente que sea agente retenedor la necesita el primer
/// día, y añadirla después obligaría a recalcular facturas ya emitidas.
pub fn calcular(lineas: &[Linea], retencion: Centavos) -> Result<Totales, ErrorFactura> {
    if lineas.is_empty() {
        return Err(ErrorFactura::SinLineas);
    }

    let mut vistos: Vec<i64> = Vec::with_capacity(lineas.len());
    let mut calculadas = Vec::with_capacity(lineas.len());

    for (indice, l) in lineas.iter().enumerate() {
        if !l.cantidad.es_positiva() {
            return Err(ErrorFactura::CantidadNoPositiva {
                indice,
                cantidad: l.cantidad.0,
            });
        }
        if l.precio_unitario.es_negativo() {
            return Err(ErrorFactura::PrecioNegativo {
                indice,
                precio: l.precio_unitario.0,
            });
        }
        // Se rechaza lo IMPOSIBLE, no lo inusual (directiva 21): una tarifa
        // fuera de [0, 100] no existe en ningún país. Que hoy Colombia use
        // 0/5/19 no convierte al 16 del corpus —ni a una tarifa futura— en un
        // error, así que no se lista un conjunto cerrado.
        if !(0..=100).contains(&l.iva_porcentaje) {
            return Err(ErrorFactura::TarifaImposible {
                indice,
                tarifa: l.iva_porcentaje,
                impuesto: "IVA",
            });
        }
        if !(0..=100).contains(&l.inc_porcentaje) {
            return Err(ErrorFactura::TarifaImposible {
                indice,
                tarifa: l.inc_porcentaje,
                impuesto: "INC",
            });
        }

        let bruto = multiplicar(l.cantidad, l.precio_unitario).ok_or(
            ErrorFactura::ImporteFueraDeRango {
                indice,
                que: "la línea",
                cantidad: l.cantidad.0,
                precio: l.precio_unitario.0,
            },
        )?;
        if l.descuento.es_negativo() || l.descuento > bruto {
            return Err(ErrorFactura::DescuentoMayorQueLinea {
                indice,
                descuento: l.descuento.0,
                bruto: bruto.0,
            });
        }
        if vistos.contains(&l.articulo_id) {
            return Err(ErrorFactura::ArticuloRepetido(l.articulo_id));
        }
        vistos.push(l.articulo_id);

        let base = bruto - l.descuento;
        let iva = porcentaje(base, l.iva_porcentaje).ok_or(ErrorFactura::ImporteFueraDeRango {
            indice,
            que: "el IVA",
            cantidad: base.0,
            precio: l.iva_porcentaje,
        })?;
        // El INC se calcula sobre la MISMA base que el IVA, no sobre
        // «base + IVA»: son dos impuestos sobre el consumo, no uno encima del
        // otro.
        let inc = porcentaje(base, l.inc_porcentaje).ok_or(ErrorFactura::ImporteFueraDeRango {
            indice,
            que: "el INC",
            cantidad: base.0,
            precio: l.inc_porcentaje,
        })?;
        calculadas.push(LineaCalculada {
            articulo_id: l.articulo_id,
            cantidad: l.cantidad,
            precio_unitario: l.precio_unitario,
            bruto,
            descuento: l.descuento,
            base,
            iva_porcentaje: l.iva_porcentaje,
            iva,
            inc_porcentaje: l.inc_porcentaje,
            inc,
            total: base + iva + inc,
        });
    }

    let subtotal: Centavos = calculadas.iter().map(|l| l.base).sum();
    let descuento: Centavos = calculadas.iter().map(|l| l.descuento).sum();
    let iva: Centavos = calculadas.iter().map(|l| l.iva).sum();
    let inc: Centavos = calculadas.iter().map(|l| l.inc).sum();

    if retencion.es_negativo() || retencion > subtotal {
        return Err(ErrorFactura::RetencionMayorQueSubtotal {
            retencion: retencion.0,
            subtotal: subtotal.0,
        });
    }

    Ok(Totales {
        lineas: calculadas,
        subtotal,
        descuento,
        iva,
        inc,
        retencion,
        total: subtotal + iva + inc - retencion,
    })
}

/// Los medios de pago que el sistema conoce, con la etiqueta que ve quien opera.
///
/// **Es UNA lista y vive aquí porque es regla, no adorno**: el art. 11 de la
/// Resolución DIAN 000165 de 2023 obliga a declarar el medio de pago en la
/// factura de contado, y el mismo vocabulario lo comparten la factura, el recibo
/// de caja y el `CHECK` de `recibo_pago` en el esquema.
///
/// Hasta el 2026-08-12 estaba escrita **tres veces**: dos listas idénticas en
/// `inventario-datos` y una tercera a mano en la plantilla del panel — que
/// ofrecía «tarjeta», valor que el dominio RECHAZA, así que registrar un pago con
/// tarjeta desde el panel era imposible. Fallaba ruidosamente, que está bien; lo
/// que no está bien es que la pantalla ofrezca lo que la regla no admite. Ahora
/// la pantalla se dibuja DESDE esta lista y no puede volver a divergir.
pub const MEDIOS_DE_PAGO: [(&str, &str); 5] = [
    ("efectivo", "Efectivo"),
    ("cheque", "Cheque"),
    ("tarjeta_credito", "Tarjeta de crédito"),
    ("tarjeta_debito", "Tarjeta débito"),
    ("transferencia", "Transferencia"),
];

/// Si el valor es uno de los medios que el dominio admite.
pub fn medio_de_pago_conocido(medio: &str) -> bool {
    MEDIOS_DE_PAGO.iter().any(|(v, _)| *v == medio)
}

/// Los valores admitidos, para el mensaje de error de quien rechaza.
pub fn medios_de_pago_admitidos() -> String {
    MEDIOS_DE_PAGO
        .iter()
        .map(|(v, _)| *v)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn linea(art: i64, u: i64, pesos: i64, iva: i64) -> Linea {
        Linea {
            articulo_id: art,
            cantidad: Cantidad::de_unidades(u),
            precio_unitario: Centavos::de_pesos(pesos),
            descuento: Centavos::CERO,
            iva_porcentaje: iva,
            inc_porcentaje: 0,
        }
    }

    /// **Una factura cuyo importe no cabe se RECHAZA; antes se emitía.**
    ///
    /// Es el desenlace que este cambio persigue: hasta el 2026-08-13,
    /// `multiplicar` truncaba en silencio y `calcular` devolvía `Ok` con un
    /// total que no era cantidad × precio. La factura salía, se numeraba contra
    /// la resolución de la DIAN y se persistía.
    #[test]
    fn una_factura_cuyo_importe_no_cabe_se_rechaza_en_vez_de_emitirse() {
        let imposible = linea(1, 20_000_000_000, 10_000_000, 19);
        let e = calcular(&[imposible], Centavos::CERO).unwrap_err();
        assert!(
            matches!(e, ErrorFactura::ImporteFueraDeRango { indice: 0, que: "la línea", .. }),
            "se esperaba ImporteFueraDeRango y llegó {e:?}"
        );

        // LA PAREJA: la factura del corpus, con cifras de verdad, sigue saliendo.
        // Sin esto, un `calcular` que rechazara todo pasaría esta prueba igual.
        let t = calcular(&[linea(1, 3, 1000, 19)], Centavos::CERO).unwrap();
        assert_eq!(t.total, Centavos::de_pesos(3570));
    }

    /// **El `?` del IVA es DEFENSA EN PROFUNDIDAD, y conviene que esté escrito.**
    ///
    /// Escribí primero una prueba que esperaba que el IVA desbordara con una base
    /// cercana al techo, y salió roja: no puede. `porcentaje` multiplica en `i128`
    /// y divide entre 100, así que con una tarifa de `[0, 100]` —que `calcular`
    /// valida ANTES— el resultado es siempre **menor o igual** que la base, que ya
    /// cabía. La rama de error es inalcanzable mientras esa validación siga ahí.
    ///
    /// Se conserva el `ok_or` igualmente, porque las dos cosas viven en sitios
    /// distintos: el día que alguien admita una tarifa mayor que 100 —una
    /// sobretasa, un recargo— el desbordamiento aparecería sin que nadie
    /// recordara esta relación. Y se conserva ESTA PRUEBA para que el motivo no
    /// se pierda: es lo único que impide que el siguiente lector borre el `?`
    /// «porque nunca falla».
    #[test]
    fn con_una_tarifa_valida_el_iva_nunca_desborda() {
        // El caso extremo posible: la base más grande que cabe y la tarifa mayor
        // que la validación admite.
        assert_eq!(
            porcentaje(Centavos(i64::MAX), 100),
            Some(Centavos(i64::MAX)),
            "el 100 % de la base máxima es la base máxima"
        );
        // Y una tarifa por ENCIMA de lo que `calcular` admite sí desborda, que es
        // lo que hace útil al `ok_or` si algún día se relaja la validación.
        assert_eq!(porcentaje(Centavos(i64::MAX), 101), None);
    }

    #[test]
    fn la_suma_es_la_del_corpus() {
        let t = calcular(&[linea(1, 3, 1000, 19), linea(2, 2, 500, 19)], Centavos::CERO).unwrap();
        assert_eq!(t.subtotal, Centavos::de_pesos(4000));
        assert_eq!(t.iva, Centavos::de_pesos(760));
        assert_eq!(t.total, Centavos::de_pesos(4760));
    }

    /// El caso que rompe calcular el IVA sobre el total: una factura que mezcla
    /// tarifas. El corpus mezcla 0 % y 16 % en 148 artículos.
    #[test]
    fn el_iva_se_calcula_por_linea_no_sobre_el_total() {
        let t = calcular(&[linea(1, 1, 1000, 19), linea(2, 1, 1000, 0)], Centavos::CERO).unwrap();
        assert_eq!(t.subtotal, Centavos::de_pesos(2000));
        assert_eq!(t.iva, Centavos::de_pesos(190), "solo la línea gravada paga IVA");
        // Si se hubiera calculado sobre el total con la tarifa de la primera
        // línea, saldrían $380. Esta prueba es la que lo impide.
        assert_ne!(t.iva, Centavos::de_pesos(380));
        assert_eq!(t.total, Centavos::de_pesos(2190));
    }

    #[test]
    fn el_descuento_baja_la_base_del_iva() {
        let l = Linea {
            descuento: Centavos::de_pesos(1000),
            ..linea(1, 10, 1000, 19)
        };
        let t = calcular(&[l], Centavos::CERO).unwrap();
        assert_eq!(t.subtotal, Centavos::de_pesos(9000));
        assert_eq!(t.iva, Centavos::de_pesos(1710)); // 19 % de 9.000, no de 10.000
        assert_eq!(t.total, Centavos::de_pesos(10710));
    }

    #[test]
    fn la_retencion_baja_el_total_pero_no_la_base_del_iva() {
        let t = calcular(&[linea(1, 1, 100_000, 19)], Centavos::de_pesos(2500)).unwrap();
        assert_eq!(t.iva, Centavos::de_pesos(19_000));
        assert_eq!(t.total, Centavos::de_pesos(116_500));
    }

    #[test]
    fn el_total_es_siempre_la_suma_de_las_lineas_mas_iva_menos_retencion() {
        // 200 facturas de forma variada: la identidad tiene que aguantar todas.
        for n in 1..=200i64 {
            let lineas: Vec<Linea> = (0..(n % 7 + 1))
                .map(|i| Linea {
                    articulo_id: i,
                    cantidad: crate::dinero::Cantidad(n * 13 % 9997 + 1),
                    precio_unitario: Centavos(n * 977 % 100_003 + 1),
                    descuento: Centavos::CERO,
                    iva_porcentaje: [0, 5, 16, 19][(n as usize + i as usize) % 4],
                    inc_porcentaje: [0, 0, 8, 16][(n as usize + i as usize) % 4],
                })
                .collect();
            let t = calcular(&lineas, Centavos::CERO).unwrap();
            let suma: Centavos = t.lineas.iter().map(|l| l.total).sum();
            assert_eq!(t.total, suma, "descuadre en la factura sintética {n}");
            assert_eq!(t.subtotal + t.iva + t.inc, suma);
        }
    }

    /// El INC va SEPARADO del IVA y sobre la MISMA base — no sobre base + IVA.
    /// Calcularlo en cascada infla el impuesto y descuadra las dos
    /// declaraciones, la de IVA y la de INC.
    #[test]
    fn el_inc_es_un_impuesto_aparte_sobre_la_misma_base() {
        let l = Linea { inc_porcentaje: 8, ..linea(1, 1, 100_000, 19) };
        let t = calcular(&[l], Centavos::CERO).unwrap();
        assert_eq!(t.subtotal, Centavos::de_pesos(100_000));
        assert_eq!(t.iva, Centavos::de_pesos(19_000));
        assert_eq!(t.inc, Centavos::de_pesos(8_000), "8 % de 100.000");
        assert_eq!(t.total, Centavos::de_pesos(127_000));
        // En cascada (8 % de 119.000) daría 9.520: eso está mal.
        assert_ne!(t.inc, Centavos::de_pesos(9_520));
        // Y las dos cifras viajan por separado, que es lo que exige la DIAN.
        assert_ne!(t.iva, t.inc);
    }

    #[test]
    fn una_tarifa_de_inc_imposible_se_rechaza_diciendo_cual_impuesto_es() {
        let l = Linea { inc_porcentaje: 101, ..linea(1, 1, 100, 19) };
        match calcular(&[l], Centavos::CERO) {
            Err(ErrorFactura::TarifaImposible { impuesto, .. }) => {
                assert_eq!(impuesto, "INC", "el error tiene que decir cuál de los dos");
            }
            otro => panic!("salió {otro:?}"),
        }
    }

    #[test]
    fn se_rechaza_lo_que_no_puede_ser() {
        assert_eq!(calcular(&[], Centavos::CERO), Err(ErrorFactura::SinLineas));
        assert_eq!(
            calcular(&[linea(1, 0, 100, 19)], Centavos::CERO),
            Err(ErrorFactura::CantidadNoPositiva { indice: 0, cantidad: 0 })
        );
        assert_eq!(
            calcular(&[linea(1, 1, 100, 101)], Centavos::CERO),
            Err(ErrorFactura::TarifaImposible { indice: 0, tarifa: 101, impuesto: "IVA" })
        );
        assert_eq!(
            calcular(&[linea(1, 1, 100, -1)], Centavos::CERO),
            Err(ErrorFactura::TarifaImposible { indice: 0, tarifa: -1, impuesto: "IVA" })
        );
        // Lo INUSUAL sí pasa: el 16 % del corpus de 2004 no es un error.
        assert!(calcular(&[linea(1, 1, 100, 16)], Centavos::CERO).is_ok());
        // Y un precio de cero tampoco: los obsequios existen (concepto 76).
        assert!(calcular(&[linea(1, 1, 0, 0)], Centavos::CERO).is_ok());
    }

    #[test]
    fn un_descuento_mayor_que_la_linea_no_produce_una_linea_negativa() {
        let l = Linea {
            descuento: Centavos::de_pesos(2000),
            ..linea(1, 1, 1000, 19)
        };
        assert!(matches!(
            calcular(&[l], Centavos::CERO),
            Err(ErrorFactura::DescuentoMayorQueLinea { .. })
        ));
    }

    /// Dos líneas del mismo artículo descuadrarían el kardex contra el detalle
    /// y harían ambiguo el costeo. Se unen antes de facturar.
    #[test]
    fn el_mismo_articulo_dos_veces_se_rechaza() {
        assert_eq!(
            calcular(&[linea(7, 1, 100, 19), linea(7, 2, 100, 19)], Centavos::CERO),
            Err(ErrorFactura::ArticuloRepetido(7))
        );
    }
}
