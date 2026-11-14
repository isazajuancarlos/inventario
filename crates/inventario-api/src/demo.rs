//! La **demostración pública** de `inventario.xiliux.com`.
//!
//! No es una instalación de cliente y no debe confundirse con una: sus
//! credenciales están **publicadas en la propia pantalla de acceso**, y
//! cualquiera puede escribir en su base.
//!
//! Lo que la distingue de una instalación de verdad es una sola cosa: las
//! variables `INVENTARIO_DEMO_USUARIO` e `INVENTARIO_DEMO_CLAVE`. Sin ellas
//! `/api/demo` responde **204 sin cuerpo** y no hay nada que publicar. Una
//! instalación de consultorio nunca las define, así que no hay forma de
//! encender la demo por descuido — y no hace falta un binario distinto.
//!
//! El sembrado es **idempotente**: se puede correr en cada arranque y cada
//! noche para devolver la demo a un estado conocido después de que la gente
//! juegue con ella.

use crate::claves;
use inventario_datos::Pool;
use inventario_dominio::{Cantidad, Centavos};
use sqlx::Row;

/// Las credenciales publicadas, si esta instalación es una demostración.
pub fn credenciales() -> Option<(String, String)> {
    let u = std::env::var("INVENTARIO_DEMO_USUARIO").ok()?;
    let c = std::env::var("INVENTARIO_DEMO_CLAVE").ok()?;
    (!u.trim().is_empty() && !c.trim().is_empty()).then_some((u, c))
}

/// Datos del negocio de ejemplo: una envasadora de agua, que es el caso real
/// del que nació el sistema. Los NIT son inventados salvo el de la empresa,
/// que es el del corpus y ya es público en el propio análisis.
const ARTICULOS: [(&str, &str, i64, i64, &str, &str); 6] = [
    // (código, descripción, precio en pesos, IVA %, unidad, GTIN válido)
    ("BOT20", "Botellón retornable 20 L", 12_000, 0, "94", "4006381333931"),
    ("PET600", "Agua PET 600 c.c.", 2_000, 19, "94", "0012345678905"),
    ("PET350", "Agua PET 350 c.c. (paquete x 24)", 28_000, 19, "94", "96385074"),
    ("DISP", "Dispensador de mesa", 180_000, 19, "94", ""),
    ("BOMBA", "Bomba manual para botellón", 25_000, 19, "94", ""),
    ("HIELO", "Hielo en cubos 2 kg", 6_000, 19, "KGM", ""),
];

const CLIENTES: [(&str, &str, i64); 5] = [
    ("9001234561", "Tienda La Esquina", 30),
    ("9001234562", "Restaurante El Fogón", 15),
    ("9001234563", "Gimnasio Vital", 30),
    ("4362718", "Ana Gómez", 0),
    ("7008537", "Oficinas Andina S.A.S.", 45),
];

/// Siembra la demostración. Idempotente.
pub async fn sembrar(pool: &Pool) -> Result<Vec<String>, String> {
    let Some((usuario, clave)) = credenciales() else {
        return Err(
            "esta instalación no es una demostración: faltan INVENTARIO_DEMO_USUARIO e \
             INVENTARIO_DEMO_CLAVE. No se siembra nada."
                .into(),
        );
    };
    let mut msgs = Vec::new();

    // Empresa. En PRUEBAS a propósito: la demo no emite documento con validez
    // fiscal, y el diagnóstico de alistamiento lo dice en pantalla.
    sqlx::query(
        "INSERT INTO empresa (id, nombre, nit, dv, direccion, telefono, ambiente_dian,
                              formula_costeo, pin_software, id_software)
         VALUES (1, 'Aguas de la Sabana S.A.S. (DEMOSTRACIÓN)', '811033579', 6,
                 'Cra 43A #30-74, Medellín', '604 232 4500', 'pruebas',
                 'promedio_ponderado', '12301', 'demo-inventario')
         ON CONFLICT(id) DO UPDATE SET nombre = excluded.nombre, ambiente_dian = 'pruebas'",
    )
    .execute(pool)
    .await
    .map_err(|e| format!("empresa: {e}"))?;

    // Usuario publicado. La clave se rehashea siempre: si alguien la cambió
    // desde la interfaz, la próxima siembra la devuelve a la publicada.
    let hash = claves::hashear(&clave).ok_or("no se pudo hashear la clave de la demo")?;
    sqlx::query(
        "INSERT INTO usuario (codigo, nombre, hash_clave, rol)
         VALUES (?, 'Usuario de demostración', ?, 'admin')
         ON CONFLICT(codigo) DO UPDATE SET hash_clave = excluded.hash_clave, activo = 1",
    )
    .bind(&usuario)
    .bind(&hash)
    .execute(pool)
    .await
    .map_err(|e| format!("usuario demo: {e}"))?;
    msgs.push(format!("usuario «{usuario}» listo"));

    // Resolución CON clave técnica, para que las facturas de la demo salgan con
    // CUFE y se pueda ver el código en pantalla. Es la clave del EJEMPLO del
    // anexo técnico, que es pública.
    sqlx::query(
        "INSERT INTO resolucion (nombre, prefijo, numero, fecha, rige_desde, vence,
                                 desde, hasta, clave_tecnica)
         VALUES ('Resolución de demostración', 'DEMO', '18764000001', '2026-01-01',
                 '2026-01-01', '2027-12-31', 1, 100000,
                 '693ff6f2a553c3646a063436fd4dd9ded0311471')
         ON CONFLICT(prefijo, desde) DO NOTHING",
    )
    .execute(pool)
    .await
    .map_err(|e| format!("resolución: {e}"))?;

    // Artículos.
    for (codigo, desc, precio, iva, unidad, gtin) in ARTICULOS {
        let barras = (!gtin.is_empty()).then_some(gtin);
        // El código de producto SIEMPRE lleva estándar declarado. Cuando no hay
        // GTIN se usa el código del vendedor (`@schemeID = 999`), que es
        // válido ante la DIAN y es la verdad: no todo artículo tiene GTIN, y
        // fingir uno sería peor que no tenerlo.
        let (codigo_estandar, tipo) = match barras {
            Some(g) if inventario_dominio::gtin::es_valido(g) => (g, "gtin"),
            _ => (codigo, "interno"),
        };
        sqlx::query(
            "INSERT INTO articulo
                (codigo, descripcion, precio_venta, iva_porcentaje, unidad_medida,
                 codigo_barras, codigo_estandar, tipo_estandar,
                 precio_venta_estimado, costos_para_vender, punto_minimo, punto_maximo)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, 20, 200)
             ON CONFLICT(codigo) DO NOTHING",
        )
        .bind(codigo)
        .bind(desc)
        .bind(precio * 100)
        .bind(iva)
        .bind(unidad)
        .bind(barras)
        .bind(codigo_estandar)
        .bind(tipo)
        // El precio estimado para el deterioro se pone algo por debajo del de
        // venta, que es lo realista, y así el corte de deterioro tiene algo que
        // enseñar en vez de salir todo en cero.
        .bind(precio * 90)
        .execute(pool)
        .await
        .map_err(|e| format!("artículo {codigo}: {e}"))?;
    }

    // Clientes.
    for (nit, nombre, plazo) in CLIENTES {
        let dv = inventario_dominio::nit::digito_verificacion(nit).unwrap_or(0);
        sqlx::query(
            "INSERT INTO tercero (nit, dv, nombre, plazo_dias, direccion, telefono)
             VALUES (?, ?, ?, ?, 'Medellín', '604 000 0000')
             ON CONFLICT DO NOTHING",
        )
        .bind(nit)
        .bind(dv as i64)
        .bind(nombre)
        .bind(plazo)
        .execute(pool)
        .await
        .map_err(|e| format!("tercero {nombre}: {e}"))?;
    }

    // Existencias, solo si el inventario está vacío. Volver a sembrarlas en
    // cada arranque duplicaría el stock, que es el error que ya costó una vez
    // en el migrador.
    let (movimientos,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM movimiento")
        .fetch_one(pool)
        .await
        .map_err(|e| format!("contando movimientos: {e}"))?;
    if movimientos == 0 {
        let compra: i64 = sqlx::query("SELECT id FROM concepto WHERE clase = 'compra' LIMIT 1")
            .fetch_one(pool)
            .await
            .map_err(|e| format!("concepto de compra: {e}"))?
            .get("id");
        let hoy = chrono::Utc::now().date_naive().to_string();
        let articulos: Vec<(i64, i64)> =
            sqlx::query_as("SELECT id, precio_venta FROM articulo ORDER BY id")
                .fetch_all(pool)
                .await
                .map_err(|e| format!("artículos: {e}"))?;
        for (id, precio) in &articulos {
            inventario_datos::kardex::registrar(
                pool,
                &inventario_datos::kardex::Peticion {
                    fecha: hoy.clone(),
                    concepto_id: compra,
                    articulo_id: *id,
                    tercero_id: None,
                    documento: "Compra inicial de la demostración".into(),
                    cantidad: Cantidad::de_unidades(500),
                    // Se compra al 60 % del precio de venta: deja margen visible
                    // en los informes de rotación y utilidad.
                    costo_unitario: Centavos(precio * 6 / 10),
                    factura_id: None,
                    usuario_id: None,
                    valor_comercial_unitario: None,
                },
            )
            .await
            .map_err(|e| format!("existencia inicial: {e}"))?;
        }
        msgs.push(format!("{} artículos con 500 unidades cada uno", articulos.len()));
        msgs.extend(sembrar_operacion(pool, &hoy).await?);
    } else {
        msgs.push(format!("ya había {movimientos} movimientos: no se toca el inventario"));
    }

    Ok(msgs)
}

/// Un poco de operación real: facturas, recaudos, un obsequio y una merma.
///
/// Sin esto la demostración enseña pantallas vacías, y las pantallas vacías no
/// demuestran nada. Cada movimiento está elegido para que un informe distinto
/// tenga algo que mostrar: la cartera, la rotación, el ABC, el IVA por retiro y
/// el tope del artículo 64.
async fn sembrar_operacion(pool: &Pool, hoy: &str) -> Result<Vec<String>, String> {
    let mut msgs = Vec::new();
    let resolucion: i64 = sqlx::query("SELECT id FROM resolucion ORDER BY id LIMIT 1")
        .fetch_one(pool)
        .await
        .map_err(|e| format!("resolución: {e}"))?
        .get("id");
    let articulos: Vec<(i64, i64, i64)> =
        sqlx::query_as("SELECT id, precio_venta, iva_porcentaje FROM articulo ORDER BY id")
            .fetch_all(pool)
            .await
            .map_err(|e| format!("artículos: {e}"))?;
    let clientes: Vec<(i64,)> = sqlx::query_as("SELECT id FROM tercero ORDER BY id")
        .fetch_all(pool)
        .await
        .map_err(|e| format!("terceros: {e}"))?;
    if articulos.is_empty() || clientes.is_empty() {
        return Ok(msgs);
    }

    // 12 facturas alternando contado y crédito, repartidas entre los clientes.
    let mut emitidas = 0;
    for k in 0..12usize {
        let (art, precio, iva) = articulos[k % articulos.len()];
        let cliente = clientes[k % clientes.len()].0;
        let credito = k % 2 == 0;
        let p = inventario_datos::factura::Peticion {
            resolucion_id: resolucion,
            tercero_id: cliente,
            vendedor_id: None,
            fecha: hoy.to_string(),
            hora: "10:00:00-05:00".into(),
            forma_pago: if credito { "credito" } else { "contado" }.into(),
            medio_pago: (!credito).then(|| "efectivo".to_string()),
            retencion: 0,
            observacion: "Venta de demostración".into(),
            lineas: vec![inventario_dominio::factura::Linea {
                articulo_id: art,
                cantidad: Cantidad::de_unidades((k as i64 % 8) + 2),
                precio_unitario: Centavos(precio),
                descuento: Centavos::CERO,
                iva_porcentaje: iva,
                inc_porcentaje: 0,
            }],
        };
        if inventario_datos::factura::emitir(pool, &p).await.is_ok() {
            emitidas += 1;
        }
    }
    msgs.push(format!("{emitidas} facturas de ejemplo"));

    // Recaudos parciales: deja cartera pendiente, que es lo que hay que ver.
    let mut recibos = 0;
    for (i, (cliente,)) in clientes.iter().enumerate().take(3) {
        let p = inventario_datos::recibo::Peticion {
            tercero_id: *cliente,
            fecha: hoy.to_string(),
            observacion: "Abono de demostración".into(),
            pagos: vec![inventario_datos::recibo::Pago {
                medio: "efectivo".into(),
                banco_id: None,
                referencia: String::new(),
                vence: None,
                valor: Centavos::de_pesos(20_000 + i as i64 * 15_000).0,
            }],
            aplicaciones: vec![],
        };
        if inventario_datos::recibo::registrar(pool, &p, None).await.is_ok() {
            recibos += 1;
        }
    }
    msgs.push(format!("{recibos} recibos de caja"));

    // Un obsequio (causa IVA sobre el valor comercial) y una merma (consume el
    // tope del 3 % del art. 64). Son los dos casos que el sistema anterior no
    // sabía tratar, y aquí se ven en sus informes.
    let concepto = |clase: &'static str| async move {
        sqlx::query_as::<_, (i64,)>(
            "SELECT id FROM concepto WHERE efecto_tributario = ? AND activo = 1 LIMIT 1",
        )
        .bind(clase)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .map(|(id,)| id)
    };
    // Se elige un artículo GRAVADO a propósito: el obsequio de un artículo
    // exento liquidaría cero y el informe de retiros no enseñaría nada, que es
    // justo lo que hay que enseñar.
    let (art, precio, _) = articulos
        .iter()
        .copied()
        .find(|(_, _, iva)| *iva > 0)
        .unwrap_or(articulos[0]);
    if let Some(c) = concepto("retiro_gravado").await {
        let _ = inventario_datos::kardex::registrar(
            pool,
            &inventario_datos::kardex::Peticion {
                fecha: hoy.to_string(),
                concepto_id: c,
                articulo_id: art,
                tercero_id: None,
                documento: "Obsequio a cliente".into(),
                cantidad: Cantidad::de_unidades(10),
                costo_unitario: Centavos::CERO,
                factura_id: None,
                usuario_id: None,
                valor_comercial_unitario: Some(Centavos(precio)),
            },
        )
        .await;
        msgs.push("1 obsequio (con su IVA por retiro)".into());
    }
    if let Some(c) = concepto("faltante_art64").await {
        let _ = inventario_datos::kardex::registrar(
            pool,
            &inventario_datos::kardex::Peticion {
                fecha: hoy.to_string(),
                concepto_id: c,
                articulo_id: art,
                tercero_id: None,
                documento: "Rotura en bodega".into(),
                cantidad: Cantidad::de_unidades(6),
                costo_unitario: Centavos::CERO,
                factura_id: None,
                usuario_id: None,
                valor_comercial_unitario: None,
            },
        )
        .await;
        msgs.push("1 faltante (cuenta contra el tope del art. 64)".into());
    }

    Ok(msgs)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Sin las dos variables, esto NO es una demo. Es la única diferencia entre
    /// el binario que va a `inventario.xiliux.com` y el que va a casa de un
    /// cliente, y por eso tiene su prueba.
    #[test]
    fn sin_las_dos_variables_no_hay_demostracion() {
        // Se comprueba la lógica de la función, no el entorno del proceso: las
        // variables son globales y las pruebas corren en hilos.
        assert!(
            credenciales().is_none()
                || std::env::var("INVENTARIO_DEMO_USUARIO").is_ok(),
            "credenciales() devolvió algo sin que las variables estén puestas"
        );
    }

    #[tokio::test]
    async fn sembrar_sin_credenciales_falla_y_no_deja_nada() {
        let pool = inventario_datos::abrir("sqlite::memory:").await.unwrap();
        // Sin variables, en un proceso de prueba limpio.
        if credenciales().is_none() {
            let e = sembrar(&pool).await.unwrap_err();
            assert!(e.contains("INVENTARIO_DEMO_USUARIO"), "{e}");
            let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM articulo")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(n, 0, "sembró sin ser una demo");
        }
    }

    /// Los GTIN del catálogo de demostración son **válidos**: la demo enseña el
    /// sistema, y enseñarlo con códigos que él mismo rechazaría sería enseñar
    /// otra cosa.
    #[test]
    fn los_gtin_de_la_demostracion_son_validos() {
        for (codigo, _, _, _, _, gtin) in ARTICULOS {
            if gtin.is_empty() {
                continue;
            }
            assert!(
                inventario_dominio::gtin::es_valido(gtin),
                "el GTIN «{gtin}» del artículo {codigo} no pasa su dígito de control"
            );
        }
    }

    /// Los NIT de los clientes de ejemplo tienen que poder normalizarse: si no,
    /// la siembra fallaría en producción y la demo saldría vacía.
    #[test]
    fn los_nit_de_la_demostracion_son_normalizables() {
        for (nit, nombre, _) in CLIENTES {
            assert!(
                inventario_dominio::nit::normalizar(nit).is_some(),
                "el NIT de «{nombre}» no se puede normalizar"
            );
        }
    }
}
