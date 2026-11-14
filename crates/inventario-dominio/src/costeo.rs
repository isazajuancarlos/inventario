//! Fórmulas de cálculo del costo y costo de adquisición.
//!
//! **NIC 2.25** (y Sección 13.18 de NIIF para PYMES, incorporadas por el
//! Decreto 2420 de 2015) permite exactamente dos fórmulas:
//!
//! - **primera entrada, primera salida (PEPS/FIFO)**, y
//! - **costo promedio ponderado**.
//!
//! **UEPS/LIFO está prohibido.** No es que «no esté implementado»: se rechaza
//! con un error que lo dice y cita la norma. Una prohibición silenciosa no es
//! una prohibición — quien la busque y no la encuentre supondrá que se puede.
//! (En US GAAP, ASC 330, sí se permite; por eso el error explica *dónde* está
//! prohibido y no solo *que* lo está.)
//!
//! La entidad debe usar **la misma fórmula** para inventarios de naturaleza y
//! uso similar, y revelarla. Por eso la fórmula es un parámetro de la empresa y
//! no una opción por movimiento.
//!
//! ## Costo de adquisición — NIC 2.11
//!
//! > «El costo de adquisición comprenderá el precio de compra, los aranceles de
//! > importación y otros impuestos (que no sean recuperables posteriormente de
//! > las autoridades fiscales) y transporte, manejo y otros costos directamente
//! > atribuibles […]. Los descuentos comerciales, las rebajas y otras partidas
//! > similares se deducirán.»
//!
//! Es decir: **el costo no es el precio de la factura del proveedor**. Si el
//! flete no se reparte, el inventario queda subvaluado y con él el costo de
//! ventas — y el error se arrastra a todos los periodos siguientes.

use crate::dinero::{Cantidad, Centavos, MIL, dividir_redondeando, multiplicar};

/// Fórmula de cálculo del costo. Es política de la entidad, no opción del
/// digitador.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Formula {
    PromedioPonderado,
    Peps,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErrorFormula {
    #[error(
        "«{0}» no es una fórmula de costeo válida: la NIC 2 párrafo 25 admite \
         «promedio_ponderado» y «peps»"
    )]
    Desconocida(String),
    #[error(
        "UEPS/LIFO está PROHIBIDO por la NIC 2 párrafo 25, incorporada al derecho \
         colombiano por el Decreto 2420 de 2015. Solo se admiten PEPS y promedio \
         ponderado. (Lo permite US GAAP ASC 330, no las NIIF.)"
    )]
    UepsProhibido,
}

impl Formula {
    pub fn como_texto(self) -> &'static str {
        match self {
            Formula::PromedioPonderado => "promedio_ponderado",
            Formula::Peps => "peps",
        }
    }

    /// Lee la fórmula de un texto de configuración.
    ///
    /// UEPS tiene su **propio** error, distinto de «desconocida». Da igual cómo
    /// lo escriban: quien intente configurarlo tiene que leer por qué no puede.
    pub fn desde_texto(s: &str) -> Result<Formula, ErrorFormula> {
        let t = s.trim().to_lowercase().replace([' ', '-'], "_");
        match t.as_str() {
            "promedio_ponderado" | "promedio" | "ppp" => Ok(Formula::PromedioPonderado),
            "peps" | "fifo" | "primeras_en_entrar_primeras_en_salir" => Ok(Formula::Peps),
            "ueps" | "lifo" | "ultimas_en_entrar_primeras_en_salir" => {
                Err(ErrorFormula::UepsProhibido)
            }
            _ => Err(ErrorFormula::Desconocida(s.to_string())),
        }
    }
}

// ---------------------------------------------------------------------------
// PEPS: capas de costo
// ---------------------------------------------------------------------------

/// Una capa de costo: lo que queda sin consumir de una entrada concreta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Capa {
    /// El movimiento de entrada que la creó. Es lo que permite auditar de qué
    /// compra salió cada peso del costo de ventas.
    pub movimiento_id: i64,
    pub cantidad: Cantidad,
    pub costo_unitario: Centavos,
}

impl Capa {
    /// Valor de la capa, o `None` si no cabe en `Centavos`.
    ///
    /// Devolvía `Centavos` a secas y el desbordamiento se perdía dentro de
    /// `multiplicar`: una capa imposible daba un valor plausible y el PEPS
    /// seguía calculando sobre él.
    pub fn valor(&self) -> Option<Centavos> {
        multiplicar(self.cantidad, self.costo_unitario)
    }

    fn error_de_rango(&self) -> ErrorPeps {
        ErrorPeps::ImporteFueraDeRango {
            movimiento_id: self.movimiento_id,
            cantidad: self.cantidad.0,
            costo_unitario: self.costo_unitario.0,
        }
    }
}

/// Lo que se consume de una capa al costear una salida.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Consumo {
    pub movimiento_id: i64,
    pub cantidad: Cantidad,
    pub costo: Centavos,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErrorPeps {
    #[error("las capas suman {hay} y se piden {piden}")]
    CapasInsuficientes { hay: Cantidad, piden: Cantidad },
    #[error("la cantidad a consumir debe ser positiva (llegó {0})")]
    CantidadNoPositiva(i64),
    /// El valor de una capa no cabe en `Centavos`. Antes del 2026-08-13 este
    /// caso truncaba en silencio y el costo de ventas salía con un número que
    /// no era cantidad × costo unitario.
    #[error("el valor de la capa {movimiento_id} no cabe en centavos (cantidad {cantidad}, costo unitario {costo_unitario})")]
    ImporteFueraDeRango {
        movimiento_id: i64,
        cantidad: i64,
        costo_unitario: i64,
    },
}

/// Consume `cantidad` de las capas **en el orden en que se reciben**, que tiene
/// que ser el de entrada (la más vieja primero). Devuelve qué se tomó de cada
/// una y el costo total.
///
/// El orden lo garantiza quien llama, no esta función: ordenar aquí por
/// `movimiento_id` supondría que los identificadores crecen con el tiempo, y
/// eso deja de ser cierto en cuanto se migra un histórico.
///
/// El **último tramo se cuadra por diferencia** para que no quede residuo: si
/// la salida agota una capa, se lleva exactamente el valor de esa capa. Es la
/// misma razón que en el promedio ponderado — multiplicar por un unitario
/// redondeado deja centavos colgando de una capa que ya no tiene unidades.
pub fn consumir(capas: &[Capa], cantidad: Cantidad) -> Result<(Vec<Consumo>, Centavos), ErrorPeps> {
    if !cantidad.es_positiva() {
        return Err(ErrorPeps::CantidadNoPositiva(cantidad.0));
    }
    let disponible: Cantidad = capas.iter().map(|c| c.cantidad).sum();
    if cantidad > disponible {
        return Err(ErrorPeps::CapasInsuficientes {
            hay: disponible,
            piden: cantidad,
        });
    }

    let mut falta = cantidad;
    let mut consumos = Vec::new();
    let mut total = Centavos::CERO;
    for capa in capas {
        if falta.0 == 0 {
            break;
        }
        if capa.cantidad.0 <= 0 {
            continue;
        }
        let toma = if capa.cantidad <= falta { capa.cantidad } else { falta };
        // Si se agota la capa, se lleva su valor exacto; si no, la parte
        // proporcional. Las dos vías coinciden cuando la capa se agota, y así
        // el valor residual de la capa nunca queda descuadrado del unitario.
        let costo = if toma == capa.cantidad {
            capa.valor().ok_or_else(|| capa.error_de_rango())?
        } else {
            multiplicar(toma, capa.costo_unitario).ok_or_else(|| capa.error_de_rango())?
        };
        consumos.push(Consumo {
            movimiento_id: capa.movimiento_id,
            cantidad: toma,
            costo,
        });
        total = total + costo;
        falta = falta - toma;
    }
    Ok((consumos, total))
}

// ---------------------------------------------------------------------------
// Costo de adquisición — NIC 2.11
// ---------------------------------------------------------------------------

/// Una línea de la compra, tal como viene en la factura del proveedor.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
pub struct LineaCompra {
    pub articulo_id: i64,
    pub cantidad: Cantidad,
    /// Precio unitario de la factura del proveedor, **antes** de repartir
    /// costos indirectos y **antes** del descuento.
    pub precio_unitario: Centavos,
    /// Descuento comercial o rebaja de esta línea. NIC 2.11 manda restarlo.
    #[serde(default)]
    pub descuento: Centavos,
}

/// Los costos que hay que **repartir** entre las líneas.
#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
pub struct CostosIndirectos {
    pub flete: Centavos,
    /// Aranceles de importación.
    pub arancel: Centavos,
    pub seguro: Centavos,
    /// Manejo, descargue, y demás directamente atribuibles.
    pub otros: Centavos,
}

impl CostosIndirectos {
    pub fn total(&self) -> Centavos {
        self.flete + self.arancel + self.seguro + self.otros
    }
}

/// El resultado: cuánto cuesta de verdad cada línea, y a cuánto sale la unidad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct LineaCosteada {
    pub articulo_id: i64,
    pub cantidad: Cantidad,
    /// `cantidad × precio − descuento`.
    pub costo_directo: Centavos,
    /// La parte de flete, arancel, seguro y otros que le tocó.
    pub costo_indirecto: Centavos,
    /// `costo_directo + costo_indirecto`. Es lo que entra al kardex.
    pub costo_total: Centavos,
    /// Derivado: `costo_total / cantidad`.
    pub costo_unitario: Centavos,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErrorCompra {
    #[error("una compra sin líneas no es una compra")]
    SinLineas,
    #[error("línea {indice}: la cantidad debe ser positiva (llegó {cantidad})")]
    CantidadNoPositiva { indice: usize, cantidad: i64 },
    #[error("línea {indice}: el precio unitario no puede ser negativo")]
    PrecioNegativo { indice: usize },
    #[error("línea {indice}: el descuento ({descuento}) supera el bruto de la línea ({bruto})")]
    DescuentoMayorQueLinea {
        indice: usize,
        descuento: i64,
        bruto: i64,
    },
    /// Un importe de la compra no cabe en `Centavos`. Ver la variante gemela de
    /// `ErrorFactura`: mismo defecto, misma decisión —rechazar, no saturar—.
    #[error("línea {indice}: el importe de {que} no cabe en centavos; revise las cifras")]
    ImporteFueraDeRango { indice: usize, que: &'static str },
    #[error("los costos indirectos no pueden ser negativos")]
    IndirectoNegativo,
    #[error(
        "todas las líneas de la compra suman cero, así que no hay sobre qué repartir \
         los {0} de costos indirectos. Reparta a mano o corrija los precios."
    )]
    BaseCero(String),
}

/// Reparte los costos indirectos **en proporción al valor** de cada línea y
/// devuelve el costo de adquisición de cada una.
///
/// El reparto es por valor y no por cantidad ni por peso porque es lo que hace
/// la práctica contable corriente y lo que un revisor espera ver; repartir por
/// cantidad le carga el mismo flete a un botellón que a una tapa.
///
/// El **último renglón absorbe la diferencia de redondeo**, para que la suma de
/// lo repartido sea exactamente el indirecto y no un centavo menos. Sin eso,
/// unos centavos se quedan fuera del inventario en cada compra.
pub fn costo_adquisicion(
    lineas: &[LineaCompra],
    indirectos: CostosIndirectos,
) -> Result<Vec<LineaCosteada>, ErrorCompra> {
    if lineas.is_empty() {
        return Err(ErrorCompra::SinLineas);
    }
    for c in [
        indirectos.flete,
        indirectos.arancel,
        indirectos.seguro,
        indirectos.otros,
    ] {
        if c.es_negativo() {
            return Err(ErrorCompra::IndirectoNegativo);
        }
    }

    let mut directos = Vec::with_capacity(lineas.len());
    for (indice, l) in lineas.iter().enumerate() {
        if !l.cantidad.es_positiva() {
            return Err(ErrorCompra::CantidadNoPositiva {
                indice,
                cantidad: l.cantidad.0,
            });
        }
        if l.precio_unitario.es_negativo() {
            return Err(ErrorCompra::PrecioNegativo { indice });
        }
        let bruto = multiplicar(l.cantidad, l.precio_unitario).ok_or(
            ErrorCompra::ImporteFueraDeRango { indice, que: "la línea" },
        )?;
        if l.descuento.es_negativo() || l.descuento > bruto {
            return Err(ErrorCompra::DescuentoMayorQueLinea {
                indice,
                descuento: l.descuento.0,
                bruto: bruto.0,
            });
        }
        directos.push(bruto - l.descuento);
    }

    let base: Centavos = directos.iter().copied().sum();
    let total_indirecto = indirectos.total();
    if base.0 == 0 && total_indirecto.0 != 0 {
        return Err(ErrorCompra::BaseCero(total_indirecto.to_string()));
    }

    let mut repartido = Centavos::CERO;
    let mut salida = Vec::with_capacity(lineas.len());
    for (i, (l, directo)) in lineas.iter().zip(&directos).enumerate() {
        let indirecto = if total_indirecto.0 == 0 {
            Centavos::CERO
        } else if i == lineas.len() - 1 {
            // El último cuadra por diferencia: lo repartido tiene que ser
            // EXACTAMENTE el indirecto.
            total_indirecto - repartido
        } else {
            Centavos(
                dividir_redondeando(
                    total_indirecto.0 as i128 * directo.0 as i128,
                    base.0 as i128,
                )
                .ok_or(ErrorCompra::ImporteFueraDeRango {
                    indice: i,
                    que: "el reparto del costo indirecto",
                })?,
            )
        };
        repartido = repartido + indirecto;
        let costo_total = *directo + indirecto;
        salida.push(LineaCosteada {
            articulo_id: l.articulo_id,
            cantidad: l.cantidad,
            costo_directo: *directo,
            costo_indirecto: indirecto,
            costo_total,
            costo_unitario: Centavos(
                dividir_redondeando(costo_total.0 as i128 * MIL as i128, l.cantidad.0 as i128)
                    .ok_or(ErrorCompra::ImporteFueraDeRango {
                        indice: i,
                        que: "el costo unitario",
                    })?,
            ),
        });
    }
    Ok(salida)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    // -- Fórmula ------------------------------------------------------------

    /// La prohibición tiene que ser AUDIBLE, no un silencio.
    #[test]
    fn ueps_se_rechaza_con_un_error_que_cita_la_norma() {
        for escrito in ["UEPS", "ueps", "LIFO", "lifo", " Ueps ", "ultimas_en_entrar_primeras_en_salir"] {
            let e = Formula::desde_texto(escrito).unwrap_err();
            assert_eq!(e, ErrorFormula::UepsProhibido, "no se rechazó «{escrito}»");
            let msg = e.to_string();
            assert!(msg.contains("NIC 2"), "el error no cita la norma: {msg}");
            assert!(msg.contains("2420"), "el error no cita el decreto: {msg}");
        }
        // Y es un error DISTINTO de «no la conozco»: quien escriba una errata
        // no debe recibir la lección sobre UEPS, y viceversa.
        assert!(matches!(
            Formula::desde_texto("promedio_pnderado"),
            Err(ErrorFormula::Desconocida(_))
        ));
    }

    #[test]
    fn las_dos_formulas_que_admite_la_nic_2_se_leen_como_las_escribe_la_gente() {
        for (t, f) in [
            ("peps", Formula::Peps),
            ("PEPS", Formula::Peps),
            ("fifo", Formula::Peps),
            ("promedio", Formula::PromedioPonderado),
            ("promedio ponderado", Formula::PromedioPonderado),
            ("promedio-ponderado", Formula::PromedioPonderado),
            ("ppp", Formula::PromedioPonderado),
        ] {
            assert_eq!(Formula::desde_texto(t), Ok(f), "«{t}»");
        }
        assert_eq!(Formula::Peps.como_texto(), "peps");
    }

    // -- PEPS ---------------------------------------------------------------

    fn capa(id: i64, u: i64, pesos: i64) -> Capa {
        Capa {
            movimiento_id: id,
            cantidad: Cantidad::de_unidades(u),
            costo_unitario: Centavos::de_pesos(pesos),
        }
    }

    /// El caso de libro: 10 a $100 y 10 a $200. PEPS saca las primeras.
    #[test]
    fn peps_consume_la_capa_mas_vieja_primero() {
        let capas = [capa(1, 10, 100), capa(2, 10, 200)];
        let (c, total) = consumir(&capas, Cantidad::de_unidades(5)).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].movimiento_id, 1);
        assert_eq!(total, Centavos::de_pesos(500), "5 × $100, no 5 × $150");

        // Y una salida que cruza la frontera toma de las dos.
        let (c, total) = consumir(&capas, Cantidad::de_unidades(15)).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].cantidad, Cantidad::de_unidades(10));
        assert_eq!(c[1].cantidad, Cantidad::de_unidades(5));
        assert_eq!(total, Centavos::de_pesos(2000)); // 1000 + 1000
    }

    /// PEPS y promedio dan **costos distintos**, y esa es toda la razón de que
    /// la NIC 2 obligue a declarar cuál se usa. Si esta prueba dejara de
    /// distinguirlos, es que una de las dos está mal implementada.
    #[test]
    fn peps_y_promedio_no_dan_lo_mismo() {
        use crate::kardex::{self, Movimiento, Saldo, Signo};
        let capas = [capa(1, 10, 100), capa(2, 10, 200)];

        let (_, costo_peps) = consumir(&capas, Cantidad::de_unidades(5)).unwrap();

        let s = kardex::aplicar(
            Saldo::VACIO,
            Movimiento {
                signo: Signo::Entrada,
                cantidad: Cantidad::de_unidades(10),
                costo_unitario: Centavos::de_pesos(100),
            },
        )
        .unwrap()
        .saldo;
        let s = kardex::aplicar(
            s,
            Movimiento {
                signo: Signo::Entrada,
                cantidad: Cantidad::de_unidades(10),
                costo_unitario: Centavos::de_pesos(200),
            },
        )
        .unwrap()
        .saldo;
        let costo_promedio = kardex::aplicar(
            s,
            Movimiento {
                signo: Signo::Salida,
                cantidad: Cantidad::de_unidades(5),
                costo_unitario: Centavos::CERO,
            },
        )
        .unwrap()
        .valor_movimiento;

        assert_eq!(costo_peps, Centavos::de_pesos(500));
        assert_eq!(costo_promedio, Centavos::de_pesos(750));
        assert_ne!(costo_peps, costo_promedio);
    }

    /// Vaciar todas las capas tiene que llevarse exactamente su valor: ni un
    /// centavo de más ni de menos. Es la conservación que hace auditable el
    /// costo de ventas.
    #[test]
    fn vaciar_las_capas_se_lleva_su_valor_exacto() {
        // Costos que no dividen bien a propósito.
        let capas = [capa(1, 3, 100), capa(2, 7, 33), capa(3, 11, 7)];
        let valor_total: Centavos = capas.iter().map(|c| c.valor().expect("cabe")).sum();
        let unidades: Cantidad = capas.iter().map(|c| c.cantidad).sum();
        let (_, total) = consumir(&capas, unidades).unwrap();
        assert_eq!(total, valor_total, "el costo no cuadra con lo que había en capas");
    }

    /// 300 consumos parciales sobre capas que se van agotando: lo consumido
    /// tiene que sumar el valor original.
    #[test]
    fn trescientos_consumos_conservan_el_valor() {
        let mut capas: Vec<Capa> = (1..=60)
            .map(|i| capa(i, i % 9 + 1, i % 977 + 13))
            .collect();
        let valor_inicial: Centavos = capas.iter().map(|c| c.valor().expect("cabe")).sum();
        let mut costeado = Centavos::CERO;

        for k in 1..=300i64 {
            let pide = Cantidad::de_unidades(k % 4 + 1);
            let disponible: Cantidad = capas.iter().map(|c| c.cantidad).sum();
            if pide > disponible {
                break;
            }
            let (consumos, total) = consumir(&capas, pide).unwrap();
            costeado = costeado + total;
            for c in consumos {
                let capa = capas.iter_mut().find(|x| x.movimiento_id == c.movimiento_id).unwrap();
                capa.cantidad = capa.cantidad - c.cantidad;
            }
            capas.retain(|c| c.cantidad.0 > 0);
        }
        let restante: Centavos = capas.iter().map(|c| c.valor().expect("cabe")).sum();
        assert_eq!(
            costeado + restante,
            valor_inicial,
            "se perdió o se inventó valor consumiendo capas"
        );
    }

    #[test]
    fn no_se_puede_consumir_mas_de_lo_que_hay_en_capas() {
        let capas = [capa(1, 3, 100)];
        assert_eq!(
            consumir(&capas, Cantidad::de_unidades(4)),
            Err(ErrorPeps::CapasInsuficientes {
                hay: Cantidad::de_unidades(3),
                piden: Cantidad::de_unidades(4),
            })
        );
        assert!(consumir(&capas, Cantidad::CERO).is_err());
        assert!(consumir(&[], Cantidad::de_unidades(1)).is_err());
    }

    // -- Costo de adquisición ------------------------------------------------

    fn lc(art: i64, u: i64, pesos: i64) -> LineaCompra {
        LineaCompra {
            articulo_id: art,
            cantidad: Cantidad::de_unidades(u),
            precio_unitario: Centavos::de_pesos(pesos),
            descuento: Centavos::CERO,
        }
    }

    /// NIC 2.11: el flete ENTRA al costo del inventario. Sin esto el inventario
    /// queda subvaluado y el costo de ventas también.
    #[test]
    fn el_flete_se_reparte_en_proporcion_al_valor() {
        let lineas = [lc(1, 10, 1000), lc(2, 10, 3000)]; // 10.000 y 30.000
        let r = costo_adquisicion(
            &lineas,
            CostosIndirectos {
                flete: Centavos::de_pesos(4000),
                ..Default::default()
            },
        )
        .unwrap();
        // 25 % y 75 % del flete.
        assert_eq!(r[0].costo_indirecto, Centavos::de_pesos(1000));
        assert_eq!(r[1].costo_indirecto, Centavos::de_pesos(3000));
        assert_eq!(r[0].costo_total, Centavos::de_pesos(11_000));
        assert_eq!(r[1].costo_total, Centavos::de_pesos(33_000));
        // Y el unitario sube: $1.100, no $1.000.
        assert_eq!(r[0].costo_unitario, Centavos::de_pesos(1100));
        assert_eq!(r[1].costo_unitario, Centavos::de_pesos(3300));
    }

    /// El descuento comercial se RESTA (NIC 2.11), y además baja la base del
    /// reparto: quien compró con descuento absorbe menos flete.
    #[test]
    fn el_descuento_comercial_se_resta_y_cambia_el_reparto() {
        let lineas = [
            LineaCompra { descuento: Centavos::de_pesos(5000), ..lc(1, 10, 1000) },
            lc(2, 10, 3000),
        ];
        let r = costo_adquisicion(
            &lineas,
            CostosIndirectos {
                flete: Centavos::de_pesos(3500),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r[0].costo_directo, Centavos::de_pesos(5000), "10.000 − 5.000");
        // Base 35.000; a la primera le toca 5/35 = 500.
        assert_eq!(r[0].costo_indirecto, Centavos::de_pesos(500));
        assert_eq!(r[1].costo_indirecto, Centavos::de_pesos(3000));
    }

    /// Ni un centavo se queda fuera del inventario, aunque el reparto no divida
    /// exacto. Es la prueba que discrimina el reparto por diferencia.
    #[test]
    fn lo_repartido_es_exactamente_el_indirecto_aunque_no_divida() {
        for indirecto in [1i64, 7, 13, 99, 1000, 123_457] {
            let lineas = [lc(1, 3, 7), lc(2, 5, 11), lc(3, 7, 13)];
            let r = costo_adquisicion(
                &lineas,
                CostosIndirectos {
                    flete: Centavos(indirecto),
                    ..Default::default()
                },
            )
            .unwrap();
            let suma: Centavos = r.iter().map(|x| x.costo_indirecto).sum();
            assert_eq!(
                suma,
                Centavos(indirecto),
                "con indirecto {indirecto} se repartieron {suma:?}"
            );
            // Y el total de la compra cuadra con directo + indirecto.
            let directo: Centavos = r.iter().map(|x| x.costo_directo).sum();
            let total: Centavos = r.iter().map(|x| x.costo_total).sum();
            assert_eq!(total, directo + Centavos(indirecto));
        }
    }

    #[test]
    fn los_cuatro_indirectos_suman_y_se_reparten_juntos() {
        let r = costo_adquisicion(
            &[lc(1, 1, 1000)],
            CostosIndirectos {
                flete: Centavos::de_pesos(100),
                arancel: Centavos::de_pesos(200),
                seguro: Centavos::de_pesos(30),
                otros: Centavos::de_pesos(70),
            },
        )
        .unwrap();
        assert_eq!(r[0].costo_indirecto, Centavos::de_pesos(400));
        assert_eq!(r[0].costo_total, Centavos::de_pesos(1400));
    }

    #[test]
    fn sin_indirectos_el_costo_es_el_de_la_factura() {
        let r = costo_adquisicion(&[lc(1, 10, 1000)], CostosIndirectos::default()).unwrap();
        assert_eq!(r[0].costo_indirecto, Centavos::CERO);
        assert_eq!(r[0].costo_unitario, Centavos::de_pesos(1000));
    }

    /// Repartir un flete entre líneas que valen cero es imposible, y se dice.
    /// Repartirlo «por partes iguales» sería inventar un criterio.
    #[test]
    fn un_flete_sin_base_sobre_la_que_repartir_se_rechaza() {
        let e = costo_adquisicion(
            &[lc(1, 10, 0)],
            CostosIndirectos {
                flete: Centavos::de_pesos(5000),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(e, ErrorCompra::BaseCero(_)), "salió {e}");
        // Pero una compra de valor cero SIN flete sí pasa: los obsequios del
        // proveedor existen.
        assert!(costo_adquisicion(&[lc(1, 10, 0)], CostosIndirectos::default()).is_ok());
    }

    #[test]
    fn se_rechaza_lo_imposible() {
        assert!(matches!(
            costo_adquisicion(&[], CostosIndirectos::default()),
            Err(ErrorCompra::SinLineas)
        ));
        assert!(matches!(
            costo_adquisicion(&[lc(1, 0, 100)], CostosIndirectos::default()),
            Err(ErrorCompra::CantidadNoPositiva { .. })
        ));
        assert!(matches!(
            costo_adquisicion(
                &[LineaCompra { descuento: Centavos::de_pesos(99_999), ..lc(1, 1, 100) }],
                CostosIndirectos::default()
            ),
            Err(ErrorCompra::DescuentoMayorQueLinea { .. })
        ));
        assert!(matches!(
            costo_adquisicion(
                &[lc(1, 1, 100)],
                CostosIndirectos { flete: Centavos(-1), ..Default::default() }
            ),
            Err(ErrorCompra::IndirectoNegativo)
        ));
    }
}
