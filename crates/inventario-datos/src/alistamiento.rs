//! Diagnóstico de alistamiento: **qué falta para poder operar**.
//!
//! Es la pieza que separa «está construido» de «se puede entregar». Un sistema
//! que se instala y arranca no está listo: le falta la empresa, la resolución,
//! los usuarios, los artículos y —para facturar electrónicamente— la clave
//! técnica y el PIN del software. Descubrir cada cosa al chocar contra ella, el
//! primer lunes y con un cliente esperando, es la peor forma de descubrirlas.
//!
//! El diagnóstico se organiza por **qué bloquea**, no por módulo:
//!
//! - `Bloqueante`: sin esto no se puede operar. Nada de facturar, nada de
//!   mover inventario.
//! - `BloqueaFacturaElectronica`: se puede operar en papel, pero no emitir
//!   documento electrónico válido.
//! - `Advertencia`: se puede operar y hay algo que va a doler pronto.
//!
//! Cada punto trae **qué hacer**, no solo qué falta. Un diagnóstico que dice
//! «falta la resolución» y no dice dónde se pide obliga a preguntar.

use crate::{Pool, Resultado};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Gravedad {
    /// Sin esto no se puede operar.
    Bloqueante,
    /// Se puede operar; no se puede emitir factura electrónica.
    BloqueaFacturaElectronica,
    /// Se puede operar y conviene resolverlo.
    Advertencia,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Punto {
    pub gravedad: Gravedad,
    /// Qué falta, en una frase.
    pub que: String,
    /// Qué hacer para resolverlo.
    pub como: String,
    /// Cuántos elementos afecta, cuando aplica.
    pub cuantos: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Diagnostico {
    /// `true` cuando no queda ningún punto bloqueante.
    pub puede_operar: bool,
    /// `true` cuando además no queda ningún punto que bloquee la factura
    /// electrónica.
    pub puede_facturar_electronicamente: bool,
    pub puntos: Vec<Punto>,
    /// Resumen de lo que sí está: sirve para saber que el diagnóstico miró algo.
    pub inventario: Resumen,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct Resumen {
    pub articulos: i64,
    pub terceros: i64,
    pub usuarios_activos: i64,
    pub conceptos: i64,
    pub resoluciones_utilizables: i64,
    pub movimientos: i64,
    pub facturas: i64,
}

fn punto(
    gravedad: Gravedad,
    que: impl Into<String>,
    como: impl Into<String>,
    cuantos: Option<i64>,
) -> Punto {
    Punto {
        gravedad,
        que: que.into(),
        como: como.into(),
        cuantos,
    }
}

/// Corre el diagnóstico completo.
///
/// `hoy` entra como parámetro y no se lee del reloj para que el resultado sea
/// reproducible: un diagnóstico que cambia entre dos llamadas del mismo minuto
/// no se puede probar ni comparar.
pub async fn diagnosticar(pool: &Pool, hoy: &str) -> Resultado<Diagnostico> {
    let mut puntos = Vec::new();
    let mut resumen = Resumen::default();

    let contar = |sql: &'static str| async move {
        sqlx::query_as::<_, (i64,)>(sql)
            .fetch_one(pool)
            .await
            .map(|(n,)| n)
    };

    // --- Empresa ----------------------------------------------------------
    //
    // La tupla lleva nombre para que se lea: cinco columnas anónimas en la firma
    // se confunden entre sí a los tres meses.
    type DatosEmpresa = (String, String, String, Option<String>, Option<String>);
    let empresa: Option<DatosEmpresa> = sqlx::query_as(
        "SELECT nombre, nit, ambiente_dian, pin_software, id_software FROM empresa WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;

    match &empresa {
        None => puntos.push(punto(
            Gravedad::Bloqueante,
            "La empresa no está configurada: el sistema no sabe a nombre de quién factura",
            "Configuración → Empresa: nombre, NIT y dirección. El dígito de verificación \
             lo calcula el sistema.",
            None,
        )),
        Some((nombre, nit, ambiente, pin, id_sw)) => {
            if nombre.trim().is_empty() || nit.trim().is_empty() {
                puntos.push(punto(
                    Gravedad::Bloqueante,
                    "La empresa está a medio configurar (falta nombre o NIT)",
                    "Configuración → Empresa.",
                    None,
                ));
            }
            if ambiente == "pruebas" {
                puntos.push(punto(
                    Gravedad::BloqueaFacturaElectronica,
                    "El ambiente DIAN está en PRUEBAS: los documentos que se emitan no \
                     tienen validez fiscal",
                    "Se cambia a producción SOLO después de aprobar el set de pruebas y de \
                     que la DIAN habilite al facturador. Ver docs/DIAN.md.",
                    None,
                ));
            }
            if pin.as_deref().unwrap_or("").trim().is_empty() {
                puntos.push(punto(
                    Gravedad::BloqueaFacturaElectronica,
                    "Falta el PIN del software: sin él no se puede calcular el CUDE de las \
                     notas crédito y débito",
                    "Lo asigna la DIAN al registrar el software propio, en el portal de \
                     habilitación. Configuración → Empresa → PIN del software.",
                    None,
                ));
            }
            if id_sw.as_deref().unwrap_or("").trim().is_empty() {
                puntos.push(punto(
                    Gravedad::BloqueaFacturaElectronica,
                    "Falta el identificador del software en el catálogo de participantes \
                     de la DIAN",
                    "Se obtiene junto con el PIN al registrar el software propio.",
                    None,
                ));
            }
        }
    }

    // --- Usuarios ---------------------------------------------------------
    resumen.usuarios_activos = contar("SELECT COUNT(*) FROM usuario WHERE activo = 1").await?;
    let admins = contar("SELECT COUNT(*) FROM usuario WHERE activo = 1 AND rol = 'admin'").await?;
    if admins == 0 {
        puntos.push(punto(
            Gravedad::Bloqueante,
            "No hay ningún administrador activo: nadie puede configurar el sistema",
            "Corra `inventario-api instalar` con INVENTARIO_CLAVE_ADMIN.",
            None,
        ));
    }
    let operativos = contar(
        "SELECT COUNT(*) FROM usuario WHERE activo = 1 AND rol IN ('cajero', 'bodega')",
    )
    .await?;
    if operativos == 0 {
        puntos.push(punto(
            Gravedad::Advertencia,
            "Solo hay administradores: no hay usuarios de mostrador ni de bodega",
            "Configuración → Usuarios. Que el dueño facture con el usuario de \
             administrador funciona, pero después no se puede saber quién hizo qué.",
            None,
        ));
    }

    // --- Conceptos de kardex ----------------------------------------------
    resumen.conceptos = contar("SELECT COUNT(*) FROM concepto WHERE activo = 1").await?;
    for (clase, para_que) in [
        ("venta", "facturar"),
        ("devolucion_venta", "anular facturas y hacer notas crédito con reintegro"),
        ("compra", "registrar compras"),
    ] {
        let n = sqlx::query_as::<_, (i64,)>(
            "SELECT COUNT(*) FROM concepto WHERE clase = ? AND activo = 1",
        )
        .bind(clase)
        .fetch_one(pool)
        .await?
        .0;
        if n == 0 {
            puntos.push(punto(
                Gravedad::Bloqueante,
                format!("No hay concepto de kardex de clase «{clase}»: no se puede {para_que}"),
                "Corra `inventario-api instalar`, que siembra los conceptos mínimos.",
                None,
            ));
        }
    }
    // Conceptos de salida sin efecto tributario decidido: los deja la migración
    // y son la puerta por la que se escapa el IVA del art. 421 lit. b).
    let sin_decidir = contar(
        "SELECT COUNT(*) FROM concepto
          WHERE activo = 1 AND signo = 'salida' AND efecto_tributario = 'no_afecta_impuestos'
            AND clase NOT IN ('venta', 'traslado')",
    )
    .await?;
    if sin_decidir > 0 {
        puntos.push(punto(
            Gravedad::Advertencia,
            "Hay conceptos de salida marcados como «no afecta impuestos» que podrían ser \
             retiros gravados",
            "Configuración → Conceptos. Un obsequio, un consumo interno o un retiro de \
             socios causa IVA sobre el valor comercial (ET art. 421 lit. b). Si alguno de \
             estos lo es, cámbielo a «retiro gravado» ANTES de operar: después habrá \
             movimientos sin impuesto.",
            Some(sin_decidir),
        ));
    }

    // --- Resoluciones -----------------------------------------------------
    let resoluciones = crate::resolucion::listar(pool, hoy).await?;
    let utilizables: Vec<_> = resoluciones.iter().filter(|r| r.impedimento.is_none()).collect();
    resumen.resoluciones_utilizables = utilizables.len() as i64;

    if resoluciones.is_empty() {
        puntos.push(punto(
            Gravedad::Bloqueante,
            "No hay ninguna resolución de facturación: no se puede emitir una factura",
            "Configuración → Resoluciones. El número, el rango y la vigencia los autoriza \
             la DIAN en el portal MUISCA.",
            None,
        ));
    } else if utilizables.is_empty() {
        puntos.push(punto(
            Gravedad::Bloqueante,
            "Todas las resoluciones están agotadas, vencidas o inactivas",
            "Solicite una autorización nueva en el portal de la DIAN y regístrela.",
            Some(resoluciones.len() as i64),
        ));
    } else {
        let por_agotarse = utilizables.iter().filter(|r| r.quedan <= 50).count() as i64;
        if por_agotarse > 0 {
            puntos.push(punto(
                Gravedad::Advertencia,
                "Alguna resolución tiene 50 números o menos",
                "La habilitación de rangos se pide con 15 días hábiles de antelación al \
                 vencimiento. No espere al último número.",
                Some(por_agotarse),
            ));
        }
        let sin_clave = utilizables
            .iter()
            .filter(|r| r.clave_tecnica.as_deref().unwrap_or("").trim().is_empty())
            .count() as i64;
        if sin_clave > 0 {
            puntos.push(punto(
                Gravedad::BloqueaFacturaElectronica,
                "Hay resoluciones sin clave técnica: sus facturas salen sin CUFE",
                "La clave técnica se consulta en el servicio de numeración de la DIAN, \
                 junto con el rango. Configuración → Resoluciones.",
                Some(sin_clave),
            ));
        }
    }

    // --- Catálogo ---------------------------------------------------------
    resumen.articulos = contar("SELECT COUNT(*) FROM articulo WHERE activo = 1").await?;
    resumen.terceros = contar("SELECT COUNT(*) FROM tercero WHERE activo = 1").await?;
    resumen.movimientos = contar("SELECT COUNT(*) FROM movimiento").await?;
    resumen.facturas = contar("SELECT COUNT(*) FROM factura").await?;

    if resumen.articulos == 0 {
        puntos.push(punto(
            Gravedad::Bloqueante,
            "No hay artículos: no hay qué vender",
            "Artículos → Nuevo, o migre desde el sistema anterior con \
             `inventario-api migrar <carpeta>`.",
            None,
        ));
    }
    if resumen.terceros == 0 {
        puntos.push(punto(
            Gravedad::Bloqueante,
            "No hay terceros: no hay a quién facturar",
            "Terceros → Nuevo, o migre desde el sistema anterior.",
            None,
        ));
    }
    if resumen.articulos > 0 && resumen.movimientos == 0 {
        puntos.push(punto(
            Gravedad::Advertencia,
            "Hay artículos pero el inventario está en cero: nada ha entrado todavía",
            "Registre las existencias iniciales en Inventario → Movimiento, con el \
             concepto «Saldo inicial». Facturar sin existencia se rechaza.",
            Some(resumen.articulos),
        ));
    }

    // Artículos incompletos para factura electrónica.
    let sin_unidad = contar(
        "SELECT COUNT(*) FROM articulo
          WHERE activo = 1 AND (unidad_medida IS NULL OR trim(unidad_medida) = '')",
    )
    .await?;
    if sin_unidad > 0 {
        puntos.push(punto(
            Gravedad::BloqueaFacturaElectronica,
            "Hay artículos sin unidad de medida, que la factura electrónica exige por ítem",
            "Artículos → editar → Unidad de medida. Los códigos los publica la DIAN en la \
             tabla 13.3.6 de su caja de herramientas.",
            Some(sin_unidad),
        ));
    }
    let sin_estandar = contar(
        "SELECT COUNT(*) FROM articulo WHERE activo = 1 AND tipo_estandar IS NULL",
    )
    .await?;
    if sin_estandar > 0 {
        puntos.push(punto(
            Gravedad::BloqueaFacturaElectronica,
            "Hay artículos sin código de producto bajo un estándar declarado",
            "Artículos → editar → Código de barras o UNSPSC. Si el código no pasa el \
             dígito de control de GS1, el sistema lo declara como código interno (999), \
             que es válido.",
            Some(sin_estandar),
        ));
    }

    // Deterioro: sin precio estimado no se puede medir el VNR (NIC 2.28-33).
    let sin_estimacion = contar(
        "SELECT COUNT(*) FROM articulo
          WHERE activo = 1 AND precio_venta_estimado IS NULL",
    )
    .await?;
    if sin_estimacion > 0 && resumen.articulos > 0 {
        puntos.push(punto(
            Gravedad::Advertencia,
            "Hay artículos sin precio de venta estimado: el corte de deterioro no los puede \
             medir",
            "Artículos → editar → Precio de venta estimado y costos para vender. Sin ellos \
             el inventario se lleva siempre al costo, que incumple la NIC 2 en cuanto el \
             mercado baje.",
            Some(sin_estimacion),
        ));
    }

    // --- Terceros con datos que impiden facturarles -----------------------
    let sin_nit = contar("SELECT COUNT(*) FROM tercero WHERE activo = 1 AND trim(nit) = ''").await?;
    if sin_nit > 0 {
        puntos.push(punto(
            Gravedad::BloqueaFacturaElectronica,
            "Hay terceros sin NIT: sus facturas no pueden llevar CUFE",
            "Terceros → editar. El NIT del adquirente entra en el cálculo del CUFE.",
            Some(sin_nit),
        ));
    }

    puntos.sort_by_key(|p| p.gravedad);

    Ok(Diagnostico {
        puede_operar: !puntos.iter().any(|p| p.gravedad == Gravedad::Bloqueante),
        puede_facturar_electronicamente: !puntos
            .iter()
            .any(|p| p.gravedad != Gravedad::Advertencia),
        puntos,
        inventario: resumen,
    })
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::prueba_apoyo;
    use inventario_dominio::{Cantidad, Centavos};

    const HOY: &str = "2026-08-02";

    fn tiene(d: &Diagnostico, aguja: &str) -> bool {
        d.puntos.iter().any(|p| p.que.contains(aguja))
    }

    /// Una base recién migrada NO puede operar, y el diagnóstico dice
    /// exactamente por qué. Esta es la razón de ser del módulo.
    #[tokio::test]
    async fn una_base_vacia_no_puede_operar_y_dice_todo_lo_que_falta() {
        let pool = crate::abrir("sqlite::memory:").await.unwrap();
        let d = diagnosticar(&pool, HOY).await.unwrap();

        assert!(!d.puede_operar);
        assert!(!d.puede_facturar_electronicamente);
        assert!(tiene(&d, "empresa no está configurada"));
        assert!(tiene(&d, "administrador activo"));
        assert!(tiene(&d, "concepto de kardex de clase «venta»"));
        assert!(tiene(&d, "No hay ninguna resolución"));
        assert!(tiene(&d, "No hay artículos"));
        assert!(tiene(&d, "No hay terceros"));

        // Y cada punto dice QUÉ HACER, no solo qué falta.
        for p in &d.puntos {
            assert!(!p.como.trim().is_empty(), "«{}» no dice cómo resolverse", p.que);
        }
    }

    /// El camino completo: se va configurando y los puntos van desapareciendo,
    /// en el orden en que un instalador los resolvería.
    #[tokio::test]
    async fn configurar_paso_a_paso_va_apagando_los_bloqueos() {
        let pool = prueba_apoyo::base().await; // empresa + admin + conceptos

        let d = diagnosticar(&pool, HOY).await.unwrap();
        assert!(!d.puede_operar, "todavía faltan resolución, artículos y terceros");
        assert!(!tiene(&d, "empresa no está configurada"));
        assert!(!tiene(&d, "administrador activo"));

        sqlx::query(
            "INSERT INTO resolucion (nombre, prefijo, desde, hasta) VALUES ('R','FE',1,1000)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let (a,): (i64,) = sqlx::query_as(
            "INSERT INTO articulo (codigo, descripcion) VALUES ('A1','Botellón') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO tercero (nit, dv, nombre) VALUES ('43627185',1,'Cliente')")
            .execute(&pool)
            .await
            .unwrap();

        let d = diagnosticar(&pool, HOY).await.unwrap();
        assert!(d.puede_operar, "ya no debería quedar nada bloqueante: {:?}", d.puntos);
        assert_eq!(d.inventario.articulos, 1);
        assert_eq!(d.inventario.resoluciones_utilizables, 1);

        // Pero avisa de que el inventario está en cero.
        assert!(tiene(&d, "inventario está en cero"));
        crate::kardex::registrar(
            &pool,
            &crate::kardex::Peticion {
                fecha: HOY.into(),
                concepto_id: prueba_apoyo::COMPRA,
                articulo_id: a,
                tercero_id: None,
                documento: String::new(),
                cantidad: Cantidad::de_unidades(10),
                costo_unitario: Centavos::de_pesos(1000),
                factura_id: None,
                usuario_id: None,
                valor_comercial_unitario: None,
            },
        )
        .await
        .unwrap();
        let d = diagnosticar(&pool, HOY).await.unwrap();
        assert!(!tiene(&d, "inventario está en cero"));
    }

    /// Poder OPERAR y poder FACTURAR ELECTRÓNICAMENTE son dos cosas distintas, y
    /// el diagnóstico no las mezcla: un negocio puede arrancar en papel mientras
    /// termina la habilitación.
    #[tokio::test]
    async fn se_puede_operar_sin_poder_facturar_electronicamente() {
        let pool = prueba_apoyo::base().await;
        sqlx::query("INSERT INTO resolucion (nombre, prefijo, desde, hasta) VALUES ('R','',1,1000)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO articulo (codigo, descripcion) VALUES ('A1','Bot')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO tercero (nit, dv, nombre) VALUES ('1',9,'Cli')")
            .execute(&pool)
            .await
            .unwrap();

        let d = diagnosticar(&pool, HOY).await.unwrap();
        assert!(d.puede_operar);
        assert!(!d.puede_facturar_electronicamente);
        // Y dice exactamente qué falta para lo electrónico.
        assert!(tiene(&d, "ambiente DIAN está en PRUEBAS"));
        assert!(tiene(&d, "PIN del software"));
        assert!(tiene(&d, "sin clave técnica"));
        assert!(tiene(&d, "sin unidad de medida"));
        assert!(tiene(&d, "sin código de producto"));

        // Ninguno de esos es bloqueante para operar.
        for p in &d.puntos {
            if p.que.contains("PIN") || p.que.contains("clave técnica") {
                assert_eq!(p.gravedad, Gravedad::BloqueaFacturaElectronica);
            }
        }
    }

    /// El camino completo hasta poder facturar electrónicamente.
    #[tokio::test]
    async fn con_todo_configurado_se_puede_facturar_electronicamente() {
        let pool = prueba_apoyo::base().await;
        sqlx::query(
            "UPDATE empresa SET ambiente_dian = 'produccion', pin_software = '12301',
                    id_software = 'abc-123'",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO resolucion (nombre, prefijo, desde, hasta, clave_tecnica)
             VALUES ('R','FE',1,1000,'693ff6f2a553c3646a063436fd4dd9ded0311471')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO articulo (codigo, descripcion, unidad_medida, codigo_estandar,
                                   tipo_estandar, precio_venta_estimado, costos_para_vender)
             VALUES ('A1','Bot','94','4006381333931','gtin',100000,0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO tercero (nit, dv, nombre) VALUES ('43627185',1,'Cli')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO usuario (codigo, nombre, hash_clave, rol)
                     VALUES ('caj','Cajera','x','cajero')")
            .execute(&pool)
            .await
            .unwrap();

        let d = diagnosticar(&pool, HOY).await.unwrap();
        assert!(d.puede_operar);
        assert!(
            d.puede_facturar_electronicamente,
            "quedaron bloqueos: {:?}",
            d.puntos
                .iter()
                .filter(|p| p.gravedad != Gravedad::Advertencia)
                .collect::<Vec<_>>()
        );
    }

    /// Un concepto de salida sin efecto tributario decidido —lo que deja la
    /// migración— se avisa ANTES de operar, no después de 161 movimientos sin
    /// IVA.
    #[tokio::test]
    async fn avisa_de_los_conceptos_de_salida_sin_efecto_tributario_decidido() {
        let pool = prueba_apoyo::base().await;
        let d = diagnosticar(&pool, HOY).await.unwrap();
        // El sembrado de prueba trae «Salidas por ajuste», de clase ajuste.
        assert!(tiene(&d, "retiros gravados"), "{:?}", d.puntos);
        let p = d.puntos.iter().find(|p| p.que.contains("retiros gravados")).unwrap();
        assert!(p.como.contains("421"), "el aviso no cita la norma: {}", p.como);
        assert!(p.cuantos.is_some_and(|n| n > 0));
    }

    /// Una resolución agotada bloquea, y lo dice distinto de «no hay ninguna».
    #[tokio::test]
    async fn una_resolucion_agotada_bloquea_y_se_distingue_de_no_tener_ninguna() {
        let pool = prueba_apoyo::base().await;
        sqlx::query(
            "INSERT INTO resolucion (nombre, prefijo, desde, hasta, ultimo_emitido)
             VALUES ('R','FE',1,10,10)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let d = diagnosticar(&pool, HOY).await.unwrap();
        assert!(tiene(&d, "agotadas, vencidas o inactivas"));
        assert!(!tiene(&d, "No hay ninguna resolución"));
        assert!(!d.puede_operar);
    }

    #[tokio::test]
    async fn avisa_cuando_a_una_resolucion_le_quedan_pocos_numeros() {
        let pool = prueba_apoyo::base().await;
        sqlx::query(
            "INSERT INTO resolucion (nombre, prefijo, desde, hasta, ultimo_emitido)
             VALUES ('R','FE',1,1000,970)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let d = diagnosticar(&pool, HOY).await.unwrap();
        let p = d
            .puntos
            .iter()
            .find(|p| p.que.contains("50 números o menos"))
            .expect("tenía que avisar");
        assert_eq!(p.gravedad, Gravedad::Advertencia);
        assert!(p.como.contains("15 días"), "no dice el plazo real: {}", p.como);
    }

    /// Los puntos salen ordenados por gravedad: lo que impide operar primero.
    #[tokio::test]
    async fn los_puntos_salen_ordenados_por_gravedad() {
        let pool = crate::abrir("sqlite::memory:").await.unwrap();
        let d = diagnosticar(&pool, HOY).await.unwrap();
        let gravedades: Vec<_> = d.puntos.iter().map(|p| p.gravedad).collect();
        let mut ordenadas = gravedades.clone();
        ordenadas.sort();
        assert_eq!(gravedades, ordenadas);
        assert_eq!(gravedades[0], Gravedad::Bloqueante);
    }
}
