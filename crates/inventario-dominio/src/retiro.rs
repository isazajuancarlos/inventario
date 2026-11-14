//! Retiro de inventarios y disminución del inventario: los dos artículos del
//! Estatuto Tributario que un sistema de inventario incumple sin enterarse.
//!
//! # 1. Retiro de inventarios — ET art. 421 literal b)
//!
//! > Se consideran venta «el retiro de bienes corporales muebles e inmuebles
//! > hechos por el responsable para su uso o para formar parte de los activos
//! > fijos de la empresa».
//!
//! **La base gravable es el valor comercial**, no el costo. Un obsequio, un
//! consumo interno o un retiro de socios **genera IVA**, y el responsable lo
//! asume.
//!
//! Esto es lo más fácil de incumplir sin darse cuenta, porque la salida de
//! inventario «funciona» igual: el kardex baja, el saldo cuadra y nadie ve nada
//! raro. En el sistema heredado había **91 salidas por obsequios, 49 por retiro
//! de socios y 21 por consumo interno** — 161 hechos generadores de IVA que no
//! se calcularon nunca.
//!
//! # 2. Disminución del inventario — ET art. 64 (modificado por Ley 1819/2016)
//!
//! Los faltantes de mercancía **de fácil destrucción o pérdida** pueden
//! disminuir el inventario final **hasta en un 3 % de la suma del inventario
//! inicial más las compras**. Por encima hace falta demostrar fuerza mayor o
//! caso fortuito.
//!
//! La baja **por obsolescencia** es deducible solo si los bienes se destruyen,
//! reciclan o chatarrizan **y** existe un documento con cantidad, descripción,
//! costo fiscal unitario y justificación, firmado por el representante legal y
//! los responsables de la destrucción.
//!
//! Un concepto de kardex llamado «salida por vencimiento», sin medir el tope ni
//! producir el acta, no sustenta nada.

use crate::dinero::{Cantidad, Centavos, dividir_redondeando, porcentaje};

// ---------------------------------------------------------------------------
// Retiro de inventarios
// ---------------------------------------------------------------------------

/// Efecto tributario de una salida de inventario que **no** es una venta.
///
/// Es una lista cerrada, y esa es la idea: al crear un concepto de kardex hay
/// que **decidir** en cuál cae. El defecto no puede ser «ninguno», porque
/// entonces todo obsequio quedaría sin IVA por omisión (directiva 20).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EfectoTributario {
    /// Sale por una venta facturada: el IVA lo lleva la factura.
    Venta,
    /// **Retiro** para uso propio, obsequios, consumo interno, retiro de
    /// socios, o para formar parte de los activos fijos. ET art. 421 lit. b):
    /// causa IVA sobre el valor comercial.
    RetiroGravado,
    /// Faltante de fácil destrucción o pérdida. Cuenta contra el tope del 3 %
    /// del art. 64 y **no** causa IVA.
    FaltanteArt64,
    /// Baja por obsolescencia con destrucción. Exige acta (art. 64) y **no**
    /// causa IVA.
    BajaConActa,
    /// Traslado entre bodegas o ajuste de conteo: no sale del patrimonio, así
    /// que no causa IVA ni consume el tope.
    NoAfectaImpuestos,
}

impl EfectoTributario {
    pub fn causa_iva(self) -> bool {
        matches!(self, EfectoTributario::RetiroGravado)
    }
    pub fn consume_tope_art64(self) -> bool {
        matches!(self, EfectoTributario::FaltanteArt64)
    }
    pub fn exige_acta(self) -> bool {
        matches!(self, EfectoTributario::BajaConActa)
    }

    pub fn como_texto(self) -> &'static str {
        match self {
            EfectoTributario::Venta => "venta",
            EfectoTributario::RetiroGravado => "retiro_gravado",
            EfectoTributario::FaltanteArt64 => "faltante_art64",
            EfectoTributario::BajaConActa => "baja_con_acta",
            EfectoTributario::NoAfectaImpuestos => "no_afecta_impuestos",
        }
    }

    /// `None` para lo que no se reconoce. **No cae a `NoAfectaImpuestos`**: ese
    /// defecto silencioso es justo lo que dejaría un obsequio sin IVA.
    pub fn desde_texto(s: &str) -> Option<EfectoTributario> {
        Some(match s.trim() {
            "venta" => EfectoTributario::Venta,
            "retiro_gravado" => EfectoTributario::RetiroGravado,
            "faltante_art64" => EfectoTributario::FaltanteArt64,
            "baja_con_acta" => EfectoTributario::BajaConActa,
            "no_afecta_impuestos" => EfectoTributario::NoAfectaImpuestos,
            _ => None?,
        })
    }
}

/// El IVA que causa un retiro.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct IvaRetiro {
    /// Base gravable: **valor comercial**, no costo.
    pub base: Centavos,
    pub tarifa: i64,
    pub iva: Centavos,
    /// El costo con el que salió del kardex. Se guarda al lado para que se vea
    /// la diferencia: es lo que hace evidente por qué la base no es el costo.
    pub costo: Centavos,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErrorRetiro {
    #[error("la tarifa de IVA {0} % está fuera de [0, 100]")]
    TarifaImposible(i64),
    #[error("la cantidad retirada debe ser positiva (llegó {0})")]
    CantidadNoPositiva(i64),
    #[error(
        "un retiro gravado (ET art. 421 lit. b) exige el VALOR COMERCIAL de los bienes, \
         y no se dio. El costo NO sirve como base gravable"
    )]
    SinValorComercial,
    /// Un importe del retiro no cabe en `Centavos`. Gemela de las de factura,
    /// compra y kardex: mismo defecto, misma decisión —rechazar, no saturar—.
    #[error("{0} no cabe en centavos; revise las cifras")]
    ImporteFueraDeRango(&'static str),
}

/// Calcula el IVA de un retiro gravado.
///
/// `valor_comercial_unitario` es obligatorio y tiene que ser el precio de venta,
/// no el costo. Si el llamante pasa `None`, se **falla**: tomar el costo como
/// base sería subdeclarar el impuesto en silencio.
pub fn iva_por_retiro(
    cantidad: Cantidad,
    valor_comercial_unitario: Option<Centavos>,
    tarifa: i64,
    costo: Centavos,
) -> Result<IvaRetiro, ErrorRetiro> {
    if !cantidad.es_positiva() {
        return Err(ErrorRetiro::CantidadNoPositiva(cantidad.0));
    }
    if !(0..=100).contains(&tarifa) {
        return Err(ErrorRetiro::TarifaImposible(tarifa));
    }
    let unitario = valor_comercial_unitario.ok_or(ErrorRetiro::SinValorComercial)?;
    if unitario.es_negativo() {
        return Err(ErrorRetiro::SinValorComercial);
    }
    let base = crate::dinero::multiplicar(cantidad, unitario)
        .ok_or(ErrorRetiro::ImporteFueraDeRango("la base gravable del retiro"))?;
    Ok(IvaRetiro {
        base,
        tarifa,
        iva: porcentaje(base, tarifa)
            .ok_or(ErrorRetiro::ImporteFueraDeRango("el IVA del retiro"))?,
        costo,
    })
}

// ---------------------------------------------------------------------------
// Artículo 64: el tope del 3 %
// ---------------------------------------------------------------------------

/// El límite del art. 64, en unidades.
///
/// La norma habla de **unidades** del inventario final y de la suma del
/// inventario inicial más las compras. Se calcula en unidades y se informa
/// también el costo, que es lo que va a la declaración.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct TopeArt64 {
    /// Unidades del inventario inicial del periodo.
    pub inventario_inicial: Cantidad,
    /// Unidades compradas en el periodo.
    pub compras: Cantidad,
    /// `inventario_inicial + compras`.
    pub base: Cantidad,
    /// El 3 % de la base.
    pub tope: Cantidad,
    /// Lo que ya se dio de baja como faltante en el periodo.
    pub consumido: Cantidad,
    /// `tope − consumido`, nunca negativo.
    pub disponible: Cantidad,
    /// Cuánto se pasó del tope, si se pasó.
    pub exceso: Cantidad,
}

impl TopeArt64 {
    pub fn se_paso(&self) -> bool {
        self.exceso.0 > 0
    }
    /// Porcentaje efectivamente consumido, en centésimas de punto (250 = 2,50 %).
    /// Se devuelve en enteros para no meter coma flotante en un dato fiscal.
    /// `None` si no cabe en `i64`. Solo puede ocurrir con una base ridículamente
    /// pequeña frente a lo consumido —el resultado son centésimas de punto, así
    /// que en operación normal vale unos pocos miles—, pero devolver un número
    /// truncado en un dato FISCAL es exactamente lo que este cambio persigue.
    pub fn porcentaje_consumido_centesimas(&self) -> Option<i64> {
        if self.base.0 == 0 {
            return Some(0);
        }
        dividir_redondeando(self.consumido.0 as i128 * 10_000, self.base.0 as i128)
    }
}

/// Calcula el tope del 3 % y cuánto queda.
pub fn tope_art64(
    inventario_inicial: Cantidad,
    compras: Cantidad,
    consumido: Cantidad,
) -> TopeArt64 {
    let base = Cantidad((inventario_inicial.0 + compras.0).max(0));
    // 3 % exacto sobre unidades en milésimas: no se redondea al alza.
    // El tope es el 3 % de una cantidad que ya cabe en `i64`, así que su
    // resultado cabe por construcción: 3/100 de un número es menor que él. El
    // `expect` documenta esa imposibilidad en vez de esconderla en un `as`
    // —que era lo que había— o de propagar un error que ningún llamante podría
    // manejar de otra forma.
    let tope = Cantidad(
        dividir_redondeando(base.0 as i128 * 3, 100)
            .expect("el 3 % de una cantidad que cabe en i64 cabe en i64"),
    );
    let disponible = Cantidad((tope.0 - consumido.0).max(0));
    let exceso = Cantidad((consumido.0 - tope.0).max(0));
    TopeArt64 {
        inventario_inicial,
        compras,
        base,
        tope,
        consumido,
        disponible,
        exceso,
    }
}

// ---------------------------------------------------------------------------
// Artículo 64: el acta de baja
// ---------------------------------------------------------------------------

/// Una línea del acta. Los campos son **los que enumera la norma**, ni más ni
/// menos: cantidad, descripción, costo fiscal unitario y justificación.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RenglonActa {
    pub articulo_id: i64,
    pub descripcion: String,
    pub cantidad: Cantidad,
    /// Costo fiscal **unitario**. La norma lo pide unitario, no total.
    pub costo_fiscal_unitario: Centavos,
    pub justificacion: String,
}

impl RenglonActa {
    /// Costo fiscal total del renglón, o `None` si no cabe en `Centavos`.
    ///
    /// Devolvía `Centavos` a secas y el desbordamiento se perdía en silencio
    /// dentro de `multiplicar`: un acta del art. 64 salía con un costo que no
    /// era cantidad × unitario, y esa acta se presenta ante la DIAN.
    pub fn costo_total(&self) -> Option<Centavos> {
        crate::dinero::multiplicar(self.cantidad, self.costo_fiscal_unitario)
    }
}

/// El acta de destrucción, reciclaje o chatarrización.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Acta {
    pub fecha: String,
    /// Destrucción, reciclaje o chatarrización. La norma exige que ocurra una
    /// de las tres; «se botó» no sustenta la deducción.
    pub metodo: MetodoDestruccion,
    pub renglones: Vec<RenglonActa>,
    pub representante_legal: String,
    /// Quienes presenciaron y ejecutaron la destrucción. La norma pide su
    /// firma; aquí se guardan sus nombres y el acta se firma en papel o con el
    /// mecanismo que use el cliente.
    pub responsables: Vec<String>,
    /// Parte cubierta por seguro o indemnización. **Solo es deducible lo NO
    /// cubierto** (art. 64), así que se descuenta.
    pub cubierto_por_seguro: Centavos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetodoDestruccion {
    Destruccion,
    Reciclaje,
    Chatarrizacion,
}

impl MetodoDestruccion {
    pub fn como_texto(self) -> &'static str {
        match self {
            MetodoDestruccion::Destruccion => "destruccion",
            MetodoDestruccion::Reciclaje => "reciclaje",
            MetodoDestruccion::Chatarrizacion => "chatarrizacion",
        }
    }
    pub fn desde_texto(s: &str) -> Option<MetodoDestruccion> {
        Some(match s.trim() {
            "destruccion" => MetodoDestruccion::Destruccion,
            "reciclaje" => MetodoDestruccion::Reciclaje,
            "chatarrizacion" => MetodoDestruccion::Chatarrizacion,
            _ => None?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErrorActa {
    #[error("el acta necesita al menos un renglón")]
    SinRenglones,
    #[error("renglón {0}: falta la descripción, y el art. 64 la exige")]
    SinDescripcion(usize),
    #[error("renglón {0}: falta la justificación, y el art. 64 la exige")]
    SinJustificacion(usize),
    #[error("renglón {0}: la cantidad debe ser positiva")]
    CantidadNoPositiva(usize),
    #[error("renglón {0}: el costo fiscal unitario no puede ser negativo")]
    CostoNegativo(usize),
    /// El costo fiscal de un renglón no cabe en `Centavos`. El acta del art. 64
    /// se presenta ante la DIAN: un total truncado ahí es un dato falso firmado.
    #[error("renglón {0}: el costo fiscal total no cabe en centavos; revise las cifras")]
    ImporteFueraDeRango(usize),
    #[error("el acta necesita el nombre del representante legal (art. 64)")]
    SinRepresentante,
    #[error("el acta necesita al menos un responsable de la destrucción (art. 64)")]
    SinResponsables,
    #[error("lo cubierto por seguro ({cubierto}) supera el costo del acta ({costo})")]
    SeguroMayorQueCosto { cubierto: i64, costo: i64 },
}

/// Lo que resulta de un acta válida.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ResultadoActa {
    pub costo_total: Centavos,
    pub cubierto_por_seguro: Centavos,
    /// `costo_total − cubierto_por_seguro`. Es lo único deducible (art. 64).
    pub deducible: Centavos,
}

/// Valida el acta contra lo que enumera el art. 64 y calcula lo deducible.
pub fn validar_acta(a: &Acta) -> Result<ResultadoActa, ErrorActa> {
    if a.renglones.is_empty() {
        return Err(ErrorActa::SinRenglones);
    }
    if a.representante_legal.trim().is_empty() {
        return Err(ErrorActa::SinRepresentante);
    }
    if a.responsables.iter().all(|r| r.trim().is_empty()) {
        return Err(ErrorActa::SinResponsables);
    }
    let mut costo_total = Centavos::CERO;
    for (i, r) in a.renglones.iter().enumerate() {
        if r.descripcion.trim().is_empty() {
            return Err(ErrorActa::SinDescripcion(i));
        }
        if r.justificacion.trim().is_empty() {
            return Err(ErrorActa::SinJustificacion(i));
        }
        if !r.cantidad.es_positiva() {
            return Err(ErrorActa::CantidadNoPositiva(i));
        }
        if r.costo_fiscal_unitario.es_negativo() {
            return Err(ErrorActa::CostoNegativo(i));
        }
        costo_total = costo_total
            + r.costo_total()
                .ok_or(ErrorActa::ImporteFueraDeRango(i))?;
    }
    if a.cubierto_por_seguro.es_negativo() || a.cubierto_por_seguro > costo_total {
        return Err(ErrorActa::SeguroMayorQueCosto {
            cubierto: a.cubierto_por_seguro.0,
            costo: costo_total.0,
        });
    }
    Ok(ResultadoActa {
        costo_total,
        cubierto_por_seguro: a.cubierto_por_seguro,
        deducible: costo_total - a.cubierto_por_seguro,
    })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    // -- Retiro gravado ------------------------------------------------------

    /// **La variante `ImporteFueraDeRango` no la alcanzaba ninguna prueba**, y
    /// lo señaló la revisión de seguridad del 2026-08-13: de las cinco familias
    /// de error que este trabajo añadió, solo la de factura tenía caso rojo.
    #[test]
    fn un_retiro_cuya_base_no_cabe_se_rechaza() {
        let e = iva_por_retiro(
            Cantidad::de_unidades(20_000_000_000),
            Some(Centavos::de_pesos(10_000_000)),
            19,
            Centavos::CERO,
        )
        .unwrap_err();
        // Se comprueba QUÉ importe, no solo que falle: `iva_por_retiro` tiene
        // dos sitios que pueden desbordar —la base y el IVA— y sin mirar la
        // carga, un mutante que intercambiara los dos mensajes pasaría. Lo
        // señaló la revisión del 2026-08-13.
        assert!(
            matches!(e, ErrorRetiro::ImporteFueraDeRango(que) if que.contains("base")),
            "se esperaba que fallara LA BASE y llegó {e:?}"
        );

        // LA PAREJA: el obsequio del corpus sigue liquidando. Sin ella, un
        // `iva_por_retiro` que rechazara todo pasaría la de arriba.
        let ok = iva_por_retiro(
            Cantidad::de_unidades(10),
            Some(Centavos::de_pesos(5_000)),
            19,
            Centavos::de_pesos(30_000),
        )
        .unwrap();
        assert_eq!(ok.base, Centavos::de_pesos(50_000));
    }

    /// El acta del art. 64 tampoco tenía caso rojo, y su costo total va a la
    /// DIAN.
    #[test]
    fn un_acta_cuyo_costo_fiscal_no_cabe_se_rechaza() {
        let renglon = |c: i64, unit: i64| RenglonActa {
            articulo_id: 1,
            descripcion: "Agua".into(),
            cantidad: Cantidad::de_unidades(c),
            costo_fiscal_unitario: Centavos::de_pesos(unit),
            justificacion: "vencimiento".into(),
        };
        assert_eq!(
            renglon(20_000_000_000, 10_000_000).costo_total(),
            None,
            "un costo total que no cabe tiene que decir que no"
        );
        // LA PAREJA: un renglón normal sí da su costo.
        assert_eq!(
            renglon(10, 3_000).costo_total(),
            Some(Centavos::de_pesos(30_000))
        );
    }

    /// El caso del corpus: un obsequio. Sale del kardex al costo, pero el IVA
    /// se liquida sobre el **valor comercial**.
    #[test]
    fn un_obsequio_causa_iva_sobre_el_valor_comercial_no_sobre_el_costo() {
        let r = iva_por_retiro(
            Cantidad::de_unidades(10),
            Some(Centavos::de_pesos(5_000)), // valor comercial
            19,
            Centavos::de_pesos(30_000), // costo: 10 × $3.000
        )
        .unwrap();
        assert_eq!(r.base, Centavos::de_pesos(50_000), "base = valor comercial");
        assert_eq!(r.iva, Centavos::de_pesos(9_500));
        assert_eq!(r.costo, Centavos::de_pesos(30_000));
        // Y esto es lo que discrimina: si la base fuera el costo, el IVA sería
        // $5.700 y el Estado recibiría $3.800 menos por cada obsequio.
        assert_ne!(Some(r.iva), porcentaje(r.costo, 19));
    }

    /// Sin valor comercial NO se liquida con el costo: se falla.
    #[test]
    fn sin_valor_comercial_se_falla_en_vez_de_usar_el_costo() {
        let e = iva_por_retiro(
            Cantidad::de_unidades(10),
            None,
            19,
            Centavos::de_pesos(30_000),
        )
        .unwrap_err();
        assert_eq!(e, ErrorRetiro::SinValorComercial);
        assert!(e.to_string().contains("421"), "el error no cita la norma: {e}");
    }

    #[test]
    fn el_efecto_tributario_no_tiene_defecto_silencioso() {
        // Un texto desconocido NO cae en «no afecta impuestos»: eso dejaría
        // todo obsequio sin IVA por omisión.
        assert_eq!(EfectoTributario::desde_texto("obsequio"), None);
        assert_eq!(EfectoTributario::desde_texto(""), None);
        assert_eq!(
            EfectoTributario::desde_texto("retiro_gravado"),
            Some(EfectoTributario::RetiroGravado)
        );
    }

    #[test]
    fn cada_efecto_sabe_que_dispara() {
        use EfectoTributario as E;
        assert!(E::RetiroGravado.causa_iva());
        assert!(!E::Venta.causa_iva(), "el IVA de la venta lo lleva la factura");
        assert!(!E::FaltanteArt64.causa_iva());
        assert!(!E::BajaConActa.causa_iva());

        assert!(E::FaltanteArt64.consume_tope_art64());
        assert!(!E::BajaConActa.consume_tope_art64(), "la baja por obsolescencia va aparte");
        assert!(!E::NoAfectaImpuestos.consume_tope_art64());

        assert!(E::BajaConActa.exige_acta());
        assert!(!E::FaltanteArt64.exige_acta());
    }

    #[test]
    fn una_tarifa_imposible_o_una_cantidad_no_positiva_se_rechazan() {
        let v = Some(Centavos::de_pesos(1000));
        assert!(iva_por_retiro(Cantidad::de_unidades(1), v, 101, Centavos::CERO).is_err());
        assert!(iva_por_retiro(Cantidad::de_unidades(1), v, -1, Centavos::CERO).is_err());
        assert!(iva_por_retiro(Cantidad::CERO, v, 19, Centavos::CERO).is_err());
        // El 16 % del corpus de 2004 es inusual, no imposible.
        assert!(iva_por_retiro(Cantidad::de_unidades(1), v, 16, Centavos::CERO).is_ok());
    }

    // -- Tope del 3 % --------------------------------------------------------

    #[test]
    fn el_tope_es_el_tres_por_ciento_de_inicial_mas_compras() {
        let t = tope_art64(
            Cantidad::de_unidades(1_000),
            Cantidad::de_unidades(9_000),
            Cantidad::de_unidades(200),
        );
        assert_eq!(t.base, Cantidad::de_unidades(10_000));
        assert_eq!(t.tope, Cantidad::de_unidades(300));
        assert_eq!(t.disponible, Cantidad::de_unidades(100));
        assert!(!t.se_paso());
        assert_eq!(t.porcentaje_consumido_centesimas(), Some(200), "2,00 %");
    }

    /// Lo que el sistema viejo no hacía: **avisar** al pasarse.
    #[test]
    fn pasarse_del_tope_se_detecta_y_se_cuantifica() {
        let t = tope_art64(
            Cantidad::de_unidades(1_000),
            Cantidad::de_unidades(9_000),
            Cantidad::de_unidades(450),
        );
        assert!(t.se_paso());
        assert_eq!(t.exceso, Cantidad::de_unidades(150));
        assert_eq!(t.disponible, Cantidad::CERO, "no queda cupo, y no es negativo");
        assert_eq!(t.porcentaje_consumido_centesimas(), Some(450), "4,50 %");
    }

    #[test]
    fn el_tope_no_se_redondea_al_alza_a_favor_del_contribuyente() {
        // Base 1.000 unidades → 3 % = 30 exactas.
        let t = tope_art64(Cantidad::de_unidades(1_000), Cantidad::CERO, Cantidad::CERO);
        assert_eq!(t.tope, Cantidad::de_unidades(30));
        // Base cero: no hay tope ni división por cero.
        let t = tope_art64(Cantidad::CERO, Cantidad::CERO, Cantidad::CERO);
        assert_eq!(t.tope, Cantidad::CERO);
        assert_eq!(t.porcentaje_consumido_centesimas(), Some(0));
    }

    // -- Acta ---------------------------------------------------------------

    fn acta() -> Acta {
        Acta {
            fecha: "2026-08-02".into(),
            metodo: MetodoDestruccion::Destruccion,
            renglones: vec![RenglonActa {
                articulo_id: 1,
                descripcion: "Botellón 20 L".into(),
                cantidad: Cantidad::de_unidades(10),
                costo_fiscal_unitario: Centavos::de_pesos(3_000),
                justificacion: "Vencido el 2026-07-31".into(),
            }],
            representante_legal: "Juan Pérez".into(),
            responsables: vec!["Ana Gómez".into()],
            cubierto_por_seguro: Centavos::CERO,
        }
    }

    #[test]
    fn un_acta_completa_produce_el_deducible() {
        let r = validar_acta(&acta()).unwrap();
        assert_eq!(r.costo_total, Centavos::de_pesos(30_000));
        assert_eq!(r.deducible, Centavos::de_pesos(30_000));
    }

    /// Art. 64: solo es deducible lo NO cubierto por el seguro.
    #[test]
    fn lo_cubierto_por_el_seguro_no_es_deducible() {
        let a = Acta {
            cubierto_por_seguro: Centavos::de_pesos(20_000),
            ..acta()
        };
        let r = validar_acta(&a).unwrap();
        assert_eq!(r.costo_total, Centavos::de_pesos(30_000));
        assert_eq!(r.deducible, Centavos::de_pesos(10_000));
        // Y no puede cubrir más de lo que costó.
        let a = Acta {
            cubierto_por_seguro: Centavos::de_pesos(50_000),
            ..acta()
        };
        assert!(matches!(
            validar_acta(&a),
            Err(ErrorActa::SeguroMayorQueCosto { .. })
        ));
    }

    /// Cada campo que enumera la norma es obligatorio, uno por uno. Un acta a
    /// la que le falte algo no sustenta la deducción, así que no se acepta.
    #[test]
    fn falta_cualquier_campo_que_exige_el_articulo_64_y_el_acta_se_rechaza() {
        let sin = |f: fn(&mut Acta)| {
            let mut a = acta();
            f(&mut a);
            validar_acta(&a).unwrap_err()
        };
        assert!(matches!(sin(|a| a.renglones.clear()), ErrorActa::SinRenglones));
        assert!(matches!(
            sin(|a| a.renglones[0].descripcion = "  ".into()),
            ErrorActa::SinDescripcion(0)
        ));
        assert!(matches!(
            sin(|a| a.renglones[0].justificacion = String::new()),
            ErrorActa::SinJustificacion(0)
        ));
        assert!(matches!(
            sin(|a| a.renglones[0].cantidad = Cantidad::CERO),
            ErrorActa::CantidadNoPositiva(0)
        ));
        assert!(matches!(
            sin(|a| a.representante_legal = " ".into()),
            ErrorActa::SinRepresentante
        ));
        assert!(matches!(
            sin(|a| a.responsables = vec!["".into()]),
            ErrorActa::SinResponsables
        ));
    }

    #[test]
    fn el_metodo_de_destruccion_es_una_lista_cerrada() {
        assert_eq!(
            MetodoDestruccion::desde_texto("reciclaje"),
            Some(MetodoDestruccion::Reciclaje)
        );
        // «se botó a la basura» no sustenta la deducción.
        assert_eq!(MetodoDestruccion::desde_texto("basura"), None);
        assert_eq!(MetodoDestruccion::desde_texto(""), None);
    }
}
