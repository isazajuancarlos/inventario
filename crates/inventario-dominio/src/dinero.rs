//! Dinero y cantidades. **Enteros siempre.**
//!
//! El sistema viejo guardaba `N(11,2)` y `N(10,2)` —coma flotante— y por eso
//! la cabecera de la factura no siempre cuadraba con el detalle. Aquí:
//!
//! - el dinero se mide en **centavos** (`Centavos`, `i64`);
//! - las cantidades en **milésimas de unidad** (`Cantidad`, `i64`), que dan
//!   tres decimales exactos y cubren tanto «3 botellones» como «1,250 kg».
//!
//! `i64` en centavos llega a **92 mil billones de pesos** —92.233.720.368.547.758,
//! exactamente— y no es el límite de nadie.
//!
//! Aquí ponía «92 billones», mil veces menos, y la cifra se había copiado a un
//! segundo sitio antes de que nadie la comprobara. La corrige la revisión de
//! seguridad del 2026-08-13 haciendo la división: `i64::MAX / 100`. El error
//! iba hacia el lado conservador —hacía parecer el techo mil veces más
//! alcanzable de lo que es—, que es la dirección menos dañina y la que más
//! tarda en notarse.

use std::fmt;

/// Milésimas por unidad. Una unidad son 1.000 milésimas.
pub const MIL: i64 = 1_000;

/// Dinero en centavos.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct Centavos(pub i64);

/// Cantidad en milésimas de unidad.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct Cantidad(pub i64);

impl Centavos {
    pub const CERO: Centavos = Centavos(0);

    /// Pesos enteros a centavos. **Para literales y constantes**: entra en pánico
    /// si desborda, porque un literal que desborda es un error de programa.
    /// Para lo que TECLEA quien opera está [`Centavos::intentar_de_pesos`].
    pub fn de_pesos(pesos: i64) -> Self {
        Centavos::intentar_de_pesos(pesos)
            .expect("un literal de pesos que desborda i64 al pasar a centavos es un error de programa")
    }

    /// Pesos a centavos, o `None` si desborda.
    ///
    /// Existe porque `pesos * 100` **desborda**, y un desbordamiento aquí no
    /// tiene respuesta razonable que dar. (Este comentario decía «en release
    /// envuelve en silencio, `overflow-checks` está apagado»; se corrigió el
    /// 2026-08-13: `Cargo.toml` lo enciende desde el día anterior, así que es
    /// pánico en los dos perfiles.) El valor entraba negativo en la
    /// aritmética de la factura. Hoy no llega a persistirse porque lo para el
    /// `CHECK` del esquema, pero eso es la red de otro: el día que haya un camino
    /// que no toque esas tablas no hay nada. Se cierra en el ORIGEN, que es donde
    /// el dato deja de ser lo que quien lo tecleó creyó teclear.
    pub fn intentar_de_pesos(pesos: i64) -> Option<Self> {
        pesos.checked_mul(100).map(Centavos)
    }

    pub fn es_cero(self) -> bool {
        self.0 == 0
    }

    pub fn es_negativo(self) -> bool {
        self.0 < 0
    }

    /// Pesos, truncando los centavos. Para mostrar, nunca para calcular.
    pub fn pesos(self) -> i64 {
        self.0 / 100
    }
}

impl Cantidad {
    pub const CERO: Cantidad = Cantidad(0);

    /// Unidades enteras a milésimas. **Para literales**; ver
    /// [`Centavos::de_pesos`].
    pub fn de_unidades(u: i64) -> Self {
        Cantidad::intentar_de_unidades(u)
            .expect("un literal de unidades que desborda i64 al pasar a milésimas es un error de programa")
    }

    /// Unidades a milésimas, o `None` si desborda. Ver
    /// [`Centavos::intentar_de_pesos`].
    pub fn intentar_de_unidades(u: i64) -> Option<Self> {
        u.checked_mul(MIL).map(Cantidad)
    }

    pub fn es_cero(self) -> bool {
        self.0 == 0
    }

    pub fn es_positiva(self) -> bool {
        self.0 > 0
    }
}

macro_rules! aritmetica {
    ($t:ty) => {
        impl $t {
            /// Suma que dice que no en vez de dar la vuelta.
            ///
            /// **Es la mitad que faltaba, y su ausencia era explotable.** Medido
            /// el 2026-08-12 por HTTP contra el binario de release: tres pagos
            /// individualmente válidos —dos de `i64::MAX`-ish y uno de 300—
            /// hacían que `recibo::registrar` guardara un recibo con
            /// `total = 84` mientras la suma real de sus `recibo_pago.valor` era
            /// 18 446 744 073 709 551 700. La cartera bajó exactamente 84
            /// centavos y quien lo tecleó vio un acuse normal con 200.
            ///
            /// Ninguna de las tres redes lo veía: `intentar_de_pesos` mira CADA
            /// pago por separado y los tres eran válidos, `cartera::repartir`
            /// solo rechaza `<= 0`, y los dos `CHECK` del esquema los pasa un 84
            /// positivo.
            ///
            /// Y el banco entero era ciego: el mismo caso es **pánico ruidoso en
            /// debug** y **dato corrupto silencioso en release**, y `cargo test`
            /// —o sea el verde local— corre en debug. Una prueba escrita de la
            /// forma obvia habría salido verde por la razón contraria a la que
            /// uno cree. Por eso el arreglo es aritmética comprobada, que se
            /// comporta IGUAL en los dos perfiles, y no una prueba más.
            pub fn sumar(self, o: Self) -> Option<Self> {
                self.0.checked_add(o.0).map(Self)
            }
        }
        impl std::ops::Add for $t {
            type Output = Self;
            fn add(self, o: Self) -> Self {
                Self(self.0 + o.0)
            }
        }
        impl std::ops::Sub for $t {
            type Output = Self;
            fn sub(self, o: Self) -> Self {
                Self(self.0 - o.0)
            }
        }
        impl std::ops::Neg for $t {
            type Output = Self;
            fn neg(self) -> Self {
                Self(-self.0)
            }
        }
        impl std::iter::Sum for $t {
            fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
                Self(iter.map(|x| x.0).sum())
            }
        }
    };
}
aritmetica!(Centavos);
aritmetica!(Cantidad);

impl fmt::Display for Centavos {
    /// `$ 1.234.567,89` — separador de miles con punto y decimal con coma, que
    /// es como se escribe el peso colombiano.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let neg = self.0 < 0;
        let abs = self.0.unsigned_abs();
        let (enteros, cent) = (abs / 100, abs % 100);
        let s = enteros.to_string();
        let mut con_puntos = String::new();
        for (i, c) in s.chars().enumerate() {
            if i > 0 && (s.len() - i) % 3 == 0 {
                con_puntos.push('.');
            }
            con_puntos.push(c);
        }
        write!(f, "{}$ {},{:02}", if neg { "-" } else { "" }, con_puntos, cent)
    }
}

impl fmt::Display for Cantidad {
    /// Sin decimales cuando la cantidad es entera; con tres cuando no.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 % MIL == 0 {
            write!(f, "{}", self.0 / MIL)
        } else {
            let neg = if self.0 < 0 { "-" } else { "" };
            let abs = self.0.unsigned_abs();
            write!(f, "{}{}.{:03}", neg, abs / MIL as u64, abs % MIL as u64)
        }
    }
}

/// División entera con **redondeo al más cercano, medio alejándose del cero**.
///
/// Es la regla que usa la gente al facturar (`0,5` sube), y la que hace que
/// `redondear(a) + redondear(b)` no se desvíe sistemáticamente hacia abajo como
/// haría el truncamiento de Rust. Se saca aparte porque de ella dependen el IVA,
/// el costeo y el prorrateo: si cambia, cambian los tres a la vez.
///
/// # Devuelve `None` si el resultado no cabe en `i64`
///
/// Terminaba en `as i64`, que en Rust **trunca en silencio** y que
/// `overflow-checks` NO comprueba en ningún perfil, ni siquiera en debug. Por
/// ahí una factura se persistía con un total que no era cantidad × precio:
/// medido, `1.553.255.926.290.448.384` donde el valor real era
/// `20.000.000.000.000.000.000`. Un número equivocado que parece un número
/// bueno es el peor desenlace posible en un producto que factura, así que el
/// tipo lo dice ahora y el llamante decide (directiva 20).
///
/// # Pánico
/// Si `den` es cero. Es un error del programador, no un dato del usuario: cada
/// llamante de este módulo comprueba antes que el denominador no sea cero.
pub fn dividir_redondeando(num: i128, den: i128) -> Option<i64> {
    assert!(den != 0, "dividir_redondeando: denominador cero");
    let (num, den) = if den < 0 { (-num, -den) } else { (num, den) };
    let ajuste = if num >= 0 { den / 2 } else { -(den / 2) };
    i64::try_from((num + ajuste) / den).ok()
}

/// `cantidad` (milésimas) × `precio` (centavos por unidad) → centavos.
///
/// `None` si el importe no cabe en `Centavos`. **Es el sitio por el que entró el
/// defecto medido**: una factura se persistía con un total que no era
/// cantidad × precio, y nada se quejaba —`as i64` trunca en silencio y
/// `overflow-checks` no lo mira—. Con `Option`, quien factura tiene que decir
/// qué hace cuando el importe no cabe, y hoy todos contestan lo mismo:
/// rechazar la línea (directiva 20).
pub fn multiplicar(cantidad: Cantidad, precio: Centavos) -> Option<Centavos> {
    dividir_redondeando(cantidad.0 as i128 * precio.0 as i128, MIL as i128).map(Centavos)
}

/// `base` × `porcentaje` (entero: 19 = 19 %) → centavos.
///
/// `None` si no cabe. Con porcentajes de impuesto (≤ 100) solo ocurre con una
/// base ya cercana al techo de `i64`, que es exactamente el caso que la línea
/// anterior deja pasar hoy.
pub fn porcentaje(base: Centavos, porcentaje: i64) -> Option<Centavos> {
    dividir_redondeando(base.0 as i128 * porcentaje as i128, 100).map(Centavos)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// **La suma de dos importes válidos que no cabe: dice que no.**
    ///
    /// Vale en los DOS perfiles, y eso es el punto. `+` a secas entra en pánico
    /// en debug y —SIN `overflow-checks`— daría la vuelta en release, así que
    /// una prueba escrita sobre `+`
    /// mediría cosas distintas según cómo se compile — y el binario que se
    /// instala es el que nadie medía. `sumar` se comporta igual en los dos.
    ///
    /// Los números son los del caso REAL, medido por HTTP el 2026-08-12: dos
    /// pagos de `i64::MAX - 107` y uno de 300 daban un total guardado de 84.
    #[test]
    fn sumar_dos_importes_que_no_caben_da_none() {
        let casi = Centavos(9_223_372_036_854_775_700);
        assert_eq!(casi.sumar(casi), None);
        assert_eq!(Centavos(i64::MAX).sumar(Centavos(1)), None);
        assert_eq!(Cantidad(i64::MAX).sumar(Cantidad(1)), None);
    }

    /// PAREJA: lo normal sí suma, y con el valor correcto. Sin esto, un `sumar`
    /// que devolviera siempre `None` pasaría la de arriba — y dejaría el recaudo
    /// entero rechazando recibos legítimos.
    #[test]
    fn sumar_dos_importes_normales_da_el_valor() {
        assert_eq!(Centavos(150_000).sumar(Centavos(2_500)), Some(Centavos(152_500)));
        assert_eq!(Centavos(-100).sumar(Centavos(100)), Some(Centavos::CERO));
        assert_eq!(Cantidad(7_000).sumar(Cantidad(3_000)), Some(Cantidad(10_000)));
    }

    /// **El desbordamiento se DICE, no se sufre.** `pesos * 100` desborda, y
    /// sin `overflow-checks` eso daría la vuelta en silencio con el importe
    /// entrando NEGATIVO en la aritmética de la factura.
    ///
    /// Este comentario afirmaba que ése es el estado del árbol. No lo es desde
    /// el 2026-08-12: `Cargo.toml` enciende `overflow-checks` también en
    /// release, así que hoy sería pánico —un 500— en vez de un dato falso.
    /// `intentar_de_pesos` sigue siendo lo correcto porque un rechazo con
    /// mensaje es mejor que las dos cosas. Corregido el 2026-08-13 tras una
    /// revisión de seguridad.
    #[test]
    fn convertir_pesos_que_desbordan_da_none() {
        assert_eq!(Centavos::intentar_de_pesos(i64::MAX), None);
        assert_eq!(Centavos::intentar_de_pesos(i64::MAX / 100 + 1), None);
        assert_eq!(Cantidad::intentar_de_unidades(i64::MAX), None);
    }

    /// PAREJA: lo normal sí pasa, y con el valor correcto. Sin esto, un
    /// `intentar_de_pesos` que devolviera siempre `None` pasaría la de arriba.
    #[test]
    fn convertir_pesos_normales_da_el_valor() {
        assert_eq!(Centavos::intentar_de_pesos(1_500), Some(Centavos(150_000)));
        assert_eq!(Centavos::intentar_de_pesos(-1_500), Some(Centavos(-150_000)));
        assert_eq!(Cantidad::intentar_de_unidades(7), Some(Cantidad(7_000)));
    }

    #[test]
    fn el_redondeo_es_al_mas_cercano_y_simetrico() {
        assert_eq!(dividir_redondeando(5, 10), Some(1)); // 0,5 -> 1
        assert_eq!(dividir_redondeando(4, 10), Some(0));
        assert_eq!(dividir_redondeando(15, 10), Some(2)); // 1,5 -> 2
        // Simétrico: los negativos se alejan del cero igual que los positivos.
        assert_eq!(dividir_redondeando(-5, 10), Some(-1));
        assert_eq!(dividir_redondeando(-15, 10), Some(-2));
        assert_eq!(dividir_redondeando(-4, 10), Some(0));
        // Y con denominador negativo da lo mismo que con el signo arriba.
        assert_eq!(dividir_redondeando(5, -10), Some(-1));
    }

    /// Truncar sesga: 100 sumas de 0,6 dan 0 truncando y 100 redondeando. Esto
    /// es lo que impide que el detalle se aleje de la cabecera factura a
    /// factura, que es justo lo que le pasó al sistema viejo.
    #[test]
    fn el_redondeo_no_sesga_hacia_abajo_como_el_truncamiento() {
        let truncado: i64 = (0..100).map(|_| 6i64 / 10).sum();
        let redondeado: i64 = (0..100).map(|_| dividir_redondeando(6, 10).expect("cabe")).sum();
        assert_eq!(truncado, 0);
        assert_eq!(redondeado, 100);
    }

    #[test]
    fn multiplicar_cantidad_por_precio() {
        // 3 unidades a $409,00 = $1.227,00
        assert_eq!(
            multiplicar(Cantidad::de_unidades(3), Centavos::de_pesos(409)),
            Some(Centavos::de_pesos(1227))
        );
        // Media unidad a $409,00 = $204,50
        assert_eq!(
            multiplicar(Cantidad(500), Centavos::de_pesos(409)),
            Some(Centavos(20450))
        );
        // 1,333 unidades a $1,00 -> 1,333 centavos -> 1 centavo
        assert_eq!(multiplicar(Cantidad(1333), Centavos(1)), Some(Centavos(1)));
    }

    /// **EL DEFECTO QUE ESTE MÓDULO TENÍA, con su número medido.**
    ///
    /// `dividir_redondeando` terminaba en `as i64`, que trunca en silencio y que
    /// `overflow-checks` NO comprueba en ningún perfil. Con 20.000.000.000
    /// unidades a $10.000.000 la unidad, el importe real es
    /// 20.000.000.000.000.000.000 centavos —que no cabe en `i64`— y lo que salía
    /// era 1.553.255.926.290.448.384: un número plausible, distinto, y que se
    /// persistía como total de una factura.
    ///
    /// Ahora dice que no.
    #[test]
    fn un_importe_que_no_cabe_dice_que_no_en_vez_de_truncar() {
        let cantidad = Cantidad::de_unidades(20_000_000_000);
        let precio = Centavos::de_pesos(10_000_000);
        assert_eq!(multiplicar(cantidad, precio), None);

        // El valor exacto que el `as i64` producía. Se escribe aquí para que
        // esta prueba se ponga roja si alguien vuelve a meter un `as`: no basta
        // con que el resultado no sea el bueno, tiene que no HABER resultado.
        let truncado_de_antes = 1_553_255_926_290_448_384i64;
        assert_ne!(multiplicar(cantidad, precio), Some(Centavos(truncado_de_antes)));

        // LA PAREJA, y es la mitad que impide una función que diga «no» a todo:
        // el mismo cálculo con cifras de un negocio real sí da su importe.
        assert_eq!(
            multiplicar(Cantidad::de_unidades(1_000), Centavos::de_pesos(25_000)),
            Some(Centavos::de_pesos(25_000_000))
        );
    }

    /// El techo exacto, por los dos lados. Sin esto, un `None` prematuro —que
    /// rechazara importes legítimos— pasaría igual que el arreglo bueno.
    #[test]
    fn el_limite_esta_donde_dice_i64_y_no_antes() {
        assert_eq!(dividir_redondeando(i64::MAX as i128, 1), Some(i64::MAX));
        assert_eq!(dividir_redondeando(i64::MAX as i128 + 1, 1), None);
        assert_eq!(dividir_redondeando(i64::MIN as i128, 1), Some(i64::MIN));
        // Y el redondeo NO empuja fuera de rango por su cuenta: MAX con ajuste
        // sigue siendo representable porque el cálculo va en i128.
        assert_eq!(dividir_redondeando(i64::MAX as i128 * 2, 2), Some(i64::MAX));
    }

    #[test]
    fn el_iva_del_19_por_ciento() {
        assert_eq!(porcentaje(Centavos::de_pesos(100_000), 19), Some(Centavos::de_pesos(19_000)));
        // 16 %, que es el que trae el corpus de 2004.
        assert_eq!(porcentaje(Centavos::de_pesos(1000), 16), Some(Centavos::de_pesos(160)));
        assert_eq!(porcentaje(Centavos::de_pesos(1000), 0), Some(Centavos::CERO));
    }

    #[test]
    fn se_muestra_como_se_escribe_el_peso() {
        assert_eq!(Centavos(123456789).to_string(), "$ 1.234.567,89");
        assert_eq!(Centavos(0).to_string(), "$ 0,00");
        assert_eq!(Centavos(-50).to_string(), "-$ 0,50");
        assert_eq!(Centavos::de_pesos(999).to_string(), "$ 999,00");
        assert_eq!(Centavos::de_pesos(1000).to_string(), "$ 1.000,00");
    }

    #[test]
    fn la_cantidad_muestra_decimales_solo_si_los_tiene() {
        assert_eq!(Cantidad::de_unidades(3).to_string(), "3");
        assert_eq!(Cantidad(1250).to_string(), "1.250");
        assert_eq!(Cantidad(-3000).to_string(), "-3");
    }
}
