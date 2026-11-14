//! Migración del sistema Visual FoxPro.
//!
//! **La regla del módulo**: lo que no se puede mapear se RECHAZA con motivo en
//! el informe; nunca se rellena con un valor por defecto (directiva 20). Un
//! cliente huérfano no se convierte en «CLIENTE VARIOS», una fecha imposible no
//! se convierte en hoy, y un concepto vacío no se convierte en entrada.
//!
//! El informe es el producto principal, no un accesorio: dice exactamente qué
//! del sistema viejo no cabe en el nuevo y por qué, y es lo que se le enseña al
//! cliente antes de arrancar en firme.

use crate::dbf::{self, Fila};
use inventario_dominio::{Cantidad, Centavos, nit};
use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;

#[derive(Debug, Default)]
pub struct Informe {
    pub lineas: Vec<String>,
    pub rechazos: Vec<Rechazo>,
    pub cargados: Vec<(String, usize)>,
}

#[derive(Debug)]
pub struct Rechazo {
    pub tabla: String,
    pub clave: String,
    pub motivo: String,
}

impl Informe {
    fn nota(&mut self, s: impl Into<String>) {
        self.lineas.push(s.into());
    }
    fn rechazo(&mut self, tabla: &str, clave: impl Into<String>, motivo: impl Into<String>) {
        self.rechazos.push(Rechazo {
            tabla: tabla.into(),
            clave: clave.into(),
            motivo: motivo.into(),
        });
    }
    fn cargado(&mut self, tabla: &str, n: usize) {
        self.cargados.push((tabla.into(), n));
    }

    pub fn imprimir(&self) {
        println!("\n=== MIGRACIÓN ===");
        for l in &self.lineas {
            println!("{l}");
        }
        println!("\n--- cargado ---");
        for (t, n) in &self.cargados {
            println!("  {t:<24} {n}");
        }
        println!("\n--- rechazado: {} filas ---", self.rechazos.len());
        // Se agrupa por motivo: 34 líneas iguales no informan más que una con
        // su cuenta, y esconden los motivos que salen una sola vez.
        let mut por_motivo: HashMap<(String, String), Vec<String>> = HashMap::new();
        for r in &self.rechazos {
            por_motivo
                .entry((r.tabla.clone(), r.motivo.clone()))
                .or_default()
                .push(r.clave.clone());
        }
        let mut grupos: Vec<_> = por_motivo.into_iter().collect();
        grupos.sort_by_key(|((t, m), v)| (t.clone(), std::cmp::Reverse(v.len()), m.clone()));
        for ((tabla, motivo), claves) in grupos {
            println!("  [{tabla}] {motivo}  ({})", claves.len());
            let muestra: Vec<&String> = claves.iter().take(5).collect();
            println!(
                "        {}{}",
                muestra
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                if claves.len() > 5 { ", …" } else { "" }
            );
        }
        if self.rechazos.is_empty() {
            println!("  (ninguna)");
        }
    }
}

fn t(f: &Fila, campo: &str) -> String {
    f.get(campo).map(|v| v.texto().trim().to_string()).unwrap_or_default()
}
fn n(f: &Fila, campo: &str) -> Option<f64> {
    f.get(campo).and_then(|v| v.numero())
}
fn fecha(f: &Fila, campo: &str) -> Option<String> {
    f.get(campo).and_then(|v| v.fecha()).map(str::to_string)
}

/// Pesos con decimales a centavos, redondeando. `409.00` → `40900`.
fn centavos(v: f64) -> Centavos {
    Centavos((v * 100.0).round() as i64)
}
/// Unidades con decimales a milésimas.
fn milesimas(v: f64) -> Cantidad {
    Cantidad((v * 1000.0).round() as i64)
}

/// Cuenta las filas BORRADAS de cada tabla del origen. No se migran —están
/// borradas a propósito— pero el cliente tiene que saber que existían: si
/// mañana echa de menos 2.596 precios de convenio, la respuesta es que ya no
/// estaban, no que se perdieron en la migración.
fn contar_borradas(base: &Path, inf: &mut Informe) {
    for tabla in [
        "ARTICULOS", "CLIENTES", "valorconvenio", "ENCRECIBO", "DETRECIBO",
        "CATEGORIAS", "zonas", "USUARIOS", "pedidos", "DESPACHOS",
    ] {
        if let Some(t) = abrir_tabla(base, tabla)
            && t.borradas > 0
        {
            inf.nota(format!(
                "{tabla}: {} filas venían BORRADAS en el origen y no se migran ({} vivas)",
                t.borradas,
                t.filas.len()
            ));
        }
    }
}

fn abrir_tabla(base: &Path, nombre: &str) -> Option<dbf::Tabla> {
    // El corpus mezcla mayúsculas y minúsculas en los nombres de archivo
    // (`ARTICULOS.DBF` pero `valorconvenio.dbf`), y en Linux eso importa.
    for variante in [
        nombre.to_string(),
        nombre.to_uppercase(),
        nombre.to_lowercase(),
    ] {
        for ext in [".DBF", ".dbf"] {
            let p = base.join(format!("{}{ext}", variante.trim_end_matches(".DBF").trim_end_matches(".dbf")));
            if p.exists() {
                return dbf::leer(&p).ok();
            }
        }
    }
    None
}

pub async fn ejecutar(carpeta: Option<&str>) -> ExitCode {
    let Some(carpeta) = carpeta else {
        eprintln!("uso: inventario-api migrar <carpeta-con-los-DBF>");
        eprintln!("     La base destino sale de INVENTARIO_BD.");
        return ExitCode::FAILURE;
    };
    let base = Path::new(carpeta);
    if !base.is_dir() {
        eprintln!("«{carpeta}» no es una carpeta");
        return ExitCode::FAILURE;
    }
    let url = std::env::var("INVENTARIO_BD").unwrap_or_else(|_| "sqlite://inventario.db".into());
    let pool = match inventario_datos::abrir(&url).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("no se pudo abrir la base destino ({url}): {e}");
            return ExitCode::FAILURE;
        }
    };

    match migrar(&pool, base).await {
        Ok(informe) => {
            informe.imprimir();
            println!(
                "\nLa base quedó en {url}. Revise los rechazos ANTES de operar: \
                 lo rechazado NO está en el sistema nuevo."
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("la migración falló: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Migra el corpus. Devuelve el informe.
pub async fn migrar(pool: &inventario_datos::Pool, base: &Path) -> Result<Informe, sqlx::Error> {
    let mut inf = Informe::default();
    contar_borradas(base, &mut inf);

    // --- Empresa -----------------------------------------------------------
    if let Some(c) = abrir_tabla(base, "CONTROL")
        && let Some(f) = c.filas.first()
    {
        let nit_bruto = t(f, "NIT");
        match nit::normalizar(&nit_bruto) {
            Some(limpio) => {
                let dv = nit::digito_verificacion(&limpio).unwrap_or(0);
                sqlx::query(
                    "INSERT OR REPLACE INTO empresa (id, nombre, nit, dv, direccion, telefono)
                     VALUES (1, ?, ?, ?, ?, ?)",
                )
                .bind(t(f, "NOM_EMPRE"))
                .bind(&limpio)
                .bind(dv as i64)
                .bind(t(f, "DIRECCION"))
                .bind(t(f, "TEL1"))
                .execute(pool)
                .await?;
                inf.nota(format!("empresa: {} (NIT {limpio}-{dv})", t(f, "NOM_EMPRE")));
            }
            None => inf.rechazo("CONTROL", nit_bruto, "el NIT de la empresa no es un número"),
        }
    } else {
        inf.rechazo("CONTROL", "-", "no se encontró CONTROL.DBF: la empresa queda sin configurar");
    }

    // --- Catálogos simples -------------------------------------------------
    let mut ciudades: HashMap<String, i64> = HashMap::new();
    if let Some(tb) = abrir_tabla(base, "ciudades") {
        for f in &tb.filas {
            let (cod, nom) = (t(f, "CODIGO"), t(f, "NOMBRE"));
            if cod.is_empty() || nom.is_empty() {
                inf.rechazo("ciudades", format!("{cod:?}"), "código o nombre vacío");
                continue;
            }
            match insertar_catalogo(pool, "ciudad", &cod, &nom).await {
                Ok((id, choque)) => {
                    if let Some(ya) = choque {
                        inf.rechazo(
                            "ciudades",
                            format!("{cod} {nom}"),
                            format!("el código ya era de «{ya}»: los clientes de «{nom}» quedan en «{ya}»"),
                        );
                    }
                    ciudades.insert(cod, id);
                }
                Err(e) => inf.rechazo("ciudades", cod, format!("no se pudo insertar: {e}")),
            }
        }
        inf.cargado("ciudad", ciudades.len());
    }

    let mut zonas: HashMap<String, i64> = HashMap::new();
    if let Some(tb) = abrir_tabla(base, "zonas") {
        for f in &tb.filas {
            let (cod, nom) = (t(f, "CODIGO"), t(f, "NOMBRE"));
            if cod.is_empty() || nom.is_empty() {
                inf.rechazo("zonas", format!("{cod:?}"), "código o nombre vacío");
                continue;
            }
            match insertar_catalogo(pool, "zona", &cod, &nom).await {
                Ok((id, choque)) => {
                    if let Some(ya) = choque {
                        inf.rechazo(
                            "zonas",
                            format!("{cod} {nom}"),
                            format!("el código ya era de «{ya}»: los clientes de «{nom}» quedan en «{ya}»"),
                        );
                    }
                    if let Some(ciudad) = ciudades.get(&t(f, "CIUDAD")).copied() {
                        sqlx::query("UPDATE zona SET ciudad_id = ? WHERE id = ? AND ciudad_id IS NULL")
                            .bind(ciudad)
                            .bind(id)
                            .execute(pool)
                            .await?;
                    }
                    zonas.insert(cod, id);
                }
                Err(e) => inf.rechazo("zonas", cod, format!("no se pudo insertar: {e}")),
            }
        }
        inf.cargado("zona", zonas.len());
    }

    let mut barrios: HashMap<String, i64> = HashMap::new();
    if let Some(tb) = abrir_tabla(base, "BARRIOS") {
        for f in &tb.filas {
            let (cod, nom) = (t(f, "CODIGO"), t(f, "NOMBRE"));
            if cod.is_empty() || nom.is_empty() {
                continue;
            }
            match insertar_catalogo(pool, "barrio", &cod, &nom).await {
                Ok((id, choque)) => {
                    if let Some(ya) = choque {
                        inf.rechazo("barrio", format!("{cod} {nom}"), format!("el código ya era de «{ya}»"));
                    }
                    barrios.insert(cod, id);
                }
                Err(e) => inf.rechazo("barrio", cod, format!("no se pudo insertar: {e}")),
            }
        }
        inf.cargado("barrio", barrios.len());
    }

    let mut vendedores: HashMap<String, i64> = HashMap::new();
    if let Some(tb) = abrir_tabla(base, "vendedor") {
        for f in &tb.filas {
            let (cod, nom) = (t(f, "CODIGO"), t(f, "NOMBRE"));
            if cod.is_empty() || nom.is_empty() {
                continue;
            }
            match insertar_catalogo(pool, "vendedor", &cod, &nom).await {
                Ok((id, choque)) => {
                    if let Some(ya) = choque {
                        inf.rechazo("vendedor", format!("{cod} {nom}"), format!("el código ya era de «{ya}»"));
                    }
                    vendedores.insert(cod, id);
                }
                Err(e) => inf.rechazo("vendedor", cod, format!("no se pudo insertar: {e}")),
            }
        }
        inf.cargado("vendedor", vendedores.len());
    }

    let mut convenios: HashMap<String, i64> = HashMap::new();
    if let Some(tb) = abrir_tabla(base, "convenios") {
        for f in &tb.filas {
            let (cod, nom) = (t(f, "COD_CONVE"), t(f, "NOM_CONVE"));
            if cod.is_empty() || nom.is_empty() {
                continue;
            }
            match insertar_catalogo(pool, "convenio", &cod, &nom).await {
                Ok((id, choque)) => {
                    if let Some(ya) = choque {
                        inf.rechazo("convenio", format!("{cod} {nom}"), format!("el código ya era de «{ya}»"));
                    }
                    convenios.insert(cod, id);
                }
                Err(e) => inf.rechazo("convenio", cod, format!("no se pudo insertar: {e}")),
            }
        }
        inf.cargado("convenio", convenios.len());
    }

    let mut categorias: HashMap<String, i64> = HashMap::new();
    if let Some(tb) = abrir_tabla(base, "CATEGORIAS") {
        for f in &tb.filas {
            let (cod, nom) = (t(f, "COD_CATEGO"), t(f, "NOM_CATEGO"));
            if cod.is_empty() || nom.is_empty() {
                continue;
            }
            // El corpus tiene el código '05' y '06' DOS veces —una borrada y
            // otra viva con distinto nombre—. `INSERT OR IGNORE` se queda con
            // la primera; el choque se reporta para que alguien lo mire.
            match insertar_catalogo(pool, "categoria", &cod, &nom).await {
                Ok((id, choque)) => {
                    if let Some(ya) = choque {
                        inf.rechazo(
                            "CATEGORIAS",
                            format!("{cod} {nom}"),
                            format!("el código ya era de «{ya}»: se conservó el primero"),
                        );
                    }
                    categorias.insert(cod, id);
                }
                Err(e) => inf.rechazo("CATEGORIAS", cod, format!("no se pudo insertar: {e}")),
            }
        }
        inf.cargado("categoria", categorias.len());
    }

    if let Some(tb) = abrir_tabla(base, "bancos") {
        let mut n_bancos = 0;
        for f in &tb.filas {
            let (cod, nom) = (t(f, "CODBANCO"), t(f, "NOMBANCO"));
            if !cod.is_empty() && !nom.is_empty() && insertar_catalogo(pool, "banco", &cod, &nom).await.is_ok() {
                n_bancos += 1;
            }
        }
        inf.cargado("banco", n_bancos);
    }

    // --- Conceptos de kardex ----------------------------------------------
    let mut conceptos: HashMap<String, i64> = HashMap::new();
    if let Some(tb) = abrir_tabla(base, "CONCEPTOS") {
        for f in &tb.filas {
            let cod = t(f, "COD_CONCEP");
            let nom = t(f, "NOM_CONCEP");
            let signo_bruto = t(f, "SIGNO");
            // El signo vacío NO se convierte en entrada.
            let Some(signo) = inventario_dominio::kardex::Signo::desde_texto(&signo_bruto) else {
                inf.rechazo(
                    "CONCEPTOS",
                    format!("{cod} {nom}"),
                    format!("signo ilegible: {signo_bruto:?}"),
                );
                continue;
            };
            if cod.is_empty() {
                inf.rechazo("CONCEPTOS", nom, "sin código");
                continue;
            }
            // La clase se deduce de las banderas del corpus, y cuando no se
            // puede deducir queda 'ajuste' — que es el único neutro y que NO
            // dispara ninguna automatización.
            let clase = clase_de(&cod, f);
            // El efecto tributario NO se puede deducir del corpus: el sistema
            // viejo no tenía la columna. Todo concepto de SALIDA que no sea
            // venta entra como «no_afecta_impuestos» y **se reporta para
            // revisión**, porque entre ellos están los obsequios, el consumo
            // interno y el retiro de socios, que causan IVA (ET art. 421 lit.
            // b). Adivinarlo por el nombre sería una regla frágil que a la
            // tercera falla y nadie mira (directiva 23); decirlo es honesto.
            let efecto = efecto_inicial(clase);
            let id =
                insertar_concepto(pool, &mut inf, &cod, &nom, signo.como_texto(), clase, efecto).await?;
            conceptos.insert(cod, id);
        }
        inf.cargado("concepto", conceptos.len());
    }
    // La facturación necesita un concepto de venta y uno de devolución. Si el
    // corpus no los trae, se crean: sin ellos el sistema nuevo no factura, y
    // eso no se puede descubrir el primer lunes.
    for (cod, nom, signo, clase, efecto) in [
        ("VTA", "Ventas (sistema nuevo)", "salida", "venta", "venta"),
        (
            "DEV",
            "Devolución de venta (sistema nuevo)",
            "entrada",
            "devolucion_venta",
            "venta",
        ),
    ] {
        let existe: Option<(i64,)> =
            sqlx::query_as("SELECT id FROM concepto WHERE clase = ? AND activo = 1 LIMIT 1")
                .bind(clase)
                .fetch_optional(pool)
                .await?;
        if existe.is_none() {
            sqlx::query(
                "INSERT INTO concepto (codigo, nombre, signo, clase, efecto_tributario)
                 VALUES (?,?,?,?,?)",
            )
            .bind(cod)
            .bind(nom)
            .bind(signo)
            .bind(clase)
            .bind(efecto)
            .execute(pool)
            .await?;
            inf.nota(format!("se creó el concepto «{nom}»: el corpus no traía ninguno de clase {clase}"));
        }
    }

    // --- Artículos ---------------------------------------------------------
    let mut articulos: HashMap<String, i64> = HashMap::new();
    let mut saldos_iniciales: Vec<(i64, Cantidad, Centavos)> = Vec::new();
    let mut no_son_gtin = 0usize;
    if let Some(tb) = abrir_tabla(base, "ARTICULOS") {
        for f in &tb.filas {
            let cod = t(f, "COD_ARTI");
            let desc = t(f, "DESCRI1");
            if cod.is_empty() {
                inf.rechazo("ARTICULOS", desc, "sin código de artículo");
                continue;
            }
            if desc.is_empty() {
                inf.rechazo("ARTICULOS", cod, "sin descripción");
                continue;
            }
            let iva = n(f, "IVA").map(|v| v as i64).unwrap_or(0);
            if !(0..=100).contains(&iva) {
                inf.rechazo("ARTICULOS", cod, format!("tarifa de IVA imposible: {iva}"));
                continue;
            }
            // 63 de los 148 artículos traen código de barras, y **se repite**:
            // el corpus le pone `41251501` a varios PET distintos. El índice
            // único parcial del esquema lo rechazaría.
            //
            // Aquí NO se usa `INSERT OR IGNORE` para esto: se tragaría el
            // choque en silencio y el artículo entero desaparecería sin que
            // nadie lo supiera —que es lo que pasó la primera vez que se corrió
            // esta migración: entraron 106 de 148 y el informe decía que todo
            // bien—. Se comprueba antes, se guarda el artículo SIN código de
            // barras, y el choque sale en el informe.
            let barras_bruto = t(f, "COD_BARR");
            let barras = if barras_bruto.is_empty() {
                None
            } else {
                let ya: Option<(String,)> =
                    sqlx::query_as("SELECT codigo FROM articulo WHERE codigo_barras = ?")
                        .bind(&barras_bruto)
                        .fetch_optional(pool)
                        .await?;
                match ya {
                    None => Some(barras_bruto.clone()),
                    Some((otro,)) => {
                        inf.rechazo(
                            "ARTICULOS",
                            cod.clone(),
                            format!(
                                "el código de barras {barras_bruto} ya es del artículo {otro}: \
                                 este se guardó SIN código de barras"
                            ),
                        );
                        None
                    }
                }
            };

            // Se clasifica el código bajo su estándar REAL: si no cumple el
            // dígito de control de GS1 no es un GTIN, es un código interno. En
            // el corpus, `41251501` y compañía son internos, y declararlos como
            // GTIN ante la DIAN sería informar un identificador internacional
            // que no existe.
            let (codigo_estandar, tipo_estandar) = if barras_bruto.is_empty() {
                (None, None)
            } else {
                let e = inventario_dominio::gtin::clasificar(&barras_bruto);
                if e == inventario_dominio::gtin::Estandar::Interno {
                    // Se CUENTA, no se reporta uno por uno. En el corpus son 63
                    // artículos con el mismo diagnóstico, y 63 líneas idénticas
                    // esconden los rechazos que salen una sola vez — que son los
                    // que de verdad hay que mirar (directiva 23).
                    no_son_gtin += 1;
                }
                (
                    Some(barras_bruto.clone()),
                    Some(match e {
                        inventario_dominio::gtin::Estandar::Gtin => "gtin",
                        inventario_dominio::gtin::Estandar::Unspsc => "unspsc",
                        inventario_dominio::gtin::Estandar::Interno => "interno",
                    }),
                )
            };

            let insertado = sqlx::query(
                "INSERT INTO articulo
                    (codigo, codigo_barras, descripcion, presentacion, categoria_id,
                     iva_porcentaje, precio_venta, punto_minimo, punto_maximo,
                     codigo_estandar, tipo_estandar)
                 VALUES (?,?,?,?,?,?,?,?,?,?,?) RETURNING id",
            )
            .bind(&cod)
            .bind(&barras)
            .bind(&desc)
            .bind(t(f, "PRESENTA"))
            .bind(categorias.get(&t(f, "COD_CATEGO")).copied())
            .bind(iva)
            .bind(n(f, "VAL_VTA").map(centavos).unwrap_or(Centavos::CERO).0)
            .bind(n(f, "PTO_MIN").map(|v| v as i64).unwrap_or(0))
            .bind(n(f, "PTO_MAX").map(|v| v as i64).unwrap_or(0))
            .bind(&codigo_estandar)
            .bind(tipo_estandar)
            .fetch_one(pool)
            .await;

            let id = match insertado {
                Ok(r) => sqlx::Row::get::<i64, _>(&r, "id"),
                Err(_) => {
                    // La única causa que queda es el código repetido, y
                    // entonces el artículo ya está: se reusa el que hay.
                    match id_de(pool, "articulo", &cod).await {
                        Ok(id) => {
                            inf.rechazo(
                                "ARTICULOS",
                                cod.clone(),
                                "código de artículo repetido: se conservó el primero",
                            );
                            id
                        }
                        Err(e) => {
                            inf.rechazo("ARTICULOS", cod, format!("no se pudo insertar: {e}"));
                            continue;
                        }
                    }
                }
            };
            articulos.insert(cod, id);

            // El saldo inicial se convierte en un MOVIMIENTO, no en un campo.
            // Es la decisión central del sistema nuevo: la existencia se deriva
            // del kardex y no hay otro sitio donde ponerla.
            let existencia = n(f, "INV_ACTUAL").unwrap_or(0.0);
            if existencia > 0.0 {
                let costo = n(f, "VAL_COSTO").unwrap_or(0.0);
                saldos_iniciales.push((id, milesimas(existencia), centavos(costo)));
            } else if existencia < 0.0 {
                inf.rechazo(
                    "ARTICULOS",
                    t(f, "COD_ARTI"),
                    format!(
                        "existencia NEGATIVA en el sistema viejo ({existencia}): \
                         no se migra, hay que contarla físicamente"
                    ),
                );
            }
        }
        inf.cargado("articulo", articulos.len());
        if no_son_gtin > 0 {
            inf.nota(format!(
                "{no_son_gtin} artículos traen un «código de barras» que NO pasa el dígito de \
                 control de GS1: no son GTIN. Se declaran con el estándar 999 (código del \
                 vendedor), que es la verdad y es válido ante la DIAN. Informarlos como GTIN \
                 sería declarar un identificador internacional que no existe."
            ));
        }
    }

    // Saldos iniciales como movimientos de kardex.
    let concepto_inicial: Option<(i64,)> =
        sqlx::query_as("SELECT id FROM concepto WHERE clase = 'inicial' AND activo = 1 LIMIT 1")
            .fetch_optional(pool)
            .await?;
    let concepto_inicial = match concepto_inicial {
        Some((id,)) => id,
        None => {
            sqlx::query(
                "INSERT INTO concepto (codigo, nombre, signo, clase, efecto_tributario)
                 VALUES ('INI', 'Saldo inicial de la migración', 'entrada', 'inicial',
                         'no_afecta_impuestos') RETURNING id",
            )
            .fetch_one(pool)
            .await
            .map(|r| sqlx::Row::get::<i64, _>(&r, "id"))?
        }
    };
    let hoy = chrono::Utc::now().date_naive().to_string();
    let mut n_saldos = 0;
    for (articulo_id, cantidad, costo) in &saldos_iniciales {
        // Un artículo que YA tiene kardex no recibe otro saldo inicial. Sin
        // esto, correr la migración dos veces —que es justo lo que se hace
        // mientras se afina— duplicaría la existencia de todo el inventario, y
        // el descuadre no se vería hasta el primer conteo físico.
        let (ya,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM movimiento WHERE articulo_id = ?")
            .bind(articulo_id)
            .fetch_one(pool)
            .await?;
        if ya > 0 {
            continue;
        }
        let p = inventario_datos::kardex::Peticion {
            fecha: hoy.clone(),
            concepto_id: concepto_inicial,
            articulo_id: *articulo_id,
            tercero_id: None,
            documento: "migración VFP".into(),
            cantidad: *cantidad,
            costo_unitario: *costo,
            factura_id: None,
            usuario_id: None,
                valor_comercial_unitario: None,
        };
        match inventario_datos::kardex::registrar(pool, &p).await {
            Ok(_) => n_saldos += 1,
            Err(e) => inf.rechazo("ARTICULOS", articulo_id.to_string(), format!("saldo inicial: {e}")),
        }
    }
    inf.cargado("saldo inicial (movimiento)", n_saldos);

    // --- Terceros ----------------------------------------------------------
    // `IDCLIENTE` NO es la clave: se repite 4 veces y mide de 6 a 22
    // caracteres. Se guarda el mapa viejo->nuevo aquí para poder enlazar las
    // facturas, y el duplicado se reporta.
    let mut por_idcliente: HashMap<String, i64> = HashMap::new();
    if let Some(tb) = abrir_tabla(base, "CLIENTES") {
        let mut n_clientes = 0;
        for f in &tb.filas {
            let idviejo = t(f, "IDCLIENTE");
            let nombre = t(f, "NOM_CLIEN");
            let nit_bruto = t(f, "NIT_CLIEN");
            if nombre.is_empty() {
                inf.rechazo("CLIENTES", idviejo, "sin nombre");
                continue;
            }
            let Some(limpio) = nit::normalizar(&nit_bruto) else {
                inf.rechazo(
                    "CLIENTES",
                    format!("{nombre} ({nit_bruto:?})"),
                    "el NIT/cédula no contiene ningún dígito",
                );
                continue;
            };
            let dv = nit::digito_verificacion(&limpio).unwrap_or(0);
            // El plazo venía en C(3) con valores como '0.' y '': lo que no sea
            // un entero se toma como sin plazo, y se dice.
            let plazo_bruto = t(f, "PAGO");
            let plazo = match plazo_bruto.trim() {
                "" => 0,
                otro => match otro.parse::<i64>() {
                    Ok(v) if v >= 0 => v,
                    _ => {
                        inf.rechazo(
                            "CLIENTES",
                            nombre.clone(),
                            format!("plazo de pago ilegible ({plazo_bruto:?}): quedó en contado"),
                        );
                        0
                    }
                },
            };

            let id: i64 = sqlx::query(
                "INSERT INTO tercero
                    (nit, dv, nombre, direccion, telefono, telefono2, ciudad_id, zona_id,
                     barrio_id, vendedor_id, convenio_id, plazo_dias)
                 VALUES (?,?,?,?,?,?,?,?,?,?,?,?) RETURNING id",
            )
            .bind(&limpio)
            .bind(dv as i64)
            .bind(&nombre)
            .bind(t(f, "DIR_CLIEN"))
            .bind(t(f, "TEL1_CLIEN"))
            .bind(t(f, "TEL2_CLIEN"))
            .bind(ciudades.get(&t(f, "CIUDAD")).copied())
            .bind(zonas.get(&t(f, "ZONA")).copied())
            .bind(barrios.get(&t(f, "BARRIO")).copied())
            .bind(vendedores.get(&t(f, "COD_VEND")).copied())
            .bind(convenios.get(&t(f, "COD_CONVE")).copied())
            .bind(plazo)
            .fetch_one(pool)
            .await
            .map(|r| sqlx::Row::get::<i64, _>(&r, "id"))?;

            n_clientes += 1;
            if !idviejo.is_empty() && por_idcliente.insert(idviejo.clone(), id).is_some() {
                inf.rechazo(
                    "CLIENTES",
                    idviejo,
                    "IDCLIENTE repetido: las facturas de ese id se enlazan al PRIMERO",
                );
            }
        }
        inf.cargado("tercero (clientes)", n_clientes);
    }

    if let Some(tb) = abrir_tabla(base, "PROVEEDOR") {
        let mut n_prov = 0;
        for f in &tb.filas {
            let nombre = t(f, "NOM_PROVE");
            let Some(limpio) = nit::normalizar(&t(f, "ID_PROVE")) else {
                inf.rechazo("PROVEEDOR", nombre, "el NIT no contiene dígitos");
                continue;
            };
            if nombre.is_empty() {
                continue;
            }
            let dv = nit::digito_verificacion(&limpio).unwrap_or(0);
            sqlx::query(
                "INSERT INTO tercero (nit, dv, nombre, es_cliente, es_proveedor, direccion, telefono)
                 VALUES (?,?,?,0,1,?,?)",
            )
            .bind(&limpio)
            .bind(dv as i64)
            .bind(&nombre)
            .bind(t(f, "DIR_PROVE"))
            .bind(t(f, "TEL1_PROVE"))
            .execute(pool)
            .await?;
            n_prov += 1;
        }
        inf.cargado("tercero (proveedores)", n_prov);
    }

    // --- Precios por convenio ---------------------------------------------
    if let Some(tb) = abrir_tabla(base, "valorconvenio") {
        let mut n_precios = 0;
        for f in &tb.filas {
            let cod_conv = t(f, "COD_CONVE");
            let cod_art = t(f, "COD_ARTI");
            let (Some(cid), Some(aid)) = (convenios.get(&cod_conv), articulos.get(&cod_art)) else {
                inf.rechazo(
                    "valorconvenio",
                    format!("{cod_conv}/{cod_art}"),
                    "el convenio o el artículo no existen",
                );
                continue;
            };
            let Some(valor) = n(f, "VALOR") else {
                inf.rechazo("valorconvenio", format!("{cod_conv}/{cod_art}"), "precio vacío");
                continue;
            };
            sqlx::query(
                "INSERT OR REPLACE INTO precio_convenio (convenio_id, articulo_id, precio)
                 VALUES (?,?,?)",
            )
            .bind(cid)
            .bind(aid)
            .bind(centavos(valor).0)
            .execute(pool)
            .await?;
            n_precios += 1;
        }
        inf.cargado("precio_convenio", n_precios);
    }

    // --- Resoluciones ------------------------------------------------------
    if let Some(tb) = abrir_tabla(base, "CONFIFAC") {
        let mut n_res = 0;
        for f in &tb.filas {
            let (desde, hasta) = (
                n(f, "NROMINFAC").map(|v| v as i64).unwrap_or(0),
                n(f, "NROMAXFAC").map(|v| v as i64).unwrap_or(0),
            );
            let nombre = t(f, "NOMGRUFAC");
            if desde <= 0 || hasta < desde {
                inf.rechazo(
                    "CONFIFAC",
                    format!("{} {nombre}", t(f, "GRUPOFAC")),
                    format!("rango inválido [{desde}..{hasta}]"),
                );
                continue;
            }
            let ultimo = n(f, "ULT_FACTU").map(|v| v as i64).filter(|u| *u >= desde && *u <= hasta);
            // El grupo 15 del corpus tiene ULT_FACTU=0 con rango [1..5000]:
            // cero está FUERA del rango, así que es «ninguna emitida», no «se
            // emitió la cero».
            let r = sqlx::query(
                "INSERT OR IGNORE INTO resolucion
                    (nombre, prefijo, numero, fecha, desde, hasta, ultimo_emitido)
                 VALUES (?,?,?,?,?,?,?)",
            )
            .bind(&nombre)
            .bind(t(f, "PREFIJO"))
            .bind(t(f, "NRORESO"))
            .bind(fecha(f, "FECHARESO"))
            .bind(desde)
            .bind(hasta)
            .bind(ultimo)
            .execute(pool)
            .await?;
            if r.rows_affected() > 0 {
                n_res += 1;
            } else {
                inf.rechazo(
                    "CONFIFAC",
                    nombre.clone(),
                    "ya hay una resolución con ese prefijo y arranque",
                );
            }
            if ultimo == Some(hasta) {
                inf.nota(format!("AVISO: la resolución «{nombre}» llegó AGOTADA del sistema viejo"));
            }
        }
        inf.cargado("resolucion", n_res);
    }

    // --- Lo que NO se migra, dicho explícitamente --------------------------
    //
    // El histórico de facturas (3.154), su detalle (3.712) y el kardex de 2004
    // (1.321) NO se cargan como documentos vivos, y esto es una decisión, no un
    // olvido: 15 de esos números de factura están duplicados, 34 líneas de
    // detalle apuntan a facturas que no existen y el kardex lleva parado desde
    // septiembre de 2004, así que la existencia que se deduciría de él sería
    // falsa. Cargarlos daría un sistema nuevo que arranca con los defectos del
    // viejo dentro, y con apariencia de estar bien.
    //
    // Lo que sí viaja es el SALDO (arriba, como movimiento inicial) y los
    // maestros. El histórico se conserva en los DBF, que son el respaldo legal.
    if let Some(tb) = abrir_tabla(base, "facturas") {
        inf.nota(format!(
            "NO migradas: {} facturas históricas (2004-2020). Se conservan en los DBF. \
             Motivo: 15 números duplicados y un kardex parado en 2004 harían falsa \
             la existencia del sistema nuevo. Ver ANALISIS.md §5.",
            tb.filas.len()
        ));
    }
    if let Some(tb) = abrir_tabla(base, "mov_inv") {
        inf.nota(format!(
            "NO migrados: {} movimientos de kardex de 2004. El saldo sí viajó, como \
             movimiento inicial fechado hoy.",
            tb.filas.len()
        ));
    }

    Ok(inf)
}

/// Deduce la clase del concepto a partir de las banderas del corpus.
///
/// **Nunca devuelve 'venta' ni 'devolucion_venta'**: esas dos clases disparan
/// automatismos de facturación, y asignárselas a un concepto por parecido de
/// nombre haría que facturar moviera un concepto que el cliente usaba para otra
/// cosa. Los conceptos de venta del sistema nuevo se crean aparte.
fn clase_de(codigo: &str, f: &Fila) -> &'static str {
    if t(f, "INICIAL") == "S" {
        return "inicial";
    }
    if t(f, "PROVEEDOR") == "S" {
        // 11 y 12 son compras; 61..65 son devoluciones al proveedor.
        return if codigo.starts_with('6') {
            "devolucion_compra"
        } else {
            "compra"
        };
    }
    match codigo {
        "31" | "77" => "traslado",
        "79" | "71" => "consumo",
        _ => "ajuste",
    }
}

/// Inserta en un catálogo y **avisa del choque de códigos**.
///
/// Devuelve `(id, Some(nombre_que_ya_estaba))` cuando el código ya existía con
/// otro nombre. El corpus lo necesita: su ciudad `101` es a la vez BOLÍVAR y
/// CALI, y sus 17 zonas se reducen a 5 códigos —`01` es ORIENTE **y** CENTRO—.
/// Un `INSERT OR IGNORE` a secas se quedaría con el primero sin decir nada, y
/// los clientes de la otra ciudad quedarían en la equivocada. Eso es la
/// directiva 20 al revés: un dato falso que no se nota nunca.
///
/// `tabla` viene de este módulo y de ningún sitio más: es una constante del
/// código, nunca un dato de entrada.
/// Efecto tributario inicial de un concepto migrado.
///
/// **Nunca devuelve `retiro_gravado`**: el corpus no tiene el dato y adivinarlo
/// por el nombre del concepto haría que un obsequio liquidara IVA o dejara de
/// hacerlo según cómo lo hubiera escrito alguien en 2004. Se deja neutro y se
/// REPORTA para que una persona decida (directiva 20: falla ruidosamente antes
/// que rellenar).
fn efecto_inicial(clase: &str) -> &'static str {
    match clase {
        "venta" | "devolucion_venta" => "venta",
        _ => "no_afecta_impuestos",
    }
}

/// Inserta un concepto de kardex y **avisa del choque de códigos**.
///
/// Es la costura que usa el camino real y que la prueba puede alimentar con
/// texto, sin necesidad de un DBF. Devuelve el id que queda vigente para ese
/// código, choque o no, para que los movimientos que lo referencian resuelvan.
///
/// Aquí NO se usa `INSERT OR IGNORE` —regla 5 del `CLAUDE.md`—: se mira ANTES y
/// el choque se DICE. Un `OR IGNORE` a secas se queda con el primero en
/// silencio, y el segundo concepto —que puede ser justo el obsequio que causa
/// IVA (ET art. 421 lit. b)— desaparece sin que nadie lo sepa. Es el mismo
/// trato, y por el mismo motivo, que `insertar_catalogo` da a ciudades y zonas.
async fn insertar_concepto(
    pool: &inventario_datos::Pool,
    inf: &mut Informe,
    codigo: &str,
    nombre: &str,
    signo: &str,
    clase: &str,
    efecto: &str,
) -> Result<i64, sqlx::Error> {
    let previo = sqlx::query_as::<_, (i64, String)>("SELECT id, nombre FROM concepto WHERE codigo = ?")
        .bind(codigo)
        .fetch_optional(pool)
        .await?;

    // Mismo código y mismo nombre es una fila REPETIDA, no un choque. Tampoco
    // se vuelve a marcar para revisión: ya se marcó la primera vez, y una
    // alarma repetida es una que a la tercera nadie mira (directiva 23).
    if let Some((id, ya)) = previo {
        if ya != nombre {
            inf.rechazo(
                "CONCEPTOS",
                format!("{codigo} {nombre}"),
                format!(
                    "el código ya lo tenía «{ya}»; se conserva ese y este NO entra. \
                     Si son dos conceptos distintos, hay que darle otro código"
                ),
            );
        }
        return Ok(id);
    }

    let id = sqlx::query_as::<_, (i64,)>(
        "INSERT INTO concepto (codigo, nombre, signo, clase, efecto_tributario)
         VALUES (?,?,?,?,?) RETURNING id",
    )
    .bind(codigo)
    .bind(nombre)
    .bind(signo)
    .bind(clase)
    .bind(efecto)
    .fetch_one(pool)
    .await?
    .0;

    if signo == "salida" && efecto == "no_afecta_impuestos" {
        // `concat!`, no una cadena con `\` al final: la continuación de línea
        // conserva la SANGRÍA del código, y este texto lo lee una persona en el
        // informe de migración. Ya salió una vez a producción con veinte
        // espacios en medio.
        inf.rechazo(
            "CONCEPTOS",
            format!("{codigo} {nombre}"),
            concat!(
                "REVISAR el efecto tributario: si es obsequio, consumo interno o retiro de ",
                "socios, causa IVA sobre el valor comercial (ET art. 421 lit. b). ",
                "Quedó como «no_afecta_impuestos»"
            ),
        );
    }
    Ok(id)
}

async fn insertar_catalogo(
    pool: &inventario_datos::Pool,
    tabla: &str,
    codigo: &str,
    nombre: &str,
) -> Result<(i64, Option<String>), sqlx::Error> {
    let sql = format!("SELECT id, nombre FROM {tabla} WHERE codigo = ?");
    if let Some((id, ya)) = sqlx::query_as::<_, (i64, String)>(&sql)
        .bind(codigo)
        .fetch_optional(pool)
        .await?
    {
        // Mismo código y mismo nombre es una fila repetida, no un choque: las
        // 8 filas «05 CENTRO» del corpus son la misma zona escrita ocho veces.
        return Ok((id, (ya != nombre).then_some(ya)));
    }
    let sql = format!("INSERT INTO {tabla} (codigo, nombre) VALUES (?, ?) RETURNING id");
    let id = sqlx::query_as::<_, (i64,)>(&sql)
        .bind(codigo)
        .bind(nombre)
        .fetch_one(pool)
        .await?
        .0;
    Ok((id, None))
}

async fn id_de(pool: &inventario_datos::Pool, tabla: &str, codigo: &str) -> Result<i64, sqlx::Error> {
    let sql = format!("SELECT id FROM {tabla} WHERE codigo = ?");
    sqlx::query_as::<_, (i64,)>(&sql)
        .bind(codigo)
        .fetch_one(pool)
        .await
        .map(|(id,)| id)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn corpus() -> Option<&'static Path> {
        let p = Path::new("/mnt/data/inventario_2004");
        p.exists().then_some(p)
    }

    /// El choque de códigos de concepto **se dice**, y la fila repetida **no**.
    ///
    /// Los dos casos van juntos a propósito (directiva 33): con el primero solo
    /// pasaría igual un migrador que reportara todo, y con el segundo solo, uno
    /// ciego. Y esta prueba NO depende del corpus: entra por la misma costura
    /// que usa `migrar`, alimentada con texto.
    #[tokio::test]
    async fn dos_conceptos_con_el_mismo_codigo_no_se_pierden_en_silencio() {
        let pool = inventario_datos::abrir("sqlite::memory:").await.unwrap();
        let mut inf = Informe::default();

        let primero =
            insertar_concepto(&pool, &mut inf, "79", "OBSEQUIOS", "salida", "consumo", "no_afecta_impuestos")
                .await
                .unwrap();

        // ROJO: mismo código, OTRO nombre. Es la ciudad 101 del corpus aplicada
        // a los conceptos, y aquí el que se pierde puede ser el que causa IVA.
        let segundo =
            insertar_concepto(&pool, &mut inf, "79", "RETIRO DE SOCIOS", "salida", "consumo", "no_afecta_impuestos")
                .await
                .unwrap();

        assert_eq!(segundo, primero, "el choque debió devolver el id que ya estaba");
        let choques: Vec<_> = inf
            .rechazos
            .iter()
            .filter(|r| r.motivo.contains("el código ya lo tenía"))
            .collect();
        assert_eq!(
            choques.len(),
            1,
            "el choque de código no se reportó una vez y solo una: {:?}",
            inf.rechazos
        );
        assert!(
            choques[0].motivo.contains("OBSEQUIOS"),
            "el rechazo no dice cuál se conservó: {}",
            choques[0].motivo
        );

        // VERDE: mismo código y MISMO nombre es una fila repetida del DBF, no un
        // choque. Si esto también avisara, el informe real traería una alarma
        // por cada fila duplicada y nadie leería ninguna.
        let mut inf2 = Informe::default();
        insertar_concepto(&pool, &mut inf2, "79", "OBSEQUIOS", "salida", "consumo", "no_afecta_impuestos")
            .await
            .unwrap();
        assert!(
            inf2.rechazos.is_empty(),
            "una fila repetida no es un choque: {:?}",
            inf2.rechazos
        );

        // Y solo hay UN concepto con ese código: el segundo no pisó al primero.
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM concepto WHERE codigo = '79'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 1);
        let (nombre,): (String,) = sqlx::query_as("SELECT nombre FROM concepto WHERE codigo = '79'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(nombre, "OBSEQUIOS");
    }

    /// La marca de «REVISAR el efecto tributario» sale UNA vez por concepto de
    /// salida neutro, y **no sale** para una venta. Sin la segunda mitad, una
    /// marca puesta a todo pasaría por vigilante igual.
    #[tokio::test]
    async fn la_marca_de_revisar_el_iva_distingue_la_salida_neutra_de_la_venta() {
        let pool = inventario_datos::abrir("sqlite::memory:").await.unwrap();
        let mut inf = Informe::default();

        insertar_concepto(&pool, &mut inf, "79", "CONSUMO INTERNO", "salida", "consumo", "no_afecta_impuestos")
            .await
            .unwrap();
        insertar_concepto(&pool, &mut inf, "01", "VENTAS", "salida", "venta", "venta")
            .await
            .unwrap();
        insertar_concepto(&pool, &mut inf, "02", "COMPRAS", "entrada", "compra", "no_afecta_impuestos")
            .await
            .unwrap();

        let marcados: Vec<&str> = inf
            .rechazos
            .iter()
            .filter(|r| r.motivo.contains("REVISAR el efecto tributario"))
            .map(|r| r.clave.as_str())
            .collect();
        assert_eq!(
            marcados,
            vec!["79 CONSUMO INTERNO"],
            "la marca no discrimina: {:?}",
            inf.rechazos
        );

        // Y el texto sale sin la sangría del código: ya salió una vez a
        // producción con veinte espacios en medio.
        let motivo = &inf.rechazos[0].motivo;
        assert!(
            !motivo.contains("  "),
            "el aviso lleva sangría del código dentro: {motivo:?}"
        );
    }

    /// La prueba de extremo a extremo del migrador: el corpus real entero.
    #[tokio::test]
    async fn migra_el_corpus_real_y_rechaza_lo_que_no_cabe() {
        let Some(base) = corpus() else {
            eprintln!("corpus no disponible: se omite");
            return;
        };
        let pool = inventario_datos::abrir("sqlite::memory:").await.unwrap();
        let inf = migrar(&pool, base).await.unwrap();

        let cargado = |t: &str| inf.cargados.iter().find(|(n, _)| n == t).map(|(_, c)| *c);

        // Maestros: las cifras exactas del corpus.
        assert_eq!(cargado("articulo"), Some(148));
        assert_eq!(cargado("convenio"), Some(9));
        assert_eq!(cargado("vendedor"), Some(17));

        // 129 filas de ciudad, pero solo 128 códigos: el `101` es a la vez
        // BOLÍVAR y CALI. Entran 128 y el choque SALE EN EL INFORME — no se
        // pierde una ciudad en silencio, que es lo que hacía un
        // `INSERT OR IGNORE`.
        assert_eq!(cargado("ciudad"), Some(128));
        assert!(
            inf.rechazos
                .iter()
                .any(|r| r.tabla == "ciudades" && r.motivo.contains("ya era de")),
            "el choque de la ciudad 101 no se reportó"
        );
        // Lo mismo con las zonas: 17 filas y 5 códigos.
        assert_eq!(cargado("zona"), Some(5));
        assert!(
            inf.rechazos
                .iter()
                .any(|r| r.tabla == "zonas" && r.motivo.contains("ya era de")),
            "el choque de la zona 01 (ORIENTE/CENTRO) no se reportó"
        );
        // 794 clientes menos los que no tienen NIT legible.
        let clientes = cargado("tercero (clientes)").unwrap();
        assert!((700..=794).contains(&clientes), "clientes migrados: {clientes}");

        // La empresa del corpus.
        let (nombre, nit_): (String, String) =
            sqlx::query_as("SELECT nombre, nit FROM empresa WHERE id = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(nit_, "811033579");
        assert!(!nombre.is_empty());

        // El artículo con existencia NEGATIVA se rechaza, no se migra en -3.
        assert!(
            inf.rechazos
                .iter()
                .any(|r| r.motivo.contains("NEGATIVA") && r.clave == "1102001"),
            "no se reportó la existencia negativa del artículo 1102001"
        );
        let (negativos,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM movimiento WHERE existencia_despues < 0")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(negativos, 0, "entró una existencia negativa");

        // La resolución agotada del corpus (grupo 08: 5650 de 5650) queda avisada.
        assert!(
            inf.lineas.iter().any(|l| l.contains("AGOTADA")),
            "no se avisó de la resolución agotada"
        );

        // El histórico NO entra, y se dice.
        let (facturas,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM factura")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(facturas, 0);
        assert!(inf.lineas.iter().any(|l| l.contains("NO migradas")));

        // Y lo que sí entró queda coherente: la foto del kardex cuadra con la
        // historia de cada artículo.
        let ids: Vec<(i64,)> = sqlx::query_as("SELECT id FROM articulo")
            .fetch_all(&pool)
            .await
            .unwrap();
        for (id,) in ids {
            let foto = inventario_datos::kardex::saldo(&pool, id).await.unwrap();
            let recorrido = inventario_datos::kardex::recomputar(&pool, id).await.unwrap();
            assert_eq!(foto.existencia, recorrido.existencia.0, "artículo {id}");
            assert_eq!(foto.valor, recorrido.valor.0, "artículo {id}");
        }

        // Y hay conceptos de venta y devolución, sin los cuales no se factura.
        for clase in ["venta", "devolucion_venta"] {
            let (n,): (i64,) =
                sqlx::query_as("SELECT COUNT(*) FROM concepto WHERE clase = ? AND activo = 1")
                    .bind(clase)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert!(n >= 1, "no quedó concepto de clase {clase}");
        }

        inf.imprimir();
    }

    /// Migrar dos veces no duplica los maestros.
    #[tokio::test]
    async fn la_migracion_se_puede_repetir_sin_duplicar_catalogos() {
        let Some(base) = corpus() else {
            eprintln!("corpus no disponible: se omite");
            return;
        };
        let pool = inventario_datos::abrir("sqlite::memory:").await.unwrap();
        migrar(&pool, base).await.unwrap();
        let (a1,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM articulo")
            .fetch_one(&pool).await.unwrap();
        let (c1,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ciudad")
            .fetch_one(&pool).await.unwrap();

        let (m1,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM movimiento")
            .fetch_one(&pool).await.unwrap();
        let (e1,): (i64,) = sqlx::query_as("SELECT COALESCE(SUM(cantidad), 0) FROM movimiento")
            .fetch_one(&pool).await.unwrap();

        migrar(&pool, base).await.unwrap();
        let (a2,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM articulo")
            .fetch_one(&pool).await.unwrap();
        let (c2,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ciudad")
            .fetch_one(&pool).await.unwrap();
        let (m2,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM movimiento")
            .fetch_one(&pool).await.unwrap();
        let (e2,): (i64,) = sqlx::query_as("SELECT COALESCE(SUM(cantidad), 0) FROM movimiento")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(a1, a2, "los artículos se duplicaron al repetir");
        assert_eq!(c1, c2, "las ciudades se duplicaron al repetir");
        // La que de verdad duele: repetir la migración NO puede duplicar la
        // existencia. El descuadre no se vería hasta el primer conteo físico.
        assert_eq!(m1, m2, "se duplicaron los movimientos de kardex");
        assert_eq!(e1, e2, "SE DUPLICÓ LA EXISTENCIA del inventario");
        assert!(m1 > 0, "la migración no cargó ningún saldo: la prueba no mide nada");
    }

    #[test]
    fn la_clase_deducida_nunca_es_venta() {
        // Ninguna combinación de banderas del corpus puede producir 'venta':
        // esa clase dispara la facturación y no se asigna por parecido.
        for codigo in ["11", "12", "13", "31", "51", "52", "71", "77", "79", "99"] {
            for prov in ["S", ""] {
                for ini in ["S", ""] {
                    let mut f = Fila::new();
                    f.insert("PROVEEDOR".into(), dbf::Valor::Texto(prov.into()));
                    f.insert("INICIAL".into(), dbf::Valor::Texto(ini.into()));
                    let c = clase_de(codigo, &f);
                    assert_ne!(c, "venta", "código {codigo} prov={prov} ini={ini}");
                    assert_ne!(c, "devolucion_venta");
                }
            }
        }
    }
}
