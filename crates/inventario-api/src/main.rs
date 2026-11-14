//! El servidor de inventario: lee la configuración, abre la base y sirve la API.

mod claves;
mod config;
mod dbf;
mod demo;
mod migrar;
mod respaldo;
mod rutas;
mod sembrar;
mod sesion;
/// El panel, tejido en el servidor con maud (directiva 26).
mod vista;

use std::process::ExitCode;
use std::sync::Arc;

const USO: &str = "uso: inventario-api [servir | clave-firma | hashear <clave> | \
                   instalar | migrar <carpeta-dbf> | respaldar <archivo> | \
                   sembrar-demo]";

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        None | Some("servir") => servir().await,

        // Genera la clave de firma de sesión. Se corre UNA vez al instalar y su
        // salida va al entorno. No se genera al arrancar a propósito: una clave
        // nueva en cada reinicio cerraría todas las sesiones sin explicar por qué.
        Some("clave-firma") => match clave_aleatoria() {
            Ok(hex) => {
                println!("INVENTARIO_CLAVE_FIRMA={hex}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("no se pudo generar la clave: {e}");
                ExitCode::FAILURE
            }
        },

        // Produce un hash Argon2 con los MISMOS parámetros que verifica el
        // acceso. Un hash de otra herramienta deja al usuario fuera y el fallo
        // aparece lejos de su causa.
        Some("hashear") => match args.get(2).filter(|c| !c.is_empty()) {
            None => {
                eprintln!("uso: inventario-api hashear <clave>");
                ExitCode::FAILURE
            }
            Some(clave) => match claves::hashear(clave) {
                Some(h) => {
                    println!("{h}");
                    ExitCode::SUCCESS
                }
                None => {
                    eprintln!("no se pudo generar el hash");
                    ExitCode::FAILURE
                }
            },
        },

        // Crea la empresa, el usuario administrador y los conceptos de kardex.
        Some("instalar") => sembrar::instalar().await,

        // Migra el sistema Visual FoxPro. Ver `migrar.rs`.
        Some("migrar") => migrar::ejecutar(args.get(2).map(String::as_str)).await,

        // Copia de seguridad VERIFICADA. Ver `respaldo.rs`.
        Some("respaldar") => respaldo::ejecutar(args.get(2).map(String::as_str)).await,

        // Siembra la DEMOSTRACIÓN pública. Solo funciona si el entorno define
        // INVENTARIO_DEMO_USUARIO e INVENTARIO_DEMO_CLAVE; una instalación de
        // cliente no las define y este subcomando se niega.
        Some("sembrar-demo") => sembrar_demo().await,

        // Un subcomando mal escrito NO arranca el servidor «por si acaso»: se
        // rechaza. Arrancarlo sería obedecer algo que nadie pidió.
        Some(otro) => {
            eprintln!("subcomando desconocido: {otro:?}\n{USO}");
            ExitCode::FAILURE
        }
    }
}

async fn sembrar_demo() -> ExitCode {
    let url = std::env::var("INVENTARIO_BD").unwrap_or_else(|_| "sqlite://inventario.db".into());
    let pool = match inventario_datos::abrir(&url).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("no se pudo abrir la base ({url}): {e}");
            return ExitCode::FAILURE;
        }
    };
    // Los conceptos y el administrador van primero: la demo se monta encima de
    // una instalación normal, no en lugar de ella.
    if let Err(e) =
        sembrar::sembrar(&pool, std::env::var("INVENTARIO_CLAVE_ADMIN").ok().as_deref()).await
    {
        eprintln!("instalación base: {e}");
        return ExitCode::FAILURE;
    }
    match demo::sembrar(&pool).await {
        Ok(msgs) => {
            for m in msgs {
                println!("{m}");
            }
            println!("Demostración sembrada en {url}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("sembrar-demo: {e}");
            ExitCode::FAILURE
        }
    }
}

fn clave_aleatoria() -> std::io::Result<String> {
    use std::io::Read;
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

async fn servir() -> ExitCode {
    let cfg = match config::Config::desde_entorno() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("configuración: {e}");
            return ExitCode::FAILURE;
        }
    };

    let pool = match inventario_datos::abrir(&cfg.url_bd).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("no se pudo abrir la base ({}): {e}", cfg.url_bd);
            return ExitCode::FAILURE;
        }
    };

    let app = rutas::router(rutas::Estado {
        pool,
        clave_firma: Arc::new(cfg.clave_firma),
        cookie_segura: cfg.cookie_segura,
        horas_sesion: cfg.horas_sesion,
    });

    let escucha = match tokio::net::TcpListener::bind(&cfg.direccion).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("no se pudo escuchar en {}: {e}", cfg.direccion);
            return ExitCode::FAILURE;
        }
    };
    // Se imprime la dirección REAL, no la pedida: con el puerto 0 son distintas,
    // y es como la simulación de extremo a extremo sabe a dónde hablar.
    match escucha.local_addr() {
        Ok(d) => println!("inventario escuchando en http://{d}"),
        Err(e) => eprintln!("aviso: no se pudo leer la dirección local: {e}"),
    }

    if let Err(e) = axum::serve(escucha, app).await {
        eprintln!("el servidor terminó con error: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
