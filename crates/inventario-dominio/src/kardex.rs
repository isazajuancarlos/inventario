//! El kardex: la **única** fuente de la existencia.
//!
//! El sistema viejo guardaba `ARTICULOS.INV_ACTUAL` como campo mutable y lo
//! recalculaba con un proceso por lotes (`recalsalidas.prg`) filtrado por un
//! mes y un año globales. Resultado medido: facturó dieciséis años con el
//! kardex parado, y la existencia que mostraba era ficción.
//!
//! Aquí no existe ese campo. La existencia es lo que dice el kardex, y cada
//! movimiento persiste la foto (`existencia_despues`, `valor_despues`) para que
//! el saldo de un artículo se lea sin recorrer su historia entera. Las dos
//! cosas tienen que coincidir siempre — `datos::kardex` tiene la prueba que lo
//! comprueba sobre la base real.
//!
//! ## Costo promedio ponderado sin deriva
//!
//! No se guarda un «costo promedio» redondeado del que luego se multiplique:
//! eso acumula error movimiento a movimiento. Se llevan **dos acumuladores
//! exactos** —existencia y valor total del inventario— y el promedio se deriva
//! cuando alguien lo quiere ver. La salida se cuesta *proporcionalmente* al
//! valor acumulado, que es la definición del promedio ponderado y no pierde ni
//! un centavo: lo que sale del acumulador de valor es exactamente lo que se
//! carga al costo.

use crate::dinero::{Cantidad, Centavos, MIL, dividir_redondeando};

/// Hacia dónde mueve el inventario un concepto. En el sistema viejo era un
/// campo `C(1)` con `'+'` o `'-'`; aquí no hay un tercer valor posible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Signo {
    Entrada,
    Salida,
}

impl Signo {
    pub fn como_texto(self) -> &'static str {
        match self {
            Signo::Entrada => "entrada",
            Signo::Salida => "salida",
        }
    }

    /// Lee el signo del corpus heredado (`CONCEPTOS.SIGNO`).
    ///
    /// Devuelve `None` para cualquier otra cosa —incluido el vacío— en vez de
    /// suponer una dirección: los 3 movimientos con concepto vacío del corpus
    /// tienen que RECHAZARSE, no entrar como entradas por defecto.
    pub fn desde_texto(s: &str) -> Option<Signo> {
        match s.trim() {
            "+" | "entrada" => Some(Signo::Entrada),
            "-" | "salida" => Some(Signo::Salida),
            _ => None,
        }
    }
}

/// El saldo de un artículo: los dos acumuladores exactos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Saldo {
    /// Unidades en milésimas.
    pub existencia: Cantidad,
    /// Valor total del inventario de ese artículo, en centavos.
    pub valor: Centavos,
}

impl Saldo {
    pub const VACIO: Saldo = Saldo {
        existencia: Cantidad::CERO,
        valor: Centavos::CERO,
    };

    /// Costo promedio ponderado por unidad, **derivado**.
    ///
    /// # Un `None` con DOS significados eran dos campos (directiva 32)
    ///
    /// Esta función devolvía `Option` y metía en el mismo `None` dos cosas que
    /// no se parecen en nada:
    ///
    /// - **no hay existencia positiva** — no hay nada que promediar, y es un
    ///   estado NORMAL del inventario;
    /// - **el promedio no cabe en `i64`** — hay existencia y hay valor, pero el
    ///   cociente se sale. Eso es un saldo imposible, no un estado normal.
    ///
    /// **El techo, para poder juzgar si es alcanzable en una instalación
    /// concreta**: hace falta `|valor| / existencia > 9,22 × 10¹⁵` centavos por
    /// milésima, o sea del orden de **92 mil billones de pesos por unidad**. La
    /// vía que
    /// lo producía —el truncamiento de `multiplicar`— se cerró en `baf4429`;
    /// quedan los datos heredados de antes de ese arreglo, una base editada a
    /// mano, y un costo unitario cercano a `i64::MAX` tecleado sobre una
    /// cantidad de una milésima.
    ///
    /// Y el colapso tenía consecuencia: `datos::kardex::ultimo_costo` filtra por
    /// `existencia_despues > 0` en el propio SQL, así que su `None` **solo puede
    /// ser el segundo caso** — y quien lo recibía lo trataba como el primero,
    /// costeando la salida a CERO. Un saldo imposible se convertía en un
    /// movimiento con costo cero, en silencio y con la transacción confirmada.
    ///
    /// Lo señaló la revisión de seguridad del 2026-08-13.
    pub fn costo_promedio(&self) -> Result<Centavos, SinPromedio> {
        if self.existencia.0 <= 0 {
            return Err(SinPromedio::SinExistencia);
        }
        dividir_redondeando(self.valor.0 as i128 * MIL as i128, self.existencia.0 as i128)
            .map(Centavos)
            .ok_or(SinPromedio::NoCabe)
    }
}

/// Por qué no hay costo promedio. Ver [`Saldo::costo_promedio`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SinPromedio {
    /// No hay existencia positiva contra la que promediar. Estado NORMAL: un
    /// artículo recién creado, o uno que se agotó.
    #[error("no hay existencia positiva: no hay promedio que dar")]
    SinExistencia,
    /// Hay existencia y hay valor, pero el cociente no cabe en `i64`. Eso es un
    /// saldo imposible, y **no** debe leerse como «no hay promedio».
    #[error("el costo promedio no cabe en centavos: el saldo es imposible")]
    NoCabe,
}

/// Lo que se le pide al kardex.
#[derive(Debug, Clone, Copy)]
pub struct Movimiento {
    pub signo: Signo,
    /// Siempre positiva. La dirección la pone `signo`, no el signo del número:
    /// una cantidad negativa en una entrada es un dato ambiguo, y se rechaza.
    pub cantidad: Cantidad,
    /// Costo unitario. Solo lo usan las **entradas**; en una salida se ignora,
    /// porque el costo de salida lo fija el promedio, no quien digita.
    pub costo_unitario: Centavos,
}

/// El efecto de aplicar un movimiento.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Efecto {
    pub saldo: Saldo,
    /// Lo que ese movimiento suma (entrada) o resta (salida) al valor del
    /// inventario, en centavos y siempre positivo.
    pub valor_movimiento: Centavos,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErrorKardex {
    #[error("la cantidad de un movimiento debe ser mayor que cero (llegó {0})")]
    CantidadNoPositiva(i64),
    /// El valor del movimiento no cabe en `Centavos`.
    ///
    /// Antes del 2026-08-13 esto truncaba en silencio dentro de `multiplicar` y
    /// el kardex seguía acumulando sobre un valor que no era cantidad × costo:
    /// el saldo quedaba mal para siempre, sin un solo error.
    #[error("el valor del movimiento no cabe en centavos (cantidad {cantidad}, costo unitario {costo_unitario})")]
    ImporteFueraDeRango { cantidad: i64, costo_unitario: i64 },
    /// El SALDO contra el que se iba a costear no es representable: tiene
    /// existencia positiva pero su promedio por unidad no cabe en `i64`.
    ///
    /// No es lo mismo que `ImporteFueraDeRango`, y por eso es otra variante: allí
    /// el que no cabe es el importe del MOVIMIENTO; aquí el movimiento puede
    /// caber perfectamente y lo que está corrupto es el punto de partida.
    #[error(
        // OJO A LA BARRA: sin ella, una cadena partida conserva el salto Y la
        // sangría del código —así salían diez espacios en medio del mensaje que
        // ve el operador—. Con ella se quitan las dos cosas pero se junta SIN
        // separador, así que el espacio va ANTES de la barra. Las dos mitades de
        // la trampa están escritas en el `CLAUDE.md` de este repositorio.
        "el saldo del que se va a costear no es representable \
         (existencia {existencia}, valor {valor}): su promedio por unidad no cabe en centavos"
    )]
    SaldoNoRepresentable { existencia: i64, valor: i64 },
    #[error("el costo unitario de una entrada no puede ser negativo (llegó {0})")]
    CostoNegativo(i64),
    #[error(
        "existencia insuficiente: hay {hay} y se piden {piden}. \
         Use `aplicar_permitiendo_negativo` si el negocio admite saldo negativo."
    )]
    ExistenciaInsuficiente { hay: Cantidad, piden: Cantidad },
}

/// Aplica un movimiento **exigiendo** que haya existencia.
///
/// Es el camino por defecto a propósito: el corpus heredado tiene un artículo
/// en −3 unidades, y un saldo negativo es una venta que nadie pudo entregar.
pub fn aplicar(saldo: Saldo, mov: Movimiento) -> Result<Efecto, ErrorKardex> {
    validar(&mov)?;
    if mov.signo == Signo::Salida && mov.cantidad > saldo.existencia {
        return Err(ErrorKardex::ExistenciaInsuficiente {
            hay: saldo.existencia,
            piden: mov.cantidad,
        });
    }
    calcular(saldo, mov)
}

/// Aplica un movimiento **aceptando** que el saldo quede negativo.
///
/// Camino permisivo explícito, pedido por el llamante (directiva 20): hay
/// negocios donde la salida se registra antes que la entrada que la respalda.
/// Cuando la existencia no es positiva no hay promedio con el que costear, así
/// que la salida se cuesta al **último costo promedio conocido**, que el
/// llamante pasa; si nunca hubo uno, el costo es cero.
///
/// **Y NO QUEDA «MARCADO COMO TAL», aunque aquí se dijera que sí.** `Efecto`
/// tiene `saldo` y `valor_movimiento`, y nada más: no hay campo, ni columna, ni
/// bandera que distinga una salida costeada a cero por falta de historia de una
/// costeada a cero de verdad. Lo señaló la revisión de seguridad del
/// 2026-08-13. Se corrige la frase en vez de inventar la marca: construirla es
/// una decisión de alcance, y prometerla sin tenerla es peor que las dos.
pub fn aplicar_permitiendo_negativo(
    saldo: Saldo,
    mov: Movimiento,
    ultimo_costo: Option<Centavos>,
) -> Result<Efecto, ErrorKardex> {
    validar(&mov)?;
    if mov.signo == Signo::Salida && saldo.existencia.0 <= 0 {
        let unitario = ultimo_costo.unwrap_or(Centavos::CERO);
        let valor = crate::dinero::multiplicar(mov.cantidad, unitario).ok_or(
            ErrorKardex::ImporteFueraDeRango {
                cantidad: mov.cantidad.0,
                costo_unitario: unitario.0,
            },
        )?;
        return Ok(Efecto {
            saldo: Saldo {
                existencia: saldo.existencia - mov.cantidad,
                valor: saldo.valor - valor,
            },
            valor_movimiento: valor,
        });
    }
    calcular(saldo, mov)
}

fn validar(mov: &Movimiento) -> Result<(), ErrorKardex> {
    if !mov.cantidad.es_positiva() {
        return Err(ErrorKardex::CantidadNoPositiva(mov.cantidad.0));
    }
    if mov.signo == Signo::Entrada && mov.costo_unitario.es_negativo() {
        return Err(ErrorKardex::CostoNegativo(mov.costo_unitario.0));
    }
    Ok(())
}

fn calcular(saldo: Saldo, mov: Movimiento) -> Result<Efecto, ErrorKardex> {
    let fuera_de_rango = || ErrorKardex::ImporteFueraDeRango {
        cantidad: mov.cantidad.0,
        costo_unitario: mov.costo_unitario.0,
    };
    Ok(match mov.signo {
        Signo::Entrada => {
            let valor = crate::dinero::multiplicar(mov.cantidad, mov.costo_unitario)
                .ok_or_else(fuera_de_rango)?;
            Efecto {
                saldo: Saldo {
                    existencia: saldo.existencia + mov.cantidad,
                    valor: saldo.valor + valor,
                },
                valor_movimiento: valor,
            }
        }
        Signo::Salida => {
            // Proporcional al valor acumulado: es la definición del promedio
            // ponderado, y no deja residuo. Si la salida se lleva TODA la
            // existencia, se lleva TODO el valor y el saldo queda en (0, 0)
            // exacto — no en «0 unidades y 3 centavos», que es el resto que
            // deja multiplicar por un promedio redondeado.
            let valor = if mov.cantidad == saldo.existencia {
                // SE LLEVA TODO: no hay división. **Esta rama NO se bloquea
                // aunque el saldo sea imposible, y es deliberado**: bloquearla
                // sería quitar la salida en vez de protegerla, que es el error
                // que esta misma familia de arreglos ya cometió una vez con la
                // entrada correctiva (directiva 21).
                //
                // **Y REPARA SOLO CON PROMEDIO PONDERADO. Aquí decía que repara
                // a secas, y es falso con PEPS.** Con promedio, el saldo queda
                // en (0, 0) exacto. Con `Formula::Peps`, la capa de datos
                // reescribe el saldo DESPUÉS de esta función
                // —`efecto.saldo.valor -= total - efecto.valor_movimiento`— y
                // sobre un saldo corrupto eso aterriza en (0, basura), no en
                // (0, 0). Medido por la revisión de seguridad del 2026-08-13:
                //
                //     promedio: Saldo { existencia: 0, valor: 0 }
                //     PEPS:     Saldo { existencia: 0, valor: 36893488147419083 }
                //
                // Antes, una entrada posterior BLANQUEABA ese resto —el
                // promedio volvía a caber y la guarda de abajo callaba para
                // siempre—.
                //
                // CERRADO EL 2026-08-14 en la capa de datos, que es su sitio,
                // POR ABSORCIÓN: tras el ajuste de PEPS, si no queda existencia
                // el valor se pone a cero —cero unidades es cero valor, sea el
                // sobrante basura de un saldo corrupto o el residuo de ±unos
                // centavos que el redondeo del costeo por capas deja al vender
                // en tandas—. Con eso no sobrevive nada que blanquear.
                //
                // NO se RECHAZA, y esa fue mi primera versión, que una revisión
                // de confianza 8 tumbó: rechazar bloqueaba ventas de cierre
                // legítimas, porque `valor_despues` y la suma de las capas
                // derivan por redondeo. Lo fijan, en `inventario-datos`,
                // `peps_absorbe_la_basura_al_cerrar_en_vez_de_blanquearla` y su
                // pareja `una_venta_de_cierre_con_residuo_de_redondeo_no_se_bloquea`.
                saldo.valor
            } else {
                // COSTEO PROPORCIONAL, y aquí sí se comprueba el punto de
                // partida antes de dividir.
                //
                // El cociente `valor × cantidad / existencia` puede caber
                // perfectamente aunque el saldo sea IMPOSIBLE: con existencia 2
                // milésimas y valor 3,69 × 10¹⁶, una salida de 1 milésima da
                // 1,84 × 10¹⁶ —que cabe— mientras el promedio por unidad
                // (valor × 1000 / existencia) no cabe ni de lejos. O sea que la
                // guarda del importe NO protege de esto: sale un número
                // plausible costeado contra un saldo que no significa nada.
                //
                // Medido por la revisión de seguridad del 2026-08-13, que lo
                // dejó anotado como el agujero de fondo: «el sitio no es la
                // guarda del llamante, es `calcular`, que divide por
                // `saldo.existencia` sin preguntarse si ese saldo es
                // representable».
                if saldo.costo_promedio() == Err(SinPromedio::NoCabe) {
                    return Err(ErrorKardex::SaldoNoRepresentable {
                        existencia: saldo.existencia.0,
                        valor: saldo.valor.0,
                    });
                }
                Centavos(
                    dividir_redondeando(
                        saldo.valor.0 as i128 * mov.cantidad.0 as i128,
                        saldo.existencia.0 as i128,
                    )
                    .ok_or_else(fuera_de_rango)?,
                )
            };
            Efecto {
                saldo: Saldo {
                    existencia: saldo.existencia - mov.cantidad,
                    valor: saldo.valor - valor,
                },
                valor_movimiento: valor,
            }
        }
    })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// **La variante `ImporteFueraDeRango` no la alcanzaba ninguna prueba.**
    ///
    /// Lo señaló la revisión de seguridad del 2026-08-13: de las cinco familias
    /// de error que este trabajo añadió, solo la de factura tenía caso rojo. Un
    /// `ok_or` que nunca dispara es indistinguible de uno cableado a la
    /// condición equivocada.
    #[test]
    fn un_movimiento_cuyo_valor_no_cabe_se_rechaza() {
        let saldo = Saldo::VACIO;
        let mov = Movimiento {
            signo: Signo::Entrada,
            cantidad: Cantidad::de_unidades(20_000_000_000),
            costo_unitario: Centavos::de_pesos(10_000_000),
        };
        let e = aplicar(saldo, mov).unwrap_err();
        assert!(
            matches!(e, ErrorKardex::ImporteFueraDeRango { .. }),
            "se esperaba ImporteFueraDeRango y llegó {e:?}"
        );

        // LA PAREJA: una entrada de cifras normales sí mueve el saldo. Sin ella,
        // un `aplicar` que rechazara todo pasaría la de arriba.
        let ok = aplicar(saldo, entrada(10, 1_000)).unwrap();
        assert_eq!(ok.saldo.existencia, Cantidad::de_unidades(10));
        assert_eq!(ok.valor_movimiento, Centavos::de_pesos(10_000));
    }

    /// **LOS DOS «SIN PROMEDIO» NO SON EL MISMO, y antes no se podían
    /// distinguir.**
    ///
    /// Es la pareja que el cambio hace posible: hasta el 2026-08-13 los dos
    /// eran `None`, así que un llamante no tenía forma de saber si estaba ante
    /// un artículo agotado —normal— o ante un saldo imposible. Y uno de ellos
    /// costeaba la salida a CERO por esa confusión.
    #[test]
    fn sin_existencia_y_saldo_imposible_se_distinguen() {
        // LA MITAD NORMAL: agotado. No hay nada que promediar y no pasa nada.
        let agotado = Saldo {
            existencia: Cantidad::CERO,
            valor: Centavos::CERO,
        };
        assert_eq!(agotado.costo_promedio(), Err(SinPromedio::SinExistencia));

        // LA MITAD IMPOSIBLE: una milésima de unidad con un valor enorme. El
        // promedio por unidad es valor × 1000, y no cabe.
        let imposible = Saldo {
            existencia: Cantidad(1),
            valor: Centavos(i64::MAX / 500),
        };
        assert_eq!(imposible.costo_promedio(), Err(SinPromedio::NoCabe));

        // Y LA TERCERA, que impide una función que diga «error» a todo: un
        // saldo corriente sí da su promedio.
        let normal = Saldo {
            existencia: Cantidad::de_unidades(10),
            valor: Centavos::de_pesos(1_000),
        };
        assert_eq!(normal.costo_promedio(), Ok(Centavos::de_pesos(100)));
    }

    /// **NO SE COSTEA CONTRA UN SALDO QUE NO SIGNIFICA NADA — y las tres
    /// reparaciones siguen pasando.**
    ///
    /// El agujero de fondo que dejó anotado la revisión del 2026-08-13: el
    /// cociente `valor × cantidad / existencia` cabe perfectamente aunque el
    /// promedio POR UNIDAD del saldo no quepa, así que la guarda del importe no
    /// protege de esto. Salía un número plausible costeado contra basura.
    ///
    /// Las tres parejas son la mitad que impide el sesgo a cerrar (directiva
    /// 21): esta familia de arreglos ya quitó una vez el camino de reparación,
    /// y aquí hay TRES que no se pueden tocar.
    #[test]
    fn una_salida_parcial_no_se_cuesta_contra_un_saldo_imposible() {
        // Existencia positiva, valor enorme: el promedio por unidad no cabe.
        let imposible = Saldo {
            existencia: Cantidad(2),
            valor: Centavos(i64::MAX / 250),
        };
        let parcial = Movimiento {
            signo: Signo::Salida,
            cantidad: Cantidad(1),
            costo_unitario: Centavos::CERO,
        };
        let e = aplicar(imposible, parcial).unwrap_err();
        assert!(
            matches!(e, ErrorKardex::SaldoNoRepresentable { .. }),
            "se costeó contra un saldo imposible: {e:?}"
        );

        // Y EL MENSAJE, IMPRESO. El `matches!` de arriba mira el discriminante,
        // así que sigue verde con el texto partido, con diez espacios en medio o
        // con dos palabras pegadas — que son las TRES formas en que este
        // repositorio ya se ha tragado la trampa de la cadena partida. Esto lo
        // convierte de directiva en prosa a prueba que falla (directiva 6).
        let texto = e.to_string();
        assert!(
            !texto.contains("  ") && !texto.contains('\n'),
            "el literal se volvió a partir mal: {texto:?}"
        );
        assert!(
            texto.contains("no es representable (existencia"),
            "la barra juntó dos palabras sin separador: {texto:?}"
        );

        // PAREJA 1 — LLEVÁRSELO TODO SÍ PASA, porque es lo que lo repara: deja
        // el saldo en (0, 0) exacto y no hay división.
        let todo = Movimiento {
            signo: Signo::Salida,
            cantidad: Cantidad(2),
            costo_unitario: Centavos::CERO,
        };
        let r = aplicar(imposible, todo).expect("llevárselo todo repara, no se bloquea");
        assert_eq!(r.saldo.existencia, Cantidad::CERO);
        assert_eq!(r.saldo.valor, Centavos::CERO);

        // PAREJA 2 — UNA ENTRADA SIGUE PASANDO. No cuesta contra el saldo, y es
        // la otra forma de repararlo.
        let r = aplicar(imposible, entrada(10, 1_000))
            .expect("una entrada correctiva no se bloquea");
        assert!(r.valor_movimiento.0 > 0);

        // PAREJA 3 — UN SALDO NORMAL se cuesta como siempre. Sin esto, una
        // guarda que rechazara todo pasaría las de arriba.
        let sano = Saldo {
            existencia: Cantidad::de_unidades(10),
            valor: Centavos::de_pesos(1_000),
        };
        let r = aplicar(sano, Movimiento {
            signo: Signo::Salida,
            cantidad: Cantidad::de_unidades(3),
            costo_unitario: Centavos::CERO,
        })
        .expect("una salida normal se cuesta");
        assert_eq!(r.valor_movimiento, Centavos::de_pesos(300));
    }

    fn entrada(u: i64, pesos: i64) -> Movimiento {
        Movimiento {
            signo: Signo::Entrada,
            cantidad: Cantidad::de_unidades(u),
            costo_unitario: Centavos::de_pesos(pesos),
        }
    }
    fn salida(u: i64) -> Movimiento {
        Movimiento {
            signo: Signo::Salida,
            cantidad: Cantidad::de_unidades(u),
            costo_unitario: Centavos::CERO,
        }
    }

    #[test]
    fn el_signo_vacio_no_se_convierte_en_entrada() {
        assert_eq!(Signo::desde_texto("+"), Some(Signo::Entrada));
        assert_eq!(Signo::desde_texto("-"), Some(Signo::Salida));
        // Los 3 movimientos con concepto vacío del corpus caen aquí.
        assert_eq!(Signo::desde_texto(""), None);
        assert_eq!(Signo::desde_texto(" "), None);
        assert_eq!(Signo::desde_texto("E"), None);
    }

    /// El caso de libro de texto: 10 a $100 y 10 a $200 promedian $150.
    #[test]
    fn el_promedio_ponderado_es_el_de_los_libros() {
        let s = aplicar(Saldo::VACIO, entrada(10, 100)).unwrap().saldo;
        assert_eq!(s.costo_promedio(), Ok(Centavos::de_pesos(100)));
        let s = aplicar(s, entrada(10, 200)).unwrap().saldo;
        assert_eq!(s.existencia, Cantidad::de_unidades(20));
        assert_eq!(s.costo_promedio(), Ok(Centavos::de_pesos(150)));
        assert_eq!(s.valor, Centavos::de_pesos(3000));
    }

    /// La salida se cuesta al promedio y **no lo mueve**.
    #[test]
    fn la_salida_no_cambia_el_promedio() {
        let s = aplicar(Saldo::VACIO, entrada(10, 100)).unwrap().saldo;
        let s = aplicar(s, entrada(10, 200)).unwrap().saldo;
        let e = aplicar(s, salida(5)).unwrap();
        assert_eq!(e.valor_movimiento, Centavos::de_pesos(750)); // 5 × 150
        assert_eq!(e.saldo.costo_promedio(), Ok(Centavos::de_pesos(150)));
        assert_eq!(e.saldo.existencia, Cantidad::de_unidades(15));
    }

    /// Sin esto el promedio deriva. Un costo que no divide exacto (100/3) se
    /// reparte entre muchas salidas: al final la existencia es cero y el valor
    /// **también**, sin centavos colgando.
    #[test]
    fn vaciar_el_inventario_lo_deja_en_cero_exacto_sin_residuo() {
        // 3 × $100 + 4 × $33 = 43.200 centavos entre 7 unidades: el promedio
        // es 6.171,43 centavos, que NO divide exacto. Es el caso que separa el
        // costeo proporcional del que multiplica por un promedio redondeado.
        let s = aplicar(Saldo::VACIO, entrada(3, 100)).unwrap().saldo;
        let mut s = aplicar(s, entrada(4, 33)).unwrap().saldo;
        assert_ne!(
            s.valor.0 % (s.existencia.0 / crate::dinero::MIL),
            0,
            "el caso dejó de tener residuo; ya no separa un método del otro"
        );
        let mut costeado = Centavos::CERO;
        let valor_inicial = s.valor;
        for _ in 0..7 {
            let e = aplicar(s, salida(1)).unwrap();
            costeado = costeado + e.valor_movimiento;
            s = e.saldo;
        }
        assert_eq!(s.existencia, Cantidad::CERO);
        assert_eq!(s.valor, Centavos::CERO, "quedó valor sin existencia");
        assert_eq!(costeado, valor_inicial, "el costo no cuadra con lo que entró");
    }

    /// 500 movimientos alternos: lo que entra menos lo que sale es exactamente
    /// lo que queda. Es la conservación que el sistema viejo no tenía.
    #[test]
    fn quinientos_movimientos_conservan_valor_y_existencia() {
        let mut s = Saldo::VACIO;
        let (mut entro, mut salio) = (Centavos::CERO, Centavos::CERO);
        let (mut u_entro, mut u_salio) = (Cantidad::CERO, Cantidad::CERO);
        for i in 1..=500i64 {
            if i % 3 != 0 {
                let m = entrada(i % 17 + 1, i % 977 + 7);
                let e = aplicar(s, m).unwrap();
                entro = entro + e.valor_movimiento;
                u_entro = u_entro + m.cantidad;
                s = e.saldo;
            } else {
                let pide = Cantidad::de_unidades(i % 5 + 1);
                if pide <= s.existencia {
                    let e = aplicar(s, salida(pide.0 / 1000)).unwrap();
                    salio = salio + e.valor_movimiento;
                    u_salio = u_salio + pide;
                    s = e.saldo;
                }
            }
        }
        assert_eq!(s.existencia, u_entro - u_salio);
        assert_eq!(s.valor, entro - salio);
        assert!(s.costo_promedio().is_ok());
    }

    #[test]
    fn no_se_puede_sacar_lo_que_no_hay() {
        let s = aplicar(Saldo::VACIO, entrada(3, 100)).unwrap().saldo;
        assert_eq!(
            aplicar(s, salida(4)),
            Err(ErrorKardex::ExistenciaInsuficiente {
                hay: Cantidad::de_unidades(3),
                piden: Cantidad::de_unidades(4),
            })
        );
        // Sacar justo lo que hay sí se puede.
        assert!(aplicar(s, salida(3)).is_ok());
    }

    #[test]
    fn el_camino_negativo_hay_que_pedirlo_y_usa_el_ultimo_costo() {
        let e = aplicar_permitiendo_negativo(
            Saldo::VACIO,
            salida(3),
            Some(Centavos::de_pesos(500)),
        )
        .unwrap();
        assert_eq!(e.saldo.existencia, Cantidad::de_unidades(-3));
        assert_eq!(e.valor_movimiento, Centavos::de_pesos(1500));
        // Sin existencia positiva no hay promedio que mostrar, y no se inventa.
        // Y AHORA SE DICE POR QUÉ: sin existencia, no por un saldo imposible.
        assert_eq!(e.saldo.costo_promedio(), Err(SinPromedio::SinExistencia));
    }

    #[test]
    fn una_cantidad_no_positiva_se_rechaza_en_vez_de_invertir_el_signo() {
        let m = Movimiento {
            signo: Signo::Entrada,
            cantidad: Cantidad::de_unidades(-5),
            costo_unitario: Centavos::de_pesos(10),
        };
        assert_eq!(aplicar(Saldo::VACIO, m), Err(ErrorKardex::CantidadNoPositiva(-5000)));
        let m = Movimiento { cantidad: Cantidad::CERO, ..m };
        assert_eq!(aplicar(Saldo::VACIO, m), Err(ErrorKardex::CantidadNoPositiva(0)));
    }

    #[test]
    fn una_entrada_con_costo_negativo_se_rechaza() {
        let m = Movimiento {
            signo: Signo::Entrada,
            cantidad: Cantidad::de_unidades(1),
            costo_unitario: Centavos(-1),
        };
        assert_eq!(aplicar(Saldo::VACIO, m), Err(ErrorKardex::CostoNegativo(-1)));
    }

    /// Directiva 8: la prueba tiene que DISCRIMINAR. Si el costeo de salida se
    /// hiciera con el promedio redondeado en vez de proporcional al acumulado,
    /// el inventario quedaría con valor y sin unidades. Aquí se rompe a
    /// propósito y se comprueba que el fallo aparece.
    #[test]
    fn costear_con_el_promedio_redondeado_si_deja_residuo() {
        let s = aplicar(Saldo::VACIO, entrada(3, 100)).unwrap().saldo;
        let s = aplicar(s, entrada(4, 33)).unwrap().saldo;
        // La versión MALA: multiplicar por el promedio redondeado.
        let prom = s.costo_promedio().unwrap();
        let mut valor = s.valor;
        for _ in 0..7 {
            valor = valor - crate::dinero::multiplicar(Cantidad::de_unidades(1), prom).expect("cabe");
        }
        assert_ne!(
            valor,
            Centavos::CERO,
            "si esto da cero, el caso dejó de discriminar y hay que buscar otro costo feo"
        );
    }
}
