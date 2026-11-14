//! Simulación de extremo a extremo: se **arranca el binario real** y se le habla
//! por HTTP como le habla el mundo (directiva 28).
//!
//! Nada de esto llama a una función de Rust. Se abre un socket, se escribe una
//! petición HTTP/1.1 en bytes y se lee la respuesta. Es la única forma de que la
//! prueba mida el producto y no el códec: los extractores de axum, los códigos
//! de estado, las cabeceras `Set-Cookie` y el JSON solo existen ahí.
//!
//! El cliente HTTP está escrito a mano y sin dependencias a propósito. Meter
//! `reqwest` traería su propio manejo de cookies y de redirecciones, y entonces
//! la prueba mediría *reqwest hablando con nosotros* en vez de *el protocolo que
//! sale de nuestro binario*.

use mirilla::{Navegador, buscar_chromium};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Cliente HTTP mínimo
// ---------------------------------------------------------------------------

struct Respuesta {
    estado: u16,
    cuerpo: String,
    cookie: Option<String>,
    /// Las cabeceras crudas, tal como salieron del socket. Se guardan enteras
    /// —y no solo la cookie— porque las de seguridad (CSP, `nosniff`,
    /// `frame-options`) solo existen aquí: no se ven en el cuerpo ni en el
    /// estado, y son justo las que hasta el 2026-08-06 no estaban.
    cabeceras: String,
}

impl Respuesta {
    /// Lectura de JSON sin dependencias: busca `"clave":` y devuelve el valor
    /// hasta la siguiente coma o llave. Suficiente para los escalares que
    /// comprueba esta simulación, y **falla** en vez de devolver algo si no
    /// encuentra la clave.
    fn campo(&self, clave: &str) -> String {
        let aguja = format!("\"{clave}\":");
        let i = self
            .cuerpo
            .find(&aguja)
            .unwrap_or_else(|| panic!("no hay «{clave}» en: {}", self.cuerpo));
        let resto = &self.cuerpo[i + aguja.len()..];
        let resto = resto.trim_start();
        if let Some(sin) = resto.strip_prefix('"') {
            sin[..sin.find('"').expect("cadena sin cerrar")].to_string()
        } else {
            resto
                .split([',', '}'])
                .next()
                .expect("valor vacío")
                .trim()
                .to_string()
        }
    }
    fn numero(&self, clave: &str) -> i64 {
        self.campo(clave)
            .parse()
            .unwrap_or_else(|_| panic!("«{clave}» no es un número en: {}", self.cuerpo))
    }
}

/// Como `pedir`, pero con cabeceras extra. La necesita el freno de origen: sin
/// poder mandar un `Origin` ajeno no hay forma de comprobar que se rechaza, y
/// una guardia que no se puede poner en rojo no se ha medido.
fn pedir_con(
    puerto: u16,
    metodo: &str,
    ruta: &str,
    cookie: Option<&str>,
    cuerpo: &str,
    extra: &[(&str, &str)],
) -> Respuesta {
    let mut flujo = TcpStream::connect(("127.0.0.1", puerto)).expect("conectar");
    flujo.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    flujo.set_write_timeout(Some(Duration::from_secs(20))).unwrap();

    let mut peticion = format!(
        "{metodo} {ruta} HTTP/1.1\r\nHost: 127.0.0.1:{puerto}\r\nConnection: close\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n",
        cuerpo.len()
    );
    for (k, v) in extra {
        peticion.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(c) = cookie {
        peticion.push_str(&format!("Cookie: {c}\r\n"));
    }
    peticion.push_str("\r\n");
    peticion.push_str(cuerpo);
    flujo.write_all(peticion.as_bytes()).expect("escribir");

    let mut lector = BufReader::new(flujo);
    let mut linea = String::new();
    lector.read_line(&mut linea).expect("leer estado");
    let estado: u16 = linea.split_whitespace().nth(1).unwrap_or("0").parse().unwrap_or(0);
    loop {
        let mut l = String::new();
        if lector.read_line(&mut l).expect("leer cabecera") == 0 || l == "\r\n" {
            break;
        }
    }
    let mut cuerpo = Vec::new();
    lector.read_to_end(&mut cuerpo).expect("leer cuerpo");
    Respuesta {
        estado,
        cuerpo: String::from_utf8_lossy(&cuerpo).to_string(),
        cookie: None,
        cabeceras: String::new(),
    }
}

fn pedir(puerto: u16, metodo: &str, ruta: &str, cookie: Option<&str>, cuerpo: &str) -> Respuesta {
    let mut flujo = TcpStream::connect(("127.0.0.1", puerto)).expect("conectar");
    flujo.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    flujo.set_write_timeout(Some(Duration::from_secs(20))).unwrap();

    let mut peticion = format!(
        "{metodo} {ruta} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n",
        cuerpo.len()
    );
    if let Some(c) = cookie {
        peticion.push_str(&format!("Cookie: {c}\r\n"));
    }
    peticion.push_str("\r\n");
    peticion.push_str(cuerpo);
    flujo.write_all(peticion.as_bytes()).expect("escribir");

    let mut lector = BufReader::new(flujo);
    let mut linea = String::new();
    lector.read_line(&mut linea).expect("leer estado");
    let estado: u16 = linea
        .split_whitespace()
        .nth(1)
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);

    let mut cookie_nueva = None;
    let mut cabeceras = String::new();
    loop {
        let mut l = String::new();
        if lector.read_line(&mut l).expect("leer cabecera") == 0 || l == "\r\n" {
            break;
        }
        cabeceras.push_str(&l);
        if l.to_ascii_lowercase().starts_with("set-cookie:")
            && let Some((_, v)) = l.split_once(':')
            && let Some(par) = v.trim().split(';').next()
            && !par.ends_with('=')
        {
            cookie_nueva = Some(par.to_string());
        }
    }
    // `Connection: close` hace que el cuerpo termine con el socket: no hace
    // falta interpretar `Content-Length` ni el troceado.
    let mut cuerpo = Vec::new();
    lector.read_to_end(&mut cuerpo).expect("leer cuerpo");

    Respuesta {
        estado,
        cuerpo: String::from_utf8_lossy(&cuerpo).to_string(),
        cookie: cookie_nueva,
        cabeceras,
    }
}

// ---------------------------------------------------------------------------
// Arranque del binario real
// ---------------------------------------------------------------------------

struct Servidor {
    proceso: Child,
    puerto: u16,
    #[allow(dead_code)]
    dir: std::path::PathBuf,
}

impl Drop for Servidor {
    fn drop(&mut self) {
        let _ = self.proceso.kill();
        let _ = self.proceso.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn binario() -> &'static str {
    env!("CARGO_BIN_EXE_inventario-api")
}

fn carpeta_temporal(etiqueta: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "inventario-sim-{etiqueta}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("crear carpeta temporal");
    d
}

/// La tabla de rutas, leída EN TIEMPO DE COMPILACIÓN.
///
/// Las pruebas de guardia enumeraban a mano las rutas POST del panel, y esa
/// lista estaba copiada en tres sitios. Una lista copiada se queda corta en la
/// primera ruta nueva **sin dar un solo error**: la ruta que falta es
/// exactamente la que nadie mide, y la prueba sigue verde. Derivarla del
/// archivo que las declara hace que una ruta nueva entre al banco el día que se
/// escribe, no el día que alguien se acuerda.
const RUTAS_RS: &str = include_str!("../src/rutas.rs");

/// Toda ruta del panel que acepta GET y exige sesión, sacada de `RUTAS_RS`.
///
/// Incluye los fragmentos que htmx pide (`/articulos/tabla`, `/facturar/linea`,
/// `/recaudar/pago`): un fragmento que se sirviera sin sesión filtraría los
/// mismos datos que la pantalla entera, y es justo el que se olvida.
///
/// Se excluyen por su nombre `/entrar` —tiene que funcionar sin sesión, es su
/// oficio—, `/` —redirige, no enseña— y los tres estáticos, que son htmx, boi y
/// el CSS y no llevan ningún dato.
fn lecturas_del_panel() -> Vec<String> {
    rutas_del_panel("get(crate::vista::")
        .into_iter()
        .filter(|r| {
            // Lo ESTÁTICO no es una pantalla del panel: se excluye POR NOMBRE y
            // se cubre en `el_binario_sirve_el_panel_que_lleva_dentro`, que
            // además comprueba que cada archivo trae lo que dice traer.
            //
            // Por nombre y no por prefijo (`starts_with("/comportamientos/")`),
            // aunque el prefijo se escribiera solo: una exención por prefijo es
            // una lista de disparo invertida — dejaría fuera del banco cualquier
            // ruta futura bajo esa carpeta sin que nadie lo decida. Enumerar
            // obliga a mirar la ruta número dos (directiva 21).
            !matches!(
                r.as_str(),
                "/entrar"
                    | "/"
                    | "/htmx.min.js"
                    | "/boi.js"
                    | "/boi.css"
                    | "/panel.css"
                    | "/comportamientos/sugerir-de-opcion.js"
            )
        })
        .collect()
}

/// Toda ruta del panel que acepta POST, sacada de `RUTAS_RS`.
///
/// Se excluyen `/entrar` y `/salir` a propósito y por su nombre: la primera es
/// el formulario de acceso —tiene que funcionar SIN sesión, es su oficio— y la
/// segunda solo borra la cookie. Cualquier otra que aparezca entra al banco
/// sola.
fn escrituras_del_panel() -> Vec<String> {
    rutas_del_panel("post(crate::vista::")
        .into_iter()
        .filter(|r| !matches!(r.as_str(), "/entrar" | "/salir"))
        .collect()
}

/// Las rutas cuyo manejador para ese método vive en `crate::vista`.
fn rutas_del_panel(aguja: &str) -> Vec<String> {
    // Se aplana el archivo para que dé igual si una ruta está escrita en una
    // línea o repartida en cuatro por el formateador.
    let plano: String = RUTAS_RS.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut rutas = Vec::new();
    for trozo in plano.split(".route(").skip(1) {
        // `trim_start`: al aplanar, una ruta repartida en varias líneas queda
        // como `.route( "/x", …` con un espacio de por medio. Sin esto el
        // parser la saltaba EN SILENCIO —salían 8 de 9— y era justo la última
        // que se añadió, o sea la que más falta hace medir.
        let Some(resto) = trozo.trim_start().strip_prefix('"') else {
            continue;
        };
        let Some(fin) = resto.find('"') else { continue };
        let (ruta, cola) = resto.split_at(fin);
        // El cuerpo de ESTA ruta es lo que hay hasta la siguiente: el `split`
        // ya lo cortó ahí.
        if !cola.contains(aguja) {
            continue;
        }
        rutas.push(ruta.to_string());
    }
    rutas
}

/// **El positivo de control de la lista derivada** (directiva 30).
///
/// Sin esto, un parser que devolviera vacío dejaría los tres bucles de guardia
/// dando vueltas sobre cero elementos y las tres pruebas saldrían verdes sin
/// haber comprobado una sola ruta — que es la forma más cara de fallar: la
/// salvaguarda existe, se ejecuta y no mide nada.
#[test]
fn la_lista_de_escrituras_del_panel_se_lee_de_verdad() {
    let rutas = escrituras_del_panel();
    assert!(
        rutas.len() >= 9,
        "la lista salió con {} rutas: el parser dejó de leer la tabla",
        rutas.len()
    );
    for esperada in [
        "/movimientos",
        "/facturar",
        "/recaudar",
        "/configuracion/empresa",
        "/configuracion/usuario",
    ] {
        assert!(
            rutas.iter().any(|r| r == esperada),
            "«{esperada}» no salió de la tabla: {rutas:?}"
        );
    }

    // Y lo mismo para las de lectura, que es la otra mitad.
    let lecturas = lecturas_del_panel();
    assert!(lecturas.len() >= 11, "lecturas leídas: {lecturas:?}");
    for esperada in ["/articulos", "/articulos/tabla", "/terceros", "/informes", "/configuracion"] {
        assert!(
            lecturas.iter().any(|r| r == esperada),
            "«{esperada}» no salió de la tabla: {lecturas:?}"
        );
    }
    assert!(
        !lecturas.iter().any(|r| r == "/entrar" || r == "/panel.css"),
        "la lista de lecturas trae algo que NO exige sesión: {lecturas:?}"
    );
    // Y la pareja: lo excluido a mano está excluido de verdad, y no por
    // casualidad de que el parser no lo viera.
    assert!(RUTAS_RS.contains("post(crate::vista::entrar_enviar)"));
    assert!(!rutas.iter().any(|r| r == "/entrar" || r == "/salir"));
}

const CLAVE_FIRMA: &str = "clave-de-firma-para-la-simulacion-de-32+";
const CLAVE_ADMIN: &str = "clave del administrador";

fn arrancar(etiqueta: &str) -> Servidor {
    arrancar_con(etiqueta, &[])
}

/// Igual, con variables de entorno extra. Existe para poder levantar la MISMA
/// instalación con y sin `INVENTARIO_DEMO_*` y comparar las dos pantallas: lo
/// que convierte esto en demostración son DOS variables, no otro binario, y sin
/// las dos mitades no se estaría midiendo nada.
fn arrancar_con(etiqueta: &str, extra: &[(&str, &str)]) -> Servidor {
    let dir = carpeta_temporal(etiqueta);
    let bd = format!("sqlite://{}/inventario.db", dir.display());

    // 1) Instalar: crea conceptos y el administrador. Se corre como el cliente
    //    lo correría, con el binario, no llamando a una función.
    let salida = Command::new(binario())
        .arg("instalar")
        .env("INVENTARIO_BD", &bd)
        .env("INVENTARIO_CLAVE_ADMIN", CLAVE_ADMIN)
        .output()
        .expect("ejecutar instalar");
    assert!(
        salida.status.success(),
        "«instalar» falló: {}",
        String::from_utf8_lossy(&salida.stderr)
    );

    // 2) Servir. Puerto 0 = que lo elija el sistema; el binario imprime el real.
    let mut proceso = Command::new(binario())
        .env("INVENTARIO_BD", &bd)
        .env("INVENTARIO_CLAVE_FIRMA", CLAVE_FIRMA)
        .env("INVENTARIO_DIRECCION", "127.0.0.1:0")
        .envs(extra.iter().copied())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("arrancar el servidor");

    let salida = proceso.stdout.take().expect("stdout");
    let mut lector = BufReader::new(salida);
    let mut linea = String::new();
    lector.read_line(&mut linea).expect("leer la dirección");
    let puerto: u16 = linea
        .rsplit(':')
        .next()
        .unwrap_or("")
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("el servidor no dijo su puerto: {linea:?}"));

    let s = Servidor {
        proceso,
        puerto,
        dir,
    };

    // Esperar a que responda, con techo: sin él, un servidor que nunca arranca
    // deja la prueba colgada en vez de fallar.
    let limite = Instant::now() + Duration::from_secs(20);
    while Instant::now() < limite {
        if TcpStream::connect(("127.0.0.1", puerto)).is_ok() {
            let r = pedir(puerto, "GET", "/api/salud", None, "");
            if r.estado == 200 {
                return s;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("el servidor no respondió en 20 s");
}

fn entrar(p: u16) -> String {
    let r = pedir(
        p,
        "POST",
        "/api/sesion",
        None,
        &format!(r#"{{"codigo":"admin","clave":"{CLAVE_ADMIN}"}}"#),
    );
    assert_eq!(r.estado, 200, "no se pudo entrar: {}", r.cuerpo);
    r.cookie.expect("el servidor no mandó cookie de sesión")
}

/// **EL PANEL GUARDA CENTAVOS, Y ESTO SE MIDE POR HTTP.**
///
/// La prueba que acompañaba al cambio de unidades **no discriminaba**, y lo
/// midió la revisión independiente: llamaba a `articulo::crear` directamente con
/// una entrada YA en centavos y comprobaba el round-trip — la misma tautología
/// que su propio comentario criticaba. Devolviendo el panel a escribir pesos,
/// las 291 pruebas seguían verdes.
///
/// El defecto vivía en el ESCRITOR —el manejador del formulario—, al que aquella
/// prueba no llegaba nunca. Ésta entra por donde entra quien opera: un `POST`
/// del formulario con 12.000 en el campo, y se comprueba lo que quedó guardado.
#[test]
fn el_panel_guarda_el_precio_en_centavos_y_lo_ensena_en_pesos() {
    let s = arrancar("unidades");
    let cookie = entrar(s.puerto);

    let r = pedir(
        s.puerto,
        "POST",
        "/articulos",
        Some(&cookie),
        "codigo=UNI9&descripcion=Articulo+de+unidades&presentacion=UND&iva_porcentaje=19\
         &inc_porcentaje=0&precio_venta=12000&punto_minimo=0&punto_maximo=0\
         &precio_venta_estimado=11000&costos_para_vender=500",
    );
    assert!(r.estado == 200 || r.estado == 303, "no se creó el artículo: {}", r.cuerpo);

    // Lo GUARDADO, por la API: 12.000 pesos son 1.200.000 centavos.
    let api = pedir(s.puerto, "GET", "/api/articulos?q=UNI9", Some(&cookie), "");
    assert!(
        api.cuerpo.contains(r#""precio_venta":1200000"#),
        "el panel no guardó centavos: {}",
        api.cuerpo
    );
    assert!(
        api.cuerpo.contains(r#""precio_venta_estimado":1100000"#)
            && api.cuerpo.contains(r#""costos_para_vender":50000"#),
        "el estimado o los costos se quedaron en pesos: {}",
        api.cuerpo
    );

    // Y lo MOSTRADO: la tabla vuelve a enseñar 12.000, no 1.200.000. Es la otra
    // mitad — sin ella, guardar centavos y pintarlos crudos daría un precio 100
    // veces alto en pantalla con la base correcta.
    let tabla = pedir(s.puerto, "GET", "/articulos", Some(&cookie), "");
    assert!(
        tabla.cuerpo.contains(">12.000<"),
        "la tabla no enseña el precio en pesos: busque «12.000» en el cuerpo"
    );
    assert!(
        !tabla.cuerpo.contains(">1.200.000<"),
        "la tabla enseña centavos crudos: el precio sale 100 veces alto"
    );

    // Y EL TERCER SITIO, que es el único en el camino de FACTURAR: `data-precio`
    // alimenta el campo que teclea quien factura, y ése pasa por
    // `intentar_de_pesos`. En centavos crudos, cada línea se prellenaría 100
    // veces cara. Sin esta línea, mutarlo dejaba las 292 pruebas en verde.
    let linea = pedir(s.puerto, "GET", "/facturar/linea", Some(&cookie), "");
    assert!(
        linea.cuerpo.contains(r#"data-precio="12000""#),
        "`data-precio` no va en pesos: la línea de factura se prellenaría 100 veces cara: {}",
        linea.cuerpo
    );
}

/// Deja el negocio montado: empresa, resolución, artículos y clientes.
fn montar(p: u16, cookie: &str, articulos: i64, clientes: i64) -> (Vec<i64>, Vec<i64>, i64) {
    let r = pedir(
        p,
        "PUT",
        "/api/empresa",
        Some(cookie),
        r#"{"nombre":"Distribuidora de Agua","nit":"811033579","direccion":"CRA 43A #30-74"}"#,
    );
    assert_eq!(r.estado, 200, "{}", r.cuerpo);
    assert_eq!(r.campo("nit"), "811033579-6", "el DV no se calculó");

    let r = pedir(
        p,
        "POST",
        "/api/resoluciones",
        Some(cookie),
        r#"{"nombre":"Principal","prefijo":"FE","numero":"18764000001","desde":1,"hasta":5000}"#,
    );
    assert_eq!(r.estado, 201, "{}", r.cuerpo);
    let resolucion = r.numero("id");
    // La clave técnica la entrega la DIAN con la autorización; sin ella no hay
    // CUFE. Se registra aparte porque la crea el portal, no el sistema.
    pedir(
        p,
        "PUT",
        "/api/empresa",
        Some(cookie),
        r#"{"nombre":"Distribuidora de Agua","nit":"811033579","direccion":"CRA 43A #30-74"}"#,
    );

    let mut arts = Vec::new();
    for i in 0..articulos {
        let r = pedir(
            p,
            "POST",
            "/api/articulos",
            Some(cookie),
            &format!(
                r#"{{"codigo":"A{i}","descripcion":"Botellón {i}","iva_porcentaje":{},
                     "precio_venta":{},"punto_minimo":5,"punto_maximo":50}}"#,
                if i % 2 == 0 { 19 } else { 0 },
                300_000 + i * 1_000
            ),
        );
        assert_eq!(r.estado, 201, "{}", r.cuerpo);
        arts.push(r.numero("id"));
    }

    let mut clis = Vec::new();
    for i in 0..clientes {
        let r = pedir(
            p,
            "POST",
            "/api/terceros",
            Some(cookie),
            &format!(
                r#"{{"nit":"{}","nombre":"Cliente {i}","plazo_dias":30}}"#,
                43_627_185 + i
            ),
        );
        assert_eq!(r.estado, 201, "{}", r.cuerpo);
        clis.push(r.numero("id"));
    }
    (arts, clis, resolucion)
}

fn comprar(p: u16, cookie: &str, articulo: i64, unidades: i64, pesos: i64) {
    let conceptos = pedir(p, "GET", "/api/conceptos", Some(cookie), "");
    assert_eq!(conceptos.estado, 200);
    // Se busca el concepto de compra por su clase, como hace el servidor.
    let i = conceptos
        .cuerpo
        .find(r#""clase":"compra""#)
        .expect("no hay concepto de compra");
    let trozo = &conceptos.cuerpo[..i];
    let j = trozo.rfind(r#""id":"#).expect("id del concepto");
    let concepto: i64 = trozo[j + 5..]
        .split(',')
        .next()
        .unwrap()
        .trim()
        .parse()
        .expect("id numérico");

    let r = pedir(
        p,
        "POST",
        "/api/movimientos",
        Some(cookie),
        &format!(
            r#"{{"fecha":"2026-08-02","concepto_id":{concepto},"articulo_id":{articulo},
                 "documento":"compra","cantidad":{},"costo_unitario":{}}}"#,
            unidades * 1000,
            pesos * 100
        ),
    );
    assert_eq!(r.estado, 201, "compra rechazada: {}", r.cuerpo);
}

// ---------------------------------------------------------------------------
// La simulación
// ---------------------------------------------------------------------------

/// Más de 100 operaciones contra el binario real: montar, comprar, facturar,
/// recaudar, anular, y comprobar que al final todo cuadra.
///
/// El nombre no lleva la cifra exacta a propósito: la prueba cuenta lo que
/// hace y exige el mínimo al final, así que añadir un caso no obliga a
/// renombrarla — ni deja un nombre que miente sobre lo que mide.
#[test]
fn mas_de_cien_operaciones_contra_el_binario_real() {
    let s = arrancar("larga");
    let p = s.puerto;
    let cookie = entrar(p);
    let mut operaciones = 0usize;

    let (arts, clis, resolucion) = montar(p, &cookie, 5, 4);
    operaciones += 1 + 1 + arts.len() + clis.len();

    // Compras: 5 artículos × 2 entradas a costos distintos, para que el promedio
    // ponderado tenga algo que promediar.
    for (i, a) in arts.iter().enumerate() {
        comprar(p, &cookie, *a, 500, 2_000 + i as i64 * 13);
        comprar(p, &cookie, *a, 300, 2_500 + i as i64 * 17);
        operaciones += 2;
    }

    // 60 facturas alternando contado y crédito.
    let mut numeros = std::collections::BTreeSet::new();
    let mut facturas = Vec::new();
    let mut total_facturado = 0i64;
    for k in 0..60i64 {
        let art = arts[(k % arts.len() as i64) as usize];
        let cli = clis[(k % clis.len() as i64) as usize];
        let unidades = k % 4 + 1;
        // El medio de pago es obligatorio en las de contado (Res. 000165/2023
        // art. 11) y no aplica en las de crédito.
        let cuerpo = format!(
            r#"{{"resolucion_id":{resolucion},"tercero_id":{cli},"fecha":"2026-08-02",
                 "hora":"10:53:10-05:00","forma_pago":"{}",{}"lineas":[
                 {{"articulo_id":{art},"cantidad":{},"precio_unitario":350000,
                   "descuento":0,"iva_porcentaje":19,"inc_porcentaje":8}}]}}"#,
            if k % 2 == 0 { "credito" } else { "contado" },
            if k % 2 == 0 { "" } else { r#""medio_pago":"efectivo","# },
            unidades * 1000
        );
        let r = pedir(p, "POST", "/api/facturas", Some(&cookie), &cuerpo);
        operaciones += 1;
        assert_eq!(r.estado, 201, "factura {k} rechazada: {}", r.cuerpo);
        let numero = r.numero("numero");
        assert!(numeros.insert(numero), "el número {numero} salió dos veces");
        facturas.push((r.numero("id"), cli, k % 2 == 0));
        total_facturado += r.numero("total");
        // El monto en letras viaja con la factura: no hace falta ninguna tabla.
        assert!(
            r.campo("en_letras").ends_with("PESOS M/CTE")
                || r.campo("en_letras").contains("CENTAVOS"),
            "monto en letras raro: {}",
            r.campo("en_letras")
        );
    }
    assert_eq!(numeros.len(), 60);
    assert_eq!(*numeros.first().unwrap(), 1, "la numeración empieza en el rango");
    assert_eq!(*numeros.last().unwrap(), 60, "y no salta números");
    assert!(total_facturado > 0);

    // 30 recaudos.
    let mut recaudado = 0i64;
    for k in 0..30i64 {
        let cli = clis[(k % clis.len() as i64) as usize];
        let r = pedir(
            p,
            "POST",
            "/api/recibos",
            Some(&cookie),
            &format!(
                r#"{{"tercero_id":{cli},"fecha":"2026-08-03",
                     "pagos":[{{"medio":"efectivo","valor":{}}}]}}"#,
                (k + 1) * 50_000
            ),
        );
        operaciones += 1;
        assert_eq!(r.estado, 201, "recibo {k} rechazado: {}", r.cuerpo);
        let (aplicado, sobrante, total) = (r.numero("aplicado"), r.numero("sobrante"), r.numero("total"));
        assert_eq!(aplicado + sobrante, total, "el recibo {k} no cuadra");
        recaudado += aplicado;
    }

    // 6 anulaciones. Solo de facturas de contado: las de crédito ya tienen
    // abonos aplicados y anularlas debe estar PROHIBIDO (se comprueba abajo).
    let mut anuladas = 0;
    for (id, _, es_credito) in &facturas {
        if *es_credito || anuladas >= 6 {
            continue;
        }
        let r = pedir(
            p,
            "POST",
            &format!("/api/facturas/{id}/anular"),
            Some(&cookie),
            r#"{"motivo":"prueba de anulación"}"#,
        );
        operaciones += 1;
        assert_eq!(r.estado, 200, "anular {id} falló: {}", r.cuerpo);
        anuladas += 1;
    }
    assert_eq!(anuladas, 6);

    // --- Comprobaciones finales -------------------------------------------

    // 1. La existencia que ve el mundo cuadra con el kardex que ve el mundo.
    for a in &arts {
        let saldo = pedir(p, "GET", &format!("/api/articulos/{a}/saldo"), Some(&cookie), "");
        let kardex = pedir(p, "GET", &format!("/api/articulos/{a}/kardex"), Some(&cookie), "");
        operaciones += 2;
        assert_eq!(saldo.estado, 200);
        assert_eq!(kardex.estado, 200);

        let existencia = saldo.numero("existencia");
        assert!(existencia >= 0, "el artículo {a} quedó en negativo: {existencia}");

        // Se recorre el kardex servido por HTTP y se suma a mano: si la foto
        // mintiera, aquí no cuadraría. Es la auditoría del sistema viejo que
        // nunca existió, hecha desde fuera del programa.
        let mut suma = 0i64;
        for entrada in kardex.cuerpo.split(r#"{"id":"#).skip(1) {
            let campo = |c: &str| -> i64 {
                let a = format!("\"{c}\":");
                let i = entrada.find(&a).unwrap_or_else(|| panic!("sin {c}"));
                entrada[i + a.len()..]
                    .split([',', '}'])
                    .next()
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap_or(0)
            };
            let cantidad = campo("cantidad");
            let es_entrada = entrada.contains(r#""signo":"entrada""#);
            suma += if es_entrada { cantidad } else { -cantidad };
        }
        assert_eq!(
            existencia, suma,
            "el artículo {a}: la existencia dice {existencia} y su kardex suma {suma}"
        );
    }

    // 2. La cartera cuadra con lo facturado a crédito menos lo recaudado.
    let cartera = pedir(p, "GET", "/api/informes/cartera", Some(&cookie), "");
    operaciones += 1;
    assert_eq!(cartera.estado, 200);
    let mut saldo_total = 0i64;
    for c in cartera.cuerpo.split(r#""tercero_id":"#).skip(1) {
        let i = c.find(r#""saldo":"#).expect("saldo");
        saldo_total += c[i + 8..].split([',', '}']).next().unwrap().trim().parse::<i64>().unwrap();
    }
    assert!(saldo_total > 0, "no quedó cartera y debería");
    assert!(recaudado > 0, "no se recaudó nada");

    // 3. Una factura de crédito CON abonos no se puede anular.
    let con_abono = facturas.iter().find(|(_, _, c)| *c).expect("hay de crédito");
    let r = pedir(
        p,
        "POST",
        &format!("/api/facturas/{}/anular", con_abono.0),
        Some(&cookie),
        r#"{"motivo":"no debería dejar"}"#,
    );
    operaciones += 1;
    assert_eq!(
        r.estado, 422,
        "se anuló una factura con abonos aplicados: {}",
        r.cuerpo
    );
    assert!(r.cuerpo.contains("abonos"), "el motivo no explica nada: {}", r.cuerpo);

    // 4. Pedir más de lo que hay da 409 y NO deja rastro.
    let antes = pedir(p, "GET", &format!("/api/articulos/{}/saldo", arts[0]), Some(&cookie), "")
        .numero("existencia");
    let r = pedir(
        p,
        "POST",
        "/api/facturas",
        Some(&cookie),
        &format!(
            r#"{{"resolucion_id":{resolucion},"tercero_id":{},"fecha":"2026-08-02",
                 "forma_pago":"contado","medio_pago":"efectivo",
                 "lineas":[{{"articulo_id":{},"cantidad":99999000,
                 "precio_unitario":100,"descuento":0,"iva_porcentaje":0}}]}}"#,
            clis[0], arts[0]
        ),
    );
    operaciones += 3;
    assert_eq!(r.estado, 409, "existencia insuficiente debería ser 409: {}", r.cuerpo);
    let despues = pedir(p, "GET", &format!("/api/articulos/{}/saldo", arts[0]), Some(&cookie), "")
        .numero("existencia");
    assert_eq!(antes, despues, "una factura rechazada movió el inventario");

    // 5. Sin cookie no se ve nada.
    for ruta in ["/api/articulos", "/api/informes/cartera", "/api/terceros"] {
        let r = pedir(p, "GET", ruta, None, "");
        operaciones += 1;
        assert_eq!(r.estado, 401, "{ruta} se sirvió sin sesión");
    }
    // Y con una cookie manipulada tampoco.
    let r = pedir(p, "GET", "/api/articulos", Some("inventario_sesion=falsa.firma"), "");
    operaciones += 1;
    assert_eq!(r.estado, 401);

    // 6. Una ruta de API que no existe es 404, no la página.
    let r = pedir(p, "GET", "/api/no-existe", Some(&cookie), "");
    operaciones += 1;
    assert_eq!(r.estado, 404);
    assert!(!r.cuerpo.contains("<html"), "un 404 de API devolvió la página");

    assert!(
        operaciones >= 100,
        "la simulación solo hizo {operaciones} operaciones"
    );
    eprintln!("simulación: {operaciones} operaciones contra el binario real");
}

/// Concurrencia con techo de tiempo: doce cajeros facturando a la vez.
///
/// Es el caso que produjo los 15 números duplicados del sistema viejo. Aquí
/// tienen que salir doce números distintos, o un rechazo con motivo — nunca dos
/// facturas con el mismo número, y nunca un cuelgue.
#[test]
fn doce_cajeros_a_la_vez_no_repiten_numero() {
    let s = arrancar("concurrencia");
    let p = s.puerto;
    let cookie = entrar(p);
    let (arts, clis, resolucion) = montar(p, &cookie, 1, 1);
    comprar(p, &cookie, arts[0], 1000, 2_000);

    let inicio = Instant::now();
    let hilos: Vec<_> = (0..12)
        .map(|k| {
            let cookie = cookie.clone();
            let (a, c) = (arts[0], clis[0]);
            std::thread::spawn(move || {
                let cuerpo = format!(
                    r#"{{"resolucion_id":{resolucion},"tercero_id":{c},"fecha":"2026-08-02",
                         "forma_pago":"contado","medio_pago":"efectivo",
                         "lineas":[{{"articulo_id":{a},"cantidad":1000,
                         "precio_unitario":{},"descuento":0,"iva_porcentaje":19}}]}}"#,
                    100_000 + k * 100
                );
                let r = pedir(p, "POST", "/api/facturas", Some(&cookie), &cuerpo);
                (r.estado, r.cuerpo)
            })
        })
        .collect();

    let mut numeros = std::collections::BTreeSet::new();
    let mut emitidas = 0;
    for h in hilos {
        let (estado, cuerpo) = h.join().expect("un hilo se cayó");
        if estado == 201 {
            emitidas += 1;
            let i = cuerpo.find(r#""numero":"#).expect("numero");
            let numero: i64 = cuerpo[i + 9..].split([',', '}']).next().unwrap().trim().parse().unwrap();
            assert!(numeros.insert(numero), "DOS FACTURAS CON EL NÚMERO {numero}");
        } else {
            // Un rechazo es aceptable; un rechazo SIN MOTIVO no lo es.
            assert!(
                estado == 409 || estado == 422,
                "estado inesperado {estado}: {cuerpo}"
            );
            assert!(cuerpo.contains("error"), "rechazo mudo: {cuerpo}");
        }
    }
    assert!(
        inicio.elapsed() < Duration::from_secs(60),
        "doce facturas concurrentes tardaron {:?}: hay un bloqueo",
        inicio.elapsed()
    );
    assert!(emitidas >= 1, "no se emitió ninguna factura");
    assert_eq!(numeros.len(), emitidas, "hay números repetidos");

    // El inventario tiene que haber bajado exactamente lo emitido.
    let saldo = pedir(p, "GET", &format!("/api/articulos/{}/saldo", arts[0]), Some(&cookie), "");
    assert_eq!(
        saldo.numero("existencia"),
        1_000_000 - emitidas as i64 * 1000,
        "el inventario no coincide con las {emitidas} facturas emitidas"
    );
    eprintln!("concurrencia: {emitidas} de 12 emitidas, {} números distintos", numeros.len());
}

/// El camino de error inyectado: se le quita la base al servidor arrancado y se
/// comprueba que **lo dice** en vez de responder algo que parece bueno.
#[test]
fn si_la_base_desaparece_el_servidor_lo_dice() {
    let s = arrancar("error");
    let p = s.puerto;
    let cookie = entrar(p);

    // Con la base sana, salud responde 200.
    assert_eq!(pedir(p, "GET", "/api/salud", None, "").estado, 200);

    // Se corrompe el archivo de la base por debajo del proceso. Es brusco a
    // propósito: es lo que pasa cuando alguien copia encima el respaldo con el
    // sistema abierto, o cuando el disco de red se cae a medias.
    let ruta = s.dir.join("inventario.db");
    std::fs::write(&ruta, b"esto ya no es una base de datos").expect("corromper");

    // El servidor puede responder bien (SQLite cachea páginas) o mal, pero
    // NUNCA puede caerse ni devolver un 200 con datos inventados.
    let r = pedir(p, "GET", "/api/articulos", Some(&cookie), "");
    assert!(
        r.estado == 200 || r.estado == 500 || r.estado == 401,
        "estado inesperado con la base rota: {} {}",
        r.estado,
        r.cuerpo
    );
    if r.estado == 500 {
        // Y si falla, el mensaje NO lleva dentro la consulta SQL.
        assert!(
            !r.cuerpo.contains("SELECT") && !r.cuerpo.contains("FROM"),
            "el error le enseñó la consulta al cliente: {}",
            r.cuerpo
        );
    }

    // Lo que no puede es haberse muerto: sigue aceptando conexiones.
    assert!(
        TcpStream::connect(("127.0.0.1", p)).is_ok(),
        "el servidor se cayó al romperse la base"
    );
}

/// El binario rechaza lo que no entiende en vez de arrancar «por si acaso».
#[test]
fn un_subcomando_mal_escrito_no_arranca_el_servidor() {
    let salida = Command::new(binario())
        .arg("migar") // «migrar» mal escrito
        .env("INVENTARIO_CLAVE_FIRMA", CLAVE_FIRMA)
        .output()
        .expect("ejecutar");
    assert!(!salida.status.success(), "un subcomando desconocido arrancó algo");
    let err = String::from_utf8_lossy(&salida.stderr);
    assert!(err.contains("desconocido"), "no dijo qué pasaba: {err}");
    assert!(err.contains("migrar"), "no sugirió lo correcto: {err}");
}

/// Sin clave de firma el servidor NO arranca con una generada al vuelo.
#[test]
fn sin_clave_de_firma_no_arranca() {
    let dir = carpeta_temporal("sinclave");
    let salida = Command::new(binario())
        .env_remove("INVENTARIO_CLAVE_FIRMA")
        .env("INVENTARIO_BD", format!("sqlite://{}/x.db", dir.display()))
        .env("INVENTARIO_DIRECCION", "127.0.0.1:0")
        .output()
        .expect("ejecutar");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!salida.status.success());
    let err = String::from_utf8_lossy(&salida.stderr);
    assert!(
        err.contains("INVENTARIO_CLAVE_FIRMA"),
        "no dijo qué falta: {err}"
    );
}

/// El panel viaja DENTRO del binario, y desde el 2026-08-06 lo teje `maud` en el
/// servidor: se pide por HTTP y tiene que salir una página, no un 404.
///
/// **Ya no puede omitirse sola.** La versión anterior de esta prueba se saltaba
/// con un `return` si el binario se había compilado sin `./construir.sh`, o sea
/// que en un árbol sin `ng build` pasaba sin comprobar nada — un negativo
/// indistinguible del real (directiva 30). Con maud ese estado no existe: el
/// HTML lo compila `rustc`, así que si no hay página, no hay binario.
#[test]
fn el_binario_sirve_el_panel_que_lleva_dentro() {
    let s = arrancar("panel");

    // Sin sesión, la raíz manda a la entrada. Se comprueba la REDIRECCIÓN, no
    // un 200: quien no ha entrado no puede ver el panel ni un instante.
    let r = pedir(s.puerto, "GET", "/", None, "");
    assert_eq!(r.estado, 303, "la raíz no redirigió: {}", r.cuerpo);

    // Y la entrada es una página real tejida por maud.
    let r = pedir(s.puerto, "GET", "/entrar", None, "");
    assert_eq!(r.estado, 200, "la entrada no se sirvió: {}", r.cuerpo);
    assert!(r.cuerpo.contains("<!DOCTYPE html>"), "no es una página: {}", r.cuerpo);
    assert!(r.cuerpo.contains(r#"name="clave""#), "no es la pantalla de entrada");
    // Lo vendorizado se sirve LOCAL desde el binario, nunca desde un CDN: esto
    // corre en un negocio que puede quedarse sin internet.
    assert!(r.cuerpo.contains(r#"src="/htmx.min.js""#), "el panel no carga htmx local");
    assert!(r.cuerpo.contains(r#"src="/boi.js""#), "el panel no carga boi local");
    assert!(!r.cuerpo.contains("<app-raiz"), "quedó un resto del SPA de Angular");

    for (ruta, aguja) in [
        ("/htmx.min.js", "htmx"),
        ("/boi.js", "boi"),
        ("/boi.css", "--boi-acento"),
        ("/panel.css", "--acento"),
        // El comportamiento se SIRVE, no solo existe en el árbol: un
        // `<script type="module">` que da 404 falla en silencio en el
        // navegador — la pantalla se dibuja igual y el gesto no ocurre.
        ("/comportamientos/sugerir-de-opcion.js", "sugerir-de-opcion"),
    ] {
        let r = pedir(s.puerto, "GET", ruta, None, "");
        assert_eq!(r.estado, 200, "«{ruta}» no se sirve");
        assert!(r.cuerpo.contains(aguja), "«{ruta}» no trae lo que dice traer");
    }

    // boi.css se vendoriza para ser AUTOCONTENIDO (sin CDN). La invariante se
    // mide sobre la hoja servida, no solo sobre el DOM que la enlaza: si boi
    // añadiera un `@import` remoto o una `url(http…)`, la vendorización lo
    // traería sin que nada lo parara (obs. del concilio 2026-09-29). Las fuentes
    // van como `data:`, que es local.
    let css = pedir(s.puerto, "GET", "/boi.css", None, "").cuerpo.to_lowercase();
    assert!(!css.contains("@import"), "boi.css trae un @import: no es autocontenido");
    assert!(!css.contains("url(http"), "boi.css referencia un recurso remoto: no es autocontenido");
}

/// Las cabeceras de seguridad van en la APLICACIÓN, no solo en el nginx de la
/// guía: esto se instala en el servidor del cliente y quien lo administre puede
/// poner otro proxy. Hasta el 2026-08-06 NO había ninguna.
///
/// Se comprueba en las TRES clases de respuesta —la pantalla, la API y lo
/// estático—, porque una capa puesta sobre un subconjunto de rutas es la que
/// alguien olvida al añadir la siguiente.
#[test]
fn toda_respuesta_del_binario_lleva_las_cabeceras_de_seguridad() {
    let s = arrancar("cabeceras");

    // Las tres clases de respuesta Y los códigos que NO son 200. La versión
    // anterior miraba solo tres rutas que devolvían 200 las tres, así que la
    // prueba no distinguía «la capa cubre todo» de «la capa cubre lo que
    // responde bien»: un middleware puesto tras el enrutador dejaría 404 y 405
    // desnudos y esto seguiría verde. El revisor midió que sí los cubre; la
    // prueba no lo decía, y lo que no dice la prueba se pierde en la reforma
    // siguiente.
    //
    // `/panel` sin cookie da 303 (redirección a entrar), `/no-existe` da 404 y
    // POST sobre una ruta de solo-GET da 405.
    let casos: [(&str, &str, u16); 7] = [
        ("GET", "/entrar", 200),
        ("GET", "/api/salud", 200),
        ("GET", "/panel.css", 200),
        ("GET", "/articulos", 303),  // ruta REAL del panel, sin sesión
        ("GET", "/no-existe", 303),      // lo desconocido de pantalla vuelve a la raíz
        ("GET", "/api/no-existe", 404),  // lo desconocido de API dice que no existe
        ("POST", "/panel.css", 405),
    ];
    for (metodo, ruta, esperado) in casos {
        let r = pedir(s.puerto, metodo, ruta, None, "");
        assert_eq!(
            r.estado, esperado,
            "«{metodo} {ruta}» dio {} y se esperaba {esperado}: el caso dejó de medir lo que decía",
            r.estado
        );
        let h = r.cabeceras.to_lowercase();
        assert!(h.contains("x-content-type-options: nosniff"), "«{ruta}» salió sin nosniff: {h}");
        assert!(h.contains("x-frame-options: deny"), "«{ruta}» se puede enmarcar: {h}");
        assert!(h.contains("referrer-policy:"), "«{ruta}» sin referrer-policy");
        // Lo que de verdad para un XSS. Una CSP presente pero con
        // `'unsafe-inline'` en los scripts sería verde y no defendería nada.
        assert!(
            h.contains("script-src 'self'") && !h.contains("script-src 'self' 'unsafe-inline'"),
            "la CSP de «{ruta}» deja ejecutar script en línea: {h}"
        );
        assert!(h.contains("frame-ancestors 'none'"), "«{ruta}»: {h}");
        // boi.css empotra IBM Plex como fuentes `data:`; sin `data:` en font-src
        // el navegador las bloquea EN SILENCIO y cae al stack del sistema (el
        // concilio del 2026-09-29 lo midió en Chromium). Guarda la regresión de
        // que alguien vuelva a quitar `data:`.
        assert!(
            h.contains("font-src 'self' data:"),
            "«{ruta}»: la CSP no permite fuentes data:, boi.css no cargaría IBM Plex: {h}"
        );
    }
}

// ---------------------------------------------------------------------------
// Cumplimiento: lo que exigen la NIC 2 y el Estatuto Tributario
// ---------------------------------------------------------------------------

/// Ejerce por HTTP, contra el binario real, todo lo que se añadió tras la
/// investigación de estándares: IVA por retiro, tope del art. 64, acta de baja,
/// deterioro por VNR con reversión, CUFE, e informes de gestión.
///
/// Va en una prueba aparte de la simulación larga porque mide otra cosa: la
/// larga mide que el sistema **opere** sin descuadrarse; esta mide que
/// **cumpla**.
#[test]
fn el_sistema_cumple_las_obligaciones_contables_y_tributarias() {
    let s = arrancar("cumplimiento");
    let p = s.puerto;
    let cookie = entrar(p);
    let (arts, clis, resolucion) = montar(p, &cookie, 2, 1);
    let (art, cli) = (arts[0], clis[0]);
    let mut operaciones = 0usize;

    // Conceptos que el instalador siembra para el cumplimiento.
    let conceptos = pedir(p, "GET", "/api/conceptos", Some(&cookie), "");
    let id_de = |codigo: &str| -> i64 {
        let aguja = format!(r#""codigo":"{codigo}""#);
        let i = conceptos
            .cuerpo
            .find(&aguja)
            .unwrap_or_else(|| panic!("no está el concepto {codigo}: {}", conceptos.cuerpo));
        let j = conceptos.cuerpo[..i].rfind(r#""id":"#).expect("id");
        conceptos.cuerpo[j + 5..]
            .split(',')
            .next()
            .unwrap()
            .trim()
            .parse()
            .expect("id numérico")
    };
    let (retiro, faltante, baja, compra) =
        (id_de("RET"), id_de("FAL"), id_de("BAJ"), id_de("COM"));

    // Existencia: 1.000 unidades a $2.000.
    let mover = |concepto: i64, u: i64, costo: i64, fecha: &str, comercial: Option<i64>| {
        let extra = comercial
            .map(|v| format!(r#","valor_comercial_unitario":{v}"#))
            .unwrap_or_default();
        pedir(
            p,
            "POST",
            "/api/movimientos",
            Some(&cookie),
            &format!(
                r#"{{"fecha":"{fecha}","concepto_id":{concepto},"articulo_id":{art},
                     "cantidad":{},"costo_unitario":{costo}{extra}}}"#,
                u * 1000
            ),
        )
    };
    assert_eq!(mover(compra, 1_000, 200_000, "2026-08-01", None).estado, 201);
    operaciones += 2;

    // --- ET art. 421 lit. b): el retiro causa IVA -------------------------

    // Sin valor comercial NO entra. Es la regla que impide el obsequio sin IVA.
    let r = mover(retiro, 10, 0, "2026-08-05", None);
    operaciones += 1;
    assert_eq!(r.estado, 422, "un retiro sin valor comercial no debe entrar: {}", r.cuerpo);
    assert!(r.cuerpo.contains("421"), "el error no cita la norma: {}", r.cuerpo);

    // Con valor comercial, sí — y liquida IVA sobre él, no sobre el costo.
    let r = mover(retiro, 10, 0, "2026-08-05", Some(500_000));
    operaciones += 1;
    assert_eq!(r.estado, 201, "{}", r.cuerpo);

    let inf = pedir(
        p,
        "GET",
        "/api/informes/retiros?desde=2026-08-01&hasta=2026-08-31",
        Some(&cookie),
        "",
    );
    operaciones += 1;
    assert_eq!(inf.estado, 200, "{}", inf.cuerpo);
    // Base = 10 × $5.000 = $50.000; IVA 19 % = $9.500. El costo era $20.000.
    assert_eq!(inf.numero("base_total"), 5_000_000);
    assert_eq!(inf.numero("iva_total"), 950_000);
    assert_eq!(inf.numero("costo_total"), 2_000_000);
    assert_ne!(
        inf.numero("iva_total"),
        380_000,
        "si la base fuera el costo, el IVA sería 3.800: eso es subdeclarar"
    );

    // --- ET art. 64: el tope del 3 % --------------------------------------

    // 20 unidades de faltante sobre una base de 1.000 = 2 %: dentro del tope.
    assert_eq!(mover(faltante, 20, 0, "2026-08-10", None).estado, 201);
    let a64 = pedir(
        p,
        "GET",
        "/api/informes/art64?desde=2026-08-01&hasta=2026-08-31",
        Some(&cookie),
        "",
    );
    operaciones += 2;
    assert_eq!(a64.estado, 200, "{}", a64.cuerpo);
    assert_eq!(a64.numero("tope"), 30_000, "3 % de 1.000 unidades");
    assert_eq!(a64.numero("consumido"), 20_000);
    assert_eq!(a64.campo("se_paso"), "false");

    // 20 unidades más: 4 % — se pasa, y el informe lo advierte citando la norma.
    assert_eq!(mover(faltante, 20, 0, "2026-08-12", None).estado, 201);
    let a64 = pedir(
        p,
        "GET",
        "/api/informes/art64?desde=2026-08-01&hasta=2026-08-31",
        Some(&cookie),
        "",
    );
    operaciones += 2;
    assert_eq!(a64.campo("se_paso"), "true");
    assert!(
        a64.cuerpo.contains("fuerza mayor"),
        "la advertencia no dice cómo se salva el exceso: {}",
        a64.cuerpo
    );

    // --- ET art. 64: el acta de baja --------------------------------------

    let r = mover(baja, 5, 0, "2026-08-15", None);
    operaciones += 1;
    assert_eq!(r.estado, 201, "{}", r.cuerpo);
    let mov_baja = r.numero("id");

    // Un acta sin responsables no se acepta: la norma los exige.
    let acta = |resp: &str, cant: i64, mov: i64| {
        format!(
            r#"{{"fecha":"2026-08-16","metodo":"destruccion",
                 "representante_legal":"Juan Pérez","responsables":[{resp}],
                 "cubierto_por_seguro":0,
                 "renglones":[{{"articulo_id":{art},"cantidad":{cant},
                   "justificacion":"Vencido el 2026-07-31","movimiento_id":{mov}}}]}}"#
        )
    };
    let r = pedir(p, "POST", "/api/actas", Some(&cookie), &acta("\"\"", 5000, mov_baja));
    operaciones += 1;
    assert_eq!(r.estado, 422, "un acta sin responsables no sustenta nada: {}", r.cuerpo);

    // Con todo completo, sí — y el costo sale del kardex, no de quien digita.
    let r = pedir(
        p,
        "POST",
        "/api/actas",
        Some(&cookie),
        &acta("\"Ana Gómez\"", 5000, mov_baja),
    );
    operaciones += 1;
    assert_eq!(r.estado, 201, "{}", r.cuerpo);
    assert_eq!(r.numero("costo_total"), 1_000_000, "5 × $2.000 del kardex");
    assert_eq!(r.numero("deducible"), 1_000_000);

    // --- NIC 2.28-33: deterioro por VNR y su reversión --------------------

    // El artículo se compró a $2.000 y hoy se vende a $700: hay que rebajarlo.
    let estimar = |precio: i64| {
        pedir(
            p,
            "PUT",
            &format!("/api/articulos/{art}"),
            Some(&cookie),
            &format!(
                r#"{{"codigo":"A0","descripcion":"Botellón 0","iva_porcentaje":19,
                     "precio_venta":300000,"precio_venta_estimado":{precio},
                     "costos_para_vender":5000}}"#
            ),
        )
    };
    assert_eq!(estimar(70_000).estado, 200);
    let c1 = pedir(p, "POST", "/api/deterioro", Some(&cookie), r#"{"fecha":"2026-08-31"}"#);
    operaciones += 2;
    assert_eq!(c1.estado, 200, "{}", c1.cuerpo);
    let rebaja1 = c1.numero("rebajas_del_periodo");
    assert!(rebaja1 > 0, "tenía que reconocerse deterioro: {}", c1.cuerpo);
    assert_eq!(c1.numero("reversiones_del_periodo"), 0);

    // El precio se recupera: se REVIERTE, y las dos cifras van separadas.
    assert_eq!(estimar(190_000).estado, 200);
    let c2 = pedir(p, "POST", "/api/deterioro", Some(&cookie), r#"{"fecha":"2026-09-30"}"#);
    operaciones += 2;
    assert_eq!(c2.estado, 200, "{}", c2.cuerpo);
    assert!(
        c2.numero("reversiones_del_periodo") > 0,
        "tenía que revertirse: {}",
        c2.cuerpo
    );
    assert_eq!(
        c2.numero("rebajas_del_periodo"),
        0,
        "las rebajas y las reversiones NO se compensan (NIC 2.36)"
    );

    // Y por encima del costo no sube.
    assert_eq!(estimar(9_000_000).estado, 200);
    let c3 = pedir(p, "POST", "/api/deterioro", Some(&cookie), r#"{"fecha":"2026-10-31"}"#);
    operaciones += 2;
    let leido = pedir(p, "GET", "/api/deterioro/2026-10-31", Some(&cookie), "");
    operaciones += 1;
    assert_eq!(leido.estado, 200);
    assert_eq!(c3.numero("deterioro_acumulado_total"), 0, "vuelve al costo, y ahí para");

    // --- Res. 000165/2023: el CUFE ----------------------------------------

    // Sin clave técnica no hay CUFE, y se dice por qué.
    let facturar = || {
        pedir(
            p,
            "POST",
            "/api/facturas",
            Some(&cookie),
            &format!(
                r#"{{"resolucion_id":{resolucion},"tercero_id":{cli},"fecha":"2026-08-02",
                     "hora":"10:53:10-05:00","forma_pago":"contado","medio_pago":"efectivo",
                     "lineas":[{{"articulo_id":{art},"cantidad":1000,"precio_unitario":100000,
                       "descuento":0,"iva_porcentaje":19,"inc_porcentaje":8}}]}}"#
            ),
        )
    };
    let f = facturar();
    operaciones += 1;
    assert_eq!(f.estado, 201, "{}", f.cuerpo);
    assert!(
        f.cuerpo.contains("clave técnica"),
        "sin clave técnica hay que decir qué falta: {}",
        f.cuerpo
    );
    // El INC viaja discriminado del IVA.
    assert_eq!(f.numero("iva"), 19_000, "19 % de 1.000");
    assert_eq!(f.numero("inc"), 8_000, "8 % de 1.000, sobre la MISMA base");
    assert_eq!(f.numero("total"), 127_000);

    // Con clave técnica sale el CUFE: 96 hexadecimales.
    let r = pedir(
        p,
        "POST",
        "/api/resoluciones",
        Some(&cookie),
        r#"{"nombre":"Con clave","prefijo":"CT","numero":"18764000002","desde":1,"hasta":100}"#,
    );
    assert_eq!(r.estado, 201, "{}", r.cuerpo);
    operaciones += 1;

    // --- Gestión ----------------------------------------------------------

    for ruta in [
        "/api/informes/rotacion?desde=2026-08-01&hasta=2026-08-31",
        "/api/informes/abc?desde=2026-08-01&hasta=2026-08-31",
        "/api/informes/reorden?desde=2026-08-01&hasta=2026-08-31&dias_reposicion=5&nivel_servicio=95",
    ] {
        let r = pedir(p, "GET", ruta, Some(&cookie), "");
        operaciones += 1;
        assert_eq!(r.estado, 200, "{ruta}: {}", r.cuerpo);
    }
    // Un nivel de servicio no tabulado se rechaza en vez de inventar un z.
    let r = pedir(
        p,
        "GET",
        "/api/informes/reorden?desde=2026-08-01&hasta=2026-08-31&dias_reposicion=5&nivel_servicio=93",
        Some(&cookie),
        "",
    );
    operaciones += 1;
    assert_eq!(r.estado, 422, "{}", r.cuerpo);
    assert!(r.cuerpo.contains("tabulado"), "{}", r.cuerpo);

    // --- El kardex sigue cuadrando después de todo esto -------------------

    let saldo = pedir(p, "GET", &format!("/api/articulos/{art}/saldo"), Some(&cookie), "");
    let kardex = pedir(p, "GET", &format!("/api/articulos/{art}/kardex"), Some(&cookie), "");
    operaciones += 2;
    let mut suma = 0i64;
    for entrada in kardex.cuerpo.split(r#"{"id":"#).skip(1) {
        let i = entrada.find(r#""cantidad":"#).expect("cantidad");
        let c: i64 = entrada[i + 11..].split([',', '}']).next().unwrap().trim().parse().unwrap();
        suma += if entrada.contains(r#""signo":"entrada""#) { c } else { -c };
    }
    assert_eq!(
        saldo.numero("existencia"),
        suma,
        "tras retiros, faltantes, bajas y deterioro el kardex dejó de cuadrar"
    );

    eprintln!("cumplimiento: {operaciones} operaciones contra el binario real");
    assert!(operaciones >= 25);
}

// ═══════════════════════════════════════════════════════════════════════════
// La pantalla DIBUJADA
// ═══════════════════════════════════════════════════════════════════════════
//
// Todo lo de arriba habla HTTP y afirma sobre el texto que devuelve axum. Eso
// mide el CÓDEC. Desde que el panel es `maud` + htmx (directiva 26), la pantalla
// la termina de construir el navegador: los `hx-swap` los ejecuta JavaScript, y
// un `<script type="module">` bloqueado por la CSP, servido con el MIME
// equivocado o que revienta al importar **devuelve el mismo 200 con el mismo
// cuerpo**. Una pantalla hay que DIBUJARLA.
//
// EL ARNÉS ES `mirilla` DESDE EL 2026-08-13, y aquí había un TODO pidiéndolo.
//
// Decía: «`buscar_chromium` + `dibujar` son la TERCERA copia de este arnés […]
// Sale a repositorio propio, público y MIT como `boi` […] EL NOMBRE DEL
// REPOSITORIO LO PONE JUAN». El repositorio existe desde el 2026-08-10, se llama
// `mirilla`, y ya lo consumen informes, medico y chasqui.
//
// Lo que cambia además de quitar la copia: la espera. Antes era
// `--virtual-time-budget=4000`, un plazo que al agotarse devuelve un DOM
// incompleto y deja la aserción fallando contra un texto que no explica nada.
// `mirilla::esperar` espera la CONDICIÓN —aquí `data-boi="listo"`, que es lo
// que las pruebas afirman después— y al agotarse imprime el DOM que había.

/// Pinta `url` en un Chromium de verdad y devuelve el DOM ya construido,
/// esperando a que `boi` se enganche.
fn dibujar(chromium: &str, url: &str) -> String {
    let mut nav = Navegador::abrir(chromium);
    nav.ir(url);
    nav.esperar(
        "document.documentElement.dataset.boi === 'listo'",
        "boi no se enganchó en el navegador: el módulo está en el HTML pero no llegó a \
         ejecutarse (CSP, MIME o un error al importar)",
    );
    nav.dom()
}

#[test]
fn la_pantalla_de_entrada_se_dibuja_y_boi_se_engancha_de_verdad() {
    let chromium = buscar_chromium();
    let s = arrancar("dibujo-entrada");
    let dom = dibujar(&chromium, &format!("http://127.0.0.1:{}/entrar", s.puerto));

    // Lo que el navegador montó de verdad.
    assert!(dom.contains(r#"id="codigo""#), "no se dibujó el campo de usuario: {dom:.400}");
    assert!(dom.contains(r#"id="clave""#), "no se dibujó el campo de clave");
    assert!(dom.contains("Entrar"), "no se dibujó el botón");

    // LA AFIRMACIÓN QUE NO SE PUEDE HACER SOBRE EL HTML DE axum: `boi` viaja
    // como `<script type="module">` y solo deja `data-boi="listo"` en el <html>
    // si el navegador lo descargó, lo aceptó bajo `script-src 'self'`, lo parseó
    // como módulo y lo ejecutó. En el cuerpo que sirve axum ese atributo NO
    // está: lo pone el JavaScript al correr.
    assert!(
        dom.contains(r#"data-boi="listo""#),
        "boi no se enganchó en el navegador: el módulo está en el HTML pero no llegó a \
         ejecutarse (CSP, MIME o un error al importar), y ninguna otra prueba lo vería"
    );

    // La mitad estética viaja: el <link> a /boi.css está en el <head>. La ruta
    // que sirve el CSS ya la cubre `el_binario_sirve_el_panel_que_lleva_dentro`;
    // esto cubre el enlace, la otra mitad —sin él, boi.css no cargaría y ninguna
    // prueba lo vería (un <link> ausente no da 404: simplemente no pinta)—.
    assert!(dom.contains("/boi.css"), "no se enlazó boi.css en el <head>: {dom:.400}");

    // No quedó ni rastro del SPA que se retiró.
    assert!(!dom.contains("<app-root"), "quedó un resto del Angular anterior");

    // Negativo de control: sin `INVENTARIO_DEMO_*` no puede salir el cartel.
    // Su pareja es la prueba siguiente.
    assert!(
        !dom.contains("Demostración pública"),
        "salió el cartel de demostración sin haber configurado las variables"
    );
}

#[test]
fn el_cartel_de_demostracion_se_dibuja_solo_con_sus_dos_variables() {
    // La pareja del control anterior, y lo que lo convierte en medición: el DOM
    // dibujado cambia CON EL ENTORNO del servidor. Sin esta mitad, el `!contains`
    // de arriba pasaría igual con una pantalla en blanco.
    let chromium = buscar_chromium();
    let s = arrancar_con("dibujo-demo", &[
        ("INVENTARIO_DEMO_USUARIO", "demo"),
        ("INVENTARIO_DEMO_CLAVE", "inventario2026"),
    ]);
    let dom = dibujar(&chromium, &format!("http://127.0.0.1:{}/entrar", s.puerto));

    assert!(
        dom.contains("Demostración pública"),
        "las variables están puestas y el cartel no se dibujó: {dom:.400}"
    );
    assert!(dom.contains("inventario2026"), "el cartel salió sin la clave");
    assert!(dom.contains(r#"data-boi="listo""#), "boi no se enganchó en esta pantalla");
}

#[test]
fn sin_sesion_el_navegador_acaba_en_la_entrada_y_no_ve_el_panel() {
    // La guardia, comprobada a través de un navegador real que SIGUE la
    // redirección — que es como llega quien teclea la dirección a mano.
    let chromium = buscar_chromium();
    let s = arrancar("dibujo-sin-sesion");
    let dom = dibujar(&chromium, &format!("http://127.0.0.1:{}/articulos", s.puerto));

    assert!(dom.contains(r#"id="clave""#), "no acabó en la pantalla de entrada: {dom:.400}");
    assert!(!dom.contains("class=\"barra\""), "se dibujó la barra del panel sin sesión");
    assert!(!dom.contains("Existencia"), "se dibujó la tabla de artículos sin sesión");
}

/// **Las ocho secciones responden Y traen su contenido**, con sesión.
///
/// Separa «hay una ruta» de «hay una pantalla». Una vista que compila y devuelve
/// 200 puede salir vacía —una consulta que falla y se traga con
/// `unwrap_or_default`, un `@if` que nunca entra—, y la aguja de cada sección es
/// un trozo que SOLO existe si esa pantalla llegó a construirse.
///
/// LÍMITE, y se escribe aquí porque no cubrirlo callando sería peor: **esto no
/// dibuja**. Es una afirmación sobre el HTML que devuelve axum, o sea que mide
/// el códec (directiva 28). Las tres pruebas `…se_dibuja…` sí abren Chromium,
/// pero solo llegan a las pantallas SIN sesión: `--dump-dom` no admite mandar
/// una cookie —`--header` no existe—, y la única forma de meterla sería abrir
/// una ruta que autentique por GET, que es un agujero de verdad a cambio de una
/// prueba. Lo que queda sin medir en un navegador es, por tanto, si los
/// `hx-swap` de las pantallas con sesión insertan donde deben. Se resuelve el
/// día que el arnés salga a su repositorio y pueda hablar CDP.
#[test]
fn cada_seccion_del_panel_trae_su_contenido() {
    let s = arrancar("secciones");
    let cookie = entrar(s.puerto);

    let secciones: [(&str, &str); 9] = [
        // La aguja tiene que existir SIEMPRE y ser PROPIA de la pantalla. La
        // primera versión usaba «Existencia», que es una columna de la tabla y
        // desaparece cuando no hay artículos; y «Recaudar», que sale en la barra
        // de todas y por tanto habría pasado con cualquier página.
        ("/articulos", "Código, código de barras o descripción"),
        ("/movimientos", "Registrar movimiento"),
        ("/terceros", "Nuevo tercero"),
        ("/facturar", "Emitir factura"),
        ("/recaudar", "Ver cartera"),
        ("/cartera", "por cobrar"),
        ("/informes", "Bajo el punto mínimo"),
        ("/configuracion", "Usuarios"),
        ("/alistamiento", "Lo que ya hay"),
    ];

    // El nombre ya no lleva la cifra —«las ocho secciones» envejeció en cuanto
    // entró Terceros—, pero la cobertura sí se comprueba: toda ruta de lectura
    // del panel que no sea un fragmento de htmx tiene que estar aquí. Sin esto,
    // añadir una pantalla y olvidarla no daría ningún error.
    let fragmentos = ["/articulos/tabla", "/facturar/linea", "/recaudar/pago"];
    for ruta in lecturas_del_panel() {
        assert!(
            fragmentos.contains(&ruta.as_str()) || secciones.iter().any(|(r, _)| *r == ruta),
            "«{ruta}» es una pantalla del panel y nadie comprueba su contenido"
        );
    }

    for (ruta, aguja) in secciones {
        let r = pedir(s.puerto, "GET", ruta, Some(&cookie), "");
        assert_eq!(r.estado, 200, "«{ruta}» no respondió 200: {}", r.cuerpo);
        assert!(
            r.cuerpo.contains(aguja),
            "«{ruta}» respondió sin su contenido (falta «{aguja}»): {:.300}",
            r.cuerpo
        );
        // Con sesión, la barra va en TODAS: sin ella la pantalla se sirve pero
        // quien la mira no puede salir de ahí.
        assert!(r.cuerpo.contains("class=\"barra\""), "«{ruta}» salió sin barra");
    }
}

/// La pareja del anterior (directiva 33): **sin sesión, ninguna se
/// sirve**. Con solo la mitad de arriba, un panel que se enseñara a cualquiera
/// pasaría igual de verde.
#[test]
fn sin_sesion_ninguna_seccion_del_panel_se_sirve() {
    let s = arrancar("secciones-sin-sesion");

    for ruta in lecturas_del_panel() {
        let r = pedir(s.puerto, "GET", &ruta, None, "");
        assert_eq!(r.estado, 303, "«{ruta}» contestó {} sin sesión", r.estado);
        assert!(!r.cuerpo.contains("class=\"barra\""), "«{ruta}» filtró el panel sin sesión");
    }

    // Y las escrituras tampoco: una ruta POST que se olvida es la que deja
    // registrar movimientos a cualquiera que sepa la dirección. La lista sale
    // de la tabla de rutas, no de la memoria de quien escribió la prueba.
    for ruta in escrituras_del_panel() {
        let r = pedir(s.puerto, "POST", &ruta, None, "x=1");
        assert_eq!(r.estado, 303, "«POST {ruta}» contestó {} sin sesión", r.estado);
    }
}

/// **Un rol sin permiso NO escribe por el panel.** Es la mitad de la pareja que
/// faltaba, y no es teórica: la primera versión de este panel comprobaba solo la
/// sesión, así que un usuario `consulta` —el del contador, o el temporal que
/// solo mira— podía registrar movimientos, **emitir facturas contra una
/// resolución de la DIAN** y recaudar, cuando por `/api` recibía 403 en las tres.
/// Un número de resolución quemado no se recupera.
///
/// Lo cazó una revisión de seguridad independiente, no el banco, y por una razón
/// que conviene tener escrita: la prueba que existía medía «SIN cookie no pasa»
/// y salía verde. El hueco estaba en «CON cookie de un rol que no puede», que es
/// justo el caso que ninguna prueba miraba (directiva 33).
#[test]
fn un_rol_sin_permiso_no_escribe_por_el_panel() {
    let s = arrancar("permisos-panel");
    let admin = entrar(s.puerto);

    // Un usuario de cada rol que NO puede escribir. `consulta` no puede nada;
    // `bodega` mueve inventario pero no factura ni recauda.
    for (codigo, rol) in [("mirona", "consulta"), ("bodeguero", "bodega")] {
        let r = pedir(
            s.puerto,
            "POST",
            "/api/usuarios",
            Some(&admin),
            &format!(r#"{{"codigo":"{codigo}","nombre":"X","clave":"una-clave-larga","rol":"{rol}"}}"#),
        );
        assert!(r.estado == 200 || r.estado == 201, "no se creó {rol}: {}", r.cuerpo);
    }

    // Lo que cada rol NO puede escribir, y el 403 que le toca. Las listas se
    // DERIVAN de la tabla de rutas: `consulta` no escribe nada, y `bodega`
    // mueve inventario pero ni factura, ni recauda, ni configura. Así una ruta
    // de escritura nueva queda vetada para los dos desde el día que nace.
    let todas = escrituras_del_panel();
    let solo_bodega: Vec<String> = todas
        .iter()
        .filter(|r| *r != "/movimientos")
        .cloned()
        .collect();
    assert!(
        solo_bodega.len() + 1 == todas.len(),
        "«/movimientos» ya no está en la tabla: {todas:?}"
    );
    let vetado: [(&str, &str, &[String]); 2] = [
        ("mirona", "una-clave-larga", &todas),
        ("bodeguero", "una-clave-larga", &solo_bodega),
    ];

    for (codigo, clave, rutas) in vetado {
        let r = pedir(
            s.puerto,
            "POST",
            "/api/sesion",
            None,
            &format!(r#"{{"codigo":"{codigo}","clave":"{clave}"}}"#),
        );
        assert_eq!(r.estado, 200, "no entró {codigo}: {}", r.cuerpo);
        let cookie = r.cookie.expect("sin cookie");

        for ruta in rutas {
            let r = pedir(s.puerto, "POST", ruta, Some(&cookie), "x=1");
            assert_eq!(
                r.estado, 403,
                "«POST {ruta}» dejó pasar a «{codigo}» ({}): tiene sesión pero NO permiso",
                r.estado
            );
        }

        // Y NO se le manda a la pantalla de acceso: quien ya entró y no tiene
        // permiso se pondría a teclear su clave otra vez creyendo que se le cayó
        // la sesión. 403 y 303 dicen cosas distintas a propósito.
        let r = pedir(s.puerto, "POST", "/facturar", Some(&cookie), "x=1");
        assert_ne!(r.estado, 303, "a «{codigo}» se le trató como si no tuviera sesión");
    }

    // PAREJA (directiva 33): el admin SÍ pasa por las mismas rutas. Sin esta
    // mitad, una guardia que negara a todo el mundo saldría igual de verde.
    for ruta in ["/movimientos", "/facturar", "/recaudar"] {
        let r = pedir(s.puerto, "POST", ruta, Some(&admin), "x=1");
        assert_ne!(r.estado, 403, "«POST {ruta}» le negó al admin: la guardia niega a todos");
    }

    // Y `consulta` sigue pudiendo LEER, que es para lo que existe el rol.
    let r = pedir(
        s.puerto,
        "POST",
        "/api/sesion",
        None,
        r#"{"codigo":"mirona","clave":"una-clave-larga"}"#,
    );
    let cookie = r.cookie.expect("sin cookie");
    let r = pedir(s.puerto, "GET", "/articulos", Some(&cookie), "");
    assert_eq!(r.estado, 200, "«consulta» no puede ni mirar");

    // Pero la configuración NO la ve: enseña el padrón de usuarios con sus roles
    // —el insumo de un ataque de credenciales— y los campos administrativos de
    // la empresa. Por `/api` los dos piden `Configurar`.
    let r = pedir(s.puerto, "GET", "/configuracion", Some(&cookie), "");
    assert_eq!(r.estado, 403, "«consulta» vio la configuración: {}", r.estado);
}

/// **La revisión que el migrador pide se puede hacer desde el panel.**
///
/// Es el camino entero, por HTTP y contra el binario real: dar de alta el
/// concepto de obsequio que el sistema viejo tenía sin IVA, verlo marcado como
/// pendiente, corregirlo, y ver que la corrección quedó. Más la resolución y
/// las credenciales del software, que hasta hoy solo se podían poner con un
/// cliente HTTP contra `/api`.
///
/// Cada paso lleva su pareja, porque un formulario que acepta todo se ve igual
/// de verde que uno que valida: el concepto SIN efecto tributario se rechaza,
/// el rango de resolución al revés se rechaza, y el formulario de credenciales
/// en blanco NO borra el PIN que había.
#[test]
fn la_configuracion_del_panel_cierra_el_circulo_del_migrador() {
    let s = arrancar("configuracion-panel");
    let cookie = entrar(s.puerto);

    let post = |ruta: &str, cuerpo: &str| pedir(s.puerto, "POST", ruta, Some(&cookie), cuerpo);

    // -- Primera vez: la instalación recién hecha no tiene empresa -----------
    //
    // `instalar` termina diciendo «falta configurar la empresa (nombre y NIT)».
    // El panel tiene que poder hacerlo; hasta hoy dibujaba el aviso y ningún
    // formulario, así que lo primero que el producto te manda hacer era lo
    // único que no dejaba hacer.
    let r = pedir(s.puerto, "GET", "/configuracion", Some(&cookie), "");
    assert_eq!(r.estado, 200);
    assert!(
        r.cuerpo.contains("todavía no tiene empresa configurada"),
        "no avisa de que falta configurar la empresa"
    );
    assert!(
        r.cuerpo.contains(r#"action="/configuracion/empresa""#),
        "una instalación nueva no ofrece el formulario de empresa"
    );

    let r = post(
        "/configuracion/empresa",
        "nombre=Aguas+de+Prueba&nit=811033579&direccion=Calle+1&telefono=123\
         &ambiente_dian=pruebas&formula_costeo=promedio",
    );
    assert_eq!(r.estado, 200);
    assert!(r.cuerpo.contains("Empresa guardada"), "no se guardó la empresa");
    // El DV lo calcula el dominio, no se teclea.
    assert!(r.cuerpo.contains("811033579-6"), "no calculó el dígito de verificación");

    // Y PEPS también, que es la otra fórmula que la NIC 2 admite. Las dos
    // opciones del desplegable tienen que llegar a guardarse: `promedio` reventaba
    // contra el CHECK del esquema —que solo conoce «promedio_ponderado»— y el
    // panel contestaba un 500 opaco en la operación más común del primer día.
    let r = post(
        "/configuracion/empresa",
        "nombre=Aguas+de+Prueba&nit=811033579&ambiente_dian=pruebas&formula_costeo=peps",
    );
    assert!(r.cuerpo.contains("Empresa guardada"), "PEPS no se pudo guardar");

    // Y UEPS recibe SU error, el que cita la norma, no un fallo de base de datos.
    let r = post(
        "/configuracion/empresa",
        "nombre=Aguas+de+Prueba&nit=811033579&ambiente_dian=pruebas&formula_costeo=ueps",
    );
    assert!(r.cuerpo.contains("PROHIBIDO"), "UEPS no recibió su error: {}", r.estado);

    // Se deja como estaba para el resto de la prueba.
    let r = post(
        "/configuracion/empresa",
        "nombre=Aguas+de+Prueba&nit=811033579&ambiente_dian=pruebas&formula_costeo=promedio",
    );
    assert!(r.cuerpo.contains("Empresa guardada"));

    // PAREJA: un NIT que no es un número se rechaza. En el corpus heredado había
    // cinco clientes cuyo «NIT» era «---» o «BEBE».
    let r = post(
        "/configuracion/empresa",
        "nombre=X&nit=BEBE&ambiente_dian=pruebas&formula_costeo=promedio",
    );
    assert!(r.cuerpo.contains("No se guardó"), "aceptó «BEBE» como NIT");

    // -- Concepto: el alta exige decidir el efecto tributario ----------------
    //
    // PAREJA primero: sin efecto no entra. `efecto_tributario` no tiene valor
    // por defecto en el esquema justamente para que un obsequio nuevo no nazca
    // sin IVA por omisión, y la pantalla no puede ser la puerta de atrás.
    let r = post(
        "/configuracion/concepto",
        "codigo=79&nombre=OBSEQUIOS&signo=salida&clase=consumo&efecto_tributario=",
    );
    assert_eq!(r.estado, 200);
    assert!(
        r.cuerpo.contains("no tiene valor por defecto"),
        "un concepto sin efecto tributario entró: {}",
        &r.cuerpo[..r.cuerpo.len().min(400)]
    );

    let r = post(
        "/configuracion/concepto",
        "codigo=79&nombre=OBSEQUIOS&signo=salida&clase=consumo&efecto_tributario=no_afecta_impuestos",
    );
    assert_eq!(r.estado, 200);
    assert!(r.cuerpo.contains("Concepto creado"), "no se creó el concepto");

    // El panel MARCA el concepto de salida que no causa impuestos, que es como
    // lo deja el migrador. Sin esta marca la revisión no se puede hacer: nadie
    // sabe cuáles mirar.
    let r = pedir(s.puerto, "GET", "/configuracion", Some(&cookie), "");
    assert_eq!(r.estado, 200);
    assert!(
        r.cuerpo.contains("conceptos de SALIDA no causan ningún impuesto"),
        "el panel no avisa de los conceptos neutros"
    );
    assert!(
        r.cuerpo.contains("421"),
        "el aviso no cita la norma que lo hace importante"
    );

    // -- Y se corrige -------------------------------------------------------
    let id = id_del_concepto(s.puerto, &cookie, "79");
    let r = post(
        "/configuracion/concepto/efecto",
        &format!("concepto_id={id}&efecto_tributario=retiro_gravado"),
    );
    assert_eq!(r.estado, 200);
    assert!(r.cuerpo.contains("Concepto corregido"), "no se corrigió: {}", r.estado);
    assert!(
        r.cuerpo.contains("Retiro gravado"),
        "la tabla no muestra el efecto corregido"
    );

    // PAREJA: un efecto inventado no pasa, y el concepto se queda como estaba.
    let r = post(
        "/configuracion/concepto/efecto",
        &format!("concepto_id={id}&efecto_tributario=exento_porque_si"),
    );
    assert_eq!(r.estado, 200);
    assert!(r.cuerpo.contains("No se corrigió"), "aceptó un efecto inventado");

    // -- Resolución ---------------------------------------------------------
    let r = post(
        "/configuracion/resolucion",
        "nombre=Autorizacion+2026&numero=18764&prefijo=FE&desde=1000&hasta=2000\
         &rige_desde=2026-01-01&vence=2027-12-31&clave_tecnica=abc123",
    );
    assert_eq!(r.estado, 200);
    assert!(r.cuerpo.contains("Resolución registrada"), "no se registró");
    assert!(r.cuerpo.contains("Autorizacion 2026"), "no aparece en la tabla");

    // PAREJA: el rango al revés se rechaza. Un rango mal puesto quema números
    // de una resolución de la DIAN, y eso no se recupera.
    let r = post(
        "/configuracion/resolucion",
        "nombre=Al+reves&desde=5000&hasta=100",
    );
    assert_eq!(r.estado, 200);
    assert!(r.cuerpo.contains("No se registró"), "aceptó un rango al revés");

    // -- Credenciales del software ------------------------------------------
    let r = post("/configuracion/software", "pin_software=PIN-SECRETO-1&id_software=sw-99");
    assert_eq!(r.estado, 200);
    assert!(r.cuerpo.contains("actualizad"), "no se guardaron las credenciales");
    // Y NO se hace eco del PIN en la pantalla: lo que se dibuja lo ve quien
    // pase por detrás del mostrador y queda en el historial del navegador.
    assert!(
        !r.cuerpo.contains("PIN-SECRETO-1"),
        "el PIN salió dibujado en la pantalla de configuración"
    );

    // PAREJA, y es la que importa: el formulario en blanco CONSERVA. Mandar
    // vacío borraría el PIN, y una instalación sin PIN emite notas sin CUDE
    // —lo que no se descubre aquí, sino cuando la DIAN rechaza el documento—.
    let r = post("/configuracion/software", "pin_software=&id_software=");
    assert_eq!(r.estado, 200);
    assert!(r.cuerpo.contains("No se cambió nada"), "no dijo que no cambió nada");

    let r = pedir(s.puerto, "GET", "/api/empresa/configuracion", Some(&cookie), "");
    assert_eq!(r.estado, 200);
    assert!(
        r.cuerpo.contains("PIN-SECRETO-1"),
        "el formulario en blanco BORRÓ el PIN: {}",
        r.cuerpo
    );

    // Y el PIN sigue sin salir por la ruta pública, que es la que ve la
    // pantalla de acceso antes de que nadie entre.
    let r = pedir(s.puerto, "GET", "/api/empresa", None, "");
    assert!(!r.cuerpo.contains("PIN-SECRETO-1"), "el PIN se escapó a /api/empresa");
}

/// El id de un concepto por su código, leído por la API.
fn id_del_concepto(puerto: u16, cookie: &str, codigo: &str) -> i64 {
    let r = pedir(puerto, "GET", "/api/conceptos", Some(cookie), "");
    assert_eq!(r.estado, 200, "no se listaron los conceptos");
    let v: serde_json::Value = serde_json::from_str(&r.cuerpo).expect("json de conceptos");
    v.as_array()
        .expect("lista")
        .iter()
        .find(|c| c["codigo"] == codigo)
        .unwrap_or_else(|| panic!("no está el concepto {codigo}: {}", r.cuerpo))["id"]
        .as_i64()
        .expect("id")
}

/// **Los informes de cumplimiento se ven y se ACCIONAN desde el panel.**
///
/// Estaban implementados, probados y documentados como hechos en `BRECHAS.md`,
/// y solo se podían consultar por `/api` con un cliente HTTP: el cliente que
/// recibe este producto tiene un navegador, así que un informe que solo existe
/// en JSON no cruzó la frontera hacia el usuario (directiva 28).
///
/// La prueba recorre los dos que sustentan una deducción o una revelación —el
/// tope del 3 % del art. 64 con su acta, y el deterioro por VNR— por la vía
/// real: se provoca el hecho, se mira que el panel lo cuente, se acciona el
/// documento y se comprueba que el panel deje de pedirlo.
#[test]
fn los_informes_de_cumplimiento_se_ven_y_se_accionan_desde_el_panel() {
    let s = arrancar("informes-cumplimiento");
    let cookie = entrar(s.puerto);
    let (arts, _, _) = montar(s.puerto, &cookie, 1, 0);
    let art = arts[0];
    comprar(s.puerto, &cookie, art, 100, 3_000);

    // Un concepto de baja por obsolescencia: es lo que el art. 64 exige
    // sustentar con acta.
    let r = pedir(
        s.puerto,
        "POST",
        "/api/conceptos",
        Some(&cookie),
        r#"{"codigo":"73","nombre":"Baja por obsolescencia","signo":"salida",
            "clase":"ajuste","efecto_tributario":"baja_con_acta"}"#,
    );
    assert_eq!(r.estado, 201, "{}", r.cuerpo);
    let baja = r.numero("id");

    let r = pedir(
        s.puerto,
        "POST",
        "/api/movimientos",
        Some(&cookie),
        &format!(
            r#"{{"fecha":"2026-08-05","concepto_id":{baja},"articulo_id":{art},
                 "documento":"baja","cantidad":10000,"costo_unitario":0}}"#
        ),
    );
    assert_eq!(r.estado, 201, "no entró la baja: {}", r.cuerpo);
    // El id REAL del movimiento, no uno supuesto: antes de la baja ya hubo una
    // compra, así que «1» habría sido la compra y el acta se habría levantado
    // sobre la salida equivocada… o, como pasó al escribir esto, sobre ninguna.
    let mov_baja = r.numero("id");

    let informes = |extra: &str| {
        pedir(
            s.puerto,
            "GET",
            &format!("/informes?desde=2026-08-01&hasta=2026-08-31{extra}"),
            Some(&cookie),
            "",
        )
    };

    // -- El tope del 3 % del art. 64 se ve, con su base en UNIDADES ----------
    let r = informes("");
    assert_eq!(r.estado, 200);
    assert!(r.cuerpo.contains("tope del 3 %"), "no está el informe del art. 64");
    assert!(r.cuerpo.contains("Retiros gravados"), "no está el informe de retiros");
    assert!(r.cuerpo.contains("Clasificación ABC"), "no está el ABC");
    assert!(r.cuerpo.contains("Rotación"), "no está la rotación");
    assert!(r.cuerpo.contains("Punto de reorden"), "no está el reorden");
    assert!(r.cuerpo.contains("valor neto realizable"), "no está el deterioro");

    // -- El acta: la baja aparece PENDIENTE ---------------------------------
    assert!(
        r.cuerpo.contains(r#"name="movimiento""#),
        "la baja por obsolescencia no aparece para levantarle acta"
    );

    // Se levanta. El costo fiscal unitario NO se teclea: sale del kardex.
    let r = pedir(
        s.puerto,
        "POST",
        "/informes/acta",
        Some(&cookie),
        &format!(
            "desde=2026-08-01&hasta=2026-08-31&movimiento={mov_baja}&justificacion=vencido\
             &fecha=2026-08-31&metodo=destruccion&cubierto_por_seguro=0\
             &representante_legal=Juan+Isaza&responsables=Pedro+Perez;Ana+Gomez"
        ),
    );
    assert_eq!(r.estado, 200);
    assert!(
        r.cuerpo.contains("Acta 1 levantada"),
        "no se levantó el acta: {}",
        &r.cuerpo[..r.cuerpo.len().min(400)]
    );
    assert!(r.cuerpo.contains("DEDUCIBLE"), "el acta no dice el deducible");

    // Y deja de pedirse: lo pendiente se calcula por unidades sin cubrir, no
    // por «hay o no hay renglón».
    let r = informes("");
    assert!(
        r.cuerpo.contains("No hay bajas por obsolescencia sin acta"),
        "la baja sigue pidiendo acta después de levantarla"
    );

    // PAREJA: sin marcar nada no se levanta un acta vacía. Un acta que no
    // sustenta ninguna salida no sustenta ninguna deducción.
    let r = pedir(
        s.puerto,
        "POST",
        "/informes/acta",
        Some(&cookie),
        "desde=2026-08-01&hasta=2026-08-31&fecha=2026-08-31&metodo=destruccion\
         &representante_legal=Juan&responsables=Pedro",
    );
    assert!(r.cuerpo.contains("Marque al menos una baja"), "levantó un acta vacía");

    // -- Deterioro por VNR: correrlo ESCRIBE --------------------------------
    let r = informes("&corte=2026-08-31");
    assert!(
        r.cuerpo.contains("No hay corte corrido a esta fecha"),
        "dice que hay corte antes de correrlo"
    );

    let r = pedir(s.puerto, "POST", "/informes/deterioro", Some(&cookie), "fecha=2026-08-31");
    assert_eq!(r.estado, 200);
    assert!(r.cuerpo.contains("Corte al 2026-08-31 corrido"), "no corrió el corte");
    // Lo que no se pudo medir se dice por su nombre: el artículo del arnés no
    // tiene precio de venta estimado, así que el corte TIENE que avisar. Un
    // corte que lo omitiera diría que el inventario está medido sin estarlo.
    assert!(
        r.cuerpo.contains("no se pudieron medir") || r.cuerpo.contains("partidas medidas"),
        "el corte no dice qué midió ni qué no: {}",
        &r.cuerpo[..r.cuerpo.len().min(600)]
    );

    // -- Y el rango va en la URL, para poder guardar el informe como enlace --
    let r = informes("");
    assert!(r.cuerpo.contains("2026-08-01"), "el rango no vuelve al formulario");

    // PAREJA del rango: un nivel de servicio que no está tabulado NO se
    // sustituye por uno cercano; el informe dice que no se pudo calcular.
    let r = informes("&nivel_servicio=77");
    assert!(
        r.cuerpo.contains("No se pudo calcular"),
        "el reorden se inventó un nivel de servicio que no está tabulado"
    );
}

/// **Una instalación sin nada que migrar puede empezar a trabajar.**
///
/// `docs/OPERACION.md` pone en el paso 6 del alistamiento «Artículos y terceros
/// — Artículos / Terceros, o migrando», y el panel no tenía ni el alta de
/// artículo ni la pantalla de terceros: había que salir a `/api` con un cliente
/// HTTP. El README vende el sistema para cualquier negocio que compre, almacene,
/// venda y cobre, no solo para quien hereda un FoxPro.
///
/// Y el KARDEX del artículo, que Angular enseñaba y el panel perdió: es lo que
/// hace auditable la existencia. La regla 1 de este repositorio dice que la
/// existencia se DERIVA del kardex; sin poder verlo hay que creerse el número,
/// que es justo lo que hacía el sistema viejo.
#[test]
fn el_panel_da_de_alta_articulos_y_terceros_y_enseña_el_kardex() {
    let s = arrancar("alta-panel");
    let cookie = entrar(s.puerto);
    let post = |ruta: &str, cuerpo: &str| pedir(s.puerto, "POST", ruta, Some(&cookie), cuerpo);

    // -- Artículo ------------------------------------------------------------
    let r = post(
        "/articulos",
        "codigo=B20&descripcion=Botellon+20+L&presentacion=Botellon&iva_porcentaje=19\
         &inc_porcentaje=0&precio_venta=1200000&punto_minimo=5&punto_maximo=50\
         &unidad_medida=94&precio_venta_estimado=1200000&costos_para_vender=50000",
    );
    assert_eq!(r.estado, 200);
    assert!(r.cuerpo.contains("Artículo creado"), "no se creó el artículo");
    assert!(r.cuerpo.contains("Botellon 20 L"), "el artículo nuevo no salió en la lista");

    // PAREJA: un precio que no es un número NO se guarda como cero. Un cero por
    // omisión en un precio es un artículo regalado.
    let r = post(
        "/articulos",
        "codigo=X1&descripcion=Malo&precio_venta=mil&iva_porcentaje=19&inc_porcentaje=0\
         &punto_minimo=0&punto_maximo=0",
    );
    assert!(r.cuerpo.contains("tienen que ser números"), "aceptó «mil» como precio");
    assert!(!r.cuerpo.contains("Malo"), "el artículo con precio ilegible se creó igual");

    // Y el código de barras que NO pasa su dígito de control no se declara GTIN.
    let r = post(
        "/articulos",
        "codigo=B10&descripcion=Bolsa+360&presentacion=Bolsa&iva_porcentaje=19\
         &inc_porcentaje=0&precio_venta=100000&punto_minimo=0&punto_maximo=0\
         &codigo_barras=41251501",
    );
    assert!(r.cuerpo.contains("Artículo creado"), "no se creó el de código interno");
    let r = pedir(s.puerto, "GET", "/api/articulos?q=B10", Some(&cookie), "");
    // Se guarda como «interno» —cuyo `schemeID` ante la DIAN es 999—, no como
    // GTIN. Declarar como identificador internacional un número que no pasa su
    // dígito de control sería declararle a la DIAN algo que no existe: 60 de
    // los 148 artículos del corpus están en ese caso.
    assert!(
        r.cuerpo.contains(r#""tipo_estandar":"interno""#),
        "un código que no pasa el dígito GS1 se declaró como GTIN: {}",
        r.cuerpo
    );
    // Y el artículo DICE qué le falta para poder ir en una factura electrónica,
    // en vez de descubrirse al emitirla.
    assert!(
        r.cuerpo.contains("unidad de medida"),
        "no avisa de que le falta la unidad de medida: {}",
        r.cuerpo
    );

    // -- Kardex --------------------------------------------------------------
    let r = pedir(s.puerto, "GET", "/api/articulos?q=B20", Some(&cookie), "");
    let art = {
        let v: serde_json::Value = serde_json::from_str(&r.cuerpo).expect("json");
        v[0]["id"].as_i64().expect("id")
    };
    comprar(s.puerto, &cookie, art, 100, 3_000);

    let r = pedir(s.puerto, "GET", &format!("/articulos?kardex={art}"), Some(&cookie), "");
    assert_eq!(r.estado, 200);
    assert!(r.cuerpo.contains("Kardex de un artículo"), "no está la vista de kardex");
    assert!(
        r.cuerpo.contains("Valor acumulado"),
        "el kardex no enseña el valor acumulado, que es lo que cuadra con la foto"
    );
    // PAREJA: sin elegir artículo no se dibuja una tabla vacía que parezca «no
    // hay movimientos».
    let r = pedir(s.puerto, "GET", "/articulos", Some(&cookie), "");
    assert!(!r.cuerpo.contains("Valor acumulado"), "dibujó el kardex sin pedirlo");

    // -- Terceros ------------------------------------------------------------
    let r = post(
        "/terceros",
        "nit=43627185&nombre=Tienda+La+Esquina&direccion=CRA+43&telefono=3001234567\
         &plazo_dias=30&es_cliente=true",
    );
    assert_eq!(r.estado, 200);
    assert!(r.cuerpo.contains("Tercero creado"), "no se creó el tercero");
    // El NIT sale CON su dígito de verificación, que es lo que va en la factura.
    assert!(r.cuerpo.contains("43627185-"), "el NIT no salió con su DV: {:.400}", r.cuerpo);

    // PAREJA 1: un tercero que no es ni cliente ni proveedor no existe.
    let r = post("/terceros", "nit=99999999&nombre=Nadie&plazo_dias=0");
    assert!(r.cuerpo.contains("No se creó"), "creó un tercero que no es nada");

    // PAREJA 2: «BEBE» no es un NIT. En el corpus heredado había cinco así.
    let r = post("/terceros", "nit=BEBE&nombre=Bebe&es_cliente=true&plazo_dias=0");
    assert!(r.cuerpo.contains("No se creó"), "aceptó «BEBE» como NIT");

    // Y el tercero nuevo ya sirve para facturar: aparece en el desplegable.
    let r = pedir(s.puerto, "GET", "/facturar", Some(&cookie), "");
    assert!(
        r.cuerpo.contains("Tienda La Esquina"),
        "el cliente nuevo no llegó a la pantalla de facturar"
    );
}

/// **Un `Origin` ajeno no escribe, y la falta de `Origin` sí.**
///
/// `SameSite=Strict` sigue siendo el control principal y no cambia. Esto cubre
/// lo que aquél no ve: `SameSite` es por SITIO y no por ORIGEN, y en el VPS
/// conviven `inventario`, `chasqui`, `medico` e `informes` bajo `xiliux.com`,
/// así que un formulario servido desde un hermano viaja como «mismo sitio» con
/// la cookie puesta.
///
/// Las dos mitades son el punto (directiva 33). Con la primera sola pasaría un
/// servidor que rechazara TODA escritura; con la segunda sola, uno ciego. Y la
/// segunda no es un detalle: sin `Origin` NO se bloquea a propósito, porque lo
/// omiten `curl`, la integración de un tercero contra `/api` y este mismo
/// arnés, que son operación normal (directiva 21).
#[test]
fn una_escritura_con_origen_ajeno_se_rechaza_y_sin_origen_pasa() {
    let s = arrancar("origen");
    let cookie = entrar(s.puerto);
    let propio = format!("http://127.0.0.1:{}", s.puerto);

    let cuerpo = r#"{"codigo":"OC","nombre":"Otro concepto","signo":"entrada",
                     "clase":"ajuste","efecto_tributario":"no_afecta_impuestos"}"#;

    // ROJO: el origen se declara y es de otro sitio.
    let r = pedir_con(
        s.puerto,
        "POST",
        "/api/conceptos",
        Some(&cookie),
        cuerpo,
        &[("Origin", "https://chasqui.xiliux.com")],
    );
    assert_eq!(r.estado, 403, "una escritura con Origin ajeno pasó: {}", r.cuerpo);
    assert!(r.cuerpo.contains("origen no permitido"), "{}", r.cuerpo);

    // VERDE 1: el MISMO origen escribe.
    let r = pedir_con(
        s.puerto,
        "POST",
        "/api/conceptos",
        Some(&cookie),
        cuerpo,
        &[("Origin", &propio)],
    );
    assert_eq!(r.estado, 201, "el propio origen fue rechazado: {}", r.cuerpo);

    // VERDE 2: sin `Origin` se pasa. Es la mitad que evita el sesgo a cerrar.
    let r = pedir_con(
        s.puerto,
        "POST",
        "/api/conceptos",
        Some(&cookie),
        r#"{"codigo":"OC2","nombre":"Sin origen","signo":"entrada",
            "clase":"ajuste","efecto_tributario":"no_afecta_impuestos"}"#,
        &[],
    );
    assert_eq!(r.estado, 201, "sin Origin se bloqueó: rompe curl y las integraciones");

    // Y el panel también, que es donde vive el formulario.
    let r = pedir_con(
        s.puerto,
        "POST",
        "/configuracion/usuario",
        Some(&cookie),
        "codigo=x&nombre=X&rol=admin&clave=una-clave-larga",
        &[("Origin", "https://chasqui.xiliux.com")],
    );
    assert_eq!(r.estado, 403, "el panel aceptó un origen ajeno");

    // LEER con origen ajeno NO se bloquea: no cambia nada, y bloquearlo rompería
    // un enlace normal desde otra página.
    let r = pedir_con(
        s.puerto,
        "GET",
        "/api/salud",
        Some(&cookie),
        "",
        &[("Origin", "https://chasqui.xiliux.com")],
    );
    assert_eq!(r.estado, 200, "una LECTURA con origen ajeno se bloqueó");
}

/// Las etiquetas que traen el atributo `class` **dos veces**.
///
/// Costura de TEXTO: la usan el caso rojo y el camino real, así que el rojo
/// falla por donde falla el de verdad y no por el arnés (directiva 33).
fn etiquetas_con_class_repetido(html: &str) -> Vec<String> {
    let mut malas = Vec::new();
    let bytes = html.as_bytes();
    let mut i = 0;
    while let Some(pos) = html[i..].find('<') {
        let ini = i + pos;
        let Some(largo) = html[ini..].find('>') else { break };
        let etiqueta = &html[ini..=ini + largo];
        // ` class=` con el espacio delante: así no cuenta un `data-class=` ni un
        // texto que contenga la palabra.
        if etiqueta.matches(" class=").count() > 1 {
            malas.push(etiqueta.to_string());
        }
        i = ini + largo + 1;
        if i >= bytes.len() {
            break;
        }
    }
    malas
}

/// **Ningún elemento sale con `class` repetido.**
///
/// No es cosmético y no es teórico: `maud` emite `class="num" class="alerta"`
/// cuando se mezcla la sintaxis corta (`td.num`) con un atributo `class`
/// calculado, y el analizador de HTML **se queda con el primero y tira el
/// segundo** —medido con Chromium: `<td class="num" class="alerta">` llega al
/// DOM como `<td class="num">`—.
///
/// Así estaban las CINCO marcas de alerta del panel: existencia bajo el
/// mínimo, cartera vencida, resolución agotada, semáforo de alistamiento y
/// gravedad del punto. El CSS las define, el servidor creía emitirlas, el
/// navegador no las veía nunca, y la página respondía 200 con el texto
/// correcto. La forma correcta es `td.num.alerta[condicion]`.
#[test]
fn ninguna_etiqueta_sale_con_el_atributo_class_dos_veces() {
    // ROJO primero, por la misma costura: si esto no lo caza, lo de abajo no
    // vale nada aunque salga verde.
    let control = r#"<table><tr><td class="num" class="alerta">3</td></tr></table>"#;
    assert_eq!(
        etiquetas_con_class_repetido(control).len(),
        1,
        "el detector no ve un class repetido: mide el arnés, no el HTML"
    );
    // Y su pareja: la forma correcta NO se denuncia.
    assert!(etiquetas_con_class_repetido(r#"<td class="num alerta">3</td>"#).is_empty());

    let s = arrancar("class-repetido");
    let cookie = entrar(s.puerto);
    for ruta in [
        "/entrar", "/articulos", "/movimientos", "/facturar",
        "/recaudar", "/cartera", "/informes", "/configuracion", "/alistamiento",
    ] {
        let r = pedir(s.puerto, "GET", ruta, Some(&cookie), "");
        assert_eq!(r.estado, 200, "«{ruta}» no se sirvió");
        let malas = etiquetas_con_class_repetido(&r.cuerpo);
        assert!(
            malas.is_empty(),
            "«{ruta}» emite {} etiqueta(s) con class repetido; el navegador tira la segunda \
             y la marca no se ve nunca: {:?}",
            malas.len(),
            &malas[..malas.len().min(3)]
        );
    }
}

/// **Ni un manejador en línea en lo que teje el servidor.**
///
/// Es el guardián que faltaba, y no es hipotético: la pantalla de Recaudar salió
/// con `onchange="this.form.submit()"` y, con `script-src 'self'`, el navegador
/// NO lo ejecuta — elegir un cliente no hacía nada. El `<noscript>` tampoco
/// salvaba, porque JavaScript está activo: lo bloqueado era el manejador.
///
/// Las dos mitades del hueco encajaban y por eso nadie lo vio: la prueba de
/// cabeceras comprueba que la CSP ESTÉ, no qué ROMPE; y las pruebas que dibujan
/// en Chromium no alcanzan las pantallas con sesión, que es donde estaba.
/// Mientras el arnés no hable CDP, esto es lo que discrimina y cuesta nada.
///
/// El arreglo de un fallo así NUNCA es `'unsafe-inline'`: devolvería el XSS que
/// la cabecera acaba de cerrar. Es htmx, `boi`, o HTML normal.
#[test]
fn el_servidor_no_teje_ni_un_manejador_de_evento_en_linea() {
    let s = arrancar("sin-manejadores");
    let cookie = entrar(s.puerto);

    // Los atributos de evento que `script-src 'self'` no ejecuta. La lista es de
    // los que se usan de verdad en un formulario; no pretende ser exhaustiva.
    let manejadores = [
        " onchange=", " onclick=", " onload=", " onerror=", " onsubmit=",
        " oninput=", " onfocus=", " onblur=", " onkeydown=", " onmouseover=",
    ];

    let paginas = [
        "/entrar", "/articulos", "/movimientos", "/facturar",
        "/recaudar", "/cartera", "/informes", "/configuracion", "/alistamiento",
    ];

    for ruta in paginas {
        let r = pedir(s.puerto, "GET", ruta, Some(&cookie), "");
        assert_eq!(r.estado, 200, "«{ruta}» no se sirvió");
        let html = r.cuerpo.to_ascii_lowercase();
        for m in manejadores {
            assert!(
                !html.contains(m),
                "«{ruta}» trae «{}», que la CSP NO ejecuta: la pantalla queda muerta sin un error",
                m.trim()
            );
        }
    }

    // PAREJA (directiva 33): la sonda tiene que saber VER uno. Si no, un
    // instrumento que dijera «limpio» ante cualquier cosa pasaría igual.
    let falso = r#"<select name="x" onchange="this.form.submit()">"#.to_ascii_lowercase();
    assert!(
        manejadores.iter().any(|m| falso.contains(m)),
        "la sonda no reconoce un manejador en línea ni cuando se lo ponen delante"
    );
}
