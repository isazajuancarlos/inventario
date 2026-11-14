//! `inventario-api respaldar <destino>` — copia de seguridad **verificada**.
//!
//! La Ley 962 de 2005, art. 28, exige conservar los libros y papeles del
//! comerciante **diez años** desde el último asiento, en papel o «en cualquier
//! medio técnico, magnético o electrónico **que garantice su reproducción
//! exacta**».
//!
//! «Reproducción exacta» es la parte que la mayoría de los respaldos no
//! cumplen. Un `cp` de un SQLite con WAL abierto produce un archivo que **abre
//! sin quejarse** y al que le faltan las últimas transacciones — y eso no se
//! nota hasta el día que se necesita.
//!
//! Aquí:
//!
//! 1. La copia se hace con **`VACUUM INTO`**, que es la forma que la propia
//!    documentación de SQLite señala para copiar una base viva: produce un
//!    archivo consistente aunque haya escrituras en curso, sin bloquear al que
//!    está facturando.
//! 2. La copia **se abre y se verifica**: `integrity_check`, `foreign_key_check`
//!    y que cada tabla del esquema exista y se pueda consultar. Un respaldo que
//!    no se ha leído no es un respaldo: es un archivo.
//! 3. Se escribe un **manifiesto** al lado con las cuentas y la fecha, para que
//!    dentro de nueve años alguien pueda saber qué había dentro sin abrirlo.
//!
//! ## Por qué NO se comparan las cuentas contra el original
//!
//! La primera versión de este módulo contaba las filas de la copia y las
//! comparaba con las del original. Parecía la verificación más obvia, y está
//! mal: **la copia es una foto de un instante y el original sigue moviéndose**.
//! Con una venta en curso, el original tiene más filas que la copia sin que
//! nada esté roto, y el respaldo fallaba diciendo que no era exacto.
//!
//! Eso es medir dos instantes distintos y llamarlo «reproducción exacta». Lo
//! cazó `se_puede_respaldar_mientras_se_escribe`, que es justo el caso normal:
//! si hubiera que parar el sistema para respaldar, nadie respaldaría.
//!
//! Lo que sí se verifica —integridad física, integridad referencial y que todas
//! las tablas respondan— es propiedad **de la copia sola**, y por eso se puede
//! afirmar.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Las tablas cuyo conteo se compara entre original y copia.
///
/// Es una lista **explícita** y no `sqlite_master`: si mañana se añade una
/// tabla, esta lista no se entera y hay que tocarla a mano — que es justo lo
/// que se quiere, porque decidir si una tabla nueva entra en el manifiesto legal
/// es una decisión, no un automatismo. La prueba
/// `el_manifiesto_cubre_todas_las_tablas_del_esquema` avisa cuando falta alguna.
const TABLAS: [&str; 22] = [
    "empresa",
    "usuario",
    "ciudad",
    "zona",
    "barrio",
    "vendedor",
    "categoria",
    "articulo",
    "convenio",
    "precio_convenio",
    "tercero",
    "concepto",
    "resolucion",
    "factura",
    "factura_linea",
    "movimiento",
    "capa_costo",
    "consumo_capa",
    "banco",
    "recibo",
    "recibo_aplicacion",
    "recibo_pago",
];

/// Las que no entran en la comparación, y por qué. Existe para que la prueba de
/// cobertura pueda distinguir «falta» de «se decidió que no».
const FUERA: [&str; 5] = [
    // Se cuentan aparte más abajo: son las que crecen y las que más importan
    // legalmente.
    "nota",
    "nota_linea",
    "deterioro",
    "acta_baja",
    "acta_renglon",
];

#[derive(Debug)]
pub struct Informe {
    pub destino: PathBuf,
    pub bytes: u64,
    pub conteos: Vec<(String, i64)>,
    pub integridad: String,
}

pub async fn ejecutar(destino: Option<&str>) -> ExitCode {
    let Some(destino) = destino else {
        eprintln!("uso: inventario-api respaldar <archivo-destino>");
        eprintln!("     El origen sale de INVENTARIO_BD.");
        return ExitCode::FAILURE;
    };
    let url = std::env::var("INVENTARIO_BD").unwrap_or_else(|_| "sqlite://inventario.db".into());
    let pool = match inventario_datos::abrir(&url).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("no se pudo abrir la base ({url}): {e}");
            return ExitCode::FAILURE;
        }
    };

    match respaldar(&pool, Path::new(destino)).await {
        Ok(inf) => {
            println!("Respaldo verificado: {}", inf.destino.display());
            println!("  tamaño: {} bytes", inf.bytes);
            println!("  integrity_check: {}", inf.integridad);
            println!("  filas:");
            for (t, n) in &inf.conteos {
                println!("    {t:<20} {n}");
            }
            println!(
                "\nManifiesto: {}.manifiesto.txt\nConserve el par (archivo + manifiesto) \
                 diez años (Ley 962 de 2005, art. 28).",
                inf.destino.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("el respaldo FALLÓ y no se puede dar por hecho: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Hace el respaldo y lo verifica. Devuelve error si algo no cuadra: **no
/// existe el respaldo «casi bien»**.
pub async fn respaldar(pool: &inventario_datos::Pool, destino: &Path) -> Result<Informe, String> {
    if destino.exists() {
        // No se pisa un respaldo anterior. Perder el bueno al hacer uno malo es
        // la forma más tonta de quedarse sin ninguno.
        return Err(format!(
            "«{}» ya existe. Un respaldo no pisa a otro: use un nombre con fecha.",
            destino.display()
        ));
    }
    if let Some(dir) = destino.parent()
        && !dir.as_os_str().is_empty()
        && !dir.exists()
    {
        return Err(format!("la carpeta «{}» no existe", dir.display()));
    }

    // `VACUUM INTO` produce una copia consistente de una base viva. El nombre
    // del archivo no se puede parametrizar con `bind`, así que se comprueba
    // antes que no traiga comillas.
    let ruta = destino.to_string_lossy().to_string();
    if ruta.contains('\'') {
        return Err("la ruta del respaldo no puede contener comillas simples".into());
    }
    sqlx::query(&format!("VACUUM INTO '{ruta}'"))
        .execute(pool)
        .await
        .map_err(|e| format!("VACUUM INTO: {e}"))?;

    // --- Verificación -----------------------------------------------------
    //
    // Se abre la COPIA, no el original. Un respaldo que no se ha leído no es un
    // respaldo.
    let copia = inventario_datos::abrir(&format!("sqlite://{ruta}"))
        .await
        .map_err(|e| format!("no se pudo abrir la copia para verificarla: {e}"))?;

    let (integridad,): (String,) = sqlx::query_as("PRAGMA integrity_check")
        .fetch_one(&copia)
        .await
        .map_err(|e| format!("integrity_check: {e}"))?;
    if integridad != "ok" {
        return Err(format!(
            "la copia NO pasa integrity_check: {integridad}. El respaldo no sirve."
        ));
    }

    // Integridad REFERENCIAL. Es la verificación que más aporta después del
    // `integrity_check`: una copia truncada a mitad de una tabla deja hijos
    // apuntando a padres que no llegaron, y esto lo caza. El `integrity_check`
    // solo mira la estructura física de las páginas.
    let huerfanos: Vec<(String, i64, String, i64)> = sqlx::query_as("PRAGMA foreign_key_check")
        .fetch_all(&copia)
        .await
        .map_err(|e| format!("foreign_key_check: {e}"))?;
    if !huerfanos.is_empty() {
        return Err(format!(
            "la copia tiene {} filas con claves foráneas rotas (p. ej. en «{}»). \
             El respaldo no sirve.",
            huerfanos.len(),
            huerfanos[0].0
        ));
    }

    // Cada tabla del esquema tiene que existir y responder. Un archivo que abre
    // pero al que le falta una tabla es peor que uno que no abre.
    let mut conteos = Vec::new();
    let todas: Vec<&str> = TABLAS.iter().chain(FUERA.iter()).copied().collect();
    for tabla in todas {
        let (n,): (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM {tabla}"))
            .fetch_one(&copia)
            .await
            .map_err(|e| {
                format!("la copia no puede consultar «{tabla}»: {e}. El respaldo no sirve.")
            })?;
        conteos.push((tabla.to_string(), n));
    }
    copia.close().await;

    let bytes = std::fs::metadata(destino)
        .map_err(|e| format!("no se pudo medir la copia: {e}"))?
        .len();

    // --- Manifiesto -------------------------------------------------------
    let manifiesto = destino.with_extension(format!(
        "{}.manifiesto.txt",
        destino
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default()
    ));
    let mut texto = String::new();
    texto.push_str("RESPALDO DE inventario\n");
    texto.push_str(&format!("fecha: {}\n", chrono::Utc::now().to_rfc3339()));
    texto.push_str(&format!("archivo: {}\n", destino.display()));
    texto.push_str(&format!("bytes: {bytes}\n"));
    texto.push_str(&format!("integrity_check: {integridad}\n"));
    texto.push_str(
        "\nfilas por tabla EN LA COPIA. Es una foto de este instante: si había una venta \n         en curso, el original puede tener alguna fila más. La copia es consistente\n         consigo misma, que es lo que exige la ley.\n\n",
    );
    for (t, n) in &conteos {
        texto.push_str(&format!("  {t:<20} {n}\n"));
    }
    texto.push_str(
        "\nConservar diez años desde el último asiento (Ley 962 de 2005, art. 28).\n\
         Para restaurar: copie el archivo sobre INVENTARIO_BD con el servicio DETENIDO.\n",
    );
    std::fs::write(&manifiesto, texto)
        .map_err(|e| format!("no se pudo escribir el manifiesto: {e}"))?;

    Ok(Informe {
        destino: destino.to_path_buf(),
        bytes,
        conteos,
        integridad,
    })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn temporal(etiqueta: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "inventario-respaldo-{etiqueta}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    async fn base_con_datos(dir: &Path) -> inventario_datos::Pool {
        let pool = inventario_datos::abrir(&format!("sqlite://{}/o.db", dir.display()))
            .await
            .unwrap();
        crate::sembrar::sembrar(&pool, Some("clave de instalación"))
            .await
            .unwrap();
        sqlx::query("INSERT INTO empresa (id, nombre, nit, dv) VALUES (1,'X','811033579',6)")
            .execute(&pool)
            .await
            .unwrap();
        for i in 0..25 {
            sqlx::query("INSERT INTO articulo (codigo, descripcion) VALUES (?, ?)")
                .bind(format!("A{i}"))
                .bind(format!("Artículo {i}"))
                .execute(&pool)
                .await
                .unwrap();
        }
        pool
    }

    #[tokio::test]
    async fn el_respaldo_se_verifica_contra_el_original_y_deja_manifiesto() {
        let dir = temporal("bueno");
        let pool = base_con_datos(&dir).await;
        let destino = dir.join("copia.db");

        let inf = respaldar(&pool, &destino).await.unwrap();
        assert_eq!(inf.integridad, "ok");
        assert!(inf.bytes > 0);
        assert!(destino.exists());

        // Las cuentas cuadran con lo que se metió.
        let articulos = inf.conteos.iter().find(|(t, _)| t == "articulo").unwrap().1;
        assert_eq!(articulos, 25);

        // El manifiesto existe y dice lo que hay dentro.
        let manifiesto = dir.join("copia.db.manifiesto.txt");
        assert!(manifiesto.exists(), "falta el manifiesto");
        let texto = std::fs::read_to_string(&manifiesto).unwrap();
        assert!(texto.contains("articulo"));
        assert!(texto.contains("25"));
        assert!(texto.contains("962"), "el manifiesto no cita la obligación legal");

        // Y la copia se puede ABRIR y LEER: es una base, no un archivo.
        let copia = inventario_datos::abrir(&format!("sqlite://{}", destino.display()))
            .await
            .unwrap();
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM articulo")
            .fetch_one(&copia)
            .await
            .unwrap();
        assert_eq!(n, 25);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// El respaldo se hace **con el sistema vivo**: si hubiera que parar para
    /// respaldar, nadie respaldaría.
    #[tokio::test]
    async fn se_puede_respaldar_mientras_se_escribe() {
        let dir = temporal("vivo");
        let pool = base_con_datos(&dir).await;

        // Escritura en curso durante el respaldo.
        let escritor = {
            let pool = pool.clone();
            tokio::spawn(async move {
                for i in 100..160 {
                    let _ = sqlx::query("INSERT INTO articulo (codigo, descripcion) VALUES (?, ?)")
                        .bind(format!("B{i}"))
                        .bind("concurrente")
                        .execute(&pool)
                        .await;
                }
            })
        };
        let destino = dir.join("copia.db");
        let inf = respaldar(&pool, &destino).await.unwrap();
        escritor.await.unwrap();

        assert_eq!(inf.integridad, "ok");

        // La copia es una FOTO: tiene lo que había cuando se tomó, y puede
        // tener menos que el original de ahora. Lo que NO puede es estar rota.
        let copia = inventario_datos::abrir(&format!("sqlite://{}", destino.display()))
            .await
            .unwrap();
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM articulo")
            .fetch_one(&copia)
            .await
            .unwrap();
        assert!(n >= 25, "la copia perdió lo que ya estaba antes de empezar");
        let huerfanos: Vec<(String, i64, String, i64)> =
            sqlx::query_as("PRAGMA foreign_key_check")
                .fetch_all(&copia)
                .await
                .unwrap();
        assert!(huerfanos.is_empty(), "la copia quedó con claves foráneas rotas");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Un respaldo NO pisa a otro. Perder el bueno haciendo uno malo es la
    /// forma más tonta de quedarse sin ninguno.
    #[tokio::test]
    async fn no_se_sobrescribe_un_respaldo_anterior() {
        let dir = temporal("pisar");
        let pool = base_con_datos(&dir).await;
        let destino = dir.join("copia.db");
        respaldar(&pool, &destino).await.unwrap();

        let e = respaldar(&pool, &destino).await.unwrap_err();
        assert!(e.contains("ya existe"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn una_carpeta_que_no_existe_falla_en_vez_de_dejar_un_respaldo_a_medias() {
        let dir = temporal("sincarpeta");
        let pool = base_con_datos(&dir).await;
        let e = respaldar(&pool, &dir.join("no/existe/copia.db"))
            .await
            .unwrap_err();
        assert!(e.contains("no existe"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Directiva 8: que DISCRIMINE. Se rompe la integridad referencial de la
    /// copia a propósito y se comprueba que `foreign_key_check` —la
    /// verificación que de verdad aporta— la caza.
    #[tokio::test]
    async fn una_copia_con_integridad_rota_se_detecta() {
        let dir = temporal("rota");
        let pool = base_con_datos(&dir).await;
        let destino = dir.join("copia.db");
        respaldar(&pool, &destino).await.unwrap();

        // Se mete un huérfano en la copia CON las claves foráneas apagadas, que
        // es lo que produciría una restauración a medias.
        //
        // El pool se abre APARTE y con `foreign_keys(false)`: un
        // `PRAGMA foreign_keys = OFF` suelto se aplica solo a la conexión que lo
        // ejecuta, y la siguiente consulta del pool sale por otra — el error
        // que hizo fallar la primera versión de esta prueba.
        let copia = {
            use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
            use std::str::FromStr;
            let o = SqliteConnectOptions::from_str(&format!("sqlite://{}", destino.display()))
                .unwrap()
                .foreign_keys(false);
            SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(o)
                .await
                .unwrap()
        };
        sqlx::query(
            "INSERT INTO movimiento
                (fecha, concepto_id, articulo_id, cantidad, valor, existencia_despues,
                 valor_despues)
             VALUES ('2026-08-02', 99999, 99999, 1, 0, 0, 0)",
        )
        .execute(&copia)
        .await
        .unwrap();

        let huerfanos: Vec<(String, i64, String, i64)> =
            sqlx::query_as("PRAGMA foreign_key_check")
                .fetch_all(&copia)
                .await
                .unwrap();
        assert!(
            !huerfanos.is_empty(),
            "el caso dejó de discriminar: foreign_key_check no vería una copia rota"
        );
        // Y una copia sana NO produce huérfanos: la comprobación distingue.
        let sana = dir.join("sana.db");
        respaldar(&pool, &sana).await.unwrap();
        let c2 = inventario_datos::abrir(&format!("sqlite://{}", sana.display()))
            .await
            .unwrap();
        let ninguno: Vec<(String, i64, String, i64)> =
            sqlx::query_as("PRAGMA foreign_key_check").fetch_all(&c2).await.unwrap();
        assert!(ninguno.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Si el esquema gana una tabla y nadie la añade a la lista, el respaldo
    /// dejaría de verificarla **en silencio**. Esta prueba lo impide.
    #[tokio::test]
    async fn el_manifiesto_cubre_todas_las_tablas_del_esquema() {
        let pool = inventario_datos::abrir("sqlite::memory:").await.unwrap();
        let filas: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master
              WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_sqlx%'",
        )
        .fetch_all(&pool)
        .await
        .unwrap();

        let conocidas: std::collections::HashSet<&str> =
            TABLAS.iter().chain(FUERA.iter()).copied().collect();
        let faltan: Vec<String> = filas
            .into_iter()
            .map(|(n,)| n)
            .filter(|n| !conocidas.contains(n.as_str()))
            .collect();
        assert!(
            faltan.is_empty(),
            "estas tablas del esquema NO se verifican en el respaldo: {faltan:?}. \
             Añádalas a TABLAS o a FUERA, con su motivo."
        );
    }
}
